//! REST endpoints for delegation trust management (ADR-2604131500 P1.4).
//!
//! GET   /api/trust     — list trust entries
//! PATCH /api/trust     — set trust level for a scope
//! POST  /api/trust/pin — pin trust level (prevent automatic escalation)

use axum::{
    extract::{Query, State},
    Json,
};
use http::StatusCode;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::state::SharedState;

fn no_state_port() -> (StatusCode, Json<Value>) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({ "error": "IStatePort not initialized (no SpacetimeDB backend)" })),
    )
}

const TRUST_KEY_PREFIX: &str = "trust:";
const VALID_LEVELS: &[&str] = &["observe", "suggest", "act", "silent"];

#[derive(Debug, Deserialize)]
pub struct TrustQueryParams {
    pub project: Option<String>,
}

/// GET /api/trust — list all delegation trust entries.
///
/// Trust entries are stored in HexFlo memory with keys prefixed by `trust:`.
/// When the `delegation_trust` SpacetimeDB table is published, this will
/// migrate to a direct table query.
pub async fn get_trust(
    State(state): State<SharedState>,
    Query(params): Query<TrustQueryParams>,
) -> (StatusCode, Json<Value>) {
    let port = match &state.state_port {
        Some(p) => p,
        None => return no_state_port(),
    };

    // Search HexFlo memory for trust entries
    let search_query = match &params.project {
        Some(proj) => format!("trust:{}", proj),
        None => "trust:".to_string(),
    };

    let entries = port.hexflo_memory_search(&search_query).await.unwrap_or_default();

    let trust_entries: Vec<Value> = entries
        .into_iter()
        .filter_map(|(_key, value)| serde_json::from_str::<Value>(&value).ok())
        .collect();

    (StatusCode::OK, Json(json!(trust_entries)))
}

#[derive(Debug, Deserialize)]
pub struct SetTrustRequest {
    pub project_id: String,
    pub scope: String,
    pub level: String,
}

/// PATCH /api/trust — set trust level for a project scope.
///
/// Validates `level` is one of: observe, suggest, act, silent.
pub async fn set_trust(
    State(state): State<SharedState>,
    Json(body): Json<SetTrustRequest>,
) -> (StatusCode, Json<Value>) {
    let port = match &state.state_port {
        Some(p) => p,
        None => return no_state_port(),
    };

    if !VALID_LEVELS.contains(&body.level.as_str()) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": format!("Invalid trust level '{}'. Must be one of: {}", body.level, VALID_LEVELS.join(", "))
            })),
        );
    }

    let key = format!("{}{}:{}", TRUST_KEY_PREFIX, body.project_id, body.scope);

    // Read existing trust entry to capture old_level for history
    let old_level = match port.hexflo_memory_retrieve(&key).await {
        Ok(Some(v)) => serde_json::from_str::<Value>(&v)
            .ok()
            .and_then(|obj| obj["level"].as_str().map(|s| s.to_string()))
            .unwrap_or_default(),
        _ => String::new(),
    };

    let now = chrono::Utc::now();
    let value = serde_json::to_string(&json!({
        "project_id": body.project_id,
        "scope": body.scope,
        "level": body.level,
        "pinned": false,
        "updated_at": now.to_rfc3339(),
    }))
    .unwrap_or_default();

    match port.hexflo_memory_store(&key, &value, "global").await {
        Ok(()) => {
            // Store a history entry for this trust change
            let ts = now.format("%Y%m%d%H%M%S%.3f").to_string();
            let history_key = format!("trust-history:{}:{}", body.project_id, ts);
            let history_value = serde_json::to_string(&json!({
                "project_id": body.project_id,
                "scope": body.scope,
                "old_level": old_level,
                "new_level": body.level,
                "reason": "human",
                "changed_at": now.to_rfc3339(),
            }))
            .unwrap_or_default();

            // Best-effort — don't fail the main operation if history write fails
            let _ = port
                .hexflo_memory_store(&history_key, &history_value, "global")
                .await;

            (StatusCode::OK, Json(json!({ "ok": true, "previous_level": old_level })))
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        ),
    }
}

/// GET /api/trust/history — list trust change history for a project.
///
/// History entries are stored in HexFlo memory with keys prefixed by
/// `trust-history:{project_id}:`. Returns up to 50 entries sorted newest first.
pub async fn get_trust_history(
    State(state): State<SharedState>,
    Query(params): Query<TrustQueryParams>,
) -> (StatusCode, Json<Value>) {
    let port = match &state.state_port {
        Some(p) => p,
        None => return no_state_port(),
    };

    let search_query = match &params.project {
        Some(proj) => format!("trust-history:{}", proj),
        None => "trust-history:".to_string(),
    };

    let entries = port
        .hexflo_memory_search(&search_query)
        .await
        .unwrap_or_default();

    let mut history_entries: Vec<Value> = entries
        .into_iter()
        .filter_map(|(_key, value)| serde_json::from_str::<Value>(&value).ok())
        .collect();

    // Sort by changed_at descending (newest first)
    history_entries.sort_by(|a, b| {
        let a_ts = a["changed_at"].as_str().unwrap_or("");
        let b_ts = b["changed_at"].as_str().unwrap_or("");
        b_ts.cmp(a_ts)
    });

    // Limit to 50 entries
    history_entries.truncate(50);

    (StatusCode::OK, Json(json!(history_entries)))
}

#[derive(Debug, Deserialize)]
pub struct PinTrustRequest {
    pub project_id: String,
    pub scope: String,
}

/// POST /api/trust/pin — pin the current trust level to prevent automatic escalation.
pub async fn pin_trust(
    State(state): State<SharedState>,
    Json(body): Json<PinTrustRequest>,
) -> (StatusCode, Json<Value>) {
    let port = match &state.state_port {
        Some(p) => p,
        None => return no_state_port(),
    };

    let key = format!("{}{}:{}", TRUST_KEY_PREFIX, body.project_id, body.scope);
    let current = match port.hexflo_memory_retrieve(&key).await {
        Ok(Some(v)) => serde_json::from_str::<Value>(&v).unwrap_or(json!({})),
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "No trust entry found for this project/scope" })),
            );
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            );
        }
    };

    let mut updated = current;
    updated["pinned"] = json!(true);
    updated["pinned_at"] = json!(chrono::Utc::now().to_rfc3339());

    let value = serde_json::to_string(&updated).unwrap_or_default();
    match port.hexflo_memory_store(&key, &value, "global").await {
        Ok(()) => (StatusCode::OK, Json(json!({ "ok": true }))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        ),
    }
}
