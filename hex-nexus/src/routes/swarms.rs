use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::ports::state::IStatePort;
use crate::state::SharedState;

// ── Request types (formerly in persistence.rs) ──────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSwarmRequest {
    #[serde(default)]
    pub project_id: String,
    pub name: String,
    pub topology: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateTaskRequest {
    pub status: Option<String>,
    pub result: Option<String>,
    pub agent_id: Option<String>,
}

// ── Helpers ─────────────────────────────────────────────

fn state_port(
    state: &SharedState,
) -> Result<&dyn IStatePort, (StatusCode, Json<Value>)> {
    state
        .state_port
        .as_deref()
        .ok_or_else(|| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "error": "State port not available" })),
            )
        })
}

fn state_err(e: impl std::fmt::Display) -> (StatusCode, Json<Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "error": format!("{}", e) })),
    )
}

// ── Handlers ────────────────────────────────────────────

/// POST /api/swarms — create a new swarm
pub async fn create_swarm(
    State(state): State<SharedState>,
    Json(body): Json<CreateSwarmRequest>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    let port = state_port(&state)?;
    let id = uuid::Uuid::new_v4().to_string();
    let topology = body.topology.as_deref().unwrap_or("hierarchical");

    port.swarm_init(&id, &body.name, topology, &body.project_id)
        .await
        .map_err(|e| state_err(e))?;

    // Build the response value matching the old Swarm shape
    let now = chrono::Utc::now().to_rfc3339();
    let val = json!({
        "id": id,
        "projectId": body.project_id,
        "name": body.name,
        "topology": topology,
        "status": "active",
        "createdAt": now,
        "updatedAt": now,
    });

    // Broadcast swarm creation to connected chat clients
    let _ = state.ws_tx.send(crate::state::WsEnvelope {
        topic: "hexflo".into(),
        event: "swarm_created".into(),
        data: val.clone(),
    });

    Ok((StatusCode::CREATED, Json(val)))
}

/// GET /api/swarms/active — list all non-completed swarms
pub async fn list_active_swarms(
    State(state): State<SharedState>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let port = state_port(&state)?;

    let swarms = port.swarm_list_active().await.map_err(|e| state_err(e))?;
    Ok(Json(serde_json::to_value(swarms).unwrap()))
}

/// GET /api/swarms/:id — get swarm with tasks and agents
pub async fn get_swarm(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let port = state_port(&state)?;

    // Find the swarm among active swarms
    let swarms = port.swarm_list_active().await.map_err(|e| state_err(e))?;
    let swarm = swarms.into_iter().find(|s| s.id == id).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "Swarm not found" })),
        )
    })?;

    // Fetch tasks for this swarm
    let tasks = port
        .swarm_task_list(Some(&id))
        .await
        .map_err(|e| state_err(e))?;

    let val = json!({
        "swarm": swarm,
        "tasks": tasks,
    });

    Ok(Json(val))
}

/// PATCH /api/swarms/:id/tasks/:taskId — update task status/result
pub async fn update_task(
    State(state): State<SharedState>,
    Path((_swarm_id, task_id)): Path<(String, String)>,
    Json(body): Json<UpdateTaskRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let port = state_port(&state)?;

    // Apply agent assignment if provided
    if let Some(ref agent_id) = body.agent_id {
        port.swarm_task_assign(&task_id, agent_id)
            .await
            .map_err(|e| state_err(e))?;
    }

    // Apply status change
    if let Some(ref status) = body.status {
        match status.as_str() {
            "completed" => {
                let result = body.result.as_deref().unwrap_or("");
                port.swarm_task_complete(&task_id, result)
                    .await
                    .map_err(|e| state_err(e))?;
            }
            "failed" => {
                let reason = body.result.as_deref().unwrap_or("unknown failure");
                port.swarm_task_fail(&task_id, reason)
                    .await
                    .map_err(|e| state_err(e))?;
            }
            _ => {
                // For other status values (e.g. "in_progress"), assign is
                // the closest semantic operation; the status will be
                // reflected by the agent assignment above.
            }
        }
    }

    // Broadcast task update to connected chat clients
    let _ = state.ws_tx.send(crate::state::WsEnvelope {
        topic: "hexflo".into(),
        event: "task_updated".into(),
        data: json!({
            "task_id": task_id,
            "status": body.status,
            "result": body.result,
        }),
    });

    Ok(Json(json!({ "ok": true })))
}

/// GET /api/work-items/incomplete — all in-flight work across all swarms
pub async fn get_incomplete_work(
    State(state): State<SharedState>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let port = state_port(&state)?;

    // Get all tasks across all swarms
    let all_tasks = port
        .swarm_task_list(None)
        .await
        .map_err(|e| state_err(e))?;

    // Filter to incomplete tasks only
    let incomplete: Vec<_> = all_tasks
        .into_iter()
        .filter(|t| t.status != "completed" && t.status != "failed")
        .collect();

    Ok(Json(serde_json::to_value(incomplete).unwrap()))
}
