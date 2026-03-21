//! SpacetimeDB inference adapter — routes LLM calls through hex-nexus HTTP API.
//!
//! Calls `POST /api/inference/complete` on the nexus, which routes through
//! registered inference providers (Ollama, vLLM, OpenAI-compat) with
//! Anthropic as fallback.

use async_trait::async_trait;
use hex_core::domain::messages::{ContentBlock, StopReason};
use hex_core::ports::inference::{
    futures_stream, InferenceCapabilities, InferenceError, InferenceRequest, InferenceResponse,
    StreamChunk,
};
use serde_json::json;

/// Inference adapter that routes LLM calls through hex-nexus HTTP API.
///
/// The nexus handles provider selection, API key management, and fallback
/// logic. This adapter simply serializes the request and deserializes the
/// response.
pub struct SpacetimeInferenceAdapter {
    nexus_url: String,
    #[allow(dead_code)]
    agent_id: String,
    http: reqwest::Client,
}

impl SpacetimeInferenceAdapter {
    pub fn new(nexus_url: String, agent_id: String) -> Self {
        Self {
            nexus_url,
            agent_id,
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
        }
    }
}

/// Convert hex-core Messages to OpenAI-compatible JSON messages.
fn messages_to_json(messages: &[hex_core::domain::messages::Message]) -> Vec<serde_json::Value> {
    messages
        .iter()
        .map(|msg| {
            let role = match msg.role {
                hex_core::domain::messages::Role::User => "user",
                hex_core::domain::messages::Role::Assistant => "assistant",
            };
            // Flatten text content blocks into a single string for
            // OpenAI-compatible APIs. Tool use blocks are serialized as JSON.
            let has_tool_blocks = msg.content.iter().any(|b| {
                matches!(b, ContentBlock::ToolUse { .. } | ContentBlock::ToolResult { .. })
            });
            if has_tool_blocks {
                // Send structured content blocks
                json!({ "role": role, "content": msg.content })
            } else {
                // Send plain text (more compatible)
                let text = msg.text_content();
                json!({ "role": role, "content": text })
            }
        })
        .collect()
}

#[async_trait]
impl hex_core::ports::inference::IInferencePort for SpacetimeInferenceAdapter {
    async fn complete(
        &self,
        request: InferenceRequest,
    ) -> Result<InferenceResponse, InferenceError> {
        let url = format!("{}/api/inference/complete", self.nexus_url);

        let body = json!({
            "model": request.model,
            "messages": messages_to_json(&request.messages),
            "system": request.system_prompt,
            "max_tokens": request.max_tokens,
        });

        let start = std::time::Instant::now();

        let resp = self
            .http
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| InferenceError::Network(format!("nexus unreachable: {e}")))?;

        let status = resp.status().as_u16();
        if status >= 400 {
            let body = resp.text().await.unwrap_or_default();
            return Err(InferenceError::ApiError { status, body });
        }

        let data: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| InferenceError::Network(format!("invalid JSON response: {e}")))?;

        let content_text = data["content"]
            .as_str()
            .unwrap_or("(empty)")
            .to_string();
        let model_used = data["model"]
            .as_str()
            .unwrap_or("unknown")
            .to_string();
        let input_tokens = data["input_tokens"].as_u64().unwrap_or(0);
        let output_tokens = data["output_tokens"].as_u64().unwrap_or(0);
        let latency_ms = start.elapsed().as_millis() as u64;

        Ok(InferenceResponse {
            content: vec![ContentBlock::Text { text: content_text }],
            model_used,
            stop_reason: StopReason::EndTurn,
            input_tokens,
            output_tokens,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            latency_ms,
        })
    }

    async fn stream(
        &self,
        _request: InferenceRequest,
    ) -> Result<
        Box<dyn futures_stream::Stream<Item = StreamChunk> + Send + Unpin>,
        InferenceError,
    > {
        // Streaming not yet supported through nexus HTTP bridge.
        // The WebSocket path (/ws/chat) supports streaming, but this adapter
        // uses synchronous HTTP for simplicity.
        Err(InferenceError::ProviderUnavailable(
            "Streaming not supported via nexus HTTP bridge; use complete() instead".into(),
        ))
    }

    fn capabilities(&self) -> InferenceCapabilities {
        // Capabilities are dynamic (depend on which providers are registered
        // in the nexus). Return a reasonable default — the nexus handles
        // actual capability negotiation.
        InferenceCapabilities {
            models: vec![],
            supports_tool_use: false,
            supports_thinking: false,
            supports_caching: false,
            supports_streaming: false,
            max_context_tokens: 128_000,
            cost_per_mtok_input: 0.0,
            cost_per_mtok_output: 0.0,
        }
    }
}
