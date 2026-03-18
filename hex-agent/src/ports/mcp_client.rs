//! Port for MCP client operations (ADR-033).

use async_trait::async_trait;
use crate::domain::mcp::{McpServerConfig, McpToolDef, McpToolResult};

#[derive(Debug, thiserror::Error)]
pub enum McpError {
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),
    #[error("Tool call failed: {0}")]
    ToolCallFailed(String),
    #[error("Server error: {code} — {message}")]
    ServerError { code: i64, message: String },
    #[error("Timeout: {0}")]
    Timeout(String),
    #[error("Protocol error: {0}")]
    ProtocolError(String),
}

/// Port for connecting to and calling tools on MCP servers.
#[async_trait]
pub trait McpClientPort: Send + Sync {
    /// Connect to an MCP server, perform handshake, and discover tools.
    /// Returns the list of tools available on this server.
    async fn connect_and_discover(&self, config: &McpServerConfig) -> Result<Vec<McpToolDef>, McpError>;

    /// Call a tool on a connected server.
    async fn call_tool(
        &self,
        server_name: &str,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<McpToolResult, McpError>;

    /// Check if a server is connected.
    fn is_connected(&self, server_name: &str) -> bool;

    /// Disconnect from all servers.
    async fn disconnect_all(&self);
}
