//! REST endpoints for the tool-call event log (ADR-2604012137, ADR-2604020900).
//!
//! POST /api/events  — write one event; broadcasts to WebSocket dashboard.
//! GET  /api/events  — query recent events for a session (initial dashboard load).

use axum::{
    extract::{Query, State},
    Json,
};
use http::StatusCode;
use serde::Deserialize;
use serde_json::json;

use crate::ports::events::InsertEventRequest;
use crate::state::{SharedState, WsEnvelope};

// ── POST /api/events ──────────────────────────────────────────────────────

/// Insert one tool-call event and broadcast it to WebSocket dashboard clients.
pub async fn post_event(
    State(state): State<SharedState>,
    Json(body): Json<InsertEventRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let (id, created_at) = state.event_adapter.insert_event(&body).await;

    let _ = state.ws_tx.send(WsEnvelope {
        topic: "events".to_string(),
        event: "tool_event".to_string(),
        data: json!({
            "id": id,
            "session_id": body.session_id,
            "agent_id": body.agent_id,
            "event_type": body.event_type,
            "tool_name": body.tool_name,
            "exit_code": body.exit_code,
            "duration_ms": body.duration_ms,
            "model_used": body.model_used,
            "context_strategy": body.context_strategy,
            "rl_action": body.rl_action,
            "input_tokens": body.input_tokens,
            "output_tokens": body.output_tokens,
            "cost_usd": body.cost_usd,
            "hex_layer": body.hex_layer,
            "created_at": created_at,
        }),
    });

    (StatusCode::CREATED, Json(json!({ "id": id, "created_at": created_at })))
}

// ── GET /api/events ───────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ListEventsParams {
    pub session_id: Option<String>,
    pub limit: Option<u32>,
}

/// List tool-call events for a session, newest first.
pub async fn list_events(
    State(state): State<SharedState>,
    Query(params): Query<ListEventsParams>,
) -> (StatusCode, Json<serde_json::Value>) {
    let limit = params.limit.unwrap_or(100);
    let events = state.event_adapter.list_events(params.session_id.as_deref(), limit).await;
    let count = events.len();
    (StatusCode::OK, Json(json!({ "events": events, "count": count })))
}
