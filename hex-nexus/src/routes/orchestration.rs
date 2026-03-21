use axum::{
    extract::{Path, State},
    Json,
};
use http::StatusCode;
use serde::Deserialize;
use serde_json::json;
use utoipa::ToSchema;

use crate::orchestration::agent_manager::SpawnConfig;
use crate::state::SharedState;

fn no_manager() -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({ "error": "Agent manager not initialized" })),
    )
}

fn no_executor() -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({ "error": "Workplan executor not initialized" })),
    )
}

// ── Agent Routes ───────────────────────────────────────

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SpawnRequest {
    /// Absolute path to the project directory.
    pub project_dir: String,
    /// LLM model override (e.g. "claude-sonnet-4-20250514").
    pub model: Option<String>,
    /// Agent name / type (e.g. "hex-coder", "planner", "tester").
    pub agent_name: Option<String>,
    /// Override hub URL for the spawned agent.
    pub hub_url: Option<String>,
    /// Override hub auth token for the spawned agent.
    pub hub_token: Option<String>,
    /// Secret key names to inject into the agent process (ADR-026).
    pub secret_keys: Option<Vec<String>>,
}

/// POST /api/agents/spawn — spawn a new hex-agent process
#[utoipa::path(
    post,
    path = "/api/agents/spawn",
    request_body = SpawnRequest,
    responses(
        (status = 200, description = "Agent spawned successfully"),
        (status = 500, description = "Spawn failed"),
        (status = 503, description = "Agent manager not initialized"),
    ),
    tag = "agents"
)]
pub async fn spawn_agent(
    State(state): State<SharedState>,
    Json(body): Json<SpawnRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let mgr = match &state.agent_manager {
        Some(m) => m,
        None => return no_manager(),
    };

    let config = SpawnConfig {
        project_dir: body.project_dir,
        model: body.model,
        agent_name: body.agent_name,
        hub_url: body.hub_url,
        hub_token: body.hub_token,
        secret_keys: body.secret_keys.unwrap_or_default(),
    };

    match mgr.spawn_agent(config).await {
        Ok(agent) => {
            // Broadcast agent spawn to connected chat clients
            let _ = state.ws_tx.send(crate::state::WsEnvelope {
                topic: "hexflo".into(),
                event: "agent_spawned".into(),
                data: json!({ "agent": &agent }),
            });
            (
                StatusCode::OK,
                Json(json!({
                    "agent": agent,
                    "status": "spawned",
                })),
            )
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e })),
        ),
    }
}

// DEPRECATED(ADR-039): Browser will use SpacetimeDB direct subscription
/// GET /api/agents — list all tracked agents
///
/// **Deprecated**: This route will be replaced by SpacetimeDB direct subscription (ADR-039).
#[utoipa::path(
    get,
    path = "/api/agents",
    responses(
        (status = 200, description = "List of all tracked agents"),
        (status = 503, description = "Agent manager not initialized"),
    ),
    tag = "agents"
)]
pub async fn list_agents(
    State(state): State<SharedState>,
) -> (StatusCode, Json<serde_json::Value>) {
    let mgr = match &state.agent_manager {
        Some(m) => m,
        None => return no_manager(),
    };

    match mgr.list_agents().await {
        Ok(agents) => (StatusCode::OK, Json(json!({ "agents": agents }))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e })),
        ),
    }
}

/// GET /api/agents/:id — get agent details
pub async fn get_agent(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    let mgr = match &state.agent_manager {
        Some(m) => m,
        None => return no_manager(),
    };

    match mgr.get_agent(&id).await {
        Ok(Some(agent)) => (StatusCode::OK, Json(json!({ "agent": agent }))),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "Agent not found" })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e })),
        ),
    }
}

/// DELETE /api/agents/:id — terminate an agent
pub async fn terminate_agent(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    let mgr = match &state.agent_manager {
        Some(m) => m,
        None => return no_manager(),
    };

    match mgr.terminate_agent(&id).await {
        Ok(true) => {
            // Broadcast agent termination to connected chat clients
            let _ = state.ws_tx.send(crate::state::WsEnvelope {
                topic: "hexflo".into(),
                event: "agent_terminated".into(),
                data: json!({ "agent_id": &id }),
            });
            (StatusCode::OK, Json(json!({ "ok": true, "terminated": id })))
        }
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "Agent not found" })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e })),
        ),
    }
}

/// POST /api/agents/health — trigger health check for all agents
pub async fn health_check(
    State(state): State<SharedState>,
) -> (StatusCode, Json<serde_json::Value>) {
    let mgr = match &state.agent_manager {
        Some(m) => m,
        None => return no_manager(),
    };

    match mgr.check_health().await {
        Ok(dead) => (
            StatusCode::OK,
            Json(json!({
                "ok": true,
                "deadAgents": dead,
                "deadCount": dead.len(),
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e })),
        ),
    }
}

// ── Workplan Routes ────────────────────────────────────

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExecuteWorkplanRequest {
    /// Path to the workplan JSON file.
    pub workplan_path: String,
}

/// POST /api/workplan/execute — start workplan execution
#[utoipa::path(
    post,
    path = "/api/workplan/execute",
    request_body = ExecuteWorkplanRequest,
    responses(
        (status = 200, description = "Workplan execution started"),
        (status = 500, description = "Execution failed"),
        (status = 503, description = "Workplan executor not initialized"),
    ),
    tag = "workplan"
)]
pub async fn execute_workplan(
    State(state): State<SharedState>,
    Json(body): Json<ExecuteWorkplanRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let exec = match state.workplan_executor.get() {
        Some(e) => e,
        None => return no_executor(),
    };

    match exec.start(&body.workplan_path).await {
        Ok(result) => (
            StatusCode::OK,
            Json(json!({
                "execution": result,
                "status": "started",
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e })),
        ),
    }
}

/// GET /api/workplan/status — get current workplan execution state
pub async fn workplan_status(
    State(state): State<SharedState>,
) -> (StatusCode, Json<serde_json::Value>) {
    let exec = match state.workplan_executor.get() {
        Some(e) => e,
        None => return no_executor(),
    };

    match exec.get_status().await {
        Ok(Some(status)) => (StatusCode::OK, Json(json!({ "execution": status }))),
        Ok(None) => (StatusCode::OK, Json(json!({ "execution": null }))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e })),
        ),
    }
}

/// POST /api/workplan/pause — pause the current execution
pub async fn pause_workplan(
    State(state): State<SharedState>,
) -> (StatusCode, Json<serde_json::Value>) {
    let exec = match state.workplan_executor.get() {
        Some(e) => e,
        None => return no_executor(),
    };

    match exec.pause().await {
        Ok(true) => (StatusCode::OK, Json(json!({ "ok": true, "status": "paused" }))),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "No running execution to pause" })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e })),
        ),
    }
}

/// POST /api/workplan/resume — resume a paused execution
pub async fn resume_workplan(
    State(state): State<SharedState>,
) -> (StatusCode, Json<serde_json::Value>) {
    let exec = match state.workplan_executor.get() {
        Some(e) => e,
        None => return no_executor(),
    };

    match exec.resume().await {
        Ok(true) => (StatusCode::OK, Json(json!({ "ok": true, "status": "resumed" }))),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "No paused execution to resume" })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e })),
        ),
    }
}

// ═══════════════════════════════════════════════════════════
// WORKPLAN REPORTING (ADR-046)
// ═══════════════════════════════════════════════════════════

/// GET /api/workplan/list — list all workplan executions (active + historical)
pub async fn list_workplans(
    State(state): State<SharedState>,
) -> (StatusCode, Json<serde_json::Value>) {
    let exec = match state.workplan_executor.get() {
        Some(e) => e,
        None => return no_executor(),
    };

    match exec.list_all().await {
        Ok(executions) => {
            let active_count = executions.iter()
                .filter(|e| e.status == crate::orchestration::workplan_executor::ExecutionStatus::Running
                    || e.status == crate::orchestration::workplan_executor::ExecutionStatus::Paused)
                .count();

            (StatusCode::OK, Json(json!({
                "ok": true,
                "data": {
                    "total": executions.len(),
                    "activeCount": active_count,
                    "completedCount": executions.len() - active_count,
                    "executions": executions,
                }
            })))
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e })),
        ),
    }
}

/// GET /api/workplan/{id} — detailed status of a specific workplan execution
pub async fn get_workplan(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    let exec = match state.workplan_executor.get() {
        Some(e) => e,
        None => return no_executor(),
    };

    match exec.get_by_id(&id).await {
        Ok(Some(execution)) => (StatusCode::OK, Json(json!({ "ok": true, "data": execution }))),
        Ok(None) => (StatusCode::NOT_FOUND, Json(json!({ "error": format!("Workplan '{}' not found", id) }))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e }))),
    }
}

/// GET /api/workplan/{id}/report — aggregate report for a workplan execution
pub async fn workplan_report(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    let exec = match state.workplan_executor.get() {
        Some(e) => e,
        None => return no_executor(),
    };

    match exec.get_by_id(&id).await {
        Ok(Some(execution)) => {
            let duration_minutes = {
                let started = chrono::DateTime::parse_from_rfc3339(&execution.started_at).ok();
                let ended = chrono::DateTime::parse_from_rfc3339(&execution.updated_at).ok();
                match (started, ended) {
                    (Some(s), Some(e)) => Some(e.signed_duration_since(s).num_minutes()),
                    _ => None,
                }
            };

            let mut agents_used = execution.agents.clone();
            agents_used.sort();
            agents_used.dedup();

            let gates_passed = execution.gate_results.iter().filter(|g| g.passed).count();
            let gates_failed = execution.gate_results.iter().filter(|g| !g.passed).count();

            (StatusCode::OK, Json(json!({
                "ok": true,
                "data": {
                    "workplan": {
                        "id": execution.id,
                        "feature": execution.feature,
                        "status": execution.status,
                        "workplanPath": execution.workplan_path,
                    },
                    "summary": {
                        "durationMinutes": duration_minutes,
                        "phasesTotal": execution.total_phases,
                        "phasesCompleted": execution.completed_phases,
                        "tasksTotal": execution.total_tasks,
                        "tasksCompleted": execution.completed_tasks,
                        "tasksFailed": execution.failed_tasks,
                        "agentsUsed": agents_used,
                        "gatesPassed": gates_passed,
                        "gatesFailed": gates_failed,
                    },
                    "phases": execution.phase_results,
                    "gates": execution.gate_results,
                }
            })))
        }
        Ok(None) => (StatusCode::NOT_FOUND, Json(json!({ "error": format!("Workplan '{}' not found", id) }))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e }))),
    }
}
