//! REST endpoints for decision resolution (ADR-2604131500 P1.4).
//!
//! POST /api/{project_id}/decisions/{decision_id}  — legacy WS broadcast
//! POST /api/decisions/{id}                        — resolve via inbox ack

use axum::{
    extract::{Path, State},
    Json,
};
use http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::state::{DecisionRequest, SharedState, WsEnvelope};

/// Legacy project-scoped decision handler (WS broadcast).
pub async fn handle_decision(
    State(state): State<SharedState>,
    Path((project_id, decision_id)): Path<(String, String)>,
    Json(body): Json<DecisionRequest>,
) -> (StatusCode, Json<Value>) {
    // Broadcast decision response via WS
    if state.ws_tx.send(WsEnvelope {
        topic: format!("project:{}:decisions", project_id),
        event: "decision-response".to_string(),
        data: json!({
            "decisionId": decision_id,
            "selectedOption": body.selected_option,
            "respondedBy": "human",
            "timestamp": chrono::Utc::now().timestamp_millis()
        }),
    }).is_err() {
        tracing::warn!("WS broadcast failed for decision {}: no receivers", decision_id);
    }

    (StatusCode::OK, Json(json!({ "ok": true })))
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ResolveDecisionRequest {
    pub action: String,
    pub value: Option<String>,
}

/// POST /api/decisions/{id} — resolve a pending decision.
///
/// Acknowledges the corresponding inbox notification and broadcasts the
/// resolution over WebSocket so dashboards update in real time.
pub async fn resolve_decision(
    State(state): State<SharedState>,
    Path(id): Path<u64>,
    Json(body): Json<ResolveDecisionRequest>,
) -> (StatusCode, Json<Value>) {
    // Validate action
    let valid_actions = ["approve", "reject", "override"];
    if !valid_actions.contains(&body.action.as_str()) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": format!(
                    "Invalid action '{}'. Must be one of: {}",
                    body.action,
                    valid_actions.join(", ")
                )
            })),
        );
    }

    let port = match &state.state_port {
        Some(p) => p,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "error": "IStatePort not initialized (no SpacetimeDB backend)" })),
            );
        }
    };

    // Acknowledge the inbox notification (system agent resolves it)
    if let Err(e) = port.inbox_acknowledge(id, "system").await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("Failed to acknowledge decision: {}", e) })),
        );
    }

    // Broadcast resolution via WS for real-time dashboard updates
    let _ = state.ws_tx.send(WsEnvelope {
        topic: "decisions".to_string(),
        event: "decision-resolved".to_string(),
        data: json!({
            "id": id,
            "action": body.action,
            "value": body.value,
            "resolved_by": "human",
            "resolved_at": chrono::Utc::now().to_rfc3339(),
        }),
    });

    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "id": id,
            "action": body.action,
        })),
    )
}
