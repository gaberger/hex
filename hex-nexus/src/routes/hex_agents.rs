//! Unified agent registry REST endpoints (ADR-058).
//!
//! These replace the fragmented /api/agents (orchestration) and
//! /api/remote-agents routes with a single set of endpoints
//! backed by the hex_agent SpacetimeDB table.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::ports::state::{IStatePort, ProjectRegistration};
use crate::state::SharedState;

#[derive(Debug, Deserialize)]
pub struct ConnectRequest {
    /// Client-provided agent ID for identity persistence across nexus restarts (ADR-065 P6).
    /// If provided and valid UUID, reuse it. Otherwise generate a new one.
    pub agent_id: Option<String>,
    pub name: Option<String>,
    pub host: Option<String>,
    pub project_dir: Option<String>,
    pub model: Option<String>,
    pub session_id: Option<String>,
    pub capabilities: Option<Value>,
}

fn state_port(state: &SharedState) -> Result<&dyn IStatePort, (StatusCode, Json<Value>)> {
    state.state_port.as_deref().ok_or_else(|| {
        (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "error": "State port not available" })))
    })
}

/// POST /api/hex-agents/connect — register or re-register an agent
pub async fn connect_agent(
    State(state): State<SharedState>,
    Json(body): Json<ConnectRequest>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    let port = state_port(&state)?;

    // ADR-065 P6: reuse client-provided agent_id if it's a valid UUID
    let id = body.agent_id
        .filter(|aid| !aid.is_empty() && uuid::Uuid::parse_str(aid).is_ok())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let name = body.name.unwrap_or_else(|| format!("agent-{}", &id[..8]));
    let host = body.host.unwrap_or_else(|| {
        gethostname::gethostname().to_string_lossy().to_string()
    });
    let project_dir = body.project_dir.unwrap_or_default();
    let model = body.model.unwrap_or_default();
    let session_id = body.session_id.unwrap_or_default();
    let caps = body.capabilities.map(|c| c.to_string()).unwrap_or_else(|| "{}".to_string());

    // ADR-065 P1: auto-register project from project_dir if not already known
    let project_id = if !project_dir.is_empty() {
        let project_name = std::path::Path::new(&project_dir)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if !project_name.is_empty() {
            // Try to find existing project, register if not found
            match port.project_find(&project_name).await {
                Ok(Some(record)) => record.id,
                _ => {
                    let new_pid = uuid::Uuid::new_v4().to_string();
                    let _ = port.project_register(ProjectRegistration {
                        id: new_pid.clone(),
                        name: project_name,
                        description: String::new(),
                        root_path: project_dir.clone(),
                        ast_is_stub: true,
                    }).await;
                    new_pid
                }
            }
        } else {
            std::env::var("HEX_PROJECT_ID").unwrap_or_default()
        }
    } else {
        std::env::var("HEX_PROJECT_ID").unwrap_or_default()
    };

    port.hex_agent_connect(&id, &name, &host, &project_id, &project_dir, &model, &session_id, &caps)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))))?;

    // Write session file for the agent
    let sessions_dir = std::env::var("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("/tmp"))
        .join(".hex/sessions");
    let _ = std::fs::create_dir_all(&sessions_dir);
    if !session_id.is_empty() {
        let session_file = sessions_dir.join(format!("agent-{}.json", session_id));
        let session_data = json!({
            "agentId": id,
            "name": name,
            "sessionId": session_id,
            "project": project_id,
            "registeredAt": chrono::Utc::now().to_rfc3339(),
        });
        let _ = std::fs::write(&session_file, serde_json::to_string_pretty(&session_data).unwrap_or_default());
    }

    Ok((StatusCode::CREATED, Json(json!({
        "agentId": id,
        "name": name,
        "projectId": project_id,
        "status": "online",
    }))))
}

/// GET /api/hex-agents — list all agents
pub async fn list_agents(
    State(state): State<SharedState>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let port = state_port(&state)?;
    let agents = port.hex_agent_list().await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))))?;
    Ok(Json(json!({ "agents": agents })))
}

/// GET /api/hex-agents/:id — get agent details
pub async fn get_agent(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let port = state_port(&state)?;
    match port.hex_agent_get(&id).await {
        Ok(Some(agent)) => Ok(Json(agent)),
        Ok(None) => Err((StatusCode::NOT_FOUND, Json(json!({ "error": "Agent not found" })))),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() })))),
    }
}

/// POST /api/hex-agents/:id/heartbeat — update heartbeat
pub async fn heartbeat(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let port = state_port(&state)?;
    port.hex_agent_heartbeat(&id).await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))))?;
    Ok(Json(json!({ "ok": true })))
}

/// DELETE /api/hex-agents/:id — disconnect agent
pub async fn disconnect_agent(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let port = state_port(&state)?;
    port.hex_agent_disconnect(&id).await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))))?;
    Ok(Json(json!({ "ok": true })))
}

/// POST /api/hex-agents/evict — clean up dead agents
pub async fn evict_dead(
    State(state): State<SharedState>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let port = state_port(&state)?;
    // First mark inactive agents, then evict dead ones
    let _ = port.hex_agent_mark_inactive().await;
    port.hex_agent_evict_dead().await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))))?;
    Ok(Json(json!({ "ok": true })))
}
