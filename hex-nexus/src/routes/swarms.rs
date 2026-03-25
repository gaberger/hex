use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::ports::state::{IStatePort, StateError};
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
pub struct CreateTaskRequest {
    pub title: String,
    /// Comma-separated task IDs this task depends on (empty or absent = no deps).
    #[serde(default)]
    pub depends_on: String,
    /// Agent to assign immediately on creation (optional).
    #[serde(default, rename = "agentId")]
    pub agent_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateTaskRequest {
    pub status: Option<String>,
    pub result: Option<String>,
    pub agent_id: Option<String>,
    /// CAS version — must match current task.version (ADR-2603241900).
    /// Omit to skip version check (legacy / force-assign).
    pub version: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct FailSwarmRequest {
    #[serde(default = "default_fail_reason")]
    pub reason: String,
}

fn default_fail_reason() -> String {
    "manually failed".to_string()
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
    headers: axum::http::HeaderMap,
    Json(body): Json<CreateSwarmRequest>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    let port = state_port(&state)?;
    let id = uuid::Uuid::new_v4().to_string();
    let topology = body.topology.as_deref().unwrap_or("hierarchical");
    let created_by = headers
        .get("x-hex-agent-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    port.swarm_init(&id, &body.name, topology, &body.project_id, created_by)
        .await
        .map_err(state_err)?;

    // Build the response value matching the old Swarm shape
    let now = chrono::Utc::now().to_rfc3339();
    let val = json!({
        "id": id,
        "projectId": body.project_id,
        "name": body.name,
        "topology": topology,
        "status": "active",
        "createdBy": created_by,
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

// DEPRECATED(ADR-039): Browser will use SpacetimeDB direct subscription
/// GET /api/swarms/active — list all non-completed swarms (with tasks)
pub async fn list_active_swarms(
    State(state): State<SharedState>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let port = state_port(&state)?;

    let swarms = port.swarm_list_active().await.map_err(state_err)?;

    // Enrich each swarm with its tasks so CLI can show counts + agent assignments
    let mut enriched = Vec::with_capacity(swarms.len());
    for swarm in &swarms {
        let tasks = port
            .swarm_task_list(Some(&swarm.id))
            .await
            .unwrap_or_default();
        let mut val = serde_json::to_value(swarm).unwrap();
        val["tasks"] = serde_json::to_value(&tasks).unwrap();
        enriched.push(val);
    }

    Ok(Json(Value::Array(enriched)))
}

// DEPRECATED(ADR-039): Browser will use SpacetimeDB direct subscription
/// GET /api/swarms/:id — get swarm with tasks and agents
pub async fn get_swarm(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let port = state_port(&state)?;

    // Find the swarm among active swarms
    let swarms = port.swarm_list_active().await.map_err(state_err)?;
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
        .map_err(state_err)?;

    let val = json!({
        "swarm": swarm,
        "tasks": tasks,
    });

    Ok(Json(val))
}

/// PATCH /api/swarms/:id — mark a swarm as completed
pub async fn complete_swarm(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let port = state_port(&state)?;
    port.swarm_complete(&id).await.map_err(state_err)?;

    let _ = state.ws_tx.send(crate::state::WsEnvelope {
        topic: "hexflo".into(),
        event: "swarm_completed".into(),
        data: json!({ "id": id }),
    });

    Ok(Json(json!({ "ok": true, "id": id })))
}

/// POST /api/swarms/:id/fail — mark a swarm as failed
pub async fn fail_swarm(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(body): Json<FailSwarmRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let port = state_port(&state)?;
    port.swarm_fail(&id, &body.reason).await.map_err(state_err)?;

    let _ = state.ws_tx.send(crate::state::WsEnvelope {
        topic: "hexflo".into(),
        event: "swarm_failed".into(),
        data: json!({ "id": id, "reason": body.reason }),
    });

    Ok(Json(json!({ "ok": true, "id": id })))
}

/// POST /api/swarms/:id/tasks — create a new task in a swarm
pub async fn create_task(
    State(state): State<SharedState>,
    Path(swarm_id): Path<String>,
    Json(body): Json<CreateTaskRequest>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    let port = state_port(&state)?;
    let id = uuid::Uuid::new_v4().to_string();

    port.swarm_task_create(&id, &swarm_id, &body.title, &body.depends_on)
        .await
        .map_err(state_err)?;

    // Assign to agent immediately if provided
    let assigned_agent = if let Some(ref aid) = body.agent_id {
        port.swarm_task_assign(&id, aid, None)
            .await
            .map_err(state_err)?;
        aid.clone()
    } else {
        String::new()
    };

    let now = chrono::Utc::now().to_rfc3339();
    let val = json!({
        "id": id,
        "swarmId": swarm_id,
        "title": body.title,
        "status": "pending",
        "agentId": assigned_agent,
        "result": "",
        "dependsOn": body.depends_on,
        "createdAt": now,
        "completedAt": "",
    });

    let _ = state.ws_tx.send(crate::state::WsEnvelope {
        topic: "hexflo".into(),
        event: "task_created".into(),
        data: val.clone(),
    });

    Ok((StatusCode::CREATED, Json(val)))
}

/// PATCH /api/swarms/:id/tasks/:taskId — update task status/result
pub async fn update_task(
    State(state): State<SharedState>,
    Path((_swarm_id, task_id)): Path<(String, String)>,
    Json(body): Json<UpdateTaskRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let port = state_port(&state)?;

    // Apply agent assignment if provided (CAS: pass version if supplied)
    if let Some(ref agent_id) = body.agent_id {
        port.swarm_task_assign(&task_id, agent_id, body.version)
            .await
            .map_err(|e| {
                // Surface CAS conflicts as 409 rather than 500
                if matches!(e, StateError::Conflict(_)) {
                    (StatusCode::CONFLICT, Json(json!({ "error": format!("{}", e) })))
                } else {
                    state_err(e)
                }
            })?;
    }

    // Apply status change
    if let Some(ref status) = body.status {
        match status.as_str() {
            "completed" => {
                let result = body.result.as_deref().unwrap_or("");
                port.swarm_task_complete(&task_id, result)
                    .await
                    .map_err(state_err)?;
            }
            "failed" => {
                let reason = body.result.as_deref().unwrap_or("unknown failure");
                port.swarm_task_fail(&task_id, reason)
                    .await
                    .map_err(state_err)?;
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

/// PATCH /api/swarms/tasks/:taskId — convenience route (no swarm ID needed)
/// Used by MCP tools where task ID is globally unique.
pub async fn update_task_by_id(
    state: State<SharedState>,
    Path(task_id): Path<String>,
    body: Json<UpdateTaskRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    // Delegate to update_task with a dummy swarm ID (handler ignores it)
    update_task(state, Path(("_".to_string(), task_id)), body).await
}

/// GET /api/hexflo/tasks/:taskId — get task with parent swarm status
/// Used by pre-agent hooks to validate task + swarm in one call.
pub async fn get_task_by_id(
    State(state): State<SharedState>,
    Path(task_id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let port = state_port(&state)?;

    // Get all tasks (no filter) and find the one we want
    let tasks = port.swarm_task_list(None).await.map_err(state_err)?;
    let task = tasks.into_iter().find(|t| t.id == task_id).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "Task not found" })),
        )
    })?;

    // Look up parent swarm to get its status
    let swarms = port.swarm_list_active().await.map_err(state_err)?;
    let swarm_status = swarms
        .iter()
        .find(|s| s.id == task.swarm_id)
        .map(|s| s.status.as_str())
        .unwrap_or("unknown");

    Ok(Json(json!({
        "id": task.id,
        "swarmId": task.swarm_id,
        "title": task.title,
        "status": task.status,
        "agentId": task.agent_id,
        "result": task.result,
        "createdAt": task.created_at,
        "completedAt": task.completed_at,
        "swarmStatus": swarm_status,
    })))
}

/// GET /api/agents/:id/swarm — get the swarm owned by this agent (if any)
pub async fn get_agent_swarm(
    State(state): State<SharedState>,
    Path(agent_id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let port = state_port(&state)?;
    match port.swarm_owned_by_agent(&agent_id).await.map_err(state_err)? {
        Some(swarm) => Ok(Json(serde_json::to_value(swarm).unwrap_or_default())),
        None => Err((StatusCode::NOT_FOUND, Json(json!({ "error": "No active swarm owned by agent" })))),
    }
}

/// POST /api/swarms/:id/transfer — transfer swarm ownership to a new agent
pub async fn transfer_swarm(
    State(state): State<SharedState>,
    Path(swarm_id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let new_owner = body.get("new_owner_agent_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, Json(json!({ "error": "new_owner_agent_id required" }))))?;
    let port = state_port(&state)?;
    port.swarm_transfer(&swarm_id, new_owner).await.map_err(state_err)?;
    Ok(Json(json!({ "ok": true, "swarmId": swarm_id, "newOwnerAgentId": new_owner })))
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
        .map_err(state_err)?;

    // Filter to incomplete tasks only
    let incomplete: Vec<_> = all_tasks
        .into_iter()
        .filter(|t| t.status != "completed" && t.status != "failed")
        .collect();

    Ok(Json(serde_json::to_value(incomplete).unwrap()))
}
