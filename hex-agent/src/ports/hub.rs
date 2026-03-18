use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Messages sent between hex-agent and hex-hub over WebSocket.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum HubMessage {
    /// Agent registers with the hub
    #[serde(rename = "agent_register")]
    Register {
        agent_id: String,
        agent_name: String,
        project_dir: String,
    },
    /// Streaming text chunk from agent
    #[serde(rename = "stream_chunk")]
    StreamChunk { text: String },
    /// Tool call started
    #[serde(rename = "tool_call")]
    ToolCall {
        tool_name: String,
        tool_input: serde_json::Value,
    },
    /// Tool call result
    #[serde(rename = "tool_result")]
    ToolResultMsg {
        tool_name: String,
        content: String,
        is_error: bool,
    },
    /// Token usage update
    #[serde(rename = "token_update")]
    TokenUpdate {
        input_tokens: u32,
        output_tokens: u32,
        total_input: u64,
        total_output: u64,
    },
    /// Agent status change
    #[serde(rename = "agent_status")]
    AgentStatus { status: String, detail: String },
    /// Chat message from hub to agent (user input routed through hub)
    #[serde(rename = "chat_message")]
    ChatMessage { content: String },
    /// Agent completed its task
    #[serde(rename = "agent_done")]
    Done {
        agent_id: String,
        summary: String,
        exit_code: i32,
    },
}

/// Port for communicating back to hex-hub.
///
/// When hex-agent is spawned by hex-hub, it connects back via WebSocket
/// to stream output, report status, and receive commands.
#[async_trait]
pub trait HubClientPort: Send + Sync {
    /// Connect to the hub WebSocket.
    async fn connect(&self, hub_url: &str, auth_token: &str) -> Result<(), HubError>;

    /// Send a message to the hub.
    async fn send(&self, message: HubMessage) -> Result<(), HubError>;

    /// Receive the next message from the hub (blocking).
    async fn recv(&self) -> Result<HubMessage, HubError>;

    /// Check if connected.
    fn is_connected(&self) -> bool;

    /// Disconnect from the hub.
    async fn disconnect(&self) -> Result<(), HubError>;
}

#[derive(Debug, thiserror::Error)]
pub enum HubError {
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),
    #[error("Send failed: {0}")]
    SendFailed(String),
    #[error("Receive failed: {0}")]
    ReceiveFailed(String),
    #[error("Not connected")]
    NotConnected,
}
