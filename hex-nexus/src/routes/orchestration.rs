use axum::{
    extract::{Path, Query, State},
    Json,
};
use http::StatusCode;
use serde::Deserialize;
use serde_json::json;
use utoipa::ToSchema;

use crate::middleware::capability_auth::require_capability;
use crate::orchestration::agent_manager::SpawnConfig;
use crate::state::SharedState;
use hex_core::domain::capability::VerifiedClaims;

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
    /// Task prompt sent to the agent via stdin.
    pub prompt: Option<String>,
    /// Git branch for worktree isolation (ADR-004). Created if absent.
    pub worktree_branch: Option<String>,
    /// If true, spawn hex-agent in daemon mode (polls HexFlo and spawns swarms).
    pub daemon: Option<bool>,
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
    claims: Option<axum::Extension<VerifiedClaims>>,
    Json(body): Json<SpawnRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    // ADR-2604051800 P1: Agent spawning requires admin capability
    if let Err(status) = require_capability(
        claims.as_ref().map(|c| &c.0),
        |c| c.is_admin(),
    ) {
        return (status, Json(json!({"error": "insufficient capability: admin required to spawn agents"})));
    }

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
        prompt: body.prompt,
        worktree_branch: body.worktree_branch,
        wait_for_completion: false,
        daemon: body.daemon.unwrap_or(false),
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

/// POST /api/agents/connect — register an incoming agent (remote or Claude Code session)
///
/// Accepts optional fields to populate agent metadata:
///   - `host`        — hostname (default: "unknown")
///   - `name`        — agent display name (default: "remote-{host}")
///   - `project_dir` — project root path
///   - `model`       — LLM model identifier
///   - `session_id`  — Claude Code session ID (stored in metadata)
pub async fn connect_agent(
    State(state): State<SharedState>,
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let host = body["host"].as_str().unwrap_or("unknown").to_string();
    let agent_id = uuid::Uuid::new_v4().to_string();
    let agent_name = body["name"].as_str()
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("remote-{}", host));
    let project_dir = body["project_dir"].as_str().unwrap_or("").to_string();
    let model = body["model"].as_str().unwrap_or("").to_string();

    // Resolve project_id from project_dir by matching against registered projects
    let mut project_id = body["project_id"].as_str().unwrap_or("").to_string();
    if project_id.is_empty() && !project_dir.is_empty() {
        if let Some(sp) = state.state_port.as_ref() {
            if let Ok(projects) = sp.project_list().await {
                if let Some(proj) = projects.iter().find(|p| p.root_path == project_dir) {
                    project_id = proj.id.clone();
                }
            }
        }
    }

    // Register via state_port (SpacetimeDB primary, SQLite fallback)
    if let Some(sp) = state.state_port.as_ref() {
        let info = crate::ports::state::AgentInfo {
            id: agent_id.clone(),
            name: agent_name.clone(),
            project_dir: project_dir.clone(),
            project_id: project_id.clone(),
            model: model.clone(),
            status: crate::ports::state::AgentStatus::Running,
            started_at: chrono::Utc::now().to_rfc3339(),
        };
        if let Err(e) = sp.agent_register(info).await {
            tracing::warn!(agent_id = %agent_id, error = %e, "Failed to register agent in state backend");
        }
    }

    // Broadcast connection event
    let _ = state.ws_tx.send(crate::state::WsEnvelope {
        topic: "hexflo".into(),
        event: "agent_connected".into(),
        data: json!({ "agentId": &agent_id, "host": &host, "name": &agent_name }),
    });

    (StatusCode::OK, Json(json!({
        "agentId": agent_id,
        "status": "connected",
        "host": host,
        "name": agent_name,
    })))
}

/// POST /api/agents/disconnect — deregister an agent by ID (no PID management)
///
/// Used by Claude Code sessions and remote agents that registered via /connect.
/// Unlike DELETE /api/agents/:id (which goes through AgentManager for PID cleanup),
/// this route calls state_port.agent_remove() directly.
pub async fn disconnect_agent(
    State(state): State<SharedState>,
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let agent_id = match body["agentId"].as_str() {
        Some(id) => id.to_string(),
        None => return (StatusCode::BAD_REQUEST, Json(json!({ "error": "missing agentId" }))),
    };

    // Mark agent as completed with endedAt timestamp (ADR-2603311000).
    // Using agent_update_status instead of agent_remove so the session history
    // is preserved for audit — only status + endedAt change.
    if let Some(sp) = state.state_port.as_ref() {
        let _ = sp
            .agent_update_status(&agent_id, crate::ports::state::AgentStatus::Completed, None)
            .await;
    }

    // Broadcast disconnection event
    let _ = state.ws_tx.send(crate::state::WsEnvelope {
        topic: "hexflo".into(),
        event: "agent_disconnected".into(),
        data: json!({ "agent_id": &agent_id }),
    });

    (StatusCode::OK, Json(json!({
        "ok": true,
        "agentId": agent_id,
        "status": "disconnected",
    })))
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

/// GET /api/workplan/by-path?path=<filename> — find most recent execution for a workplan file
///
/// Searches HexFlo memory for all keys prefixed with `workplan:`, deserializes each
/// `ExecutionState`, filters to those whose `workplan_path` ends with the given filename,
/// and returns the most recently updated match. Returns 404 if none found.
#[derive(Debug, Deserialize)]
pub struct ByPathQuery {
    pub path: String,
}

pub async fn workplan_by_path(
    State(state): State<SharedState>,
    Query(params): Query<ByPathQuery>,
) -> (StatusCode, Json<serde_json::Value>) {
    let sp = match state.state_port.as_ref() {
        Some(sp) => sp,
        None => return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "error": "State port not available" }))),
    };

    let entries = match sp.hexflo_memory_search("workplan:").await {
        Ok(e) => e,
        Err(e) => return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        ),
    };

    let filename = &params.path;
    let mut matches: Vec<crate::orchestration::workplan_executor::ExecutionState> = entries
        .into_iter()
        .filter_map(|(_, value)| serde_json::from_str(&value).ok())
        .filter(|es: &crate::orchestration::workplan_executor::ExecutionState| {
            es.workplan_path.ends_with(filename.as_str())
        })
        .collect();

    if matches.is_empty() {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": format!("No workplan execution found for '{}'", filename) })),
        );
    }

    matches.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

    (StatusCode::OK, Json(json!({ "ok": true, "data": matches[0] })))
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
///
/// Includes git correlation data (ADR-046): commit details from the git timeline
/// are matched against agent IDs and task results to provide a unified view.
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
            // Also collect agent names from phase results
            for pr in &execution.phase_results {
                agents_used.extend(pr.agent_ids.clone());
            }
            agents_used.sort();
            agents_used.dedup();

            let gates_passed = execution.gate_results.iter().filter(|g| g.passed).count();
            let gates_failed = execution.gate_results.iter().filter(|g| !g.passed).count();

            // ADR-046: Git correlation — find commits linked to this workplan's agents
            let git_commits = build_git_correlation(&state, &execution).await;

            (StatusCode::OK, Json(json!({
                "ok": true,
                "data": {
                    "workplan": {
                        "id": execution.id,
                        "feature": execution.feature,
                        "status": execution.status,
                        "workplanPath": execution.workplan_path,
                        "startedAt": execution.started_at,
                        "updatedAt": execution.updated_at,
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
                    "commits": git_commits,
                }
            })))
        }
        Ok(None) => (StatusCode::NOT_FOUND, Json(json!({ "error": format!("Workplan '{}' not found", id) }))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e }))),
    }
}

/// ADR-046: Build git correlation data for a workplan execution.
///
/// Scans recent commits for references to agent IDs or task IDs that appear
/// in the workplan's phase results, filtering to commits made during the
/// execution window.
async fn build_git_correlation(
    _state: &SharedState,
    execution: &crate::orchestration::workplan_executor::ExecutionState,
) -> serde_json::Value {
    // Determine project root from workplan path or cwd
    let project_root: Option<std::path::PathBuf> = {
        let wp = std::path::Path::new(&execution.workplan_path);
        // Workplan is usually in docs/workplans/<file>.json — go up to project root
        wp.parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.parent())
            .map(|p| p.to_path_buf())
            .filter(|p| p.join(".git").exists())
            .or_else(|| {
                // Try parent directly (flat layout)
                wp.parent()
                    .map(|p| p.to_path_buf())
                    .filter(|p| p.join(".git").exists())
            })
    };

    let root = match project_root {
        Some(r) => r,
        None => match std::env::current_dir() {
            Ok(d) => d,
            Err(_) => return json!([]),
        },
    };

    // Collect all agent IDs referenced in this execution
    let mut known_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    for pr in &execution.phase_results {
        for aid in &pr.agent_ids {
            known_ids.insert(aid.clone());
        }
    }
    for aid in &execution.agents {
        known_ids.insert(aid.clone());
    }

    // Use the git correlation module to find task-linked commits
    match crate::git::correlation::find_task_commits(&root, 100) {
        Ok(links) => {
            // Filter to commits within the execution time window
            let started_ts = chrono::DateTime::parse_from_rfc3339(&execution.started_at)
                .map(|dt| dt.timestamp())
                .unwrap_or(0);

            let filtered: Vec<_> = links
                .into_iter()
                .filter(|link| {
                    // Include if commit is after execution start
                    if link.commit_timestamp < started_ts {
                        return false;
                    }
                    // Include if any task_id matches a known agent ID
                    if link.task_ids.iter().any(|tid| known_ids.contains(tid)) {
                        return true;
                    }
                    // Include if agent name matches
                    if let Some(ref agent) = link.agent_name {
                        if known_ids.contains(agent) {
                            return true;
                        }
                    }
                    // Include all commits in the time window (they're likely related)
                    true
                })
                .collect();

            json!(filtered)
        }
        Err(_) => json!([]),
    }
}
