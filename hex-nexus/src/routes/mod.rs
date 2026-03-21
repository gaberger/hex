pub mod adrs;
pub mod agents;
pub mod files;
pub mod analysis;
pub mod chat;
pub mod commands;
pub mod coordination;
pub mod decisions;
pub mod fleet;
pub mod git;
pub mod hexflo;
pub mod inference;
pub mod orchestration;
pub mod projects;
pub mod push;
pub mod query;
pub mod rl;
pub mod secrets;
pub mod sessions;
pub mod swarms;
pub mod openapi;
pub mod ws;

use axum::{Router, Json, routing::{get, post, put, patch, delete}, extract::DefaultBodyLimit};
use axum::response::{IntoResponse, Redirect};
use tower_http::cors::{CorsLayer, AllowOrigin};
use http::{HeaderValue, Method};
use serde_json::json;
use utoipa::OpenApi;
use crate::state::SharedState;
use crate::middleware::auth::auth_layer;
use crate::middleware::deprecation::deprecation_layer;
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

async fn get_version() -> Json<serde_json::Value> {
    Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "buildHash": env!("HEX_HUB_BUILD_HASH"),
        "name": "hex-hub",
    }))
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
        // Project management
        .route("/api/projects", get(projects::list_projects))
        .route("/api/projects/register", post(projects::register)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/projects/init", post(files::init_project)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/projects/{id}", delete(projects::unregister))
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
        .route("/api/analyze", post(analysis::analyze_path)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/{project_id}/analyze", get(analysis::analyze_project))
        .route("/api/{project_id}/analyze/text", get(analysis::analyze_project_text))
        // ADR compliance (ADR-045) — check code against accepted ADRs
        .route("/api/analyze/adr-compliance", post(analysis::analyze_adr_compliance)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/{project_id}/analyze/adr-compliance", get(analysis::analyze_project_adr_compliance))
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

        // Swarm persistence (WRITE routes kept — they call SpacetimeDB reducers)
        .route("/api/swarms", post(swarms::create_swarm)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/swarms/active", get(swarms::list_active_swarms))
        .route("/api/swarms/{id}", get(swarms::get_swarm))
        .route("/api/swarms/{id}/tasks", post(swarms::create_task)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/swarms/{id}/tasks/{task_id}", patch(swarms::update_task)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        // Convenience route for MCP tools (no swarm ID needed — task ID is globally unique)
        .route("/api/hexflo/tasks/{task_id}", patch(swarms::update_task_by_id)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/work-items/incomplete", get(swarms::get_incomplete_work))
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
        .route("/api/coordination/unstaged", get(coordination::get_unstaged))
        .route("/api/coordination/cleanup", post(coordination::cleanup_stale_sessions))
        // RL (reinforcement learning) engine
        .route("/api/rl/action", post(rl::select_action)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/rl/reward", post(rl::submit_reward)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/rl/stats", get(rl::get_stats))
        .route("/api/rl/patterns", get(rl::search_patterns)
            .post(rl::store_pattern)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/rl/patterns/{id}/reinforce", post(rl::reinforce_pattern)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/rl/decay", post(rl::decay_patterns))
        // Agent orchestration
        .route("/api/agents/spawn", post(orchestration::spawn_agent)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/agents/health", post(orchestration::health_check))
        // DEPRECATED(ADR-039): Browser will use SpacetimeDB direct subscription
        .route("/api/agents", get(orchestration::list_agents))
        .route("/api/agents/{id}", get(orchestration::get_agent)
            .delete(orchestration::terminate_agent))
        // Workplan execution
        .route("/api/workplan/execute", post(orchestration::execute_workplan)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/workplan/status", get(orchestration::workplan_status))
        .route("/api/workplan/pause", post(orchestration::pause_workplan))
        .route("/api/workplan/resume", post(orchestration::resume_workplan))
        // Workplan reporting (ADR-046)
        .route("/api/workplan/list", get(orchestration::list_workplans))
        .route("/api/workplan/{id}", get(orchestration::get_workplan))
        .route("/api/workplan/{id}/report", get(orchestration::workplan_report))
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
        .route("/api/secrets/vault", post(secrets::vault_set)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/secrets/vault/{key}", get(secrets::vault_get))
        // Inference endpoint discovery (ADR-026)
        .route("/api/inference/register", post(secrets::register_inference)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/inference/endpoints", get(secrets::list_inference))
        .route("/api/inference/endpoints/{id}", delete(secrets::remove_inference))
        .route("/api/inference/health", post(secrets::check_inference_health))
        // Synchronous inference completion (hex-agent HTTP bridge)
        .route("/api/inference/complete", post(inference::inference_complete)
            .layer(DefaultBodyLimit::max(PUSH_BODY_LIMIT)))
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
        .route("/api/hexflo/cleanup", post(hexflo::cleanup));

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
        // Middleware
        .layer(axum::middleware::from_fn(deprecation_layer))
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
