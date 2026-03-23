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

// ── Inbox endpoints (ADR-060) ─────────────────────────

#[derive(Debug, Deserialize)]
pub struct NotifyRequest {
    pub agent_id: Option<String>,
    pub project_id: Option<String>,
    pub priority: u8,
    pub kind: String,
    pub payload: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct InboxQueryParams {
    pub min_priority: Option<u8>,
    pub unacked_only: Option<bool>,
}

/// POST /api/hexflo/inbox/notify — send a notification to an agent or broadcast to a project
pub async fn inbox_notify(
    State(state): State<SharedState>,
    Json(body): Json<NotifyRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let port = match &state.state_port {
        Some(p) => p,
        None => return no_state_port(),
    };

    let payload = body.payload.unwrap_or_else(|| "{}".to_string());

    if let Some(agent_id) = &body.agent_id {
        match port.inbox_notify(agent_id, body.priority, &body.kind, &payload).await {
            Ok(()) => (StatusCode::CREATED, Json(json!({ "ok": true, "target": "agent", "agentId": agent_id }))),
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))),
        }
    } else if let Some(project_id) = &body.project_id {
        match port.inbox_notify_all(project_id, body.priority, &body.kind, &payload).await {
            Ok(()) => (StatusCode::CREATED, Json(json!({ "ok": true, "target": "project", "projectId": project_id }))),
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))),
        }
    } else {
        (StatusCode::BAD_REQUEST, Json(json!({ "error": "Either agent_id or project_id is required" })))
    }
}

/// GET /api/hexflo/inbox/:agent_id — query an agent's inbox
pub async fn inbox_query(
    State(state): State<SharedState>,
    Path(agent_id): Path<String>,
    Query(params): Query<InboxQueryParams>,
) -> (StatusCode, Json<serde_json::Value>) {
    let port = match &state.state_port {
        Some(p) => p,
        None => return no_state_port(),
    };

    let unacked = params.unacked_only.unwrap_or(true);
    match port.inbox_query(&agent_id, params.min_priority, unacked).await {
        Ok(notifications) => (StatusCode::OK, Json(json!({ "notifications": notifications, "count": notifications.len() }))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))),
    }
}

#[derive(Debug, Deserialize)]
pub struct AckRequest {
    pub agent_id: String,
}

/// PATCH /api/hexflo/inbox/:id/ack — acknowledge a notification
pub async fn inbox_acknowledge(
    State(state): State<SharedState>,
    Path(notification_id): Path<u64>,
    Json(body): Json<AckRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let port = match &state.state_port {
        Some(p) => p,
        None => return no_state_port(),
    };

    match port.inbox_acknowledge(notification_id, &body.agent_id).await {
        Ok(()) => (StatusCode::OK, Json(json!({ "ok": true, "acknowledged": notification_id }))),
        Err(e) => {
            let status = if e.to_string().contains("not the target") {
                StatusCode::FORBIDDEN
            } else if e.to_string().contains("not found") {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            (status, Json(json!({ "error": e.to_string() })))
        }
    }
}

/// POST /api/hexflo/inbox/expire — expire stale notifications
pub async fn inbox_expire(
    State(state): State<SharedState>,
) -> (StatusCode, Json<serde_json::Value>) {
    let port = match &state.state_port {
        Some(p) => p,
        None => return no_state_port(),
    };

    // Default: expire notifications older than 24 hours
    match port.inbox_expire(86400).await {
        Ok(count) => (StatusCode::OK, Json(json!({ "ok": true, "expiredCount": count }))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))),
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

// ── Enforcement Rules (ADR-2603221959 P5) ──────────────

fn enforcement_rules_dir() -> std::path::PathBuf {
    let base = std::env::var("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("/tmp"));
    base.join(".hex").join("enforcement-rules")
}

/// GET /api/hexflo/enforcement-rules — list all rules
pub async fn enforcement_rules_list(
    State(_state): State<SharedState>,
) -> (StatusCode, Json<serde_json::Value>) {
    let dir = enforcement_rules_dir();
    if !dir.is_dir() {
        return (StatusCode::OK, Json(json!({ "ok": true, "rules": [], "count": 0 })));
    }

    let mut rules = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            if entry.path().extension().and_then(|e| e.to_str()) == Some("json") {
                if let Ok(content) = std::fs::read_to_string(entry.path()) {
                    if let Ok(rule) = serde_json::from_str::<serde_json::Value>(&content) {
                        rules.push(rule);
                    }
                }
            }
        }
    }

    (StatusCode::OK, Json(json!({ "ok": true, "rules": rules, "count": rules.len() })))
}

/// POST /api/hexflo/enforcement-rules — upsert a rule
pub async fn enforcement_rules_upsert(
    State(_state): State<SharedState>,
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let id = match body["id"].as_str() {
        Some(id) if !id.is_empty() => id.to_string(),
        _ => return (StatusCode::BAD_REQUEST, Json(json!({ "error": "id is required" }))),
    };

    let dir = enforcement_rules_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": "Failed to create rules dir" })));
    }

    let path = dir.join(format!("{}.json", id));
    match serde_json::to_string_pretty(&body) {
        Ok(content) => {
            if std::fs::write(&path, content).is_ok() {
                (StatusCode::OK, Json(json!({ "ok": true, "id": id })))
            } else {
                (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": "Failed to write rule" })))
            }
        }
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e.to_string() }))),
    }
}

/// PATCH /api/hexflo/enforcement-rules/toggle — enable/disable a rule
pub async fn enforcement_rules_toggle(
    State(_state): State<SharedState>,
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let id = match body["id"].as_str() {
        Some(id) => id.to_string(),
        None => return (StatusCode::BAD_REQUEST, Json(json!({ "error": "id is required" }))),
    };
    let enabled = body["enabled"].as_u64().unwrap_or(1);

    let dir = enforcement_rules_dir();
    let path = dir.join(format!("{}.json", id));
    if !path.exists() {
        return (StatusCode::NOT_FOUND, Json(json!({ "error": format!("Rule '{}' not found", id) })));
    }

    match std::fs::read_to_string(&path) {
        Ok(content) => {
            if let Ok(mut rule) = serde_json::from_str::<serde_json::Value>(&content) {
                rule["enabled"] = serde_json::json!(enabled);
                if let Ok(updated) = serde_json::to_string_pretty(&rule) {
                    let _ = std::fs::write(&path, updated);
                }
                (StatusCode::OK, Json(json!({ "ok": true, "id": id, "enabled": enabled })))
            } else {
                (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": "Failed to parse rule" })))
            }
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))),
    }
}
