//! REST endpoints for the HexFlo coordination API.
//!
//! ADR-041 Phase 3: Routes delegate to `state.state_port` (IStatePort /
//! SpacetimeStateAdapter) instead of the in-memory `state.hexflo` coordinator.

use axum::{
    extract::{Path, Query, State},
    Json,
};
use http::StatusCode;
use serde::Deserialize;
use serde_json::json;

use crate::state::SharedState;

fn no_state_port() -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({ "error": "IStatePort not initialized (no SpacetimeDB backend)" })),
    )
}

// ── Memory endpoints ───────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct MemoryStoreRequest {
    pub key: String,
    pub value: String,
    pub scope: Option<String>,
}

/// POST /api/hexflo/memory — store a key-value pair
pub async fn memory_store(
    State(state): State<SharedState>,
    Json(body): Json<MemoryStoreRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let port = match &state.state_port {
        Some(p) => p,
        None => return no_state_port(),
    };

    let scope = body.scope.as_deref().unwrap_or("global");
    match port.hexflo_memory_store(&body.key, &body.value, scope).await {
        Ok(()) => (StatusCode::OK, Json(json!({ "ok": true, "key": body.key }))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        ),
    }
}

/// GET /api/hexflo/memory/:key — retrieve a value by key
pub async fn memory_retrieve(
    State(state): State<SharedState>,
    Path(key): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    let port = match &state.state_port {
        Some(p) => p,
        None => return no_state_port(),
    };

    match port.hexflo_memory_retrieve(&key).await {
        Ok(Some(value)) => (StatusCode::OK, Json(json!({ "key": key, "value": value }))),
        Ok(None) => (StatusCode::NOT_FOUND, Json(json!({ "error": "Key not found" }))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        ),
    }
}

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    pub q: String,
}

/// GET /api/hexflo/memory/search?q=... — search memory entries
pub async fn memory_search(
    State(state): State<SharedState>,
    Query(query): Query<SearchQuery>,
) -> (StatusCode, Json<serde_json::Value>) {
    let port = match &state.state_port {
        Some(p) => p,
        None => return no_state_port(),
    };

    match port.hexflo_memory_search(&query.q).await {
        Ok(entries) => {
            // Convert Vec<(String, String)> to the same JSON shape as before
            let results: Vec<serde_json::Value> = entries
                .into_iter()
                .map(|(k, v)| json!({ "key": k, "value": v }))
                .collect();
            (StatusCode::OK, Json(json!({ "results": results })))
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        ),
    }
}

/// DELETE /api/hexflo/memory/:key — delete a memory entry
pub async fn memory_delete(
    State(state): State<SharedState>,
    Path(key): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    let port = match &state.state_port {
        Some(p) => p,
        None => return no_state_port(),
    };

    // Check existence first — hexflo_memory_delete succeeds silently when
    // the key doesn't exist, but the API contract returns 404.
    match port.hexflo_memory_retrieve(&key).await {
        Ok(None) => return (StatusCode::NOT_FOUND, Json(json!({ "error": "Key not found" }))),
        Err(e) => return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        ),
        Ok(Some(_)) => {}
    }

    match port.hexflo_memory_delete(&key).await {
        Ok(()) => (StatusCode::OK, Json(json!({ "ok": true, "deleted": key }))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        ),
    }
}

// ── Cleanup endpoint ───────────────────────────────────

/// Thresholds for agent staleness (mirrored from coordination/cleanup.rs).
const STALE_THRESHOLD_SECS: u64 = 45;
const DEAD_THRESHOLD_SECS: u64 = 120;

/// POST /api/hexflo/cleanup — trigger stale agent cleanup
pub async fn cleanup(
    State(state): State<SharedState>,
) -> (StatusCode, Json<serde_json::Value>) {
    let port = match &state.state_port {
        Some(p) => p,
        None => return no_state_port(),
    };

    // TODO(ADR-041): The old HexFlo.cleanup_stale_agents() also called
    // agent_manager.check_health() before the cleanup. That side-effect is
    // lost here. Once agent_manager is wired into routes (or a dedicated
    // health-check endpoint exists), restore that call.
    match port.swarm_cleanup_stale(STALE_THRESHOLD_SECS, DEAD_THRESHOLD_SECS).await {
        Ok(report) => (
            StatusCode::OK,
            Json(json!({
                "staleCount": report.stale_count,
                "deadCount": report.dead_count,
                "reclaimedTasks": report.reclaimed_tasks,
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        ),
    }
}
