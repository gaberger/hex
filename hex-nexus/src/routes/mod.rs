pub mod adrs;
pub mod agents;
pub mod browse;
pub mod files;
pub mod stdb;
pub mod analysis;
pub mod chat;
pub mod commands;
pub mod coordination;
pub mod decisions;
pub mod fleet;
pub mod git;
pub mod hex_agents;
pub mod hexflo;
pub mod inference;
pub mod metrics;
pub mod orchestration;
pub mod projects;
pub mod push;
pub mod quality;
pub mod query;
pub mod rl;
pub mod secrets;
pub mod sessions;
pub mod swarms;
pub mod neural_lab;
pub mod test_sessions;
pub mod openapi;
pub mod command_sessions;
pub mod exec;
pub mod inbox;
pub mod sandbox;
pub mod skills;
pub mod ws;
pub mod context;
pub mod inference_ws;

use axum::{Router, Json, routing::{get, post, patch, delete}, extract::DefaultBodyLimit};
use axum::response::{IntoResponse, Redirect};
use tower_http::cors::{CorsLayer, AllowOrigin};
use http::{HeaderValue, Method};
use serde_json::json;
use utoipa::OpenApi;
use crate::state::SharedState;
use crate::middleware::agent_guard::agent_guard;
use crate::middleware::auth::auth_layer;
use crate::middleware::deprecation::deprecation_layer;
use crate::middleware::enforcement::enforcement_layer;
use crate::embed::{serve_index, serve_chat, serve_legacy_dashboard, serve_static};

// ── OpenAPI Spec (ADR-039) ─────────────────────────────
#[derive(OpenApi)]
#[openapi(
    info(
        title = "hex-nexus API",
        version = "0.1.0",
        description = "Orchestration nexus for hex — agent management, architecture analysis, and swarm coordination.\n\nStateless compute routes are documented here. State-read routes (agents list, swarms, sessions, inference) are deprecated and will migrate to SpacetimeDB direct subscriptions (ADR-039).",
    ),
    paths(
        analysis::analyze_path,
        analysis::analyze_current_project,
        analysis::analyze_project,
        orchestration::spawn_agent,
        orchestration::list_agents,
        orchestration::execute_workplan,
    ),
    components(schemas(
        analysis::AnalyzeRequest,
        analysis::AnalyzeResponse,
        orchestration::SpawnRequest,
        orchestration::ExecuteWorkplanRequest,
    )),
    tags(
        (name = "analysis", description = "Architecture analysis (stateless compute)"),
        (name = "agents", description = "Agent lifecycle management"),
        (name = "workplan", description = "Workplan execution"),
    )
)]
struct ApiDoc;

/// GET /api/openapi.json — serve the OpenAPI 3.1 spec as JSON.
async fn openapi_json() -> impl IntoResponse {
    Json(ApiDoc::openapi())
}

/// GET /api/docs — redirect to Swagger UI (hosted) pointed at our spec.
async fn openapi_docs_redirect() -> Redirect {
    // Use the public Swagger UI petstore-style redirect with our local spec URL.
    // In production the SolidJS frontend consumes /api/openapi.json directly.
    Redirect::temporary("https://petstore.swagger.io/?url=http://localhost:5555/api/openapi.json")
}

/// DEPRECATED(ADR-065): /api/agents/connect → /api/hex-agents/connect
/// Forwards the request body to the unified endpoint. Returns Deprecation + Sunset headers.
async fn deprecated_agents_connect(
    state: axum::extract::State<crate::state::SharedState>,
    body: Json<hex_agents::ConnectRequest>,
) -> impl IntoResponse {
    tracing::warn!("DEPRECATED: /api/agents/connect called — use /api/hex-agents/connect instead");
    match hex_agents::connect_agent(state, body).await {
        Ok((status, json)) => (
            status,
            [
                ("Deprecation", "true"),
                ("Sunset", "2026-04-30"),
                ("Link", "</api/hex-agents/connect>; rel=\"successor-version\""),
            ],
            json,
        ).into_response(),
        Err((status, json)) => (status, json).into_response(),
    }
}

async fn get_version() -> Json<serde_json::Value> {
    Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "buildHash": env!("HEX_HUB_BUILD_HASH"),
        "name": "hex-hub",
    }))
}

/// GET /api/health — lightweight health check for hooks and CLI.
/// Returns nexus status and SpacetimeDB connectivity.
async fn get_health(
    axum::extract::State(state): axum::extract::State<SharedState>,
) -> Json<serde_json::Value> {
    let stdb_connected = state.agent_manager.is_some();
    Json(json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "spacetimedb": stdb_connected,
    }))
}

/// GET /api/workplans — read docs/workplans/*.json files and return summaries.
/// Gives dashboard visibility into workplan definitions on disk (distinct from
/// /api/workplan/list which tracks SpacetimeDB execution state).
async fn workplan_files() -> Json<serde_json::Value> {
    // Try cwd first, then HEX_PROJECT_ROOT
    let roots = [
        std::env::current_dir().ok(),
        std::env::var("HEX_PROJECT_ROOT")
            .ok()
            .map(std::path::PathBuf::from),
    ];

    for root in roots.iter().flatten() {
        let dir = root.join("docs/workplans");
        if !dir.is_dir() {
            continue;
        }

        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };

        let mut workplans = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let filename = entry.file_name().to_string_lossy().to_string();
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&content) {
                    let phase_count = parsed
                        .get("phases")
                        .and_then(|p| p.as_array())
                        .map(|a| a.len())
                        .unwrap_or(0);

                    let task_count: usize = parsed
                        .get("phases")
                        .and_then(|p| p.as_array())
                        .map(|phases| {
                            phases
                                .iter()
                                .filter_map(|ph| ph.get("tasks").and_then(|t| t.as_array()))
                                .map(|tasks| tasks.len())
                                .sum()
                        })
                        .unwrap_or(0);

                    workplans.push(json!({
                        "file": filename,
                        "id": parsed.get("id").and_then(|v| v.as_str()).unwrap_or(""),
                        "title": parsed.get("title").and_then(|v| v.as_str()).unwrap_or(""),
                        "priority": parsed.get("priority").and_then(|v| v.as_str()).unwrap_or(""),
                        "created_at": parsed.get("created_at").and_then(|v| v.as_str()).unwrap_or(""),
                        "phases": phase_count,
                        "tasks": task_count,
                        "related_adrs": parsed.get("related_adrs").cloned().unwrap_or(json!([])),
                    }));
                }
            }
        }

        workplans.sort_by(|a, b| {
            let pa = a["priority"].as_str().unwrap_or("");
            let pb = b["priority"].as_str().unwrap_or("");
            let order = |p: &str| match p {
                "critical" => 0,
                "high" => 1,
                "medium" => 2,
                "low" => 3,
                _ => 4,
            };
            order(pa).cmp(&order(pb))
        });

        return Json(json!({
            "ok": true,
            "count": workplans.len(),
            "workplans": workplans,
        }));
    }

    Json(json!({ "ok": false, "count": 0, "workplans": [], "error": "docs/workplans/ not found" }))
}

/// GET /api/projects/{id}/workplans — list workplan files from a project's docs/workplans/ directory.
/// Accepts optional `?root=/abs/path` query param as fallback when project lookup fails.
async fn project_workplan_files(
    axum::extract::Path(project_id): axum::extract::Path<String>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
    axum::extract::State(state): axum::extract::State<crate::state::SharedState>,
) -> Json<serde_json::Value> {
    // Resolve project root: try state port first, then ?root= fallback
    let root_path = match state.state_port.as_ref() {
        Some(sp) => match sp.project_find(&project_id).await {
            Ok(Some(p)) => p.root_path,
            _ => match params.get("root") {
                Some(r) if !r.is_empty() && std::path::Path::new(r).is_dir() => r.clone(),
                _ => return Json(json!({ "ok": false, "count": 0, "workplans": [], "error": format!("Project '{}' not found", project_id) })),
            },
        },
        None => match params.get("root") {
            Some(r) if !r.is_empty() && std::path::Path::new(r).is_dir() => r.clone(),
            _ => return Json(json!({ "ok": false, "count": 0, "workplans": [], "error": "State port not configured" })),
        },
    };

    let dir = std::path::PathBuf::from(&root_path).join("docs/workplans");
    if !dir.is_dir() {
        return Json(json!({ "ok": false, "count": 0, "workplans": [], "error": "docs/workplans/ not found" }));
    }

    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return Json(json!({ "ok": false, "count": 0, "workplans": [], "error": "Cannot read docs/workplans/" })),
    };

    let mut workplans = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let filename = entry.file_name().to_string_lossy().to_string();
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&content) {
                let phase_count = parsed.get("phases").and_then(|p| p.as_array()).map(|a| a.len()).unwrap_or(0);
                let task_count: usize = parsed.get("phases").and_then(|p| p.as_array())
                    .map(|phases| phases.iter().filter_map(|ph| ph.get("tasks").and_then(|t| t.as_array())).map(|t| t.len()).sum())
                    .unwrap_or(0);
                workplans.push(json!({
                    "file": filename,
                    "id": parsed.get("id").and_then(|v| v.as_str()).unwrap_or(""),
                    "title": parsed.get("title").and_then(|v| v.as_str()).unwrap_or(""),
                    "status": parsed.get("status").and_then(|v| v.as_str()).unwrap_or("active"),
                    "priority": parsed.get("priority").and_then(|v| v.as_str()).unwrap_or(""),
                    "created_at": parsed.get("created_at").and_then(|v| v.as_str()).unwrap_or(""),
                    "phases": phase_count,
                    "tasks": task_count,
                    "related_adrs": parsed.get("related_adrs").cloned().unwrap_or(json!([])),
                }));
            }
        }
    }

    workplans.sort_by(|a, b| {
        let pa = a["priority"].as_str().unwrap_or("");
        let pb = b["priority"].as_str().unwrap_or("");
        let order = |p: &str| match p { "critical" => 0, "high" => 1, "medium" => 2, "low" => 3, _ => 4 };
        order(pa).cmp(&order(pb))
    });

    Json(json!({ "ok": true, "count": workplans.len(), "workplans": workplans }))
}

/// GET /api/adr/next — return the next available ADR number (atomic read).
async fn adr_next_number() -> Json<serde_json::Value> {
    let number = scan_next_adr_number().await;
    Json(json!({ "next_number": number }))
}

/// POST /api/adr/reserve — atomically reserve the next ADR number.
/// Writes a placeholder file to prevent collisions between concurrent agents.
async fn adr_reserve_number(
    Json(body): Json<serde_json::Value>,
) -> (http::StatusCode, Json<serde_json::Value>) {
    let requested = body["number"].as_u64().unwrap_or(0) as u32;
    let next = scan_next_adr_number().await;

    // Use the higher of requested or scanned to avoid collisions
    let number = std::cmp::max(requested, next);

    // Write a placeholder to reserve the number
    let roots = [
        std::env::current_dir().ok(),
        std::env::var("HEX_PROJECT_ROOT").ok().map(std::path::PathBuf::from),
    ];

    for root in roots.iter().flatten() {
        let dir = root.join("docs/adrs");
        if !dir.is_dir() {
            continue;
        }
        let placeholder = dir.join(format!("ADR-{:03}-reserved.md", number));
        if !placeholder.exists() {
            let content = format!(
                "# ADR-{:03}: Reserved\n\n**Status:** Proposed\n**Date:** {}\n**Reserved-By:** hex-agent\n\nThis number has been reserved. Replace this file with the actual ADR.\n",
                number,
                chrono::Utc::now().format("%Y-%m-%d"),
            );
            let _ = std::fs::write(&placeholder, content);
        }
        return (
            http::StatusCode::OK,
            Json(json!({ "ok": true, "reserved_number": number, "placeholder": placeholder.to_string_lossy() })),
        );
    }

    (
        http::StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "ok": false, "error": "docs/adrs/ not found" })),
    )
}

/// Scan docs/adrs/ to find the highest ADR number and return next.
async fn scan_next_adr_number() -> u32 {
    let roots = [
        std::env::current_dir().ok(),
        std::env::var("HEX_PROJECT_ROOT").ok().map(std::path::PathBuf::from),
    ];

    let mut max_num: u32 = 0;
    for root in roots.iter().flatten() {
        let dir = root.join("docs/adrs");
        if !dir.is_dir() {
            continue;
        }
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if let Some(rest) = name.strip_prefix("ADR-").or_else(|| name.strip_prefix("adr-")) {
                    if let Some(num_str) = rest.split('-').next() {
                        if let Ok(num) = num_str.parse::<u32>() {
                            if num > max_num {
                                max_num = num;
                            }
                        }
                    }
                }
            }
        }
        break; // Use first found directory
    }
    max_num + 1
}

/// GET /api/tools — serve MCP tool definitions from config/mcp-tools.json.
/// Falls back to an empty list if the file is not found.
async fn tools_registry() -> Json<serde_json::Value> {
    // Try cwd first, then HEX_PROJECT_ROOT
    let paths = [
        std::path::PathBuf::from("config/mcp-tools.json"),
        std::env::var("HEX_PROJECT_ROOT")
            .map(|r| std::path::PathBuf::from(r).join("config/mcp-tools.json"))
            .unwrap_or_default(),
    ];

    for path in &paths {
        if let Ok(content) = std::fs::read_to_string(path) {
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&content) {
                return Json(json!({
                    "ok": true,
                    "version": parsed.get("version").and_then(|v| v.as_str()).unwrap_or("0.0.0"),
                    "tools": parsed.get("tools").cloned().unwrap_or(json!([])),
                    "count": parsed.get("tools").and_then(|t| t.as_array()).map(|a| a.len()).unwrap_or(0),
                }));
            }
        }
    }

    Json(json!({ "ok": false, "tools": [], "count": 0, "error": "config/mcp-tools.json not found" }))
}

const PUSH_BODY_LIMIT: usize = 256 * 1024;  // 256KB for /api/push
const EVENT_BODY_LIMIT: usize = 16 * 1024;  // 16KB for /api/event
const SMALL_BODY_LIMIT: usize = 4 * 1024;   // 4KB for register/decisions

pub fn build_router(state: SharedState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(|origin: &HeaderValue, _| {
            let s = origin.to_str().unwrap_or("");
            is_local_origin(s)
        }))
        .allow_methods([Method::GET, Method::POST, Method::PUT, Method::PATCH, Method::DELETE, Method::OPTIONS])
        .allow_headers([
            http::header::CONTENT_TYPE,
            http::header::AUTHORIZATION,
        ]);

    let router = Router::new()
        // OpenAPI spec (ADR-039) — JSON spec + docs redirect
        .route("/api/openapi.json", get(openapi_json))
        .route("/api/docs", get(openapi_docs_redirect))
        // Static + version
        .route("/", get(serve_index))
        .route("/chat", get(serve_chat))
        .route("/dashboard", get(serve_legacy_dashboard))
        .route("/assets/{*path}", get(serve_static))
        .route("/api/version", get(get_version))
        .route("/api/health", get(get_health))
        // Project management
        .route("/api/projects", get(projects::list_projects))
        .route("/api/projects/register", post(projects::register)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/projects/init", post(files::init_project)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/projects/{id}", delete(projects::unregister))
        .route("/api/projects/{id}/archive", post(projects::archive_project)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/projects/{id}/delete", post(projects::delete_project)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        // Push (projects → hub) — size-limited
        .route("/api/push", post(push::push_state)
            .layer(DefaultBodyLimit::max(PUSH_BODY_LIMIT)))
        .route("/api/event", post(push::push_event)
            .layer(DefaultBodyLimit::max(EVENT_BODY_LIMIT)))
        // Per-project queries (browser reads)
        .route("/api/{project_id}/health", get(query::get_health))
        .route("/api/{project_id}/tokens/overview", get(query::get_tokens_overview))
        .route("/api/{project_id}/tokens/{*file}", get(query::get_token_file))
        .route("/api/{project_id}/swarm", get(query::get_swarm))
        .route("/api/{project_id}/graph", get(query::get_graph))
        .route("/api/{project_id}/project", get(query::get_project))
        // ═══════════════════════════════════════════════════════════
        // META — API metadata
        // ═══════════════════════════════════════════════════════════
        // NOTE: /api/openapi.json already registered above (line 98)

        // ═══════════════════════════════════════════════════════════
        // STATELESS COMPUTE — these routes stay (filesystem + process mgmt)
        // ═══════════════════════════════════════════════════════════

        // Architecture analysis (ADR-034) — on-demand, native tree-sitter
        .route("/api/analyze", get(analysis::analyze_current_project)
            .post(analysis::analyze_path)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/{project_id}/analyze", get(analysis::analyze_project))
        // ADR compliance (ADR-045) — check code against accepted ADRs
        .route("/api/analyze/adr-compliance", post(analysis::analyze_adr_compliance)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        // ADR number reservation (atomic next-number for multi-agent coordination)
        .route("/api/adr/reserve", post(adr_reserve_number))
        .route("/api/adr/next", get(adr_next_number))
        .route("/api/exec", post(exec::exec_handler))
        // Commands (browser/MCP → hub → project, bidirectional)
        .route("/api/{project_id}/command", post(commands::send_command)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/{project_id}/command/{command_id}", get(commands::get_command))
        .route("/api/{project_id}/command/{command_id}/result", post(commands::report_result)
            .layer(DefaultBodyLimit::max(PUSH_BODY_LIMIT)))
        .route("/api/{project_id}/commands", get(commands::list_commands))
        // Decisions (browser → hub → WS)
        .route("/api/{project_id}/decisions/{decision_id}", post(decisions::handle_decision))
        // ═══════════════════════════════════════════════════════════
        // DEPRECATED STATE ROUTES — migrate to SpacetimeDB subscriptions
        // These routes add X-Deprecated headers via deprecation_layer.
        // Will be gated behind `--legacy` flag in a future release.
        // ═══════════════════════════════════════════════════════════

        // SpacetimeDB hydration + health
        .route("/api/stdb/hydrate", post(stdb::hydrate)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/stdb/health", get(stdb::health))

        // Swarm + HexFlo routes — guarded: only registered agents can mutate
        .route("/api/swarms", post(swarms::create_swarm)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/swarms/active", get(swarms::list_active_swarms))
        .route("/api/swarms/failed", get(swarms::list_failed_swarms))
        .route("/api/swarms/all", get(swarms::list_all_swarms))
        .route("/api/swarms/{id}", get(swarms::get_swarm)
            .patch(swarms::complete_swarm))
        .route("/api/swarms/{id}/fail", post(swarms::fail_swarm)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/swarms/{id}/transfer", post(swarms::transfer_swarm)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/swarms/{id}/tasks", post(swarms::create_task)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/swarms/{id}/tasks/{task_id}", patch(swarms::update_task)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        // Daemon worker task claiming (must be before /{task_id} to avoid shadowing)
        .route("/api/hexflo/tasks/claim", get(swarms::claim_task))
        // Convenience route for MCP tools (no swarm ID needed — task ID is globally unique)
        .route("/api/hexflo/tasks/{task_id}", get(swarms::get_task_by_id)
            .patch(swarms::update_task_by_id)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/work-items/incomplete", get(swarms::get_incomplete_work))
        // Neural Lab (architecture search)
        .route("/api/neural-lab/configs", get(neural_lab::list_configs)
            .post(neural_lab::create_config)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/neural-lab/configs/{id}", get(neural_lab::get_config))
        .route("/api/neural-lab/experiments/quant-calibration",
            post(crate::neural_lab_quant::run_quant_calibration_handler))
        .route("/api/neural-lab/experiments", get(neural_lab::list_experiments)
            .post(neural_lab::create_experiment)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/neural-lab/experiments/{id}", get(neural_lab::get_experiment))
        .route("/api/neural-lab/experiments/{id}/start", patch(neural_lab::start_experiment)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/neural-lab/experiments/{id}/complete", patch(neural_lab::complete_experiment)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/neural-lab/experiments/{id}/fail", patch(neural_lab::fail_experiment)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/neural-lab/frontier/{lineage}", get(neural_lab::get_frontier))
        .route("/api/neural-lab/strategies", get(neural_lab::list_strategies))
        // Quality Gate & Fix Task routes (Swarm Gate Enforcement)
        .route("/api/hexflo/quality-gate", post(quality::create_quality_gate)
            .get(quality::list_quality_gates)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/hexflo/quality-gate/{id}", patch(quality::complete_quality_gate)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/hexflo/quality-gate/{id}/fixes", get(quality::list_fixes_for_gate))
        .route("/api/hexflo/fix-task", post(quality::create_fix_task)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/hexflo/fix-task/{id}", patch(quality::complete_fix_task)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        // Coordination (multi-instance lock/claim/activity)
        .route("/api/coordination/instance/register", post(coordination::register_instance)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/coordination/instance/heartbeat", post(coordination::heartbeat_instance)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/coordination/instances", get(coordination::list_instances))
        .route("/api/coordination/worktree/lock", post(coordination::acquire_lock)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/coordination/worktree/locks", get(coordination::list_locks))
        .route("/api/coordination/worktree/lock/{key}", delete(coordination::release_lock))
        .route("/api/coordination/task/claim", post(coordination::claim_task)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/coordination/task/claim/{task_id}", delete(coordination::release_task))
        .route("/api/coordination/tasks", get(coordination::list_claims))
        .route("/api/coordination/activity", post(coordination::publish_activity)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/coordination/activities", get(coordination::get_activities))
        // RL (reinforcement learning) engine
        .route("/api/rl/action", post(rl::select_action)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/rl/reward", post(rl::submit_reward)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/rl/stats", get(rl::get_stats))
        .route("/api/metrics/cost", get(metrics::get_cost_metrics))
        .route("/api/rl/patterns", get(rl::search_patterns)
            .post(rl::store_pattern)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/rl/patterns/{id}/reinforce", post(rl::reinforce_pattern)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/rl/decay", post(rl::decay_patterns))
        // Docker sandbox agent lifecycle (ADR-docker-sandbox)
        .route("/api/agents/sandbox/spawn", post(sandbox::spawn_agent)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/agents/sandbox/{agent_id}", delete(sandbox::stop_agent))
        // Agent orchestration
        .route("/api/agents/spawn", post(orchestration::spawn_agent)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/agents/health", post(orchestration::health_check))
        // DEPRECATED(ADR-039): Browser will use SpacetimeDB direct subscription
        .route("/api/agents", get(orchestration::list_agents))
        .route("/api/agents/{id}", get(orchestration::get_agent)
            .delete(orchestration::terminate_agent))
        // DEPRECATED(ADR-065): redirect to unified /api/hex-agents/connect
        .route("/api/agents/connect", post(deprecated_agents_connect))
        .route("/api/agents/disconnect", post(orchestration::disconnect_agent)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        // Workplan execution
        .route("/api/workplan/execute", post(orchestration::execute_workplan)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/workplan/status", get(orchestration::workplan_status))
        .route("/api/workplan/pause", post(orchestration::pause_workplan))
        .route("/api/workplan/resume", post(orchestration::resume_workplan))
        // Workplan reporting (ADR-046)
        .route("/api/workplan/list", get(orchestration::list_workplans))
        .route("/api/workplan/by-path", get(orchestration::workplan_by_path))
        .route("/api/workplan/{id}", get(orchestration::get_workplan))
        .route("/api/workplan/{id}/report", get(orchestration::workplan_report))
        // Context engineering (ADR-2603312100) — hot-reload context caches
        .route("/api/context/reload", post(context::reload_context))
        // MCP tool registry — serves config/mcp-tools.json for dashboard discovery
        .route("/api/tools", get(tools_registry))
        // Workplan file definitions — reads docs/workplans/*.json from disk
        .route("/api/workplans", get(workplan_files))
        // Project-scoped workplan files (dashboard passes ?root= as fallback)
        .route("/api/projects/{id}/workplans", get(project_workplan_files))
        .route("/api/projects/{id}/report", get(projects::project_report))
        .route("/api/projects/{id}/swarms", get(projects::project_swarms))
        // Project-scoped file browsing (ADR-045)
        .route("/api/{project_id}/browse", get(browse::browse_dir))
        .route("/api/{project_id}/read/{*path}", get(browse::read_file))
        // Fleet (remote compute)
        .route("/api/fleet", get(fleet::list_nodes))
        .route("/api/fleet/register", post(fleet::register_node)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/fleet/health", post(fleet::check_health))
        .route("/api/fleet/select", get(fleet::select_best_node))
        .route("/api/fleet/{id}", get(fleet::get_node)
            .delete(fleet::unregister_node))
        .route("/api/fleet/{id}/deploy", post(fleet::deploy_to_node)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        // Secret broker (ADR-026) — localhost-only, single-use claims
        .route("/secrets/claim", post(secrets::claim_secrets)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/secrets/grant", post(secrets::grant_secret)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/secrets/revoke", post(secrets::revoke_secret)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/secrets/grants", get(secrets::list_grants))
        .route("/api/secrets/health", get(secrets::secrets_health))
        // Vault (ADR-026) — secret value storage
        .route("/api/secrets/vault", get(secrets::vault_list))
        .route("/api/secrets/vault", post(secrets::vault_set)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/secrets/vault/{key}", get(secrets::vault_get))
        // Inference endpoint discovery (ADR-026)
        .route("/api/inference/register", post(secrets::register_inference)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/inference/endpoints", get(secrets::list_inference))
        .route("/api/inference/endpoints/{id}", delete(secrets::remove_inference)
            .patch(secrets::calibrate_inference))
        .route("/api/inference/health", post(secrets::check_inference_health))
        // Synchronous inference completion (hex-agent HTTP bridge)
        .route("/api/inference/complete", post(inference::inference_complete)
            .layer(DefaultBodyLimit::max(PUSH_BODY_LIMIT)))
        // Path B task dispatch queue (ADR-2604010000 P2.2 + P2.3)
        .route("/api/inference/queue", post(inference::inference_queue)
            .layer(DefaultBodyLimit::max(PUSH_BODY_LIMIT)))
        .route("/api/inference/queue/pending", get(inference::queue_pending))
        .route("/api/inference/queue/{id}", patch(inference::queue_update))
        // SSE streaming chat endpoint (hex chat TUI — wp-cli-chat-tui)
        .route("/api/inference/chat/stream", post(inference::inference_stream))
        // ═══════════════════════════════════════════════════════════
        // HEXFLO COORDINATION — write routes stay, reads via SpacetimeDB
        // ═══════════════════════════════════════════════════════════

        // ═══════════════════════════════════════════════════════════
        // GIT INTEGRATION (ADR-044) — project-scoped git queries
        // ═══════════════════════════════════════════════════════════
        .route("/api/{project_id}/git/status", get(git::git_status))
        .route("/api/{project_id}/git/log", get(git::git_log))
        .route("/api/{project_id}/git/diff", get(git::git_diff))
        .route("/api/{project_id}/git/diff/{refspec}", get(git::git_diff_refs))
        .route("/api/{project_id}/git/branches", get(git::git_branches))
        .route("/api/{project_id}/git/worktrees", get(git::git_worktrees)
            .post(git::git_worktree_create)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/{project_id}/git/worktrees/{name}", delete(git::git_worktree_delete))
        .route("/api/{project_id}/git/log/{sha}", get(git::git_commit_detail))
        // Phase 3: Cross-cutting git intelligence
        .route("/api/{project_id}/git/task-commits", get(git::git_task_commits))
        .route("/api/{project_id}/git/violation-blame", post(git::git_violation_blame)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/{project_id}/git/timeline", get(git::git_timeline))

        // ADR (Architecture Decision Records)
        .route("/api/adrs", get(adrs::list_adrs))
        .route("/api/adrs/{id}", get(adrs::get_adr)
            .put(adrs::save_adr)
            .layer(DefaultBodyLimit::max(PUSH_BODY_LIMIT)))
        // Project-scoped ADRs (ADR-045 Phase 1)
        .route("/api/projects/{id}/adrs", get(adrs::list_project_adrs))
        .route("/api/projects/{id}/adrs/{adr_id}", get(adrs::get_project_adr)
            .put(adrs::save_project_adr)
            .layer(DefaultBodyLimit::max(PUSH_BODY_LIMIT)))

        // Skill registry (ADR-042)
        // NOTE: /sync must be registered BEFORE /{name} to avoid path conflict
        .route("/api/skills/sync", post(skills::sync_skills))
        .route("/api/skills", get(skills::list_skills))
        .route("/api/skills/{name}", get(skills::get_skill))

        // Generic file read/write (path-traversal protected)
        .route("/api/files", get(files::read_file)
            .put(files::save_file)
            .layer(DefaultBodyLimit::max(PUSH_BODY_LIMIT)))
        // Config re-sync (T15: manual refresh from repo → SpacetimeDB)
        .route("/api/config/sync", post(files::resync_config))

        // HexFlo coordination (ADR-027)
        .route("/api/hexflo/memory", post(hexflo::memory_store)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/hexflo/memory/search", get(hexflo::memory_search))
        .route("/api/hexflo/memory/{key}", get(hexflo::memory_retrieve)
            .delete(hexflo::memory_delete))
        .route("/api/hexflo/cleanup", post(hexflo::cleanup))
        // Enforcement rules (ADR-2603221959 P5)
        .route("/api/hexflo/enforcement-rules", get(hexflo::enforcement_rules_list)
            .post(hexflo::enforcement_rules_upsert)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/hexflo/enforcement-rules/toggle", patch(hexflo::enforcement_rules_toggle)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        // Agent Notification Inbox (ADR-060)
        .route("/api/hexflo/inbox/notify", post(hexflo::inbox_notify)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/hexflo/inbox/expire", post(hexflo::inbox_expire))
        .route("/api/hexflo/inbox/{agent_id}", get(hexflo::inbox_query))
        .route("/api/hexflo/inbox/{id}/ack", patch(hexflo::inbox_acknowledge)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        // Dashboard inbox — project-scoped view + ack (step-5)
        .route("/api/inbox", get(inbox::list_inbox))
        .route("/api/inbox/{id}/ack", post(inbox::ack_notification)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))

        // Unified Agent Registry (ADR-058) — hex_agent table
        // NOTE: /connect and /evict must be registered BEFORE /{id} to avoid path conflicts
        .route("/api/hex-agents/connect", post(hex_agents::connect_agent)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/hex-agents/evict", post(hex_agents::evict_dead))
        .route("/api/hex-agents", get(hex_agents::list_agents))
        .route("/api/hex-agents/{id}/swarm", get(swarms::get_agent_swarm))
        .route("/api/hex-agents/{id}", get(hex_agents::get_agent)
            .delete(hex_agents::disconnect_agent))
        .route("/api/hex-agents/{id}/heartbeat", post(hex_agents::heartbeat))
        .route("/api/hex-agents/{id}/disconnect", post(hex_agents::disconnect_agent_post))

        // Test sessions (test-results module fallback)
        .route("/api/test-sessions", post(test_sessions::record)
            .get(test_sessions::list)
            .layer(DefaultBodyLimit::max(PUSH_BODY_LIMIT)))
        .route("/api/test-sessions/trends", get(test_sessions::trends))
        .route("/api/test-sessions/flaky", get(test_sessions::flaky))
        .route("/api/test-sessions/{id}", get(test_sessions::get_session))
        // Command session proxy (step-4 stub — wired to hex-agent in step-5)
        .route("/api/command-sessions", post(command_sessions::create_session)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/command-sessions/{session_id}/search", get(command_sessions::search_session));

    // Session persistence (ADR-036 / ADR-042 P2.5) — SpacetimeDB primary, SQLite fallback
    let router = router
        .route("/api/sessions", post(sessions::create_session)
            .get(sessions::list_sessions)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/sessions/search", get(sessions::search_sessions))
        .route("/api/sessions/{id}", get(sessions::get_session)
            .patch(sessions::update_session_title)
            .delete(sessions::delete_session))
        .route("/api/sessions/{id}/messages", get(sessions::list_messages)
            .post(sessions::append_message)
            .layer(DefaultBodyLimit::max(PUSH_BODY_LIMIT)))
        .route("/api/sessions/{id}/fork", post(sessions::fork_session)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/sessions/{id}/compact", post(sessions::compact_session)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/sessions/{id}/revert", post(sessions::revert_session)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/sessions/{id}/archive", post(sessions::archive_session));

    router
        // WebSocket
        .route("/ws", get(ws::ws_handler))
        .route("/ws/chat", get(chat::chat_ws_handler))
        .route("/ws/inference", get(inference_ws::ws_inference_handler))
        // Middleware (order: outermost runs first → auth → agent_guard → enforcement → deprecation → handler)
        .layer(axum::middleware::from_fn(deprecation_layer))
        .layer(axum::middleware::from_fn_with_state(state.clone(), enforcement_layer))
        .layer(axum::middleware::from_fn_with_state(state.clone(), agent_guard))
        .layer(axum::middleware::from_fn_with_state(state.clone(), auth_layer))
        .layer(cors)
        .with_state(state)
}

fn is_local_origin(origin: &str) -> bool {
    if let Ok(url) = url::Url::parse(origin) {
        matches!(url.host_str(), Some("localhost") | Some("127.0.0.1"))
    } else {
        false
    }
}
