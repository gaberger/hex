use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use hex_core::domain::swarm_task::SwarmTaskStatus;
use hex_core::{TaskCompletionBody, TaskStatus};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::middleware::capability_auth::require_capability;
use crate::ports::state::{IStatePort, StateError};
use crate::state::SharedState;
use hex_core::domain::capability::VerifiedClaims;

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
    /// Typed status — invalid strings return 422 at deserialization (ADR-2603311000).
    pub status: Option<SwarmTaskStatus>,
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
    let raw_topology = body.topology.as_deref().unwrap_or("hierarchical");
    // "hex-pipeline" is hex-nexus's internal name for the phased dev topology.
    // SpacetimeDB only accepts the canonical set; map before forwarding.
    let topology = if raw_topology == "hex-pipeline" { "pipeline" } else { raw_topology };
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

/// GET /api/swarms/failed — list failed swarms enriched with tasks (for zombie cleanup)
pub async fn list_failed_swarms(
    State(state): State<SharedState>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let port = state_port(&state)?;

    let swarms = port.swarm_list_failed().await.map_err(state_err)?;

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

/// GET /api/swarms/all?limit=N — list all swarms (all statuses), most recent first
pub async fn list_all_swarms(
    State(state): State<SharedState>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let port = state_port(&state)?;
    let limit = params.get("limit").and_then(|v| v.parse::<usize>().ok()).unwrap_or(50);

    let swarms = port.swarm_list_all(limit).await.map_err(state_err)?;

    let mut enriched = Vec::with_capacity(swarms.len());
    for swarm in &swarms {
        let tasks = port
            .swarm_task_list(Some(&swarm.id))
            .await
            .unwrap_or_default();
        let total     = tasks.len() as u64;
        let completed = tasks.iter().filter(|t| t.status == "completed").count() as u64;
        let failed    = tasks.iter().filter(|t| t.status == "failed").count() as u64;
        let in_prog   = tasks.iter().filter(|t| t.status == "in_progress").count() as u64;
        let mut val = serde_json::to_value(swarm).unwrap();
        val["taskSummary"] = serde_json::json!({
            "total": total, "completed": completed, "failed": failed, "inProgress": in_prog
        });
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

    // Use swarm_get so completed/failed swarms are found too (not just active ones)
    let swarm = port.swarm_get(&id).await.map_err(state_err)?.ok_or_else(|| {
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
    claims: Option<axum::Extension<VerifiedClaims>>,
    Path(swarm_id): Path<String>,
    Json(body): Json<CreateTaskRequest>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    // ADR-2604051800 P1: Require swarm-write capability
    require_capability(
        claims.as_ref().map(|c| &c.0),
        |c| c.has_capability(&hex_core::Capability::SwarmWrite),
    )
    .map_err(|s| (s, Json(json!({"error": "insufficient capability: swarm_write"}))))?;

    let port = state_port(&state)?;
    let id = uuid::Uuid::new_v4().to_string();

    port.swarm_task_create(&id, &swarm_id, &body.title, &body.depends_on)
        .await
        .map_err(state_err)?;

    // Assign to agent immediately if provided (skip if agent_id is empty string)
    let assigned_agent = if let Some(ref aid) = body.agent_id {
        if !aid.is_empty() {
            port.swarm_task_assign(&id, aid, None)
                .await
                .map_err(state_err)?;
            aid.clone()
        } else {
            String::new()
        }
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
    claims: Option<axum::Extension<VerifiedClaims>>,
    Path((_swarm_id, task_id)): Path<(String, String)>,
    Json(body): Json<UpdateTaskRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    // ADR-2604051800 P1: Check task-write capability
    require_capability(
        claims.as_ref().map(|c| &c.0),
        |c| c.can_write_task(&task_id),
    )
    .map_err(|s| (s, Json(json!({"error": "insufficient capability: task_write"}))))?;

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
        match status {
            SwarmTaskStatus::Completed => {
                let result = body.result.as_deref().unwrap_or("");
                port.swarm_task_complete(&task_id, result)
                    .await
                    .map_err(state_err)?;
            }
            SwarmTaskStatus::Failed => {
                let reason = body.result.as_deref().unwrap_or("unknown failure");
                port.swarm_task_fail(&task_id, reason)
                    .await
                    .map_err(state_err)?;
            }
            _ => {
                // For other status values (e.g. InProgress, Pending), assign is
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
            "status": body.status.as_ref().map(|s| s.as_str()),
            "result": body.result,
        }),
    });

    Ok(Json(json!({ "ok": true })))
}

/// PATCH /api/swarms/tasks/:taskId and PATCH /api/hexflo/tasks/:taskId
///
/// Accepts [`TaskCompletionBody`] (the shared hex-core type) from agents and MCP tools.
/// Converts to [`UpdateTaskRequest`] internally so the CAS and status-change logic
/// in [`update_task`] is reused unchanged.
pub async fn update_task_by_id(
    state: State<SharedState>,
    Path(task_id): Path<String>,
    Json(completion): Json<TaskCompletionBody>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let req = UpdateTaskRequest {
        status: Some(match completion.status {
            TaskStatus::Completed => SwarmTaskStatus::Completed,
            TaskStatus::Failed => SwarmTaskStatus::Failed,
            TaskStatus::InProgress => SwarmTaskStatus::InProgress,
            TaskStatus::Pending | TaskStatus::Blocked => SwarmTaskStatus::Pending,
        }),
        result: completion.result.or(completion.error),
        agent_id: completion.agent_id,
        version: None,
    };
    // Pass None for claims — update_task_by_id is an internal delegation, not a direct agent call.
    // The outer route handler should enforce capabilities if needed.
    update_task(state, None, Path(("_".to_string(), task_id)), Json(req)).await
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

/// GET /api/hexflo/tasks/claim — atomically claim a pending task for an agent.
///
/// Query params: `agent_id` (required), `swarm_id` (optional filter).
/// Returns the claimed task JSON or 204 No Content if nothing is pending.
#[derive(Deserialize)]
pub struct ClaimQuery {
    agent_id: String,
    swarm_id: Option<String>,
}

pub async fn claim_task(
    State(state): State<SharedState>,
    Query(q): Query<ClaimQuery>,
) -> Result<axum::response::Response, (StatusCode, Json<Value>)> {
    use axum::response::IntoResponse;
    let port = state_port(&state)?;

    let tasks = port.swarm_task_list(q.swarm_id.as_deref()).await.map_err(state_err)?;
    let pending = tasks.into_iter().find(|t| {
        (t.status.is_empty() || t.status == "pending") && t.agent_id.is_empty()
    });

    let Some(task) = pending else {
        return Ok(StatusCode::NO_CONTENT.into_response());
    };

    // Atomically assign the task. If another agent claimed it first, return 409.
    port.swarm_task_assign(&task.id, &q.agent_id, None)
        .await
        .map_err(|e| {
            if matches!(e, StateError::Conflict(_)) {
                (StatusCode::CONFLICT, Json(json!({ "error": format!("{}", e) })))
            } else {
                state_err(e)
            }
        })?;

    Ok(Json(json!({
        "id": task.id,
        "title": task.title,
        "swarmId": task.swarm_id,
    })).into_response())
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
