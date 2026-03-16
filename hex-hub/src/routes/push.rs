use axum::{extract::State, Json};
use http::StatusCode;
use serde_json::json;

use crate::state::{EventRequest, PushRequest, SharedState, WsEnvelope};

pub async fn push_state(
    State(state): State<SharedState>,
    Json(body): Json<PushRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    // Snapshot broadcast data while holding the lock, then broadcast after releasing
    let broadcast_envelope = {
        let mut projects = state.projects.write().await;
        let entry = match projects.get_mut(&body.project_id) {
            Some(e) => e,
            None => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(json!({ "error": "Project not registered" })),
                )
            }
        };

        entry.last_push_at = chrono::Utc::now().timestamp_millis();

        let data = body.data.unwrap_or(serde_json::Value::Null);

        match body.push_type.as_str() {
            "health" => entry.state.health = Some(data),
            "tokens" => entry.state.tokens = Some(data),
            "tokenFile" => {
                if let Some(file_path) = &body.file_path {
                    entry.state.token_files.insert(file_path.clone(), data);
                }
            }
            "swarm" => entry.state.swarm = Some(data),
            "graph" => entry.state.graph = Some(data),
            "project" => {
                entry.state.project = serde_json::from_value(data).ok();
            }
            other => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({ "error": format!("Unknown state type: {}", other) })),
                )
            }
        }

        // Snapshot envelope before dropping write lock
        WsEnvelope {
            topic: format!("project:{}:state", body.project_id),
            event: "state-update".to_string(),
            data: json!({
                "projectId": body.project_id,
                "type": body.push_type,
                "timestamp": chrono::Utc::now().timestamp_millis()
            }),
        }
    };
    // Lock released — broadcast without holding it
    if state.ws_tx.send(broadcast_envelope).is_err() {
        tracing::debug!("WS broadcast: no receivers for state-update");
    }

    (StatusCode::OK, Json(json!({ "ok": true })))
}

pub async fn push_event(
    State(state): State<SharedState>,
    Json(body): Json<EventRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    // Update timestamp while holding lock, snapshot broadcast data, then release
    let broadcast_envelope = {
        let mut projects = state.projects.write().await;
        let entry = match projects.get_mut(&body.project_id) {
            Some(e) => e,
            None => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(json!({ "error": "Project not registered" })),
                )
            }
        };

        entry.last_push_at = chrono::Utc::now().timestamp_millis();

        let mut event_data = body.data.unwrap_or(serde_json::Value::Object(Default::default()));
        if let Some(obj) = event_data.as_object_mut() {
            obj.insert("project".to_string(), json!(body.project_id));
        }

        WsEnvelope {
            topic: format!("project:{}:events", body.project_id),
            event: body.event,
            data: event_data,
        }
    };
    // Lock released — broadcast without holding it
    if state.ws_tx.send(broadcast_envelope).is_err() {
        tracing::debug!("WS broadcast: no receivers for event");
    }

    (StatusCode::OK, Json(json!({ "ok": true })))
}
