use axum::{extract::State, Json};
use http::StatusCode;
use serde_json::json;

use crate::state::{EventRequest, PushRequest, SharedState, WsEnvelope};

pub async fn push_state(
    State(state): State<SharedState>,
    Json(body): Json<PushRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let sp = match state.require_state_port() {
        Ok(sp) => sp.clone(),
        Err(e) => return e,
    };

    // Verify project exists
    match sp.project_get(&body.project_id).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "Project not registered" })),
            )
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
        }
    }

    let data = body.data.unwrap_or(serde_json::Value::Null);

    // Validate push_type before updating
    match body.push_type.as_str() {
        "health" | "tokens" | "tokenFile" | "swarm" | "graph" | "project" => {}
        other => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": format!("Unknown state type: {}", other) })),
            )
        }
    }

    if let Err(e) = sp.project_update_state(
        &body.project_id,
        &body.push_type,
        data,
        body.file_path.as_deref(),
    ).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        );
    }

    // Broadcast state update
    let envelope = WsEnvelope {
        topic: format!("project:{}:state", body.project_id),
        event: "state-update".to_string(),
        data: json!({
            "projectId": body.project_id,
            "type": body.push_type,
            "timestamp": chrono::Utc::now().timestamp_millis()
        }),
    };
    if state.ws_tx.send(envelope).is_err() {
        tracing::debug!("WS broadcast: no receivers for state-update");
    }

    (StatusCode::OK, Json(json!({ "ok": true })))
}

pub async fn push_event(
    State(state): State<SharedState>,
    Json(body): Json<EventRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let sp = match state.require_state_port() {
        Ok(sp) => sp.clone(),
        Err(e) => return e,
    };

    // Verify project exists and update timestamp
    match sp.project_get(&body.project_id).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "Project not registered" })),
            )
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
        }
    }

    // Touch last_push_at via a no-op update
    let _ = sp.project_update_state(
        &body.project_id,
        "touch",
        serde_json::Value::Null,
        None,
    ).await;

    let mut event_data = body.data.unwrap_or(serde_json::Value::Object(Default::default()));
    if let Some(obj) = event_data.as_object_mut() {
        obj.insert("project".to_string(), json!(body.project_id));
    }

    let envelope = WsEnvelope {
        topic: format!("project:{}:events", body.project_id),
        event: body.event,
        data: event_data,
    };
    if state.ws_tx.send(envelope).is_err() {
        tracing::debug!("WS broadcast: no receivers for event");
    }

    (StatusCode::OK, Json(json!({ "ok": true })))
}
