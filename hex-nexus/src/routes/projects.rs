use axum::{
    extract::{Path, State},
    Json,
};
use http::StatusCode;
use serde_json::json;

use crate::state::{
    make_project_id, ProjectEntry, ProjectMeta, ProjectState, SharedState,
    WsEnvelope,
};

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

    // Hold write lock, snapshot broadcast data, release before broadcasting
    let broadcast_envelope = {
        let mut projects = state.projects.write().await;
        let is_new = !projects.contains_key(&id);

        if is_new {
            let entry = ProjectEntry {
                id: id.clone(),
                name: name.clone(),
                root_path: root_path.clone(),
                registered_at: chrono::Utc::now().timestamp_millis(),
                last_push_at: 0,
                state: ProjectState {
                    project: Some(ProjectMeta {
                        root_path: root_path.clone(),
                        name: name.clone(),
                        ast_is_stub,
                    }),
                    ..Default::default()
                },
            };
            projects.insert(id.clone(), entry);

            Some(WsEnvelope {
                topic: "hub:projects".to_string(),
                event: "project-registered".to_string(),
                data: json!({
                    "id": id,
                    "name": name,
                    "rootPath": root_path,
                    "timestamp": chrono::Utc::now().timestamp_millis()
                }),
            })
        } else {
            // Update metadata on re-register
            let entry = projects.get_mut(&id).unwrap();
            entry.name = name.clone();
            entry.state.project = Some(ProjectMeta {
                root_path: root_path.clone(),
                name: name.clone(),
                ast_is_stub,
            });
            None
        }
    };

    // Broadcast AFTER releasing write lock
    if let Some(envelope) = broadcast_envelope {
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
    let broadcast_envelope = {
        let mut projects = state.projects.write().await;
        if projects.remove(&id).is_some() {
            Some(WsEnvelope {
                topic: "hub:projects".to_string(),
                event: "project-unregistered".to_string(),
                data: json!({ "id": id, "timestamp": chrono::Utc::now().timestamp_millis() }),
            })
        } else {
            return (StatusCode::NOT_FOUND, Json(json!({ "error": "Not found" })));
        }
    };

    if let Some(envelope) = broadcast_envelope {
        if state.ws_tx.send(envelope).is_err() {
            tracing::debug!("WS broadcast: no receivers for project-unregistered");
        }
    }

    (StatusCode::OK, Json(json!({ "ok": true })))
}

pub async fn list_projects(
    State(state): State<SharedState>,
) -> Json<serde_json::Value> {
    let projects = state.projects.read().await;
    let list: Vec<serde_json::Value> = projects
        .values()
        .map(|p| {
            json!({
                "id": p.id,
                "name": p.name,
                "rootPath": p.root_path,
                "registeredAt": p.registered_at,
                "lastPushAt": p.last_push_at,
                "astIsStub": p.state.project.as_ref().map(|m| m.ast_is_stub).unwrap_or(false),
            })
        })
        .collect();
    Json(json!({ "projects": list }))
}
