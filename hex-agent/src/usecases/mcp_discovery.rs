//! MCP tool discovery usecase (ADR-033).
//!
//! Connects to configured MCP servers, discovers their tools, and merges
//! them with built-in tools for the Anthropic API.

use std::sync::Arc;
use crate::domain::{ToolDefinition, ToolInputSchema, mcp::McpServerConfig};
use crate::ports::mcp_client::{McpClientPort, McpError};

/// Result of MCP tool discovery.
pub struct DiscoveryResult {
    /// Tools discovered from MCP servers, converted to Anthropic format.
    pub tools: Vec<ToolDefinition>,
    /// Servers that failed to connect (non-fatal — we log and skip).
    pub failed: Vec<(String, McpError)>,
    /// Number of servers successfully connected.
    pub connected_count: usize,
}

/// Connect to all configured MCP servers and discover their tools.
///
/// Converts MCP tool definitions to Anthropic ToolDefinition format,
/// prefixing each tool name with `mcp__<server_name>__`.
pub async fn discover_mcp_tools(
    client: &Arc<dyn McpClientPort>,
    configs: &[McpServerConfig],
) -> DiscoveryResult {
    let mut tools = Vec::new();
    let mut failed = Vec::new();
    let mut connected = 0;

    for config in configs {
        tracing::info!(server = %config.name, command = %config.command, "Connecting to MCP server");

        match client.connect_and_discover(config).await {
            Ok(mcp_tools) => {
                tracing::info!(
                    server = %config.name,
                    tool_count = mcp_tools.len(),
                    "MCP server connected"
                );
                connected += 1;

                for mcp_tool in mcp_tools {
                    let prefixed_name = format!("mcp__{}__{}", config.name, mcp_tool.name);
                    let description = mcp_tool.description.unwrap_or_default();

                    // Convert MCP JSON Schema → ToolInputSchema.
                    // MCP input_schema is a raw JSON object with "type", "properties", "required".
                    let schema = &mcp_tool.input_schema;
                    let input_schema = ToolInputSchema {
                        schema_type: schema
                            .get("type")
                            .and_then(|v| v.as_str())
                            .unwrap_or("object")
                            .to_string(),
                        properties: schema
                            .get("properties")
                            .cloned()
                            .unwrap_or(serde_json::Value::Object(Default::default())),
                        required: schema
                            .get("required")
                            .and_then(|v| v.as_array())
                            .map(|arr| {
                                arr.iter()
                                    .filter_map(|v| v.as_str().map(String::from))
                                    .collect()
                            })
                            .unwrap_or_default(),
                    };

                    tools.push(ToolDefinition {
                        name: prefixed_name,
                        description,
                        input_schema,
                    });
                }
            }
            Err(e) => {
                tracing::warn!(
                    server = %config.name,
                    error = %e,
                    "Failed to connect to MCP server — skipping"
                );
                failed.push((config.name.clone(), e));
            }
        }
    }

    DiscoveryResult {
        tools,
        failed,
        connected_count: connected,
    }
}
