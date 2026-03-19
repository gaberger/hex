//! MCP server command.
//!
//! `hex mcp` — starts a Model Context Protocol server on stdio transport.
//! All tools delegate to the hex-nexus REST API via `NexusClient`, ensuring
//! MCP tools, CLI commands, and nexus endpoints all share the same backend.
//!
//! Tool naming: `hex_<command>` — 1:1 with CLI subcommands.

use std::io::{self, BufRead, Write};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::nexus_client::NexusClient;

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

// ─── Tool Definitions ────────────────────────────────────

/// Build the complete tool list. Each tool maps 1:1 to a CLI command
/// or nexus endpoint — no phantom tools.
fn build_tool_list() -> Value {
    serde_json::json!({
        "tools": [
            // ── Analysis ──
            {
                "name": "hex_analyze",
                "description": "Architecture health check: boundary violations, dead exports, circular deps",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Project root path to analyze" }
                    },
                    "required": ["path"]
                }
            },
            {
                "name": "hex_status",
                "description": "Show project status and service health",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            },
            // ── Swarm coordination ──
            {
                "name": "hex_swarm_init",
                "description": "Initialize a new swarm for coordinated multi-agent work",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "Swarm name" },
                        "project_id": { "type": "string", "description": "Project identifier" },
                        "topology": { "type": "string", "description": "Topology: hierarchical, mesh, star" }
                    },
                    "required": ["name", "project_id"]
                }
            },
            {
                "name": "hex_swarm_status",
                "description": "Show active swarms and their status",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            },
            // ── Task management ──
            {
                "name": "hex_task_create",
                "description": "Create a task in a swarm",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "swarm_id": { "type": "string", "description": "Swarm ID to add task to" },
                        "title": { "type": "string", "description": "Task title/description" }
                    },
                    "required": ["swarm_id", "title"]
                }
            },
            {
                "name": "hex_task_list",
                "description": "List tasks in a swarm",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "swarm_id": { "type": "string", "description": "Swarm ID (omit for all)" }
                    },
                    "required": []
                }
            },
            {
                "name": "hex_task_complete",
                "description": "Mark a task as completed",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "task_id": { "type": "string", "description": "Task ID to complete" },
                        "result": { "type": "string", "description": "Completion result/summary" }
                    },
                    "required": ["task_id"]
                }
            },
            // ── Memory ──
            {
                "name": "hex_memory_store",
                "description": "Store a key-value pair in persistent memory",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "key": { "type": "string", "description": "Memory key" },
                        "value": { "type": "string", "description": "Value to store" },
                        "scope": { "type": "string", "description": "Scope: global, swarm, agent" }
                    },
                    "required": ["key", "value"]
                }
            },
            {
                "name": "hex_memory_retrieve",
                "description": "Retrieve a value by key from persistent memory",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "key": { "type": "string", "description": "Memory key to retrieve" }
                    },
                    "required": ["key"]
                }
            },
            {
                "name": "hex_memory_search",
                "description": "Search persistent memory by query",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Search query" }
                    },
                    "required": ["query"]
                }
            },
            // ── ADR lifecycle ──
            {
                "name": "hex_adr_list",
                "description": "List Architecture Decision Records",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "status": { "type": "string", "description": "Filter by status: proposed, accepted, deprecated, superseded" }
                    },
                    "required": []
                }
            },
            {
                "name": "hex_adr_search",
                "description": "Search ADRs by keyword",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Search query" },
                        "limit": { "type": "integer", "description": "Max results (default 10)" }
                    },
                    "required": ["query"]
                }
            },
            {
                "name": "hex_adr_status",
                "description": "Show detailed status of a specific ADR",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "description": "ADR ID (e.g. ADR-027)" }
                    },
                    "required": ["id"]
                }
            },
            {
                "name": "hex_adr_abandoned",
                "description": "Find stale/abandoned proposed ADRs",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "days": { "type": "integer", "description": "Days without update to consider abandoned (default 14)" }
                    },
                    "required": []
                }
            },
            // ── Nexus daemon ──
            {
                "name": "hex_nexus_status",
                "description": "Check hex-nexus daemon status",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            },
            {
                "name": "hex_nexus_start",
                "description": "Start the hex-nexus daemon",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            },
            // ── Secrets ──
            {
                "name": "hex_secrets_status",
                "description": "Show secrets backend status",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            },
            {
                "name": "hex_secrets_has",
                "description": "Check if a secret key exists",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "key": { "type": "string", "description": "Secret key to check" }
                    },
                    "required": ["key"]
                }
            }
        ]
    })
}

// ─── Tool Dispatch ───────────────────────────────────────

/// Execute a tool call by delegating to the nexus REST API.
/// Returns MCP-formatted content result.
async fn dispatch_tool(nexus: &NexusClient, name: &str, args: &Value) -> Value {
    let result = match name {
        // ── Analysis ──
        "hex_analyze" => {
            let path = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
            // Try nexus first, fall back to offline analysis
            match nexus.post("/api/analyze", &serde_json::json!({ "path": path })).await {
                Ok(data) => Ok(data),
                Err(_) => {
                    // Run offline analysis via the analyze module
                    // Capture that nexus isn't available
                    Err(format!(
                        "hex-nexus not running. Start with: hex nexus start\n\
                         For offline analysis, run: hex analyze {}",
                        path
                    ))
                }
            }
        }

        "hex_status" => {
            nexus.get("/api/version").await.map_err(|e| e.to_string())
        }

        // ── Swarm ──
        "hex_swarm_init" => {
            let body = serde_json::json!({
                "project_id": args.get("project_id").and_then(|v| v.as_str()).unwrap_or("."),
                "name": args.get("name").and_then(|v| v.as_str()).unwrap_or("default"),
                "topology": args.get("topology").and_then(|v| v.as_str()).unwrap_or("hierarchical"),
            });
            nexus.post("/api/swarms", &body).await.map_err(|e| e.to_string())
        }

        "hex_swarm_status" => {
            nexus.get("/api/swarms").await.map_err(|e| e.to_string())
        }

        // ── Tasks ──
        "hex_task_create" => {
            let swarm_id = args.get("swarm_id").and_then(|v| v.as_str()).unwrap_or("");
            let title = args.get("title").and_then(|v| v.as_str()).unwrap_or("");
            let path = format!("/api/swarms/{}/tasks", swarm_id);
            nexus.post(&path, &serde_json::json!({ "title": title }))
                .await.map_err(|e| e.to_string())
        }

        "hex_task_list" => {
            match args.get("swarm_id").and_then(|v| v.as_str()) {
                Some(id) => nexus.get(&format!("/api/swarms/{}", id)).await.map_err(|e| e.to_string()),
                None => nexus.get("/api/swarms").await.map_err(|e| e.to_string()),
            }
        }

        "hex_task_complete" => {
            let task_id = args.get("task_id").and_then(|v| v.as_str()).unwrap_or("");
            let result_text = args.get("result").and_then(|v| v.as_str());
            let path = format!("/api/swarms/tasks/{}", task_id);
            nexus.patch(&path, &serde_json::json!({
                "status": "completed",
                "result": result_text,
            })).await.map_err(|e| e.to_string())
        }

        // ── Memory ──
        "hex_memory_store" => {
            let body = serde_json::json!({
                "key": args.get("key").and_then(|v| v.as_str()).unwrap_or(""),
                "value": args.get("value").and_then(|v| v.as_str()).unwrap_or(""),
                "scope": args.get("scope").and_then(|v| v.as_str()).unwrap_or("global"),
            });
            nexus.post("/api/hexflo/memory", &body).await.map_err(|e| e.to_string())
        }

        "hex_memory_retrieve" => {
            let key = args.get("key").and_then(|v| v.as_str()).unwrap_or("");
            nexus.get(&format!("/api/hexflo/memory/{}", key))
                .await.map_err(|e| e.to_string())
        }

        "hex_memory_search" => {
            let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
            nexus.get(&format!("/api/hexflo/memory/search?q={}", query))
                .await.map_err(|e| e.to_string())
        }

        // ── ADR ──
        "hex_adr_list" => {
            // ADR commands work on local filesystem, not nexus
            // For now delegate to nexus project endpoint if available
            let status_filter = args.get("status").and_then(|v| v.as_str()).unwrap_or("");
            let path = if status_filter.is_empty() {
                "/api/adrs".to_string()
            } else {
                format!("/api/adrs?status={}", status_filter)
            };
            nexus.get(&path).await.map_err(|e| e.to_string())
        }

        "hex_adr_search" => {
            let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10);
            nexus.get(&format!("/api/adrs/search?q={}&limit={}", query, limit))
                .await.map_err(|e| e.to_string())
        }

        "hex_adr_status" => {
            let id = args.get("id").and_then(|v| v.as_str()).unwrap_or("");
            nexus.get(&format!("/api/adrs/{}", id))
                .await.map_err(|e| e.to_string())
        }

        "hex_adr_abandoned" => {
            let days = args.get("days").and_then(|v| v.as_u64()).unwrap_or(14);
            nexus.get(&format!("/api/adrs/abandoned?days={}", days))
                .await.map_err(|e| e.to_string())
        }

        // ── Nexus daemon ──
        "hex_nexus_status" => {
            nexus.get("/api/version").await.map_err(|e| e.to_string())
        }

        "hex_nexus_start" => {
            // Cannot start nexus from within MCP (we ARE running inside hex)
            // Return guidance instead
            Ok(serde_json::json!({
                "message": "Run 'hex nexus start' from the terminal to start the daemon"
            }))
        }

        // ── Secrets ──
        "hex_secrets_status" => {
            nexus.get("/api/secrets/status").await
                .or_else(|_| Ok::<Value, String>(serde_json::json!({
                    "backend": "env",
                    "message": "Secrets available via environment variables"
                })))
                .map_err(|e: String| e)
        }

        "hex_secrets_has" => {
            let key = args.get("key").and_then(|v| v.as_str()).unwrap_or("");
            let exists = std::env::var(key).is_ok();
            Ok(serde_json::json!({ "key": key, "exists": exists }))
        }

        _ => Err(format!("Unknown tool: {}", name)),
    };

    match result {
        Ok(data) => serde_json::json!({
            "content": [{ "type": "text", "text": serde_json::to_string_pretty(&data).unwrap_or_default() }],
            "isError": false
        }),
        Err(msg) => serde_json::json!({
            "content": [{ "type": "text", "text": msg }],
            "isError": true
        }),
    }
}

// ─── Server Loop ─────────────────────────────────────────

/// Start the hex MCP server on stdio transport.
///
/// Reads JSON-RPC messages from stdin (one per line), dispatches them, and
/// writes responses to stdout. Tools delegate to hex-nexus via NexusClient.
pub async fn run_mcp_server() -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    let tool_list = build_tool_list();
    let tool_count = tool_list["tools"].as_array().map(|a| a.len()).unwrap_or(0);

    eprintln!("[hex] MCP server starting on stdio ({} tools)", tool_count);

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut stdout_lock = stdout.lock();

    for line_result in stdin.lock().lines() {
        let line = match line_result {
            Ok(l) => l,
            Err(_) => break,
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

        let _ = &req.jsonrpc;
        let id = req.id.clone().unwrap_or(Value::Null);

        let response = match req.method.as_str() {
            "initialize" => JsonRpcResponse {
                jsonrpc: "2.0",
                id,
                result: Some(serde_json::json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": { "tools": { "listChanged": false } },
                    "serverInfo": {
                        "name": "hex",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                })),
                error: None,
            },

            "initialized" => JsonRpcResponse {
                jsonrpc: "2.0",
                id,
                result: Some(Value::Null),
                error: None,
            },

            "tools/list" => JsonRpcResponse {
                jsonrpc: "2.0",
                id,
                result: Some(tool_list.clone()),
                error: None,
            },

            "tools/call" => {
                let tool_name = req.params.get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("unknown");
                let args = req.params.get("arguments")
                    .cloned()
                    .unwrap_or(Value::Object(Default::default()));

                let content = dispatch_tool(&nexus, tool_name, &args).await;

                JsonRpcResponse {
                    jsonrpc: "2.0",
                    id,
                    result: Some(content),
                    error: None,
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
        };

        write_response(&mut stdout_lock, &response)?;
    }

    eprintln!("[hex] MCP server shutting down.");
    Ok(())
}

/// Write a JSON-RPC response as a single line to stdout.
fn write_response(out: &mut impl Write, resp: &JsonRpcResponse) -> anyhow::Result<()> {
    let json = serde_json::to_string(resp)?;
    writeln!(out, "{}", json)?;
    out.flush()?;
    Ok(())
}
