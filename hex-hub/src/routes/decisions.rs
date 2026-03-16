use axum::{
    extract::{Path, State},
    Json,
};
use http::StatusCode;
use serde_json::json;

use crate::state::{DecisionRequest, SharedState, SseEvent};

pub async fn handle_decision(
    State(state): State<SharedState>,
    Path((project_id, decision_id)): Path<(String, String)>,
    Json(body): Json<DecisionRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    // Broadcast decision response via SSE
    let _ = state.sse_tx.send(SseEvent {
        project_id: Some(project_id),
        event_type: "decision-response".to_string(),
        data: json!({
            "decisionId": decision_id,
            "selectedOption": body.selected_option,
            "respondedBy": "human",
            "timestamp": chrono::Utc::now().timestamp_millis()
        }),
    });

    (StatusCode::OK, Json(json!({ "ok": true })))
}
