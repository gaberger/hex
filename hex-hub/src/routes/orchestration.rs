use axum::{
    extract::{Path, State},
    Json,
};
use http::StatusCode;
use serde::Deserialize;
use serde_json::json;

use crate::orchestration::agent_manager::{AgentManager, SpawnConfig};
use crate::orchestration::workplan_executor::WorkplanExecutor;
use crate::state::SharedState;

// ── Agent Routes ───────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpawnRequest {
    pub project_dir: String,
    pub model: Option<String>,
    pub agent_name: Option<String>,
    pub hub_url: Option<String>,
    pub hub_token: Option<String>,
}

/// POST /api/agents/spawn — spawn a new hex-agent process
pub async fn spawn_agent(
    State(state): State<SharedState>,
    Json(body): Json<SpawnRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let config = SpawnConfig {
        project_dir: body.project_dir,
        model: body.model,
        agent_name: body.agent_name,
        hub_url: body.hub_url,
        hub_token: body.hub_token,
    };

    match AgentManager::spawn_agent(&state, config).await {
        Ok(agent) => (
            StatusCode::OK,
            Json(json!({
                "agent": agent,
                "status": "spawned",
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e })),
        ),
    }
}

/// GET /api/agents — list all tracked agents
pub async fn list_agents(
    State(state): State<SharedState>,
) -> (StatusCode, Json<serde_json::Value>) {
    match AgentManager::list_agents(&state).await {
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
    match AgentManager::get_agent(&state, &id).await {
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
    match AgentManager::terminate_agent(&state, &id).await {
        Ok(true) => (StatusCode::OK, Json(json!({ "ok": true, "terminated": id }))),
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
    match AgentManager::check_health(&state).await {
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecuteWorkplanRequest {
    pub workplan_path: String,
}

/// POST /api/workplan/execute — start workplan execution
pub async fn execute_workplan(
    State(state): State<SharedState>,
    Json(body): Json<ExecuteWorkplanRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    match WorkplanExecutor::start(&state, &body.workplan_path).await {
        Ok(exec) => (
            StatusCode::OK,
            Json(json!({
                "execution": exec,
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
    match WorkplanExecutor::get_status(&state).await {
        Ok(Some(exec)) => (StatusCode::OK, Json(json!({ "execution": exec }))),
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
    match WorkplanExecutor::pause(&state).await {
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
    match WorkplanExecutor::resume(&state).await {
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
