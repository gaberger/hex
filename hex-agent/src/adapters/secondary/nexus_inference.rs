//! Nexus inference bridge adapter — routes LLM calls through hex-nexus HTTP API.
//!
//! Implements `AnthropicPort` so hex-agent can use the nexus as its primary LLM
//! provider when hub-connected. The nexus handles provider selection, API key
//! management, and fallback logic internally.
//!
//! POST /api/inference/complete
//!   Request:  { model, messages: [{role, content}], system, max_tokens }
//!   Response: { content, model, input_tokens, output_tokens }

use crate::ports::anthropic::{AnthropicError, AnthropicPort, AnthropicResponse, StreamChunk};
use crate::ports::{ApiRequestOptions, ContentBlock, Message, Role, StopReason, TokenUsage, ToolDefinition};
use async_trait::async_trait;
use futures::stream;
use reqwest::Client;
use serde_json::json;

/// Adapter that bridges hex-agent to the nexus `/api/inference/complete` endpoint.
///
/// This does NOT support tool use or streaming — those features require a direct
/// provider connection. When the nexus bridge is used, tool calls are handled by
/// falling back to the direct adapter on `StopReason::ToolUse`.
pub struct NexusInferenceAdapter {
    client: Client,
    nexus_url: String,
    model: String,
}

impl NexusInferenceAdapter {
    pub fn new(nexus_url: &str, model: &str) -> Self {
        Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .unwrap_or_else(|_| Client::new()),
            nexus_url: nexus_url.trim_end_matches('/').to_string(),
            model: model.to_string(),
        }
    }

    /// Probe the nexus inference endpoint. Returns `true` if the endpoint exists
    /// (even if it returns an error for empty input).
    pub async fn probe(nexus_url: &str) -> bool {
        let url = format!("{}/api/inference/complete", nexus_url.trim_end_matches('/'));
        let result = Client::new()
            .post(&url)
            .json(&json!({"messages": [], "model": "probe"}))
            .timeout(std::time::Duration::from_secs(3))
            .send()
            .await;
        // Any HTTP response (even 4xx/5xx) means the endpoint exists.
        // Only connection failures mean it's unavailable.
        result.is_ok()
    }

    /// Convert hex-agent messages to OpenAI-compatible JSON for the nexus.
    fn messages_to_json(messages: &[Message]) -> Vec<serde_json::Value> {
        let mut out = Vec::new();
        for msg in messages {
            let role = match msg.role {
                Role::User => "user",
                Role::Assistant => "assistant",
            };

            // Collect text parts; skip tool_use/tool_result blocks (nexus doesn't handle them)
            let mut text_parts = Vec::new();
            for block in &msg.content {
                if let ContentBlock::Text { text } = block {
                    text_parts.push(text.clone());
                }
            }
            if !text_parts.is_empty() {
                out.push(json!({
                    "role": role,
                    "content": text_parts.join("\n"),
                }));
            }
        }
        out
    }
}

#[async_trait]
impl AnthropicPort for NexusInferenceAdapter {
    async fn send_message(
        &self,
        system: &str,
        messages: &[Message],
        _tools: &[ToolDefinition],
        max_tokens: u32,
        model_override: Option<&str>,
        _options: Option<&ApiRequestOptions>,
    ) -> Result<AnthropicResponse, AnthropicError> {
        let model = model_override.unwrap_or(&self.model);
        let url = format!("{}/api/inference/complete", self.nexus_url);

        let body = json!({
            "model": model,
            "messages": Self::messages_to_json(messages),
            "system": system,
            "max_tokens": max_tokens,
        });

        let response = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| AnthropicError::Http(format!("nexus unreachable: {e}")))?;

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
        if status >= 400 {
            let text = response.text().await.unwrap_or_else(|_| "unknown error".into());
            return Err(AnthropicError::Api { status, message: text });
        }

        let data: serde_json::Value = response
            .json()
            .await
            .map_err(|e| AnthropicError::Deserialize(format!("invalid JSON: {e}")))?;

        let content_text = data["content"]
            .as_str()
            .unwrap_or("(empty)")
            .to_string();
        let model_used = data["model"]
            .as_str()
            .unwrap_or(model)
            .to_string();
        let input_tokens = data["input_tokens"].as_u64().unwrap_or(0) as u32;
        let output_tokens = data["output_tokens"].as_u64().unwrap_or(0) as u32;

        Ok(AnthropicResponse {
            content: vec![ContentBlock::Text { text: content_text }],
            stop_reason: StopReason::EndTurn,
            usage: TokenUsage {
                input_tokens,
                output_tokens,
                ..Default::default()
            },
            model: model_used,
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
        Box<dyn futures::Stream<Item = Result<StreamChunk, AnthropicError>> + Send + Unpin>,
        AnthropicError,
    > {
        // Nexus HTTP bridge doesn't support streaming — emit as single chunk.
        let resp = self
            .send_message(system, messages, tools, max_tokens, model_override, options)
            .await?;

        let mut chunks = Vec::new();
        for block in &resp.content {
            if let ContentBlock::Text { text } = block {
                chunks.push(Ok(StreamChunk::TextDelta(text.clone())));
            }
        }
        chunks.push(Ok(StreamChunk::MessageStop {
            stop_reason: resp.stop_reason,
            usage: resp.usage,
        }));

        Ok(Box::new(stream::iter(chunks)))
    }
}
