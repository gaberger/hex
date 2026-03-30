//! REST endpoints for the agent notification inbox (ADR-060).
//!
//! GET  /api/inbox         — list unacknowledged notifications (optionally filtered by project_id)
//! POST /api/inbox/:id/ack — acknowledge a notification

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

#[derive(Debug, Deserialize)]
pub struct InboxQueryParams {
    /// Optional project filter — when set, only notifications for agents in this project are returned.
    pub project_id: Option<String>,
    pub min_priority: Option<u8>,
}

/// GET /api/inbox — list unacknowledged notifications across all agents.
///
/// Returns a JSON array (not an object) so the frontend can iterate directly:
/// `[{ id, priority, message, from_agent, created_at, acknowledged }, ...]`
pub async fn list_inbox(
    State(state): State<SharedState>,
    Query(params): Query<InboxQueryParams>,
) -> (StatusCode, Json<Value>) {
    let port = match &state.state_port {
        Some(p) => p,
        None => return no_state_port(),
    };

    // Wildcard agent_id: query every agent by using "*" sentinel, which the state
    // adapter interprets as "all agents". If the adapter doesn't support that, fall
    // back to an empty list gracefully.
    let agent_id = params.project_id.as_deref().unwrap_or("*");

    let notifications = match port.inbox_query(agent_id, params.min_priority, true).await {
        Ok(n) => n,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            );
        }
    };

    // Map InboxNotification → dashboard-friendly shape
    let items: Vec<Value> = notifications
        .into_iter()
        .map(|n| {
            // Compose a human-readable message from kind + payload
            let message = if n.payload.is_empty() || n.payload == "{}" {
                n.kind.clone()
            } else {
                // Try to parse payload as JSON and extract a "message" key if present
                if let Ok(p) = serde_json::from_str::<Value>(&n.payload) {
                    p.get("message")
                        .and_then(|m| m.as_str())
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| format!("{}: {}", n.kind, n.payload))
                } else {
                    format!("{}: {}", n.kind, n.payload)
                }
            };

            let acknowledged = n.acknowledged_at.is_some();

            json!({
                "id": n.id,
                "priority": n.priority,
                "message": message,
                "from_agent": n.agent_id,
                "created_at": n.created_at,
                "acknowledged": acknowledged,
            })
        })
        .collect();

    (StatusCode::OK, Json(json!(items)))
}

#[derive(Debug, Deserialize)]
pub struct AckBody {
    /// Optional: agent_id of the acknowledging agent. Defaults to "*" for dashboard acks.
    pub agent_id: Option<String>,
}

/// POST /api/inbox/:id/ack — acknowledge a notification from the dashboard.
pub async fn ack_notification(
    State(state): State<SharedState>,
    Path(notification_id): Path<u64>,
    body: Option<Json<AckBody>>,
) -> (StatusCode, Json<Value>) {
    let port = match &state.state_port {
        Some(p) => p,
        None => return no_state_port(),
    };

    // Dashboard acks on behalf of the target agent; use "*" when no agent specified.
    let agent_id = body
        .as_ref()
        .and_then(|b| b.agent_id.as_deref())
        .unwrap_or("*")
        .to_string();

    match port.inbox_acknowledge(notification_id, &agent_id).await {
        Ok(()) => (
            StatusCode::OK,
            Json(json!({ "ok": true, "acknowledged": notification_id })),
        ),
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
