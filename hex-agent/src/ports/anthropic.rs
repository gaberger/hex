use crate::domain::{ApiRequestOptions, ContentBlock, Message, StopReason, TokenUsage, ToolDefinition};
use async_trait::async_trait;

/// Streaming chunk from the Anthropic API.
#[derive(Debug, Clone)]
pub enum StreamChunk {
    /// A text delta
    TextDelta(String),
    /// A tool_use block started
    ToolUseStart { id: String, name: String },
    /// Input JSON delta for an in-progress tool_use
    InputJsonDelta(String),
    /// The message stopped
    MessageStop {
        stop_reason: StopReason,
        usage: TokenUsage,
    },
}

/// Port for communicating with the Anthropic Messages API.
///
/// This is the core outbound port — hex-agent's reason for existing.
/// The adapter implements SSE streaming, tool_use handling, token tracking,
/// prompt caching (cache_control), and extended thinking (budget_tokens).
#[async_trait]
pub trait AnthropicPort: Send + Sync {
    /// Send a conversation to the API and get the full response.
    ///
    /// When `model_override` is `Some`, it replaces the adapter's configured model
    /// for this single request (used by RL-driven model selection).
    ///
    /// When `options` is `Some`, it enables caching, thinking budget, etc.
    async fn send_message(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[ToolDefinition],
        max_tokens: u32,
        model_override: Option<&str>,
        options: Option<&ApiRequestOptions>,
    ) -> Result<AnthropicResponse, AnthropicError>;

    /// Send a conversation and stream the response chunk by chunk.
    ///
    /// When `model_override` is `Some`, it replaces the adapter's configured model
    /// for this single request.
    async fn stream_message(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[ToolDefinition],
        max_tokens: u32,
        model_override: Option<&str>,
        options: Option<&ApiRequestOptions>,
    ) -> Result<Box<dyn futures::Stream<Item = Result<StreamChunk, AnthropicError>> + Send + Unpin>, AnthropicError>;
}

/// Full response from the Anthropic API.
#[derive(Debug, Clone)]
pub struct AnthropicResponse {
    pub content: Vec<ContentBlock>,
    pub stop_reason: StopReason,
    pub usage: TokenUsage,
    pub model: String,
}

/// Errors from the Anthropic adapter.
#[derive(Debug, thiserror::Error)]
pub enum AnthropicError {
    #[error("HTTP error: {0}")]
    Http(String),
    #[error("API error {status}: {message}")]
    Api { status: u16, message: String },
    #[error("Rate limited, retry after {retry_after_ms}ms")]
    RateLimited { retry_after_ms: u64 },
    #[error("Stream error: {0}")]
    Stream(String),
    #[error("Deserialization error: {0}")]
    Deserialize(String),
    #[error("Context window exceeded: used {used} of {max}")]
    ContextOverflow { used: u32, max: u32 },
}

impl AnthropicError {
    /// Returns `true` if this error is an HTTP 429 rate limit.
    pub fn is_rate_limited(&self) -> bool {
        matches!(self, Self::RateLimited { .. })
    }
}
