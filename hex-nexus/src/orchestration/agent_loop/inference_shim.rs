//! HTTP-backed `IInferencePort` adapter for the agent loop driver
//! (wp-sop-agent-loop P3).
//!
//! Mirrors the existing `HttpInferenceShim` in org_responder.rs (which is
//! private). Same endpoint contract — POSTs to
//! `/api/inference/complete` and returns a single-Text-block
//! `InferenceResponse`. Two copies during the rollout; collapse into one
//! source after the agent-loop path becomes the default (post-P7).

use async_trait::async_trait;
use hex_core::domain::messages::{ContentBlock, StopReason};
use hex_core::ports::inference::{
    futures_stream, HealthStatus, IInferencePort, InferenceCapabilities, InferenceError,
    InferenceRequest, InferenceResponse, ModelInfo, ModelTier, StreamChunk,
};

pub struct HttpInferenceShim {
    http: reqwest::Client,
    url: String,
}

impl HttpInferenceShim {
    pub fn new(http: reqwest::Client, url: String) -> Self {
        Self { http, url }
    }
}

#[async_trait]
impl IInferencePort for HttpInferenceShim {
    async fn complete(
        &self,
        request: InferenceRequest,
    ) -> Result<InferenceResponse, InferenceError> {
        let messages_json: Vec<serde_json::Value> = request
            .messages
            .iter()
            .map(|m| {
                let role = match m.role {
                    hex_core::domain::messages::Role::User => "user",
                    hex_core::domain::messages::Role::Assistant => "assistant",
                };
                let text = m
                    .content
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("");
                serde_json::json!({ "role": role, "content": text })
            })
            .collect();

        let body = serde_json::json!({
            "model": request.model,
            "messages": messages_json,
            "system": request.system_prompt,
            "max_tokens": request.max_tokens,
        });

        let resp = self
            .http
            .post(&self.url)
            .json(&body)
            .send()
            .await
            .map_err(|e| InferenceError::Network(e.to_string()))?;
        let status = resp.status();
        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| InferenceError::Network(e.to_string()))?;
        if !status.is_success() {
            return Err(InferenceError::ApiError {
                status: status.as_u16(),
                body: json.to_string(),
            });
        }
        let content_str = json
            .get("content")
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .to_string();
        let model_used = json
            .get("model")
            .and_then(|m| m.as_str())
            .unwrap_or(&request.model)
            .to_string();
        let input_tokens = json
            .get("input_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let output_tokens = json
            .get("output_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        Ok(InferenceResponse {
            content: vec![ContentBlock::Text { text: content_str }],
            model_used,
            stop_reason: StopReason::EndTurn,
            input_tokens,
            output_tokens,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            latency_ms: 0,
        })
    }

    async fn stream(
        &self,
        _request: InferenceRequest,
    ) -> Result<
        Box<dyn futures_stream::Stream<Item = StreamChunk> + Send + Unpin>,
        InferenceError,
    > {
        Err(InferenceError::ProviderUnavailable(
            "HttpInferenceShim does not implement stream()".to_string(),
        ))
    }

    async fn health(&self) -> Result<HealthStatus, InferenceError> {
        Ok(HealthStatus::Ok { models: vec![] })
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities {
            models: vec![ModelInfo {
                id: "http-shim".into(),
                provider: "hex-nexus".into(),
                tier: ModelTier::Local,
                context_window: 32_000,
            }],
            supports_tool_use: false,
            supports_thinking: false,
            supports_caching: false,
            supports_streaming: false,
            max_context_tokens: 32_000,
            cost_per_mtok_input: 0.0,
            cost_per_mtok_output: 0.0,
        }
    }
}
