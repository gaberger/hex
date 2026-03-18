pub mod chat;
pub mod commands;
pub mod coordination;
pub mod decisions;
pub mod fleet;
pub mod hexflo;
pub mod orchestration;
pub mod projects;
pub mod push;
pub mod query;
pub mod rl;
pub mod secrets;
pub mod swarms;
pub mod ws;

use axum::{Router, Json, routing::{get, post, patch, delete}, extract::DefaultBodyLimit};
use tower_http::cors::{CorsLayer, AllowOrigin};
use http::{HeaderValue, Method};
use serde_json::json;
use crate::state::SharedState;
use crate::middleware::auth::auth_layer;
use crate::embed::{serve_index, serve_chat};

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
        .allow_methods([Method::GET, Method::POST, Method::PATCH, Method::DELETE, Method::OPTIONS])
        .allow_headers([
            http::header::CONTENT_TYPE,
            http::header::AUTHORIZATION,
        ]);

    Router::new()
        // Static + version
        .route("/", get(serve_index))
        .route("/chat", get(serve_chat))
        .route("/api/version", get(get_version))
        // Project management
        .route("/api/projects", get(projects::list_projects))
        .route("/api/projects/register", post(projects::register)
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
        // Commands (browser/MCP → hub → project, bidirectional)
        .route("/api/{project_id}/command", post(commands::send_command)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/{project_id}/command/{command_id}", get(commands::get_command))
        .route("/api/{project_id}/command/{command_id}/result", post(commands::report_result)
            .layer(DefaultBodyLimit::max(PUSH_BODY_LIMIT)))
        .route("/api/{project_id}/commands", get(commands::list_commands))
        // Decisions (browser → hub → WS)
        .route("/api/{project_id}/decisions/{decision_id}", post(decisions::handle_decision))
        // Swarm persistence
        .route("/api/swarms", post(swarms::create_swarm)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/swarms/active", get(swarms::list_active_swarms))
        .route("/api/swarms/{id}", get(swarms::get_swarm))
        .route("/api/swarms/{id}/tasks/{task_id}", patch(swarms::update_task)
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
        .route("/api/agents", get(orchestration::list_agents))
        .route("/api/agents/{id}", get(orchestration::get_agent)
            .delete(orchestration::terminate_agent))
        // Workplan execution
        .route("/api/workplan/execute", post(orchestration::execute_workplan)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/workplan/status", get(orchestration::workplan_status))
        .route("/api/workplan/pause", post(orchestration::pause_workplan))
        .route("/api/workplan/resume", post(orchestration::resume_workplan))
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
        // Inference endpoint discovery (ADR-026)
        .route("/api/inference/register", post(secrets::register_inference)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/inference/endpoints", get(secrets::list_inference))
        .route("/api/inference/endpoints/{id}", delete(secrets::remove_inference))
        .route("/api/inference/health", post(secrets::check_inference_health))
        // HexFlo coordination (ADR-027)
        .route("/api/hexflo/memory", post(hexflo::memory_store)
            .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)))
        .route("/api/hexflo/memory/search", get(hexflo::memory_search))
        .route("/api/hexflo/memory/{key}", get(hexflo::memory_retrieve)
            .delete(hexflo::memory_delete))
        .route("/api/hexflo/cleanup", post(hexflo::cleanup))
        // WebSocket
        .route("/ws", get(ws::ws_handler))
        .route("/ws/chat", get(chat::chat_ws_handler))
        // Middleware
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
