use crate::domain::{ConversationState, Message, TokenBudget};
use async_trait::async_trait;

/// Port for managing the context window — decides what fits and what gets evicted.
#[async_trait]
pub trait ContextManagerPort: Send + Sync {
    /// Count tokens in a string (implementation may use tiktoken or char estimate).
    fn count_tokens(&self, text: &str) -> u32;

    /// Count tokens in a message (including tool blocks).
    fn count_message_tokens(&self, message: &Message) -> u32;

    /// Pack the conversation state to fit within the token budget.
    /// Returns the trimmed messages and the system prompt, respecting partitions.
    /// Older messages are evicted first; pinned tool results are preserved.
    async fn pack(
        &self,
        state: &ConversationState,
        budget: &TokenBudget,
    ) -> Result<PackedContext, ContextError>;

    /// Summarize old messages that are being evicted (for sliding window).
    async fn summarize(&self, messages: &[Message]) -> Result<String, ContextError>;
}

/// The result of packing — ready to send to the Anthropic API.
#[derive(Debug, Clone)]
pub struct PackedContext {
    pub system_prompt: String,
    pub messages: Vec<Message>,
    pub total_tokens: u32,
    pub evicted_count: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum ContextError {
    #[error("System prompt alone exceeds budget ({tokens} > {budget})")]
    SystemPromptTooLarge { tokens: u32, budget: u32 },
    #[error("Summarization failed: {0}")]
    SummarizationFailed(String),
}
