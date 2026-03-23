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

/// Compiled-in fallback: the JSON is baked in at build time via rust-embed
/// (ADR-2603221522) so the MCP server works even when `config/mcp-tools.json`
/// is not on disk (e.g. installed binary).
fn builtin_tools_json() -> String {
    crate::assets::Assets::get_str("schemas/mcp-tools.json")
        .expect("mcp-tools.json must be embedded in assets/schemas/")
}

/// Load tool definitions from `config/mcp-tools.json` at runtime.
/// Falls back to the compiled-in copy if the file is missing.
///
/// Resolution order:
///   1. `$HEX_PROJECT_ROOT/config/mcp-tools.json`  (project-local override)
///   2. `<exe_dir>/../config/mcp-tools.json`         (installed layout)
///   3. Compiled-in fallback via `include_str!`
fn build_tool_list() -> Value {
    let tools_json = load_tools_json();
    let parsed: Value = serde_json::from_str(&tools_json)
        .expect("mcp-tools.json is invalid JSON");

    // Extract just the MCP-compatible fields (name, description, inputSchema)
    let tools = parsed["tools"]
        .as_array()
        .expect("mcp-tools.json must have a 'tools' array");

    let mcp_tools: Vec<Value> = tools
        .iter()
        .map(|t| {
            serde_json::json!({
                "name": t["name"],
                "description": t["description"],
                "inputSchema": t["inputSchema"],
            })
        })
        .collect();

    serde_json::json!({ "tools": mcp_tools })
}

/// Resolve and read the tools JSON, with fallback chain.
fn load_tools_json() -> String {
    // 1. Project root from env
    if let Ok(root) = std::env::var("HEX_PROJECT_ROOT") {
        let path = std::path::Path::new(&root).join("config/mcp-tools.json");
        if let Ok(content) = std::fs::read_to_string(&path) {
            eprintln!("[hex] Loaded tools from {}", path.display());
            return content;
        }
    }

    // 2. Relative to cwd (development layout)
    let cwd_path = std::path::Path::new("config/mcp-tools.json");
    if let Ok(content) = std::fs::read_to_string(cwd_path) {
        eprintln!("[hex] Loaded tools from config/mcp-tools.json");
        return content;
    }

    // 3. Relative to executable
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let path = dir.join("../config/mcp-tools.json");
            if let Ok(content) = std::fs::read_to_string(&path) {
                eprintln!("[hex] Loaded tools from {}", path.display());
                return content;
            }
        }
    }

    // 4. Compiled-in fallback (embedded via rust-embed)
    eprintln!("[hex] Using compiled-in tool definitions");
    builtin_tools_json()
}

// ─── Enforcement (ADR-2603221959) ───────────────────────

use hex_core::domain::enforcement::DefaultEnforcer;
use hex_core::ports::enforcement::{EnforcementContext, EnforcementMode, EnforcementResult, IEnforcementPort};

/// Tools that are read-only — no enforcement needed.
const READ_ONLY_TOOLS: &[&str] = &[
    "hex_analyze", "hex_analyze_json", "hex_status", "hex_hexflo_swarm_status", "hex_hexflo_task_list",
    "hex_hexflo_memory_retrieve", "hex_hexflo_memory_search",
    "hex_adr_list", "hex_adr_search", "hex_adr_status", "hex_adr_abandoned",
    "hex_plan_list", "hex_plan_status", "hex_plan_history", "hex_plan_report",
    "hex_agent_list", "hex_nexus_status", "hex_secrets_status", "hex_secrets_has",
    "hex_inference_list",
    // Enforcement (read-only queries)
    "hex_enforce_list", "hex_enforce_mode", "hex_enforce_prompt",
    // Test history (read-only)
    "hex_test_history", "hex_test_trends",
    // Project list (read-only)
    "hex_project_list",
    // Lifecycle tools — exempt because they establish the session
    "hex_session_start", "hex_session_heartbeat", "hex_workplan_activate",
    // Git queries (read-only)
    "hex_git_status", "hex_git_log", "hex_git_diff", "hex_git_branches",
    // Secrets vault read (read-only)
    "hex_secrets_vault_get",
    // Agent lifecycle queries (read-only)
    "hex_agent_id", "hex_agent_info", "hex_agent_audit",
];

/// Build enforcement context from tool name and args.
fn build_enforcement_ctx(name: &str, args: &Value) -> EnforcementContext {
    EnforcementContext {
        agent_id: resolve_mcp_agent_id(),
        workplan_id: resolve_mcp_workplan_id(),
        task_id: args.get("task_id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        operation: name.to_string(),
        target_file: args.get("path").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        ..Default::default()
    }
}

/// Resolve agent_id from session state for MCP context.
fn resolve_mcp_agent_id() -> String {
    resolve_session_agent_id().unwrap_or_default()
}

/// Resolve workplan_id from session state for MCP context.
fn resolve_mcp_workplan_id() -> String {
    let session_id = std::env::var("CLAUDE_SESSION_ID").unwrap_or_default();
    let sessions_dir = dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        .join(".hex/sessions");
    let key = if session_id.is_empty() {
        format!("agent-{}.json", std::process::id())
    } else {
        format!("agent-{}.json", &session_id)
    };
    let path = sessions_dir.join(key);
    if let Ok(content) = std::fs::read_to_string(&path) {
        if let Ok(state) = serde_json::from_str::<Value>(&content) {
            return state["workplan_id"].as_str().unwrap_or("").to_string();
        }
    }
    String::new()
}

/// Get enforcement mode from project config.
fn get_enforcement_mode() -> EnforcementMode {
    let project_dir = std::env::var("CLAUDE_PROJECT_DIR")
        .or_else(|_| std::env::var("HEX_PROJECT_DIR"))
        .unwrap_or_else(|_| ".".to_string());
    let project_json = std::path::Path::new(&project_dir).join(".hex/project.json");
    if let Ok(content) = std::fs::read_to_string(&project_json) {
        if let Ok(project) = serde_json::from_str::<Value>(&content) {
            if let Some(mode) = project["lifecycle_enforcement"].as_str() {
                return EnforcementMode::parse(mode);
            }
        }
    }
    EnforcementMode::Mandatory
}

// ─── Tool Dispatch ───────────────────────────────────────

/// Execute a tool call by delegating to the nexus REST API.
/// Returns MCP-formatted content result.
async fn dispatch_tool(nexus: &NexusClient, name: &str, args: &Value) -> Value {
    // ADR-2603221959 P2: Enforce rules before mutating tools
    if !READ_ONLY_TOOLS.contains(&name) {
        let ctx = build_enforcement_ctx(name, args);
        let enforcer = DefaultEnforcer::new(get_enforcement_mode());
        match enforcer.check(&ctx) {
            EnforcementResult::Block(reason) => {
                return serde_json::json!({
                    "type": "text",
                    "text": format!("[BLOCKED] {}", reason),
                    "isError": true,
                });
            }
            EnforcementResult::Warn(msg) => {
                eprintln!("[hex] WARNING: {}", msg);
            }
            EnforcementResult::Allow => {}
        }
    }

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

        "hex_analyze_json" => {
            let path = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
            return match nexus.post("/api/analyze", &serde_json::json!({ "path": path })).await {
                Ok(data) => serde_json::json!({
                    "content": [{ "type": "text", "text": data.to_string() }],
                    "isError": false
                }),
                Err(e) => serde_json::json!({
                    "content": [{ "type": "text", "text": format!("{{\"error\": \"{}\"}}", e) }],
                    "isError": true
                }),
            };
        }

        "hex_status" => {
            nexus.get("/api/version").await.map_err(|e| e.to_string())
        }

        // ── Swarm ──
        "hex_hexflo_swarm_init" => {
            let body = serde_json::json!({
                "project_id": args.get("project_id").and_then(|v| v.as_str()).unwrap_or("."),
                "name": args.get("name").and_then(|v| v.as_str()).unwrap_or("default"),
                "topology": args.get("topology").and_then(|v| v.as_str()).unwrap_or("hierarchical"),
            });
            nexus.post("/api/swarms", &body).await.map_err(|e| e.to_string())
        }

        "hex_hexflo_swarm_status" => {
            nexus.get("/api/swarms/active").await.map_err(|e| e.to_string())
        }

        // ── Tasks ──
        "hex_hexflo_task_create" => {
            let swarm_id = args.get("swarm_id").and_then(|v| v.as_str()).unwrap_or("");
            let title = args.get("title").and_then(|v| v.as_str()).unwrap_or("");
            let path = format!("/api/swarms/{}/tasks", swarm_id);
            nexus.post(&path, &serde_json::json!({ "title": title }))
                .await.map_err(|e| e.to_string())
        }

        "hex_hexflo_task_list" => {
            match args.get("swarm_id").and_then(|v| v.as_str()) {
                Some(id) => nexus.get(&format!("/api/swarms/{}", id)).await.map_err(|e| e.to_string()),
                None => nexus.get("/api/swarms/active").await.map_err(|e| e.to_string()),
            }
        }

        "hex_hexflo_task_assign" => {
            let task_id = args.get("task_id").and_then(|v| v.as_str()).unwrap_or("");
            let agent_id = args.get("agent_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| resolve_session_agent_id().unwrap_or_default());
            let path = format!("/api/hexflo/tasks/{}", task_id);
            nexus.patch(&path, &serde_json::json!({
                "agent_id": agent_id,
            })).await.map_err(|e| e.to_string())
        }

        "hex_hexflo_task_complete" => {
            let task_id = args.get("task_id").and_then(|v| v.as_str()).unwrap_or("");
            let result_text = args.get("result").and_then(|v| v.as_str());
            let path = format!("/api/hexflo/tasks/{}", task_id);
            nexus.patch(&path, &serde_json::json!({
                "status": "completed",
                "result": result_text,
            })).await.map_err(|e| e.to_string())
        }

        // ── Memory ──
        "hex_hexflo_memory_store" => {
            let body = serde_json::json!({
                "key": args.get("key").and_then(|v| v.as_str()).unwrap_or(""),
                "value": args.get("value").and_then(|v| v.as_str()).unwrap_or(""),
                "scope": args.get("scope").and_then(|v| v.as_str()).unwrap_or("global"),
            });
            nexus.post("/api/hexflo/memory", &body).await.map_err(|e| e.to_string())
        }

        "hex_hexflo_memory_retrieve" => {
            let key = args.get("key").and_then(|v| v.as_str()).unwrap_or("");
            nexus.get(&format!("/api/hexflo/memory/{}", key))
                .await.map_err(|e| e.to_string())
        }

        "hex_hexflo_memory_search" => {
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

        // ── Workplan management ──
        "hex_plan_list" => {
            let dir = std::path::Path::new("docs/workplans");
            let mut plans = Vec::new();
            if dir.is_dir() {
                if let Ok(entries) = std::fs::read_dir(dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
                        if path.extension().map(|e| e == "json").unwrap_or(false) {
                            if let Ok(contents) = std::fs::read_to_string(&path) {
                                if let Ok(plan) = serde_json::from_str::<Value>(&contents) {
                                    let steps = plan.get("steps").and_then(|s| s.as_array()).map(|a| a.len()).unwrap_or(0);
                                    let done = plan.get("steps").and_then(|s| s.as_array()).map(|a| {
                                        a.iter().filter(|s| s.get("status").and_then(|v| v.as_str()) == Some("completed")).count()
                                    }).unwrap_or(0);
                                    let title = plan.get("title").and_then(|t| t.as_str()).unwrap_or(&name).to_string();
                                    plans.push(serde_json::json!({ "file": name, "title": title, "steps": steps, "completed": done }));
                                }
                            }
                        }
                    }
                }
            }
            Ok(serde_json::json!({ "plans": plans }))
        }

        "hex_plan_status" => {
            let file = args.get("file").and_then(|v| v.as_str()).unwrap_or("");
            let path = std::path::Path::new("docs/workplans").join(file);
            match std::fs::read_to_string(&path) {
                Ok(contents) => serde_json::from_str::<Value>(&contents).map_err(|e| format!("Parse error: {}", e)),
                Err(e) => Err(format!("Cannot read {}: {}", path.display(), e)),
            }
        }

        // ── Workplan execution & reporting (ADR-046) ──
        "hex_plan_execute" => {
            let file = args.get("file").and_then(|v| v.as_str()).unwrap_or("");
            let body = serde_json::json!({ "workplanPath": file });
            nexus.post("/api/workplan/execute", &body).await.map_err(|e| e.to_string())
        }

        "hex_plan_pause" => {
            nexus.post("/api/workplan/pause", &serde_json::json!({})).await.map_err(|e| e.to_string())
        }

        "hex_plan_resume" => {
            nexus.post("/api/workplan/resume", &serde_json::json!({})).await.map_err(|e| e.to_string())
        }

        "hex_plan_report" => {
            let id = args.get("id").and_then(|v| v.as_str()).unwrap_or("");
            nexus.get(&format!("/api/workplan/{}/report", id)).await.map_err(|e| e.to_string())
        }

        "hex_plan_history" => {
            nexus.get("/api/workplan/list").await.map_err(|e| e.to_string())
        }

        // ── Agent lifecycle ──
        "hex_agent_connect" => {
            let hostname = gethostname::gethostname().to_string_lossy().to_string();
            let project_dir = args.get("project_dir").and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| std::env::current_dir()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default());
            let host = args.get("host").and_then(|v| v.as_str())
                .unwrap_or(&hostname);
            let body = serde_json::json!({
                "host": host,
                "name": args.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                "project_dir": project_dir,
                "model": args.get("model").and_then(|v| v.as_str()).unwrap_or(""),
                "session_id": args.get("session_id").and_then(|v| v.as_str()).unwrap_or(""),
            });
            nexus.post("/api/agents/connect", &body).await.map_err(|e| e.to_string())
        }

        "hex_agent_disconnect" => {
            let agent_id = args.get("agent_id").and_then(|v| v.as_str()).unwrap_or("");
            let body = serde_json::json!({ "agentId": agent_id });
            nexus.post("/api/agents/disconnect", &body).await.map_err(|e| e.to_string())
        }

        "hex_agent_list" => {
            nexus.get("/api/agents").await.map_err(|e| e.to_string())
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

        // ── Inference ──
        "hex_inference_add" => {
            let body = serde_json::json!({
                "id": args.get("id").and_then(|v| v.as_str()).unwrap_or(""),
                "provider": args.get("provider_type").and_then(|v| v.as_str()).unwrap_or("ollama"),
                "url": args.get("url").and_then(|v| v.as_str()).unwrap_or(""),
                "model": args.get("model").and_then(|v| v.as_str()).unwrap_or(""),
                "requires_auth": args.get("key").is_some(),
                "secret_key": args.get("key").and_then(|v| v.as_str()).unwrap_or(""),
            });
            nexus.post("/api/inference/register", &body).await.map_err(|e| e.to_string())
        }

        "hex_inference_list" => {
            nexus.get("/api/inference/endpoints").await.map_err(|e| e.to_string())
        }

        "hex_inference_test" => {
            let target = args.get("target").and_then(|v| v.as_str()).unwrap_or("");
            Ok(serde_json::json!({
                "message": format!("Run 'hex inference test {}' from the terminal for full connectivity probe", target)
            }))
        }

        "hex_inference_discover" => {
            Ok(serde_json::json!({
                "message": "Run 'hex inference discover' from the terminal for LAN scanning"
            }))
        }

        "hex_inference_remove" => {
            let provider_id = args.get("provider_id").and_then(|v| v.as_str()).unwrap_or("");
            nexus.post(
                &format!("/api/inference/providers/{}/remove", provider_id),
                &serde_json::json!({}),
            ).await.map_err(|e| e.to_string())
        }

        // ── Agent Notification Inbox (ADR-060) ──
        "hex_inbox_notify" => {
            let mut body = serde_json::json!({
                "priority": args.get("priority").and_then(|v| v.as_u64()).unwrap_or(1),
                "kind": args.get("kind").and_then(|v| v.as_str()).unwrap_or("info"),
                "payload": args.get("payload").and_then(|v| v.as_str()).unwrap_or("{}"),
            });
            if let Some(aid) = args.get("agent_id").and_then(|v| v.as_str()) {
                body["agent_id"] = serde_json::json!(aid);
            }
            if let Some(pid) = args.get("project_id").and_then(|v| v.as_str()) {
                body["project_id"] = serde_json::json!(pid);
            }
            nexus.post("/api/hexflo/inbox/notify", &body).await.map_err(|e| e.to_string())
        }

        "hex_inbox_query" => {
            let agent_id = args.get("agent_id").and_then(|v| v.as_str()).unwrap_or("");
            let min_p = args.get("min_priority").and_then(|v| v.as_u64()).unwrap_or(0);
            let unacked = args.get("unacked_only").and_then(|v| v.as_bool()).unwrap_or(true);
            let path = format!(
                "/api/hexflo/inbox/{}?min_priority={}&unacked_only={}",
                agent_id, min_p, unacked
            );
            nexus.get(&path).await.map_err(|e| e.to_string())
        }

        "hex_inbox_ack" => {
            let id = args.get("notification_id").and_then(|v| v.as_u64()).unwrap_or(0);
            let agent_id = args.get("agent_id").and_then(|v| v.as_str()).unwrap_or("");
            let path = format!("/api/hexflo/inbox/{}/ack", id);
            nexus.patch(&path, &serde_json::json!({ "agent_id": agent_id })).await.map_err(|e| e.to_string())
        }

        // ── Enforcement ──
        "hex_enforce_list" => {
            nexus.get("/api/hexflo/enforcement-rules").await.map_err(|e| e.to_string())
        }

        "hex_enforce_mode" => {
            let mode = get_enforcement_mode();
            Ok(serde_json::json!({
                "mode": format!("{:?}", mode),
                "source": ".hex/project.json → lifecycle_enforcement",
            }))
        }

        "hex_enforce_sync" => {
            // Re-use the local rule loader from enforce.rs via nexus POST
            // Read .hex/adr-rules.toml and POST each rule
            let rules_path = std::path::Path::new(".hex/adr-rules.toml");
            let alt_path = std::env::var("CLAUDE_PROJECT_DIR")
                .map(|d| std::path::PathBuf::from(d).join(".hex/adr-rules.toml"))
                .unwrap_or_default();
            let content = std::fs::read_to_string(rules_path)
                .or_else(|_| std::fs::read_to_string(&alt_path));
            match content {
                Ok(toml_str) => {
                    match toml_str.parse::<toml::Table>() {
                        Ok(parsed) => {
                            let rules = parsed.get("rules").and_then(|r| r.as_array());
                            let mut synced = 0u32;
                            let mut errors = Vec::new();
                            if let Some(rules) = rules {
                                for rule in rules {
                                    let body = serde_json::json!({
                                        "id": rule.get("id").and_then(|v| v.as_str()).unwrap_or(""),
                                        "adr": rule.get("adr").and_then(|v| v.as_str()).unwrap_or(""),
                                        "operation": "pattern_match",
                                        "condition": "pattern_match",
                                        "severity": rule.get("severity").and_then(|v| v.as_str()).unwrap_or("error"),
                                        "enabled": 1,
                                        "project_id": "",
                                        "message": rule.get("message").and_then(|v| v.as_str()).unwrap_or(""),
                                    });
                                    match nexus.post("/api/hexflo/enforcement-rules", &body).await {
                                        Ok(_) => synced += 1,
                                        Err(e) => errors.push(format!("{}", e)),
                                    }
                                }
                            }
                            let total = rules.map(|r| r.len()).unwrap_or(0);
                            Ok(serde_json::json!({
                                "synced": synced,
                                "total": total,
                                "errors": errors,
                            }))
                        }
                        Err(e) => Err(format!("Failed to parse .hex/adr-rules.toml: {}", e)),
                    }
                }
                Err(_) => Err("No .hex/adr-rules.toml found".to_string()),
            }
        }

        "hex_enforce_prompt" => {
            let mode = format!("{:?}", get_enforcement_mode()).to_lowercase();
            let is_mandatory = mode == "mandatory";
            match crate::assets::Assets::get_str("templates/enforcement-system-prompt.md") {
                Some(template) => {
                    let output = template
                        .replace("{{mode}}", &mode)
                        .replace("{{#if mandatory}}", if is_mandatory { "" } else { "<!-- " })
                        .replace("{{else}}", if is_mandatory { "<!-- " } else { "" })
                        .replace("{{/if}}", if is_mandatory { "" } else { " -->" });
                    Ok(serde_json::json!({
                        "mode": mode,
                        "prompt": output,
                    }))
                }
                None => Err("enforcement-system-prompt.md not found in embedded assets".to_string()),
            }
        }

        // ── Test history ──
        "hex_test_history" => {
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10);
            nexus.get(&format!("/api/test-sessions?limit={}", limit))
                .await.map_err(|e| e.to_string())
        }

        "hex_test_trends" => {
            let runs = args.get("runs").and_then(|v| v.as_u64()).unwrap_or(10);
            nexus.get(&format!("/api/test-sessions/trends?runs={}", runs))
                .await.map_err(|e| e.to_string())
        }

        // ── Project management ──
        "hex_project_list" => {
            nexus.get("/api/projects").await.map_err(|e| e.to_string())
        }

        "hex_project_register" => {
            let path = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
            let abs_path = std::path::Path::new(path)
                .canonicalize()
                .unwrap_or_else(|_| std::path::PathBuf::from(path));
            let mut body = serde_json::json!({ "rootPath": abs_path.display().to_string() });
            if let Some(name) = args.get("name").and_then(|v| v.as_str()) {
                body["name"] = serde_json::json!(name);
            }
            nexus.post("/api/projects/register", &body).await.map_err(|e| e.to_string())
        }

        // ── Git queries ──
        "hex_git_status" => {
            let project_id = args.get("project_id").and_then(|v| v.as_str()).unwrap_or("");
            nexus.get(&format!("/api/{}/git/status", project_id))
                .await.map_err(|e| e.to_string())
        }

        "hex_git_log" => {
            let project_id = args.get("project_id").and_then(|v| v.as_str()).unwrap_or("");
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10);
            nexus.get(&format!("/api/{}/git/log?limit={}", project_id, limit))
                .await.map_err(|e| e.to_string())
        }

        "hex_git_diff" => {
            let project_id = args.get("project_id").and_then(|v| v.as_str()).unwrap_or("");
            nexus.get(&format!("/api/{}/git/diff", project_id))
                .await.map_err(|e| e.to_string())
        }

        "hex_git_branches" => {
            let project_id = args.get("project_id").and_then(|v| v.as_str()).unwrap_or("");
            nexus.get(&format!("/api/{}/git/branches", project_id))
                .await.map_err(|e| e.to_string())
        }

        // ── Secrets (extended) ──
        "hex_secrets_grant" => {
            let body = serde_json::json!({
                "agent_id": args.get("agent_id").and_then(|v| v.as_str()).unwrap_or(""),
                "secret_key": args.get("secret_key").and_then(|v| v.as_str()).unwrap_or(""),
                "purpose": args.get("purpose").and_then(|v| v.as_str()).unwrap_or(""),
            });
            nexus.post("/secrets/grant", &body).await.map_err(|e| e.to_string())
        }

        "hex_secrets_revoke" => {
            let body = serde_json::json!({
                "grant_id": args.get("grant_id").and_then(|v| v.as_str()).unwrap_or(""),
            });
            nexus.post("/secrets/revoke", &body).await.map_err(|e| e.to_string())
        }

        "hex_secrets_vault_set" => {
            let body = serde_json::json!({
                "key": args.get("key").and_then(|v| v.as_str()).unwrap_or(""),
                "value": args.get("value").and_then(|v| v.as_str()).unwrap_or(""),
            });
            nexus.post("/api/secrets/vault", &body).await.map_err(|e| e.to_string())
        }

        "hex_secrets_vault_get" => {
            let key = args.get("key").and_then(|v| v.as_str()).unwrap_or("");
            nexus.get(&format!("/api/secrets/vault/{}", key))
                .await.map_err(|e| e.to_string())
        }

        // ── Agent lifecycle (extended) ──
        "hex_agent_id" => {
            match resolve_session_agent_id() {
                Some(id) => Ok(serde_json::json!({ "agent_id": id })),
                None => Err("No agent_id found in session state. Call hex_session_start or run hex hook session-start first.".to_string()),
            }
        }

        "hex_agent_info" => {
            let agent_id = args.get("agent_id").and_then(|v| v.as_str()).unwrap_or("");
            nexus.get(&format!("/api/hex-agents/{}", agent_id))
                .await.map_err(|e| e.to_string())
        }

        "hex_agent_audit" => {
            // Cross-reference git log Co-Authored-By commits with HexFlo completed tasks
            let agent_id = args.get("agent_id").and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| resolve_session_agent_id().unwrap_or_default());
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(20);

            // Get completed tasks from HexFlo
            let tasks = nexus.get("/api/swarms/active").await.unwrap_or(serde_json::json!({}));

            // Get recent git log looking for Co-Authored-By or agent references
            let git_output = std::process::Command::new("git")
                .args(["log", &format!("--max-count={}", limit), "--pretty=format:%H|%s|%an|%ae|%b", "--no-merges"])
                .output();

            let mut commits = Vec::new();
            if let Ok(output) = git_output {
                let stdout = String::from_utf8_lossy(&output.stdout);
                for line in stdout.lines() {
                    let parts: Vec<&str> = line.splitn(5, '|').collect();
                    if parts.len() >= 4 {
                        let body = parts.get(4).unwrap_or(&"");
                        let is_agent_commit = body.contains("Co-Authored-By:") || body.contains(&agent_id);
                        if is_agent_commit || agent_id.is_empty() {
                            commits.push(serde_json::json!({
                                "hash": parts[0],
                                "subject": parts[1],
                                "author": parts[2],
                                "email": parts[3],
                                "agent_attributed": is_agent_commit,
                            }));
                        }
                    }
                }
            }

            Ok(serde_json::json!({
                "agent_id": agent_id,
                "commits": commits,
                "tasks": tasks,
            }))
        }

        // ── Provider-agnostic lifecycle tools (ADR-2603221959 P3) ──
        // These replace Claude Code hooks for non-Claude providers.

        "hex_session_start" => {
            let hostname = gethostname::gethostname().to_string_lossy().to_string();
            let project_dir = args.get("project_dir").and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| std::env::current_dir()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default());
            let model = args.get("model").and_then(|v| v.as_str()).unwrap_or("unknown");
            let body = serde_json::json!({
                "host": hostname,
                "name": format!("agent-{}", &hostname),
                "project_dir": project_dir,
                "model": model,
            });
            match nexus.post("/api/hex-agents/connect", &body).await {
                Ok(data) => {
                    // Also fetch active swarms and workplan context
                    let swarms = nexus.get("/api/swarms/active").await.unwrap_or(serde_json::json!({}));
                    Ok(serde_json::json!({
                        "agent_id": data["agentId"],
                        "session": "started",
                        "active_swarms": swarms.get("swarms").unwrap_or(&serde_json::json!([])),
                        "enforcement_mode": format!("{:?}", get_enforcement_mode()),
                    }))
                }
                Err(e) => Err(format!("Session start failed: {}", e)),
            }
        }

        "hex_session_heartbeat" => {
            let agent_id = resolve_mcp_agent_id();
            if agent_id.is_empty() {
                Ok(serde_json::json!({ "warning": "No agent registered — call hex_session_start first" }))
            } else {
                let _ = nexus.post("/api/hex-agents/heartbeat", &serde_json::json!({
                    "agent_id": agent_id,
                })).await;
                // Check inbox
                let inbox = nexus.get(&format!("/api/hexflo/inbox?agent_id={}", agent_id))
                    .await.unwrap_or(serde_json::json!({ "notifications": [] }));
                Ok(serde_json::json!({
                    "heartbeat": "sent",
                    "notifications": inbox.get("notifications").unwrap_or(&serde_json::json!([])),
                }))
            }
        }

        "hex_workplan_activate" => {
            let workplan_id = args.get("workplan_id").and_then(|v| v.as_str()).unwrap_or("");
            if workplan_id.is_empty() {
                Err("workplan_id is required".to_string())
            } else {
                // Store in HexFlo memory so enforcement can read it
                let _ = nexus.post("/api/hexflo/memory", &serde_json::json!({
                    "key": "active_workplan",
                    "value": workplan_id,
                    "scope": "session",
                })).await;
                // Try to load workplan details
                let details = nexus.get(&format!("/api/workplan/{}", workplan_id))
                    .await.unwrap_or(serde_json::json!({}));
                Ok(serde_json::json!({
                    "workplan_id": workplan_id,
                    "activated": true,
                    "details": details,
                }))
            }
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

/// Resolve the current session's agent_id from the persisted session state file.
/// Delegates to the canonical resolution in nexus_client (ADR-065 §4).
fn resolve_session_agent_id() -> Option<String> {
    crate::nexus_client::read_session_agent_id()
}
