//! REST API route handlers for remote agent management (ADR-040, T4-3).
//!
//! These handlers expose the remote agent lifecycle, fleet status, and
//! spawn-via-SSH functionality over HTTP. They compose the AgentLifecycleAdapter,
//! RemoteRegistryAdapter (via IRemoteRegistryPort), and RemoteAgentOrchestrator.

use axum::{
    extract::{Path, State},
    Json,
};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

use crate::adapters::agent_lifecycle::AgentLifecycleAdapter;
use crate::adapters::remote_registry::RemoteRegistryAdapter;
use crate::ports::remote_registry::IRemoteRegistryPort;
use crate::remote::transport::*;
use crate::usecases::remote_agent_orchestrator::RemoteAgentOrchestrator;

// ── Shared state for agent routes ────────────────────

/// Dedicated state type for agent routes.
/// Will be folded into SharedState in a future iteration.
pub struct AgentState {
    pub lifecycle: Arc<AgentLifecycleAdapter>,
    pub orchestrator: Arc<RemoteAgentOrchestrator>,
    pub registry: Arc<RemoteRegistryAdapter>,
}

// ── Handlers ─────────────────────────────────────────

/// GET /api/remote-agents — list all remote agents
pub async fn list_agents(
    State(state): State<Arc<AgentState>>,
) -> Json<serde_json::Value> {
    match state.registry.list_agents(None).await {
        Ok(agents) => Json(json!(agents)),
        Err(e) => Json(json!({ "error": e.to_string() })),
    }
}

/// GET /api/remote-agents/:id — get a single agent's details
pub async fn get_agent(
    State(state): State<Arc<AgentState>>,
    Path(agent_id): Path<String>,
) -> Json<serde_json::Value> {
    match state.registry.get_agent(&agent_id).await {
        Ok(Some(agent)) => Json(json!(agent)),
        Ok(None) => Json(json!({ "error": "Agent not found" })),
        Err(e) => Json(json!({ "error": e.to_string() })),
    }
}

/// POST /api/remote-agents/connect — register an incoming agent connection
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectRequest {
    pub agent_name: String,
    pub project_dir: String,
    pub capabilities: Option<serde_json::Value>,
}

pub async fn connect_agent(
    State(state): State<Arc<AgentState>>,
    Json(req): Json<ConnectRequest>,
) -> Json<serde_json::Value> {
    let caps = req
        .capabilities
        .and_then(|v| serde_json::from_value::<AgentCapabilities>(v).ok())
        .unwrap_or_default();

    match state
        .lifecycle
        .accept_agent(uuid::Uuid::new_v4().to_string(), caps, req.project_dir)
        .await
    {
        Ok(agent) => Json(json!(agent)),
        Err(e) => Json(json!({ "error": e.to_string() })),
    }
}

/// POST /api/remote-agents/spawn-remote — spawn an agent on a remote host via SSH tunnel
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpawnRemoteRequest {
    pub host: String,
    pub user: String,
    pub port: Option<u16>,
    pub project_dir: String,
    pub agent_name: Option<String>,
    pub key_path: Option<String>,
}

pub async fn spawn_remote_agent(
    State(state): State<Arc<AgentState>>,
    Json(req): Json<SpawnRemoteRequest>,
) -> Json<serde_json::Value> {
    let config = SshTunnelConfig {
        host: req.host.clone(),
        port: req.port.unwrap_or(22),
        user: req.user.clone(),
        auth: match req.key_path {
            Some(path) => SshAuth::Key {
                path,
                passphrase: None,
            },
            None => SshAuth::Agent,
        },
        ..Default::default()
    };

    let name = req
        .agent_name
        .unwrap_or_else(|| format!("{}@{}", req.user, req.host));

    match state
        .lifecycle
        .spawn_remote_agent(config, name, req.project_dir)
        .await
    {
        Ok(agent) => Json(json!(agent)),
        Err(e) => Json(json!({ "error": e.to_string() })),
    }
}

/// DELETE /api/remote-agents/:id — disconnect and clean up an agent
pub async fn disconnect_agent(
    State(state): State<Arc<AgentState>>,
    Path(agent_id): Path<String>,
) -> Json<serde_json::Value> {
    match state.lifecycle.disconnect_agent(&agent_id).await {
        Ok(()) => Json(json!({ "ok": true })),
        Err(e) => Json(json!({ "error": e.to_string() })),
    }
}

/// GET /api/remote-agents/fleet — fleet capacity summary
pub async fn fleet_status(
    State(state): State<Arc<AgentState>>,
) -> Json<serde_json::Value> {
    match state.orchestrator.fleet_status().await {
        Ok(status) => Json(json!(status)),
        Err(e) => Json(json!({ "error": e.to_string() })),
    }
}
