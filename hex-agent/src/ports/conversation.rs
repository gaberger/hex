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
    Error(String),
}

/// Port for driving a conversation — primary adapters use this
/// instead of importing the use case directly.
#[async_trait]
pub trait ConversationPort: Send + Sync {
    async fn process_message(
        &self,
        state: &mut ConversationState,
        user_input: &str,
        event_tx: &tokio::sync::mpsc::UnboundedSender<ConversationEvent>,
    ) -> Result<(), ConversationError>;
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
