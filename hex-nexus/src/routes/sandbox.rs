//! Docker sandbox agent routes — POST /api/agents/spawn and DELETE /api/agents/:agent_id
//!
//! Provides container-backed agent lifecycle management via DockerSandboxAdapter.
//! Adapters are created per-request (step-9 will wire them into AppState).

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use hex_core::domain::sandbox::SandboxConfig;
use hex_core::ports::sandbox::ISandboxPort;
use crate::state::SharedState;

#[derive(Deserialize)]
pub struct SpawnRequest {
    pub worktree_path: String,
    pub task_id: String,
    #[serde(default)]
    pub env_vars: HashMap<String, String>,
    #[serde(default)]
    pub network_allow: Vec<String>,
}

#[derive(Serialize)]
pub struct SpawnResponse {
    pub container_id: String,
    pub agent_id: String,
    pub status: String,
}

/// POST /api/agents/spawn — launch a Docker sandbox container for a hex agent.
pub async fn spawn_agent(
    State(_state): State<SharedState>,
    Json(req): Json<SpawnRequest>,
) -> Result<Json<SpawnResponse>, (StatusCode, String)> {
    // Validate worktree path exists
    let worktree = std::path::PathBuf::from(&req.worktree_path);
    if !worktree.exists() {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("worktree_path does not exist: {}", req.worktree_path),
        ));
    }

    // Build default network allow list
    let network_allow = if req.network_allow.is_empty() {
        vec![
            "host.docker.internal:3033".to_string(),
            "host.docker.internal:5555".to_string(),
            "openrouter.ai:443".to_string(),
        ]
    } else {
        req.network_allow
    };

    let config = SandboxConfig {
        worktree_path: worktree,
        task_id: req.task_id,
        env_vars: req.env_vars,
        network_allow,
        docker_host: None,
    };

    // Create DockerSandboxAdapter per-request (step-9 will wire via AppState)
    let adapter = crate::adapters::docker_sandbox::DockerSandboxAdapter::new()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let result = adapter
        .spawn(config)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(SpawnResponse {
        container_id: result.container_id,
        agent_id: result.agent_id,
        status: "starting".to_string(),
    }))
}

/// DELETE /api/agents/:agent_id — stop and remove the Docker container for the given agent.
pub async fn stop_agent(
    State(_state): State<SharedState>,
    Path(agent_id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    let adapter = crate::adapters::docker_sandbox::DockerSandboxAdapter::new()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Find container by hex-agent-id label
    let containers = adapter
        .list()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if let Some(c) = containers.iter().find(|c| c.agent_id == agent_id) {
        adapter
            .stop(&c.container_id)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err((StatusCode::NOT_FOUND, format!("agent not found: {}", agent_id)))
    }
}
