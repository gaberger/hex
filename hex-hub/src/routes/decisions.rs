use axum::{
    extract::{Path, State},
    Json,
};
use http::StatusCode;
use serde_json::json;

use crate::state::{DecisionRequest, SharedState, WsEnvelope};

pub async fn handle_decision(
    State(state): State<SharedState>,
    Path((project_id, decision_id)): Path<(String, String)>,
    Json(body): Json<DecisionRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
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
