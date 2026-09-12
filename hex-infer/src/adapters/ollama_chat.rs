//! Ollama's `/api/chat` — the path that can carry tools and a transcript.
//!
//! The adapter beside this one posts to `/api/generate` with `collapse_prompt`, which walks the
//! messages backwards and returns the LAST USER MESSAGE ONLY. That is fine for a single-shot
//! completion and it is what `hex do --fast` uses. It cannot work for the ReAct loop:
//!
//!   * `/api/generate` has no `tools` parameter, so the curated tool schema was assembled,
//!     deserialized, handed to the adapter and dropped — the word "tools" appears nowhere in it.
//!   * the transcript is discarded, so the model never sees what it already tried.
//!
//! A model given no tools does not report an error. It invents a convention — devstral-small-2:24b
//! emitted ```json {"tool": "repo_read", …}``` as prose — which `extract_tool_uses` cannot see, so
//! the loop reported "ended with no edit" and the model took the blame. The multi-step loop has
//! therefore never worked against a local model; only the `claude_code` subprocess adapter, which
//! handles tools itself, made it look like it did.

use hex_core::domain::messages::{ContentBlock, Role, StopReason};
use hex_core::domain::tools::ToolDefinition;
use hex_core::ports::inference::{InferenceError, InferenceRequest, InferenceResponse};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<ChatMessage>,
    stream: bool,
    /// Reasoning models otherwise spend their budget in a `thinking` field and return empty content.
    think: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<ChatTool>,
    options: ChatOptions,
}

#[derive(Debug, Serialize)]
struct ChatOptions {
    temperature: f32,
    num_predict: u32,
}

#[derive(Debug, Serialize, Deserialize)]
struct ChatMessage {
    role: String,
    content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tool_calls: Vec<ChatToolCall>,
}

#[derive(Debug, Serialize)]
struct ChatTool {
    #[serde(rename = "type")]
    kind: &'static str,
    function: ChatFunction,
}

#[derive(Debug, Serialize)]
struct ChatFunction {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize)]
struct ChatToolCall {
    function: ChatToolFunction,
}

#[derive(Debug, Serialize, Deserialize)]
struct ChatToolFunction {
    name: String,
    /// Ollama sends an object; some builds send a JSON string. Accept both rather than lose the call.
    arguments: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    #[serde(default)]
    message: Option<ChatMessage>,
    #[serde(default)]
    prompt_eval_count: u64,
    #[serde(default)]
    eval_count: u64,
    #[serde(default)]
    done_reason: Option<String>,
}

/// OpenAI/Ollama tool shape from hex's `ToolDefinition`.
fn to_chat_tools(tools: &[ToolDefinition]) -> Vec<ChatTool> {
    tools
        .iter()
        .map(|t| ChatTool {
            kind: "function",
            function: ChatFunction {
                name: t.name.clone(),
                description: t.description.clone(),
                parameters: serde_json::json!({
                    "type": t.input_schema.schema_type,
                    "properties": t.input_schema.properties,
                    "required": t.input_schema.required,
                }),
            },
        })
        .collect()
}

/// The whole transcript, not just the last turn.
fn to_chat_messages(request: &InferenceRequest) -> Vec<ChatMessage> {
    let mut out = Vec::new();
    if !request.system_prompt.is_empty() {
        out.push(ChatMessage {
            role: "system".into(),
            content: request.system_prompt.clone(),
            tool_calls: Vec::new(),
        });
    }
    for m in &request.messages {
        // Tool results carry no role of their own here; their text is what the model needs to see.
        let text: String = m
            .content
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.clone()),
                ContentBlock::ToolResult { content, .. } => Some(content.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        if text.is_empty() {
            continue;
        }
        out.push(ChatMessage {
            role: match m.role {
                Role::Assistant => "assistant".into(),
                Role::User => "user".into(),
            },
            content: text,
            tool_calls: Vec::new(),
        });
    }
    out
}

/// One `/api/chat` round trip, with tools.
pub async fn chat(
    base_url: &str,
    timeout: std::time::Duration,
    request: InferenceRequest,
) -> Result<InferenceResponse, InferenceError> {
    let start = std::time::Instant::now();
    let url = format!("{}/api/chat", base_url.trim_end_matches('/'));

    let body = ChatRequest {
        model: &request.model,
        messages: to_chat_messages(&request),
        stream: false,
        think: false,
        tools: to_chat_tools(&request.tools),
        options: ChatOptions {
            temperature: request.temperature,
            num_predict: request.max_tokens,
        },
    };

    let http = reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|e| InferenceError::Network(e.to_string()))?;

    let res = http
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| InferenceError::Network(e.to_string()))?;

    let status = res.status();
    if !status.is_success() {
        let text = res.text().await.unwrap_or_default();
        return Err(InferenceError::ApiError { status: status.as_u16(), body: text });
    }

    let parsed: ChatResponse = res
        .json()
        .await
        .map_err(|e| InferenceError::ProviderUnavailable(format!("ollama /api/chat: {e}")))?;

    let msg = parsed.message.unwrap_or(ChatMessage {
        role: "assistant".into(),
        content: String::new(),
        tool_calls: Vec::new(),
    });

    let mut content: Vec<ContentBlock> = Vec::new();
    if !msg.content.trim().is_empty() {
        content.push(ContentBlock::Text { text: msg.content.clone() });
    }
    for (i, call) in msg.tool_calls.iter().enumerate() {
        // Arguments arrive as an object, or as a JSON string on some builds. Parse the string form
        // rather than passing it through — a tool receiving a quoted blob instead of its arguments
        // fails in a way that looks like the model got the call wrong.
        let input = match &call.function.arguments {
            serde_json::Value::String(s) => {
                serde_json::from_str(s).unwrap_or(serde_json::Value::String(s.clone()))
            }
            other => other.clone(),
        };
        content.push(ContentBlock::ToolUse {
            id: format!("call_{i}"),
            name: call.function.name.clone(),
            input,
        });
    }

    let stop_reason = if msg.tool_calls.is_empty() {
        match parsed.done_reason.as_deref() {
            Some("length") => StopReason::MaxTokens,
            _ => StopReason::EndTurn,
        }
    } else {
        StopReason::ToolUse
    };

    Ok(InferenceResponse {
        content,
        model_used: request.model.clone(),
        stop_reason,
        input_tokens: parsed.prompt_eval_count,
        output_tokens: parsed.eval_count,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
        latency_ms: start.elapsed().as_millis() as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_core::domain::messages::Message;
    use hex_core::domain::tools::ToolInputSchema;

    fn req() -> InferenceRequest {
        InferenceRequest {
            model: "m".into(),
            system_prompt: "sys".into(),
            messages: vec![
                Message { role: Role::User, content: vec![ContentBlock::Text { text: "first".into() }] },
                Message { role: Role::Assistant, content: vec![ContentBlock::Text { text: "second".into() }] },
                Message { role: Role::User, content: vec![ContentBlock::Text { text: "third".into() }] },
            ],
            tools: vec![ToolDefinition {
                name: "repo_read".into(),
                description: "Read a file".into(),
                input_schema: ToolInputSchema {
                    schema_type: "object".into(),
                    properties: serde_json::json!({ "path": { "type": "string" } }),
                    required: vec!["path".into()],
                },
            }],
            max_tokens: 512,
            temperature: 0.0,
            thinking_budget: None,
            cache_control: false,
            priority: Default::default(),
            grammar: None,
        }
    }

    #[test]
    fn the_whole_transcript_is_sent_not_just_the_last_turn() {
        // collapse_prompt (the /api/generate path) returns ONLY the last user message, so a
        // multi-step loop re-asks the same question forever, blind to what it already tried.
        let msgs = to_chat_messages(&req());
        assert_eq!(msgs.len(), 4, "system + three turns");
        assert_eq!(msgs[0].role, "system");
        assert_eq!(msgs[1].content, "first");
        assert_eq!(msgs[3].content, "third");
    }

    #[test]
    fn tools_reach_the_wire_in_ollamas_shape() {
        let tools = to_chat_tools(&req().tools);
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].kind, "function");
        assert_eq!(tools[0].function.name, "repo_read");
        assert_eq!(tools[0].function.parameters["type"], "object");
        assert_eq!(tools[0].function.parameters["required"][0], "path");
    }

    #[test]
    fn a_request_with_no_tools_omits_the_field_entirely() {
        // `skip_serializing_if` matters: some Ollama builds reject an empty tools array outright.
        let mut r = req();
        r.tools.clear();
        let body = ChatRequest {
            model: &r.model, messages: to_chat_messages(&r), stream: false, think: false,
            tools: to_chat_tools(&r.tools),
            options: ChatOptions { temperature: 0.0, num_predict: 1 },
        };
        let json = serde_json::to_value(&body).unwrap();
        assert!(json.get("tools").is_none(), "an empty tool list must not be sent");
    }
}
