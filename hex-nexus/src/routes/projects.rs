use axum::{
    extract::{Path, State},
    Json,
};
use http::StatusCode;
use serde_json::json;

use crate::ports::state::ProjectRegistration;
use crate::state::{make_project_id, SharedState, WsEnvelope};

pub async fn register(
    State(state): State<SharedState>,
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let root_path = match body.get("rootPath").and_then(|v| v.as_str()) {
        Some(s) => s.to_string(),
        None => return (StatusCode::BAD_REQUEST, Json(json!({ "error": "Missing rootPath" }))),
    };
    let name_field = body.get("name").and_then(|v| v.as_str()).map(String::from);
    let ast_is_stub = body.get("astIsStub").and_then(|v| v.as_bool()).unwrap_or(false);

    let id = make_project_id(&root_path);
    let name = name_field
        .unwrap_or_else(|| {
            root_path
                .rsplit('/')
                .next()
                .unwrap_or("unknown")
                .to_string()
        });

    let sp = match state.require_state_port() {
        Ok(sp) => sp.clone(),
        Err(e) => return e,
    };

    // Check if project already exists
    let existing = sp.project_get(&id).await.unwrap_or(None);
    let is_new = existing.is_none();

    if let Err(e) = sp.project_register(ProjectRegistration {
        id: id.clone(),
        name: name.clone(),
        root_path: root_path.clone(),
        ast_is_stub,
    }).await {
        tracing::error!("Failed to register project: {}", e);
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() })));
    }

    // Broadcast AFTER successful registration
    if is_new {
        let envelope = WsEnvelope {
            topic: "hub:projects".to_string(),
            event: "project-registered".to_string(),
            data: json!({
                "id": id,
                "name": name,
                "rootPath": root_path,
                "timestamp": chrono::Utc::now().timestamp_millis()
            }),
        };
        if state.ws_tx.send(envelope).is_err() {
            tracing::debug!("WS broadcast: no receivers for project-registered");
        }
    }

    (StatusCode::OK, Json(json!({ "id": id, "name": name, "rootPath": root_path })))
}

pub async fn unregister(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    let sp = match state.require_state_port() {
        Ok(sp) => sp.clone(),
        Err(e) => return e,
    };

    match sp.project_unregister(&id).await {
        Ok(true) => {
            let envelope = WsEnvelope {
                topic: "hub:projects".to_string(),
                event: "project-unregistered".to_string(),
                data: json!({ "id": id, "timestamp": chrono::Utc::now().timestamp_millis() }),
            };
            if state.ws_tx.send(envelope).is_err() {
                tracing::debug!("WS broadcast: no receivers for project-unregistered");
            }
            (StatusCode::OK, Json(json!({ "ok": true })))
        }
        Ok(false) => (StatusCode::NOT_FOUND, Json(json!({ "error": "Not found" }))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))),
    }
}

pub async fn list_projects(
    State(state): State<SharedState>,
) -> Json<serde_json::Value> {
    let sp = match state.state_port.as_ref() {
        Some(sp) => sp,
        None => return Json(json!({ "projects": [] })),
    };

    let projects = sp.project_list().await.unwrap_or_default();
    let list: Vec<serde_json::Value> = projects
        .iter()
        .map(|p| {
            json!({
                "id": p.id,
                "name": p.name,
                "rootPath": p.root_path,
                "registeredAt": p.registered_at,
                "lastPushAt": p.last_push_at,
                "astIsStub": p.ast_is_stub,
            })
        })
        .collect();
    Json(json!({ "projects": list }))
}
