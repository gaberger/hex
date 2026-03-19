//! SpacetimeDB inference adapter — routes LLM calls through SpacetimeDB procedures.
//!
//! In Phase 6, this will use the SpacetimeDB SDK directly.
//! For now, it returns a clear "not yet connected" error.

use async_trait::async_trait;
use hex_core::ports::inference::{
    futures_stream, InferenceCapabilities, InferenceError, InferenceRequest, InferenceResponse,
    StreamChunk,
};

/// Inference adapter that routes LLM calls through SpacetimeDB procedures.
///
/// In Phase 6, this will use the SpacetimeDB SDK directly.
/// For now, it routes through hex-nexus HTTP API which calls the
/// inference-gateway reducers.
pub struct SpacetimeInferenceAdapter {
    #[allow(dead_code)]
    nexus_url: String,
    #[allow(dead_code)]
    agent_id: String,
    #[allow(dead_code)]
    http: reqwest::Client,
}

impl SpacetimeInferenceAdapter {
    pub fn new(nexus_url: String, agent_id: String) -> Self {
        Self {
            nexus_url,
            agent_id,
            http: reqwest::Client::new(),
        }
    }
}

// Stub implementation — returns a clear error until Phase 6.
#[async_trait]
impl hex_core::ports::inference::IInferencePort for SpacetimeInferenceAdapter {
    async fn complete(
        &self,
        _request: InferenceRequest,
    ) -> Result<InferenceResponse, InferenceError> {
        Err(InferenceError::ProviderUnavailable(
            "SpacetimeDB inference gateway not yet connected (Phase 6)".into(),
        ))
    }

    async fn stream(
        &self,
        _request: InferenceRequest,
    ) -> Result<
        Box<dyn futures_stream::Stream<Item = StreamChunk> + Send + Unpin>,
        InferenceError,
    > {
        Err(InferenceError::ProviderUnavailable(
            "SpacetimeDB inference streaming not yet connected (Phase 6)".into(),
        ))
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities {
            models: vec![],
            supports_tool_use: false,
            supports_thinking: false,
            supports_caching: false,
            supports_streaming: false,
            max_context_tokens: 0,
            cost_per_mtok_input: 0.0,
            cost_per_mtok_output: 0.0,
        }
    }
}
