pub mod commands;
pub mod decisions;
pub mod projects;
pub mod push;
pub mod query;
pub mod sse;
pub mod ws;

use axum::{Router, Json, routing::{get, post, delete}, extract::DefaultBodyLimit};
use tower_http::cors::{CorsLayer, AllowOrigin};
use http::{HeaderValue, Method};
use serde_json::json;
use crate::state::SharedState;
use crate::middleware::auth::auth_layer;
use crate::embed::serve_index;

async fn get_version() -> Json<serde_json::Value> {
    Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
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
        .allow_methods([Method::GET, Method::POST, Method::DELETE, Method::OPTIONS])
        .allow_headers([
            http::header::CONTENT_TYPE,
            http::header::AUTHORIZATION,
        ]);

    Router::new()
        // Static + version
        .route("/", get(serve_index))
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
        // SSE (hub → browser)
        .route("/api/events", get(sse::sse_handler))
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
        // Decisions (browser → hub → SSE)
        .route("/api/{project_id}/decisions/{decision_id}", post(decisions::handle_decision))
        // WebSocket
        .route("/ws", get(ws::ws_handler))
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
