//! MCP (Model Context Protocol) domain types.
//!
//! Defines the wire format for JSON-RPC 2.0 messages and MCP-specific
//! types for tool discovery and invocation (ADR-033).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// JSON-RPC 2.0 request.
#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String, // always "2.0"
    pub id: u64,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

/// JSON-RPC 2.0 response.
#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Option<u64>,
    pub result: Option<serde_json::Value>,
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    pub data: Option<serde_json::Value>,
}

/// JSON-RPC 2.0 notification (no id).
#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcNotification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

/// MCP server configuration (from .claude/settings.json).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct McpServerConfig {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(rename = "type", default = "default_transport")]
    pub transport: String,
}

fn default_transport() -> String {
    "stdio".to_string()
}

/// MCP tool definition returned by tools/list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolDef {
    pub name: String,
    pub description: Option<String>,
    #[serde(rename = "inputSchema")]
    pub input_schema: serde_json::Value,
}

/// Result from calling an MCP tool.
#[derive(Debug, Clone, Deserialize)]
pub struct McpToolResult {
    pub content: Vec<McpContent>,
    #[serde(rename = "isError", default)]
    pub is_error: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum McpContent {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image")]
    Image {
        data: String,
        #[serde(rename = "mimeType")]
        mime_type: String,
    },
}

/// Represents a live connection to an MCP server.
#[derive(Debug)]
pub struct McpConnection {
    pub server_name: String,
    pub tools: Vec<McpToolDef>,
}
