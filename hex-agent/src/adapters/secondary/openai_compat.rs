use crate::ports::anthropic::{AnthropicError, AnthropicPort, AnthropicResponse, StreamChunk};
use crate::ports::{ApiRequestOptions, ContentBlock, Message, Role, StopReason, TokenUsage, ToolDefinition};
use async_trait::async_trait;
use futures::stream::{self, Stream};
use reqwest::Client;
use serde::{Deserialize, Serialize};

/// Adapter for any OpenAI-compatible chat completions API.
///
/// Works with: MiniMax, Together AI, Groq, OpenRouter, Ollama, vLLM.
/// Translates between hex-agent's Anthropic-native types and OpenAI's format.
pub struct OpenAiCompatAdapter {
    client: Client,
    api_key: String,
    base_url: String,
    model: String,
}

impl OpenAiCompatAdapter {
    pub fn new(api_key: String, base_url: String, model: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
            base_url: base_url.trim_end_matches('/').to_string(),
            model,
        }
    }

    /// Convenience constructor for MiniMax M2.7 (Coding Plan Max).
    pub fn minimax(api_key: String) -> Self {
        Self::new(
            api_key,
            "https://api.minimax.io/v1".to_string(),
            "MiniMax-M2.7".to_string(),
        )
    }

    /// Convenience constructor for MiniMax M1 (fallback).
    pub fn minimax_fast(api_key: String) -> Self {
        Self::new(
            api_key,
            "https://api.minimax.io/v1".to_string(),
            "MiniMax-M1".to_string(),
        )
    }

    /// Convenience constructor for Ollama (local or remote).
    ///
    /// Ollama doesn't require an API key and serves on port 11434 by default.
    /// For remote hosts (e.g., Bazzite gaming rig with GPU), pass the host:
    ///
    /// ```rust,ignore
    /// // Local Ollama
    /// OpenAiCompatAdapter::ollama("qwen3:32b", None);
    ///
    /// // Remote Ollama on Bazzite
    /// OpenAiCompatAdapter::ollama("qwen3:32b", Some("http://bazzite.local:11434"));
    /// ```
    pub fn ollama(model: &str, host: Option<&str>) -> Self {
        let base_url = host
            .unwrap_or("http://127.0.0.1:11434")
            .trim_end_matches('/')
            .to_string();
        Self::new(
            String::new(), // Ollama doesn't need an API key
            format!("{}/v1", base_url),
            model.to_string(),
        )
    }

    /// Convenience constructor for vLLM (self-hosted GPU inference).
    pub fn vllm(model: &str, host: Option<&str>, api_key: Option<&str>) -> Self {
        let base_url = host
            .unwrap_or("http://127.0.0.1:8000")
            .trim_end_matches('/')
            .to_string();
        Self::new(
            api_key.unwrap_or("").to_string(),
            format!("{}/v1", base_url),
            model.to_string(),
        )
    }

    /// Convenience constructor for OpenRouter (300+ models via single API key).
    ///
    /// OpenRouter uses the standard OpenAI chat completions format but requires
    /// additional headers for analytics and supports provider preference routing.
    pub fn openrouter(api_key: String, model: String) -> Self {
        Self::new(
            api_key,
            "https://openrouter.ai/api/v1".to_string(),
            model,
        )
    }

    /// Returns true if this adapter is pointing at OpenRouter.
    fn is_openrouter(&self) -> bool {
        self.base_url.contains("openrouter.ai")
    }

    /// Create from environment variables for self-hosted models.
    ///
    /// Reads:
    /// - `HEX_OLLAMA_HOST` — Ollama URL (default: http://127.0.0.1:11434)
    /// - `HEX_OLLAMA_MODEL` — Model name (default: qwen3:32b)
    /// - `HEX_VLLM_HOST` — vLLM URL
    /// - `HEX_VLLM_MODEL` — vLLM model name
    /// - `HEX_INFERENCE_URL` — Generic OpenAI-compatible endpoint
    /// - `HEX_INFERENCE_MODEL` — Generic model name
    /// - `HEX_INFERENCE_KEY` — API key for generic endpoint
    pub fn from_env_self_hosted() -> Option<Self> {
        // Try Ollama first
        if let Ok(model) = std::env::var("HEX_OLLAMA_MODEL") {
            let host = std::env::var("HEX_OLLAMA_HOST").ok();
            return Some(Self::ollama(&model, host.as_deref()));
        }

        // Try vLLM
        if let Ok(model) = std::env::var("HEX_VLLM_MODEL") {
            let host = std::env::var("HEX_VLLM_HOST").ok();
            let key = std::env::var("HEX_VLLM_KEY").ok();
            return Some(Self::vllm(&model, host.as_deref(), key.as_deref()));
        }

        // Try generic OpenAI-compatible
        if let Ok(url) = std::env::var("HEX_INFERENCE_URL") {
            let model = std::env::var("HEX_INFERENCE_MODEL").unwrap_or("default".to_string());
            let key = std::env::var("HEX_INFERENCE_KEY").unwrap_or_default();
            return Some(Self::new(key, url, model));
        }

        None
    }

    /// Convert hex-agent messages to OpenAI chat format.
    fn to_openai_messages(system: &str, messages: &[Message]) -> Vec<OaiMessage> {
        let mut out = vec![OaiMessage {
            role: "system".to_string(),
            content: Some(system.to_string()),
            tool_calls: None,
            tool_call_id: None,
        }];

        for msg in messages {
            match msg.role {
                Role::User => {
                    // Collect text content and tool results
                    let mut text_parts = Vec::new();
                    for block in &msg.content {
                        match block {
                            ContentBlock::Text { text } => text_parts.push(text.clone()),
                            ContentBlock::ToolResult {
                                tool_use_id,
                                content,
                                ..
                            } => {
                                out.push(OaiMessage {
                                    role: "tool".to_string(),
                                    content: Some(content.clone()),
                                    tool_calls: None,
                                    tool_call_id: Some(tool_use_id.clone()),
                                });
                            }
                            _ => {}
                        }
                    }
                    if !text_parts.is_empty() {
                        out.push(OaiMessage {
                            role: "user".to_string(),
                            content: Some(text_parts.join("\n")),
                            tool_calls: None,
                            tool_call_id: None,
                        });
                    }
                }
                Role::Assistant => {
                    let mut text_parts = Vec::new();
                    let mut tool_calls = Vec::new();

                    for block in &msg.content {
                        match block {
                            ContentBlock::Text { text } => text_parts.push(text.clone()),
                            ContentBlock::ToolUse { id, name, input } => {
                                tool_calls.push(OaiToolCall {
                                    id: id.clone(),
                                    r#type: "function".to_string(),
                                    function: OaiFunction {
                                        name: name.clone(),
                                        arguments: serde_json::to_string(input)
                                            .unwrap_or_default(),
                                    },
                                });
                            }
                            _ => {}
                        }
                    }

                    out.push(OaiMessage {
                        role: "assistant".to_string(),
                        content: if text_parts.is_empty() {
                            None
                        } else {
                            Some(text_parts.join("\n"))
                        },
                        tool_calls: if tool_calls.is_empty() {
                            None
                        } else {
                            Some(tool_calls)
                        },
                        tool_call_id: None,
                    });
                }
            }
        }

        out
    }

    /// Convert hex-agent tool definitions to OpenAI function format.
    fn to_openai_tools(tools: &[ToolDefinition]) -> Vec<OaiToolDef> {
        tools
            .iter()
            .map(|t| OaiToolDef {
                r#type: "function".to_string(),
                function: OaiToolFunction {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    parameters: t.input_schema.properties.clone(),
                },
            })
            .collect()
    }

    /// Strip `<think>` tags from content (MiniMax thinking output).
    /// Returns (cleaned_text, thinking_text).
    fn strip_thinking(text: &str) -> (String, Option<String>) {
        if let Some(start) = text.find("<think>") {
            if let Some(end) = text.find("</think>") {
                let thinking = text[start + 7..end].trim().to_string();
                let cleaned = format!(
                    "{}{}",
                    text[..start].trim(),
                    text[end + 8..].trim()
                );
                return (cleaned.trim().to_string(), Some(thinking));
            }
        }
        (text.to_string(), None)
    }
}

#[async_trait]
impl AnthropicPort for OpenAiCompatAdapter {
    async fn send_message(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[ToolDefinition],
        max_tokens: u32,
        model_override: Option<&str>,
        _options: Option<&ApiRequestOptions>,
    ) -> Result<AnthropicResponse, AnthropicError> {
        let model = model_override.unwrap_or(&self.model);
        let oai_messages = Self::to_openai_messages(system, messages);

        let mut body = serde_json::json!({
            "model": model,
            "messages": oai_messages,
            "max_tokens": max_tokens,
        });

        if !tools.is_empty() {
            body["tools"] = serde_json::to_value(Self::to_openai_tools(tools))
                .unwrap_or_default();
        }

        // OpenRouter routing preferences (ADR-2603231600)
        if self.is_openrouter() {
            body["provider"] = serde_json::json!({
                "order": ["Together", "Lambda", "Fireworks"],
                "allow_fallbacks": true
            });
            body["route"] = serde_json::json!("fallback");
        }

        let mut request = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json");

        // OpenRouter-specific headers (ADR-2603231600)
        if self.is_openrouter() {
            request = request
                .header("HTTP-Referer", "https://github.com/hex-intf")
                .header("X-Title", "hex-agent");
        }

        let response = request
            .json(&body)
            .send()
            .await
            .map_err(|e| AnthropicError::Http(e.to_string()))?;

        let status = response.status().as_u16();
        if status == 429 {
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(5);
            return Err(AnthropicError::RateLimited {
                retry_after_ms: retry_after * 1000,
            });
        }
        if status == 402 {
            return Err(AnthropicError::Api {
                status,
                message: "OpenRouter: insufficient credits. Top up at https://openrouter.ai/credits".to_string(),
            });
        }
        if status >= 400 {
            let text = response
                .text()
                .await
                .unwrap_or_else(|_| "unknown error".into());
            return Err(AnthropicError::Api {
                status,
                message: text,
            });
        }

        let oai_resp: OaiChatResponse = response
            .json()
            .await
            .map_err(|e| AnthropicError::Deserialize(e.to_string()))?;

        // Convert to hex-agent types
        let choice = oai_resp
            .choices
            .first()
            .ok_or_else(|| AnthropicError::Deserialize("no choices in response".into()))?;

        let mut content = Vec::new();

        // Text content — strip thinking tags
        if let Some(ref text) = choice.message.content {
            let (cleaned, thinking) = Self::strip_thinking(text);
            if let Some(think) = thinking {
                tracing::debug!(thinking_len = think.len(), "Stripped thinking content");
            }
            if !cleaned.is_empty() {
                content.push(ContentBlock::Text { text: cleaned });
            }
        }

        // Tool calls
        if let Some(ref tool_calls) = choice.message.tool_calls {
            for tc in tool_calls {
                let input: serde_json::Value =
                    serde_json::from_str(&tc.function.arguments).unwrap_or_default();
                content.push(ContentBlock::ToolUse {
                    id: tc.id.clone(),
                    name: tc.function.name.clone(),
                    input,
                });
            }
        }

        let stop_reason = match choice.finish_reason.as_deref() {
            Some("stop") => StopReason::EndTurn,
            Some("tool_calls") => StopReason::ToolUse,
            Some("length") => StopReason::MaxTokens,
            _ => StopReason::EndTurn,
        };

        let usage = TokenUsage {
            input_tokens: oai_resp.usage.prompt_tokens,
            output_tokens: oai_resp.usage.completion_tokens,
            ..Default::default()
        };

        // Log OpenRouter actual cost if present
        if let Some(cost) = oai_resp.usage.cost {
            tracing::info!(openrouter_cost_usd = cost, model = %oai_resp.model, "OpenRouter actual cost");
        }

        Ok(AnthropicResponse {
            content,
            stop_reason,
            usage,
            model: oai_resp.model,
        })
    }

    async fn stream_message(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[ToolDefinition],
        max_tokens: u32,
        model_override: Option<&str>,
        options: Option<&ApiRequestOptions>,
    ) -> Result<
        Box<dyn Stream<Item = Result<StreamChunk, AnthropicError>> + Send + Unpin>,
        AnthropicError,
    > {
        // For now, use non-streaming and emit as a single chunk.
        // Full SSE streaming can be added later.
        let resp = self
            .send_message(system, messages, tools, max_tokens, model_override, options)
            .await?;

        let mut chunks = Vec::new();
        for block in &resp.content {
            match block {
                ContentBlock::Text { text } => {
                    chunks.push(Ok(StreamChunk::TextDelta(text.clone())));
                }
                ContentBlock::ToolUse { id, name, .. } => {
                    chunks.push(Ok(StreamChunk::ToolUseStart {
                        id: id.clone(),
                        name: name.clone(),
                    }));
                }
                _ => {}
            }
        }
        chunks.push(Ok(StreamChunk::MessageStop {
            stop_reason: resp.stop_reason,
            usage: resp.usage,
        }));

        Ok(Box::new(stream::iter(chunks)))
    }
}

// --- OpenAI API types (private, for serialization only) ---

#[derive(Debug, Serialize)]
struct OaiMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OaiToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OaiToolCall {
    id: String,
    r#type: String,
    function: OaiFunction,
}

#[derive(Debug, Serialize, Deserialize)]
struct OaiFunction {
    name: String,
    arguments: String,
}

#[derive(Debug, Serialize)]
struct OaiToolDef {
    r#type: String,
    function: OaiToolFunction,
}

#[derive(Debug, Serialize)]
struct OaiToolFunction {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct OaiChatResponse {
    model: String,
    choices: Vec<OaiChoice>,
    usage: OaiUsage,
}

#[derive(Debug, Deserialize)]
struct OaiChoice {
    message: OaiChoiceMessage,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OaiChoiceMessage {
    content: Option<String>,
    tool_calls: Option<Vec<OaiToolCall>>,
}

#[derive(Debug, Deserialize)]
struct OaiUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
    /// Actual cost in USD (OpenRouter-specific, absent for other providers)
    #[serde(default)]
    cost: Option<f64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_thinking_with_tags() {
        let input = "<think>Let me reason about this...</think>The answer is 42.";
        let (cleaned, thinking) = OpenAiCompatAdapter::strip_thinking(input);
        assert_eq!(cleaned, "The answer is 42.");
        assert_eq!(thinking.unwrap(), "Let me reason about this...");
    }

    #[test]
    fn strip_thinking_no_tags() {
        let input = "Just a plain response.";
        let (cleaned, thinking) = OpenAiCompatAdapter::strip_thinking(input);
        assert_eq!(cleaned, "Just a plain response.");
        assert!(thinking.is_none());
    }

    #[test]
    fn to_openai_messages_basic() {
        let messages = vec![
            Message {
                role: Role::User,
                content: vec![ContentBlock::Text {
                    text: "Hello".to_string(),
                }],
            },
            Message {
                role: Role::Assistant,
                content: vec![ContentBlock::Text {
                    text: "Hi there".to_string(),
                }],
            },
        ];

        let oai = OpenAiCompatAdapter::to_openai_messages("You are helpful.", &messages);
        assert_eq!(oai.len(), 3); // system + user + assistant
        assert_eq!(oai[0].role, "system");
        assert_eq!(oai[1].role, "user");
        assert_eq!(oai[2].role, "assistant");
    }

    #[test]
    fn openrouter_constructor_sets_correct_url() {
        let adapter = OpenAiCompatAdapter::openrouter("sk-or-test".to_string(), "meta-llama/llama-4-maverick".to_string());
        assert!(adapter.is_openrouter());
        assert_eq!(adapter.base_url, "https://openrouter.ai/api/v1");
        assert_eq!(adapter.model, "meta-llama/llama-4-maverick");
    }

    #[test]
    fn non_openrouter_detected_correctly() {
        let adapter = OpenAiCompatAdapter::minimax("key".to_string());
        assert!(!adapter.is_openrouter());
    }

    #[test]
    fn to_openai_tools_conversion() {
        let tools = vec![ToolDefinition {
            name: "read_file".to_string(),
            description: "Read a file".to_string(),
            input_schema: crate::domain::ToolInputSchema {
                schema_type: "object".to_string(),
                properties: serde_json::json!({
                    "path": {"type": "string"}
                }),
                required: vec!["path".to_string()],
            },
        }];

        let oai_tools = OpenAiCompatAdapter::to_openai_tools(&tools);
        assert_eq!(oai_tools.len(), 1);
        assert_eq!(oai_tools[0].function.name, "read_file");
    }
}
