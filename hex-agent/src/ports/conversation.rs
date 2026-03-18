use crate::domain::{ConversationState, StopReason, TokenUsage};
use async_trait::async_trait;

/// Events emitted during a conversation turn.
#[derive(Debug, Clone)]
pub enum ConversationEvent {
    TextChunk(String),
    ToolCallStart { name: String, input: String },
    ToolCallResult { name: String, content: String, is_error: bool },
    TokenUpdate(TokenUsage),
    TurnComplete { stop_reason: StopReason },
    /// Context was reset (e.g., before hex plan)
    ContextReset { summary: String },
    Error(String),
}

/// Summary of a conversation checkpoint — saved before context reset.
#[derive(Debug, Clone)]
pub struct ConversationCheckpoint {
    pub conversation_id: String,
    pub turn_count: u32,
    pub summary: String,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
}

/// Port for driving a conversation — primary adapters use this
/// instead of importing the use case directly.
#[async_trait]
pub trait ConversationPort: Send + Sync {
    /// Process a user message through the conversation loop.
    async fn process_message(
        &self,
        state: &mut ConversationState,
        user_input: &str,
        event_tx: &tokio::sync::mpsc::UnboundedSender<ConversationEvent>,
    ) -> Result<(), ConversationError>;

    /// Reset the context window for a fresh task (e.g., hex plan).
    ///
    /// 1. Summarizes the current conversation into a checkpoint
    /// 2. Clears message history from state
    /// 3. Optionally injects a new system prompt for the task
    /// 4. Returns the checkpoint for persistence (AgentDB/SQLite)
    async fn reset_context(
        &self,
        state: &mut ConversationState,
        new_system_prompt: Option<String>,
    ) -> Result<ConversationCheckpoint, ConversationError>;
}

#[derive(Debug, thiserror::Error)]
pub enum ConversationError {
    #[error("API error: {0}")]
    ApiError(String),
    #[error("Context error: {0}")]
    ContextError(String),
    #[error("Tool execution error: {0}")]
    ToolError(String),
}
