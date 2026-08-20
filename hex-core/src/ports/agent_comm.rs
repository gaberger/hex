//! IAgentCommPort — contract for agent-to-agent communication.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Message sent between agents via direct message or channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMessage {
    pub id: Option<u64>,
    pub from_agent: String,
    pub to_agent: Option<String>,
    pub channel: Option<String>,
    pub message: String,
    pub thread_id: Option<String>,
    pub timestamp: String,
    pub read_by: Vec<String>,
}

/// Channel configuration for team or role-based communication.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentChannel {
    pub name: String,
    pub members: Vec<String>,
    pub created_at: String,
}

/// Typing indicator for real-time presence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypingIndicator {
    pub agent: String,
    pub channel_or_dm: String,
    pub timestamp: String,
}

#[derive(Debug, thiserror::Error)]
pub enum AgentCommError {
    #[error("Agent {0} not authorized for channel {1}")]
    NotAuthorized(String, String),

    #[error("Channel {0} does not exist")]
    ChannelNotFound(String),

    #[error("Message {0} not found")]
    MessageNotFound(u64),

    #[error("Channel {0} already exists")]
    ChannelExists(String),

    #[error("Transport error: {0}")]
    Transport(String),
}

/// Port for agent-to-agent communication.
///
/// Supports:
/// - Direct messages between agents
/// - Team/role-based channels (#c-suite, #eng-team, etc.)
/// - Thread grouping for conversation context
/// - Read receipts and typing indicators
///
/// Implementations live in adapters/secondary (e.g., SpacetimeDB).
#[async_trait]
pub trait IAgentCommPort: Send + Sync {
    /// Send a direct message to another agent.
    async fn send_dm(
        &self,
        from: String,
        to: String,
        message: String,
        thread_id: Option<String>,
    ) -> Result<u64, AgentCommError>;

    /// Send a message to a channel (requires membership).
    async fn send_to_channel(
        &self,
        from: String,
        channel: String,
        message: String,
        thread_id: Option<String>,
    ) -> Result<u64, AgentCommError>;

    /// Mark a message as read by an agent.
    async fn mark_read(&self, agent: String, message_id: u64) -> Result<(), AgentCommError>;

    /// Create a new channel with specified members.
    /// Use "*" in members for public channels.
    async fn create_channel(
        &self,
        name: String,
        members: Vec<String>,
    ) -> Result<(), AgentCommError>;

    /// Set typing indicator for an agent in a channel or DM.
    async fn set_typing(&self, agent: String, channel_or_dm: String) -> Result<(), AgentCommError>;

    /// Clear typing indicator for an agent.
    async fn clear_typing(&self, agent: String) -> Result<(), AgentCommError>;

    /// Query messages for an agent (DMs + channels they're in).
    async fn query_messages(
        &self,
        agent: String,
        limit: Option<u32>,
    ) -> Result<Vec<AgentMessage>, AgentCommError>;

    /// Query messages in a specific channel.
    async fn query_channel_messages(
        &self,
        channel: String,
        limit: Option<u32>,
    ) -> Result<Vec<AgentMessage>, AgentCommError>;

    /// Query messages in a specific thread.
    async fn query_thread_messages(
        &self,
        thread_id: String,
        limit: Option<u32>,
    ) -> Result<Vec<AgentMessage>, AgentCommError>;

    /// List all channels an agent has access to.
    async fn list_channels(&self, agent: String) -> Result<Vec<AgentChannel>, AgentCommError>;

    /// Get active typing indicators for a channel or DM.
    async fn get_typing_indicators(
        &self,
        channel_or_dm: String,
    ) -> Result<Vec<TypingIndicator>, AgentCommError>;
}
