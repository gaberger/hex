//! REST endpoints for worker_pool_intent (wp-stdb-supervisor P4 + P5).
//!
//! Backs both the `hex pool` CLI subcommand (P4) and the Brain dashboard
//! supervisor panel (P5). Each endpoint maps to a STDB reducer or query.

use axum::{
    extract::{Path, Query, State},
    Json,
};
use http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::state::SharedState;

#[derive(Debug, Deserialize)]
pub struct PoolCreateRequest {
    pub id: String,
    pub role: String,
    pub desired_count: u32,
    #[serde(default)]
    pub restart_strategy: Option<String>,
    #[serde(default)]
    pub max_restarts: Option<u32>,
    #[serde(default)]
    pub max_restart_window_secs: Option<u32>,
    #[serde(default)]
    pub paused: Option<bool>,
    #[serde(default)]
    pub owner_agent_id: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PoolStatus {
    pub id: String,
    pub role: String,
    pub desired_count: u32,
    pub alive_count: u32,
    pub exited_count: u32,
    pub restart_strategy: String,
    pub max_restarts: u32,
    pub max_restart_window_secs: u32,
    pub paused: bool,
    pub in_crash_loop: bool,
}

/// POST /api/pools — create or update a pool intent.
pub async fn create_pool(
    State(state): State<SharedState>,
    Json(req): Json<PoolCreateRequest>,
) -> (StatusCode, Json<Value>) {
    let port = match state.state_port.as_ref() {
        Some(p) => p,
        None => return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "error": "no state port" }))),
    };
    let strategy = req.restart_strategy.unwrap_or_else(|| "permanent".to_string());
    let max_restarts = req.max_restarts.unwrap_or(5);
    let window = req.max_restart_window_secs.unwrap_or(60);
    let paused = req.paused.unwrap_or(false);
    let owner = req.owner_agent_id.unwrap_or_else(|| "operator".to_string());

    // Reuses the existing `query_table_on` machinery via call_reducer.
    if let Err(e) = port
        .pool_create(
            &req.id, &req.role, req.desired_count,
            &strategy, max_restarts, window, paused, &owner,
        )
        .await
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("pool_create: {}", e) })),
        );
    }
    (StatusCode::OK, Json(json!({ "ok": true, "id": req.id })))
}

/// GET /api/pools — list every pool with derived alive/exited counts.
pub async fn list_pools(
    State(state): State<SharedState>,
) -> (StatusCode, Json<Value>) {
    let port = match state.state_port.as_ref() {
        Some(p) => p,
        None => return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "error": "no state port" }))),
    };
    match port.pool_status_all().await {
        Ok(rows) => {
            let pools: Vec<PoolStatus> = rows.into_iter().map(|r| PoolStatus {
                id: r.0, role: r.1, desired_count: r.2, alive_count: r.3, exited_count: r.4,
                restart_strategy: r.5, max_restarts: r.6, max_restart_window_secs: r.7,
                paused: r.8, in_crash_loop: r.9,
            }).collect();
            (StatusCode::OK, Json(json!({ "pools": pools, "total": pools.len() })))
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("pool_status_all: {}", e) })),
        ),
    }
}

#[derive(Debug, Deserialize)]
pub struct PausedRequest {
    pub paused: bool,
}

/// PATCH /api/pools/{id}/paused — flip the paused flag.
pub async fn set_paused(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(req): Json<PausedRequest>,
) -> (StatusCode, Json<Value>) {
    let port = match state.state_port.as_ref() {
        Some(p) => p,
        None => return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "error": "no state port" }))),
    };
    if let Err(e) = port.pool_set_paused(&id, req.paused).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() })));
    }
    (StatusCode::OK, Json(json!({ "ok": true, "id": id, "paused": req.paused })))
}

#[derive(Debug, Deserialize)]
pub struct EventsQuery {
    pub limit: Option<u32>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SupervisorEventOut {
    pub id: u64,
    pub ts: String,
    pub kind: String,
    pub pool_id: String,
    pub worker_id: String,
    pub payload: String,
    pub handled: bool,
}

/// GET /api/supervisor/events?limit=N — recent supervisor_event rows for the
/// dashboard activity feed. Newest first; capped at 100 server-side.
pub async fn list_supervisor_events(
    State(state): State<SharedState>,
    Query(params): Query<EventsQuery>,
) -> (StatusCode, Json<Value>) {
    let port = match state.state_port.as_ref() {
        Some(p) => p,
        None => return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "error": "no state port" }))),
    };
    let limit = params.limit.unwrap_or(20).min(100);
    match port.supervisor_events_recent(limit).await {
        Ok(rows) => {
            let events: Vec<SupervisorEventOut> = rows.into_iter().map(|r| SupervisorEventOut {
                id: r.0, ts: r.1, kind: r.2, pool_id: r.3,
                worker_id: r.4, payload: r.5, handled: r.6,
            }).collect();
            (StatusCode::OK, Json(json!({ "events": events, "total": events.len() })))
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("supervisor_events_recent: {}", e) })),
        ),
    }
}

/// DELETE /api/pools/{id}
pub async fn delete_pool(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<Value>) {
    let port = match state.state_port.as_ref() {
        Some(p) => p,
        None => return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "error": "no state port" }))),
    };
    if let Err(e) = port.pool_delete(&id).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() })));
    }
    (StatusCode::OK, Json(json!({ "ok": true, "id": id })))
}
