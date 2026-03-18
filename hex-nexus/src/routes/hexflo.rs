//! REST endpoints for the HexFlo coordination API.

use axum::{
    extract::{Path, Query, State},
    Json,
};
use http::StatusCode;
use serde::Deserialize;
use serde_json::json;

use crate::state::SharedState;

fn no_hexflo() -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({ "error": "HexFlo not initialized (no swarm database)" })),
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
    let hexflo = match &state.hexflo {
        Some(h) => h,
        None => return no_hexflo(),
    };

    match hexflo
        .memory_store(&body.key, &body.value, body.scope.as_deref())
        .await
    {
        Ok(()) => (StatusCode::OK, Json(json!({ "ok": true, "key": body.key }))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e })),
        ),
    }
}

/// GET /api/hexflo/memory/:key — retrieve a value by key
pub async fn memory_retrieve(
    State(state): State<SharedState>,
    Path(key): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    let hexflo = match &state.hexflo {
        Some(h) => h,
        None => return no_hexflo(),
    };

    match hexflo.memory_retrieve(&key).await {
        Ok(Some(value)) => (StatusCode::OK, Json(json!({ "key": key, "value": value }))),
        Ok(None) => (StatusCode::NOT_FOUND, Json(json!({ "error": "Key not found" }))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e })),
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
    let hexflo = match &state.hexflo {
        Some(h) => h,
        None => return no_hexflo(),
    };

    match hexflo.memory_search(&query.q).await {
        Ok(entries) => (StatusCode::OK, Json(json!({ "results": entries }))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e })),
        ),
    }
}

/// DELETE /api/hexflo/memory/:key — delete a memory entry
pub async fn memory_delete(
    State(state): State<SharedState>,
    Path(key): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    let hexflo = match &state.hexflo {
        Some(h) => h,
        None => return no_hexflo(),
    };

    match hexflo.memory_delete(&key).await {
        Ok(true) => (StatusCode::OK, Json(json!({ "ok": true, "deleted": key }))),
        Ok(false) => (StatusCode::NOT_FOUND, Json(json!({ "error": "Key not found" }))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e })),
        ),
    }
}

// ── Cleanup endpoint ───────────────────────────────────

/// POST /api/hexflo/cleanup — trigger stale agent cleanup
pub async fn cleanup(
    State(state): State<SharedState>,
) -> (StatusCode, Json<serde_json::Value>) {
    let hexflo = match &state.hexflo {
        Some(h) => h,
        None => return no_hexflo(),
    };

    match hexflo.cleanup_stale_agents().await {
        Ok(report) => (StatusCode::OK, Json(json!(report))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e })),
        ),
    }
}
