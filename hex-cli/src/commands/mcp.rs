//! MCP server command.
//!
//! `hex mcp` — starts a Model Context Protocol server on stdio transport.
//! This provides tool access to hex capabilities (analyze, plan, scaffold,
//! summarize) for Claude Code and other MCP-compatible clients.
//!
//! Phase 5 S25: skeleton implementation. Full MCP protocol (tool dispatch,
//! notifications, resource subscriptions) will be added in a later phase.

use std::io::{self, BufRead, Write};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// JSON-RPC request envelope.
#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

/// JSON-RPC success response.
#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: &'static str,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

/// JSON-RPC error object.
#[derive(Debug, Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

/// Tools advertised by this MCP server.
const TOOL_NAMES: &[&str] = &[
    "hex_analyze",
    "hex_plan",
    "hex_scaffold",
    "hex_summarize",
];

/// Start the hex MCP server on stdio transport.
///
/// Reads JSON-RPC messages from stdin (one per line), dispatches them, and
/// writes responses to stdout. This replaces the TypeScript MCP adapter.
pub async fn run_mcp_server() -> anyhow::Result<()> {
    eprintln!("hex MCP server starting on stdio...");
    eprintln!(
        "Available tools: {}",
        TOOL_NAMES.join(", ")
    );

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut stdout_lock = stdout.lock();

    for line_result in stdin.lock().lines() {
        let line = match line_result {
            Ok(l) => l,
            Err(_) => break, // EOF or read error
        };

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let req: JsonRpcRequest = match serde_json::from_str(trimmed) {
            Ok(r) => r,
            Err(e) => {
                let err_resp = JsonRpcResponse {
                    jsonrpc: "2.0",
                    id: Value::Null,
                    result: None,
                    error: Some(JsonRpcError {
                        code: -32700,
                        message: format!("Parse error: {}", e),
                        data: None,
                    }),
                };
                write_response(&mut stdout_lock, &err_resp)?;
                continue;
            }
        };

        let _ = &req.jsonrpc; // acknowledge field presence

        let response = handle_request(&req);
        write_response(&mut stdout_lock, &response)?;
    }

    eprintln!("hex MCP server shutting down.");
    Ok(())
}

/// Dispatch a single JSON-RPC request to the appropriate handler.
fn handle_request(req: &JsonRpcRequest) -> JsonRpcResponse {
    let id = req.id.clone().unwrap_or(Value::Null);

    match req.method.as_str() {
        "initialize" => JsonRpcResponse {
            jsonrpc: "2.0",
            id,
            result: Some(serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "tools": {
                        "listChanged": false
                    }
                },
                "serverInfo": {
                    "name": "hex",
                    "version": env!("CARGO_PKG_VERSION")
                }
            })),
            error: None,
        },

        "initialized" => {
            // Notification — no response needed, but if id is present respond ok
            JsonRpcResponse {
                jsonrpc: "2.0",
                id,
                result: Some(Value::Null),
                error: None,
            }
        }

        "tools/list" => JsonRpcResponse {
            jsonrpc: "2.0",
            id,
            result: Some(serde_json::json!({
                "tools": [
                    {
                        "name": "hex_analyze",
                        "description": "Run architecture health check on a project",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "path": {
                                    "type": "string",
                                    "description": "Project root path to analyze"
                                }
                            },
                            "required": ["path"]
                        }
                    },
                    {
                        "name": "hex_plan",
                        "description": "Generate a workplan for a feature",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "feature": {
                                    "type": "string",
                                    "description": "Feature name or description"
                                }
                            },
                            "required": ["feature"]
                        }
                    },
                    {
                        "name": "hex_scaffold",
                        "description": "Scaffold a new hex project",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "name": {
                                    "type": "string",
                                    "description": "Project name"
                                },
                                "language": {
                                    "type": "string",
                                    "description": "Primary language (rust, typescript)"
                                }
                            },
                            "required": ["name"]
                        }
                    },
                    {
                        "name": "hex_summarize",
                        "description": "Generate token-efficient AST summary of a file or project",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "path": {
                                    "type": "string",
                                    "description": "File or directory path to summarize"
                                },
                                "level": {
                                    "type": "integer",
                                    "description": "Summary detail level (0-3)"
                                }
                            },
                            "required": ["path"]
                        }
                    }
                ]
            })),
            error: None,
        },

        "tools/call" => {
            // Tool dispatch — skeleton: all tools return a "not yet implemented" message
            let tool_name = req.params.get("name")
                .and_then(|n| n.as_str())
                .unwrap_or("unknown");

            let known = TOOL_NAMES.contains(&tool_name);

            if known {
                JsonRpcResponse {
                    jsonrpc: "2.0",
                    id,
                    result: Some(serde_json::json!({
                        "content": [{
                            "type": "text",
                            "text": format!(
                                "Tool '{}' recognized but not yet fully implemented (Phase 5 S25 skeleton). \
                                 Arguments: {}",
                                tool_name,
                                req.params.get("arguments").unwrap_or(&Value::Null)
                            )
                        }],
                        "isError": false
                    })),
                    error: None,
                }
            } else {
                JsonRpcResponse {
                    jsonrpc: "2.0",
                    id,
                    result: Some(serde_json::json!({
                        "content": [{
                            "type": "text",
                            "text": format!("Unknown tool: {}", tool_name)
                        }],
                        "isError": true
                    })),
                    error: None,
                }
            }
        }

        _ => JsonRpcResponse {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(JsonRpcError {
                code: -32601,
                message: format!("Method not found: {}", req.method),
                data: None,
            }),
        },
    }
}

/// Write a JSON-RPC response as a single line to stdout.
fn write_response(out: &mut impl Write, resp: &JsonRpcResponse) -> anyhow::Result<()> {
    let json = serde_json::to_string(resp)?;
    writeln!(out, "{}", json)?;
    out.flush()?;
    Ok(())
}
