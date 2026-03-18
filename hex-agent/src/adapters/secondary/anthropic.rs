use crate::ports::{ContentBlock, Message, StopReason, TokenUsage, ToolDefinition};
use crate::ports::anthropic::{AnthropicError, AnthropicPort, AnthropicResponse, StreamChunk};
use async_trait::async_trait;
use futures::stream::{self, Stream};
use reqwest::Client;
use serde::Deserialize;

/// Adapter for the Anthropic Messages API.
///
/// Uses raw reqwest + SSE parsing — zero SDK dependency.
/// Supports both synchronous (collect full response) and streaming modes.
pub struct AnthropicAdapter {
    client: Client,
    api_key: String,
    base_url: String,
    model: String,
}

impl AnthropicAdapter {
    pub fn new(api_key: String, model: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
            base_url: "https://api.anthropic.com".into(),
            model,
        }
    }

    pub fn with_base_url(mut self, url: String) -> Self {
        self.base_url = url;
        self
    }

    fn build_request_body(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[ToolDefinition],
        max_tokens: u32,
        stream: bool,
    ) -> serde_json::Value {
        let mut body = serde_json::json!({
            "model": self.model,
            "max_tokens": max_tokens,
            "system": system,
            "messages": messages,
            "stream": stream,
        });

        if !tools.is_empty() {
            body["tools"] = serde_json::to_value(tools).unwrap_or_default();
        }

        body
    }
}

#[async_trait]
impl AnthropicPort for AnthropicAdapter {
    async fn send_message(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[ToolDefinition],
        max_tokens: u32,
    ) -> Result<AnthropicResponse, AnthropicError> {
        let body = self.build_request_body(system, messages, tools, max_tokens, false);

        let response = self
            .client
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
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
                .unwrap_or(1000);
            return Err(AnthropicError::RateLimited {
                retry_after_ms: retry_after * 1000,
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

        let api_resp: ApiResponse = response
            .json()
            .await
            .map_err(|e| AnthropicError::Deserialize(e.to_string()))?;

        let content = api_resp
            .content
            .into_iter()
            .map(|block| match block {
                ApiContentBlock::Text { text } => ContentBlock::Text { text },
                ApiContentBlock::ToolUse { id, name, input } => {
                    ContentBlock::ToolUse { id, name, input }
                }
            })
            .collect();

        let stop_reason = match api_resp.stop_reason.as_deref() {
            Some("end_turn") => StopReason::EndTurn,
            Some("tool_use") => StopReason::ToolUse,
            Some("max_tokens") => StopReason::MaxTokens,
            Some("stop_sequence") => StopReason::StopSequence,
            _ => StopReason::EndTurn,
        };

        let usage = TokenUsage {
            input_tokens: api_resp.usage.input_tokens,
            output_tokens: api_resp.usage.output_tokens,
            ..Default::default()
        };

        Ok(AnthropicResponse {
            content,
            stop_reason,
            usage,
            model: api_resp.model,
        })
    }

    async fn stream_message(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[ToolDefinition],
        max_tokens: u32,
    ) -> Result<
        Box<dyn Stream<Item = Result<StreamChunk, AnthropicError>> + Send + Unpin>,
        AnthropicError,
    > {
        let body = self.build_request_body(system, messages, tools, max_tokens, true);

        let response = self
            .client
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| AnthropicError::Http(e.to_string()))?;

        let status = response.status().as_u16();
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

        // Read the full SSE body and parse events.
        // For a production implementation, this would use async byte streaming.
        // This simplified version buffers the response and yields parsed chunks.
        let text = response
            .text()
            .await
            .map_err(|e| AnthropicError::Stream(e.to_string()))?;

        let chunks = parse_sse_events(&text);
        Ok(Box::new(stream::iter(chunks)))
    }
}

/// Parse SSE event stream text into StreamChunk items.
fn parse_sse_events(raw: &str) -> Vec<Result<StreamChunk, AnthropicError>> {
    let mut chunks = Vec::new();
    let mut current_event = String::new();
    let mut current_data = String::new();

    for line in raw.lines() {
        if line.starts_with("event: ") {
            current_event = line[7..].to_string();
        } else if line.starts_with("data: ") {
            current_data = line[6..].to_string();
        } else if line.is_empty() && !current_event.is_empty() {
            if let Some(chunk) = parse_sse_event(&current_event, &current_data) {
                chunks.push(Ok(chunk));
            }
            current_event.clear();
            current_data.clear();
        }
    }

    chunks
}

fn parse_sse_event(event: &str, data: &str) -> Option<StreamChunk> {
    let json: serde_json::Value = serde_json::from_str(data).ok()?;

    match event {
        "content_block_start" => {
            let block = json.get("content_block")?;
            let block_type = block.get("type")?.as_str()?;
            if block_type == "tool_use" {
                Some(StreamChunk::ToolUseStart {
                    id: block.get("id")?.as_str()?.to_string(),
                    name: block.get("name")?.as_str()?.to_string(),
                })
            } else {
                None
            }
        }
        "content_block_delta" => {
            let delta = json.get("delta")?;
            let delta_type = delta.get("type")?.as_str()?;
            match delta_type {
                "text_delta" => {
                    let text = delta.get("text")?.as_str()?.to_string();
                    Some(StreamChunk::TextDelta(text))
                }
                "input_json_delta" => {
                    let partial = delta.get("partial_json")?.as_str()?.to_string();
                    Some(StreamChunk::InputJsonDelta(partial))
                }
                _ => None,
            }
        }
        "message_delta" => {
            let delta = json.get("delta")?;
            let stop_reason = match delta.get("stop_reason")?.as_str()? {
                "end_turn" => StopReason::EndTurn,
                "tool_use" => StopReason::ToolUse,
                "max_tokens" => StopReason::MaxTokens,
                _ => StopReason::EndTurn,
            };
            let usage_obj = json.get("usage")?;
            let usage = TokenUsage {
                output_tokens: usage_obj
                    .get("output_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u32,
                ..Default::default()
            };
            Some(StreamChunk::MessageStop { stop_reason, usage })
        }
        _ => None,
    }
}

// --- API response types (private, for deserialization only) ---

#[derive(Debug, Deserialize)]
struct ApiResponse {
    content: Vec<ApiContentBlock>,
    model: String,
    stop_reason: Option<String>,
    usage: ApiUsage,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum ApiContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
}

#[derive(Debug, Deserialize)]
struct ApiUsage {
    input_tokens: u32,
    output_tokens: u32,
}
