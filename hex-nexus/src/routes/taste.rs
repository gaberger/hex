//! REST endpoints for taste graph v1 (AIOS P2 P2.1).
//!
//! GET    /api/taste          — list taste entries (optional scope/category filter)
//! POST   /api/taste          — create/update a taste entry
//! DELETE /api/taste/{key}    — tombstone a taste entry
//! PATCH  /api/taste/{key}/pin — pin a taste entry

use axum::{
    extract::{Path, Query, State},
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

const TASTE_KEY_PREFIX: &str = "taste:";

#[derive(Debug, Deserialize)]
pub struct TasteQueryParams {
    pub scope: Option<String>,
    pub category: Option<String>,
}

/// GET /api/taste — list taste entries.
///
/// Taste entries are stored in HexFlo memory with keys like
/// `taste:{scope}:{category}:{name}`. Optionally filter by scope and/or category.
pub async fn get_taste(
    State(state): State<SharedState>,
    Query(params): Query<TasteQueryParams>,
) -> (StatusCode, Json<Value>) {
    let port = match &state.state_port {
        Some(p) => p,
        None => return no_state_port(),
    };

    // Build the narrowest prefix we can from the provided filters
    let search_query = match (&params.scope, &params.category) {
        (Some(scope), Some(cat)) => format!("taste:{}:{}", scope, cat),
        (Some(scope), None) => format!("taste:{}", scope),
        _ => "taste:".to_string(),
    };

    let entries = port
        .hexflo_memory_search(&search_query)
        .await
        .unwrap_or_default();

    let taste_entries: Vec<Value> = entries
        .into_iter()
        .filter_map(|(key, value)| {
            // Skip non-taste keys and tombstoned entries
            if !key.starts_with(TASTE_KEY_PREFIX) {
                return None;
            }
            let parsed: Value = serde_json::from_str(&value).ok()?;
            if parsed.get("deleted").and_then(|v| v.as_bool()).unwrap_or(false) {
                return None;
            }

            // Apply category filter when scope was not provided (broad search)
            if let Some(ref cat) = params.category {
                if parsed.get("category").and_then(|v| v.as_str()) != Some(cat.as_str()) {
                    return None;
                }
            }

            Some(parsed)
        })
        .collect();

    (StatusCode::OK, Json(json!(taste_entries)))
}

#[derive(Debug, Deserialize)]
pub struct SetTasteRequest {
    pub scope: String,
    pub category: String,
    pub name: String,
    pub value: String,
    pub confidence: Option<f64>,
}

/// POST /api/taste — create or update a taste entry.
///
/// Confidence defaults to 1.0 for manual entries. Source is always "manual".
pub async fn set_taste(
    State(state): State<SharedState>,
    Json(body): Json<SetTasteRequest>,
) -> (StatusCode, Json<Value>) {
    let port = match &state.state_port {
        Some(p) => p,
        None => return no_state_port(),
    };

    let key = format!("{}{}:{}:{}", TASTE_KEY_PREFIX, body.scope, body.category, body.name);
    let now = chrono::Utc::now();
    let confidence = body.confidence.unwrap_or(1.0);

    let entry = json!({
        "key": key,
        "scope": body.scope,
        "category": body.category,
        "name": body.name,
        "value": body.value,
        "confidence": confidence,
        "source": "manual",
        "created_at": now.to_rfc3339(),
        "last_applied_at": null,
        "pinned": false,
    });

    let serialized = serde_json::to_string(&entry).unwrap_or_default();

    match port.hexflo_memory_store(&key, &serialized, "global").await {
        Ok(()) => (StatusCode::CREATED, Json(entry)),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        ),
    }
}

/// DELETE /api/taste/{key} — tombstone a taste entry.
///
/// HexFlo memory has no delete, so we store `{"deleted": true}` as a tombstone.
pub async fn forget_taste(
    State(state): State<SharedState>,
    Path(key): Path<String>,
) -> (StatusCode, Json<Value>) {
    let port = match &state.state_port {
        Some(p) => p,
        None => return no_state_port(),
    };

    let full_key = if key.starts_with(TASTE_KEY_PREFIX) {
        key.clone()
    } else {
        format!("{}{}", TASTE_KEY_PREFIX, key)
    };

    // Verify the entry exists
    match port.hexflo_memory_retrieve(&full_key).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "No taste entry found for this key" })),
            );
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            );
        }
    }

    let tombstone = json!({
        "deleted": true,
        "deleted_at": chrono::Utc::now().to_rfc3339(),
    });
    let serialized = serde_json::to_string(&tombstone).unwrap_or_default();

    match port.hexflo_memory_store(&full_key, &serialized, "global").await {
        Ok(()) => (StatusCode::OK, Json(json!({ "ok": true }))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        ),
    }
}

/// PATCH /api/taste/{key}/pin — pin a taste entry to prevent auto-decay.
pub async fn pin_taste(
    State(state): State<SharedState>,
    Path(key): Path<String>,
) -> (StatusCode, Json<Value>) {
    let port = match &state.state_port {
        Some(p) => p,
        None => return no_state_port(),
    };

    let full_key = if key.starts_with(TASTE_KEY_PREFIX) {
        key.clone()
    } else {
        format!("{}{}", TASTE_KEY_PREFIX, key)
    };

    let current = match port.hexflo_memory_retrieve(&full_key).await {
        Ok(Some(v)) => serde_json::from_str::<Value>(&v).unwrap_or(json!({})),
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "No taste entry found for this key" })),
            );
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            );
        }
    };

    // Reject if tombstoned
    if current.get("deleted").and_then(|v| v.as_bool()).unwrap_or(false) {
        return (
            StatusCode::GONE,
            Json(json!({ "error": "Taste entry has been deleted" })),
        );
    }

    let mut updated = current;
    updated["pinned"] = json!(true);
    updated["pinned_at"] = json!(chrono::Utc::now().to_rfc3339());

    let serialized = serde_json::to_string(&updated).unwrap_or_default();
    match port.hexflo_memory_store(&full_key, &serialized, "global").await {
        Ok(()) => (StatusCode::OK, Json(json!({ "ok": true }))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        ),
    }
}
