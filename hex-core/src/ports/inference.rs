//! Inference port — pluggable LLM backend contract.
//!
//! Every inference engine (Anthropic, MiniMax, Ollama, vLLM, Claude Code)
//! implements this trait. The RL engine selects which backend to use
//! based on capabilities, cost, and quality signals.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::domain::messages::{ContentBlock, StopReason};
use crate::domain::tools::ToolDefinition;

/// A request to an inference engine.
#[derive(Debug, Clone)]
pub struct InferenceRequest {
    pub model: String,
    pub system_prompt: String,
    pub messages: Vec<crate::domain::messages::Message>,
    pub tools: Vec<ToolDefinition>,
    pub max_tokens: u32,
    pub temperature: f32,
    pub thinking_budget: Option<u32>,
    pub cache_control: bool,
    pub priority: Priority,
}

/// Priority levels for inference requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Priority {
    Low = 0,
    Normal = 1,
    High = 2,
    Critical = 3,
}

impl Default for Priority {
    fn default() -> Self {
        Self::Normal
    }
}

/// Response from an inference engine.
#[derive(Debug, Clone)]
pub struct InferenceResponse {
    pub content: Vec<ContentBlock>,
    pub model_used: String,
    pub stop_reason: StopReason,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub latency_ms: u64,
}

/// A streaming chunk from an inference engine.
#[derive(Debug, Clone)]
pub enum StreamChunk {
    TextDelta(String),
    ToolUseStart { id: String, name: String },
    InputJsonDelta(String),
    MessageStop(StopReason),
    Usage { input_tokens: u64, output_tokens: u64 },
}

/// Capabilities reported by an inference backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceCapabilities {
    pub models: Vec<ModelInfo>,
    pub supports_tool_use: bool,
    pub supports_thinking: bool,
    pub supports_caching: bool,
    pub supports_streaming: bool,
    pub max_context_tokens: u64,
    pub cost_per_mtok_input: f64,
    pub cost_per_mtok_output: f64,
}

/// Information about a model available on a backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub provider: String,
    pub tier: ModelTier,
    pub context_window: u64,
}

/// Model capability tiers — used by RL engine for cost/quality tradeoffs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModelTier {
    Opus,
    Sonnet,
    Haiku,
    Local,
}

/// The inference port — every LLM backend implements this.
#[async_trait]
pub trait IInferencePort: Send + Sync {
    /// Send a request and get a complete response.
    async fn complete(
        &self,
        request: InferenceRequest,
    ) -> Result<InferenceResponse, InferenceError>;

    /// Stream a response chunk by chunk.
    async fn stream(
        &self,
        request: InferenceRequest,
    ) -> Result<Box<dyn futures_stream::Stream<Item = StreamChunk> + Send + Unpin>, InferenceError>;

    /// What capabilities does this backend support?
    fn capabilities(&self) -> InferenceCapabilities;
}

/// We define our own Stream-like trait to avoid pulling in futures as a dep.
/// Implementors can use tokio or futures internally.
pub mod futures_stream {
    pub trait Stream {
        type Item;
        fn poll_next(
            self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Option<Self::Item>>;
    }
}

/// Errors from inference operations.
#[derive(Debug, thiserror::Error)]
pub enum InferenceError {
    #[error("Rate limited: {0}")]
    RateLimited(String),
    #[error("Budget exceeded: used {used} of {limit} tokens")]
    BudgetExceeded { used: u64, limit: u64 },
    #[error("Provider unavailable: {0}")]
    ProviderUnavailable(String),
    #[error("API error: {status} — {body}")]
    ApiError { status: u16, body: String },
    #[error("Network error: {0}")]
    Network(String),
    #[error("Unknown provider: {0}")]
    UnknownProvider(String),
}
