//! Inference client port — typed LLM completion interface for hex-agent.
//!
//! This port abstracts LLM inference calls through hex-nexus, supporting
//! task-type-aware model selection (via RL routing) and cost tracking.
//! Used by the "hex dev" TUI pipeline (ADR-2603232005).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// The type of task being performed — influences RL model selection.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskType {
    /// Chain-of-thought reasoning (planning, architecture decisions)
    Reasoning,
    /// JSON/structured output generation
    StructuredOutput,
    /// Writing new code
    CodeGeneration,
    /// Editing existing code (diffs, refactors)
    CodeEdit,
    /// General-purpose chat/assistant
    General,
}

impl std::fmt::Display for TaskType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TaskType::Reasoning => write!(f, "reasoning"),
            TaskType::StructuredOutput => write!(f, "structured_output"),
            TaskType::CodeGeneration => write!(f, "code_generation"),
            TaskType::CodeEdit => write!(f, "code_edit"),
            TaskType::General => write!(f, "general"),
        }
    }
}

/// A chat message for inference requests.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
}

/// Role in a chat conversation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ChatRole {
    System,
    User,
    Assistant,
}

/// Request for an LLM completion.
#[derive(Debug, Clone)]
pub struct InferenceRequest {
    /// Model identifier. If None, the RL router picks the best model for the task type.
    pub model: Option<String>,
    /// Conversation messages (user/assistant turns).
    pub messages: Vec<ChatMessage>,
    /// System prompt (prepended by the server).
    pub system_prompt: Option<String>,
    /// Task type hint for RL model selection.
    pub task_type: TaskType,
    /// Maximum tokens to generate.
    pub max_tokens: u32,
    /// Sampling temperature (0.0–2.0). If None, server default is used.
    pub temperature: Option<f32>,
}

impl Default for InferenceRequest {
    fn default() -> Self {
        Self {
            model: None,
            messages: Vec::new(),
            system_prompt: None,
            task_type: TaskType::General,
            max_tokens: 4096,
            temperature: None,
        }
    }
}

/// Response from an LLM completion.
#[derive(Debug, Clone)]
pub struct InferenceResponse {
    /// The generated text content.
    pub content: String,
    /// Which model actually served the request.
    pub model_used: String,
    /// Input (prompt) token count.
    pub input_tokens: u64,
    /// Output (completion) token count.
    pub output_tokens: u64,
    /// Actual cost in USD (OpenRouter-specific; None for other providers).
    pub cost_usd: Option<f64>,
    /// Wall-clock duration of the request in milliseconds.
    pub duration_ms: u64,
}

/// A streaming chunk from an LLM completion.
#[derive(Debug, Clone)]
pub enum InferenceStreamChunk {
    /// A delta of generated text.
    TextDelta(String),
    /// Final message with usage stats.
    Done {
        model_used: String,
        input_tokens: u64,
        output_tokens: u64,
        cost_usd: Option<f64>,
        duration_ms: u64,
    },
    /// An error occurred mid-stream.
    Error(String),
}

/// Errors from inference operations.
#[derive(Debug, thiserror::Error)]
pub enum InferenceClientError {
    #[error("Nexus unreachable at {url}: {cause}")]
    NexusDown { url: String, cause: String },

    #[error("Model unavailable: {0}")]
    ModelUnavailable(String),

    #[error("Rate limited — retry after {retry_after_ms}ms")]
    RateLimited { retry_after_ms: u64 },

    #[error("Budget exceeded: {0}")]
    BudgetExceeded(String),

    #[error("Inference failed: HTTP {status}: {message}")]
    ApiError { status: u16, message: String },

    #[error("Invalid response: {0}")]
    InvalidResponse(String),
}

/// Port for making LLM inference calls through hex-nexus.
#[async_trait]
pub trait InferenceClientPort: Send + Sync {
    /// Send a completion request and wait for the full response.
    async fn complete(
        &self,
        request: InferenceRequest,
    ) -> Result<InferenceResponse, InferenceClientError>;

    /// Send a completion request and receive streaming chunks.
    ///
    /// Returns a boxed stream of chunks. The last chunk is always `Done` with usage stats,
    /// or `Error` if something went wrong mid-stream.
    async fn complete_stream(
        &self,
        request: InferenceRequest,
    ) -> Result<
        Box<dyn futures::Stream<Item = InferenceStreamChunk> + Send + Unpin>,
        InferenceClientError,
    >;
}
