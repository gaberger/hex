use axum::{
    extract::{Path, Query, State},
    Json,
};
use http::StatusCode;
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::state::{HubCommand, HubCommandResult, SharedState, WsEnvelope};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendCommandRequest {
    #[serde(rename = "type")]
    pub command_type: String,
    pub payload: Option<serde_json::Value>,
    pub source: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ListParams {
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandResultRequest {
    pub status: String,
    pub data: Option<serde_json::Value>,
    pub error: Option<String>,
}

/// POST /api/{project_id}/command — send a command to a project
pub async fn send_command(
    State(state): State<SharedState>,
    Path(project_id): Path<String>,
    Json(body): Json<SendCommandRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let command_id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let source = body.source.unwrap_or_else(|| "browser".to_string());
    let payload = body.payload.unwrap_or(serde_json::Value::Object(Default::default()));

    // Single write lock: verify project exists, insert command as "dispatched" atomically
    let broadcast_data = {
        let projects = state.projects.read().await;
        if !projects.contains_key(&project_id) {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "Project not registered" })),
            );
        }
        drop(projects);

        let command = HubCommand {
            command_id: command_id.clone(),
            project_id: project_id.clone(),
            command_type: body.command_type.clone(),
            payload: payload.clone(),
            issued_at: now.clone(),
            source: source.clone(),
            status: "dispatched".to_string(),
        };

        let mut commands = state.commands.write().await;
        commands.insert(command_id.clone(), command);
        // Snapshot broadcast data before dropping lock
        json!({
            "commandId": command_id,
            "projectId": project_id,
            "type": body.command_type,
            "payload": payload,
            "issuedAt": now,
            "source": source,
        })
    };

    // Broadcast AFTER releasing the lock
    let topic = format!("project:{}:command", project_id);
    if state.ws_tx.send(WsEnvelope {
        topic,
        event: "command".to_string(),
        data: broadcast_data,
    }).is_err() {
        tracing::warn!("WS broadcast failed for command {}: no receivers", command_id);
    }

    (
        StatusCode::OK,
        Json(json!({ "commandId": command_id, "status": "dispatched" })),
    )
}

/// POST /api/{project_id}/command/{command_id}/result — project reports command result
pub async fn report_result(
    State(state): State<SharedState>,
    Path((project_id, command_id)): Path<(String, String)>,
    Json(body): Json<CommandResultRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let now = chrono::Utc::now().to_rfc3339();

    let result = HubCommandResult {
        command_id: command_id.clone(),
        status: body.status.clone(),
        data: body.data.clone(),
        error: body.error.clone(),
        completed_at: now,
    };

    // Store result and update command status in one scope
    {
        let mut results = state.results.write().await;
        results.insert(command_id.clone(), result);
    }
    {
        let mut commands = state.commands.write().await;
        if let Some(cmd) = commands.get_mut(&command_id) {
            cmd.status = body.status.clone();
        }
    }

    // Broadcast result to browser subscribers (topic: project:{id}:result)
    let topic = format!("project:{}:result", project_id);
    if state.ws_tx.send(WsEnvelope {
        topic,
        event: "command-result".to_string(),
        data: json!({
            "commandId": command_id,
            "projectId": project_id,
            "status": body.status,
            "data": body.data,
            "error": body.error,
        }),
    }).is_err() {
        tracing::warn!("WS broadcast failed for result {}: no receivers", command_id);
    }

    (StatusCode::OK, Json(json!({ "ok": true })))
}

/// GET /api/{project_id}/command/{command_id} — check command status
pub async fn get_command(
    State(state): State<SharedState>,
    Path((_project_id, command_id)): Path<(String, String)>,
) -> (StatusCode, Json<serde_json::Value>) {
    // Check for result first
    {
        let results = state.results.read().await;
        if let Some(result) = results.get(&command_id) {
            return (
                StatusCode::OK,
                Json(json!({
                    "commandId": result.command_id,
                    "status": result.status,
                    "data": result.data,
                    "error": result.error,
                    "completedAt": result.completed_at,
                })),
            );
        }
    }

    // Fall back to command status
    let commands = state.commands.read().await;
    match commands.get(&command_id) {
        Some(cmd) => (
            StatusCode::OK,
            Json(json!({
                "commandId": cmd.command_id,
                "status": cmd.status,
                "type": cmd.command_type,
                "issuedAt": cmd.issued_at,
            })),
        ),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "Command not found" })),
        ),
    }
}

/// GET /api/{project_id}/commands — list recent commands for a project
pub async fn list_commands(
    State(state): State<SharedState>,
    Path(project_id): Path<String>,
    Query(params): Query<ListParams>,
) -> Json<serde_json::Value> {
    let limit = params.limit.unwrap_or(50);
    let commands = state.commands.read().await;

    let mut project_cmds: Vec<&HubCommand> = commands
        .values()
        .filter(|c| c.project_id == project_id)
        .collect();

    // Sort by issued_at descending (most recent first)
    project_cmds.sort_by(|a, b| b.issued_at.cmp(&a.issued_at));
    project_cmds.truncate(limit);

    let list: Vec<serde_json::Value> = project_cmds
        .iter()
        .map(|c| {
            json!({
                "commandId": c.command_id,
                "type": c.command_type,
                "status": c.status,
                "source": c.source,
                "issuedAt": c.issued_at,
            })
        })
        .collect();

    Json(json!({ "commands": list }))
}
