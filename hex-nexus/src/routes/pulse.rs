//! REST endpoint for the multi-project pulse view (ADR-2604131500 P6.1).
//!
//! GET /api/pulse — one-glance state for every registered project.

use axum::{
    extract::State,
    Json,
};
use http::StatusCode;
use serde_json::{json, Value};

use crate::state::SharedState;

fn no_state_port() -> (StatusCode, Json<Value>) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({ "error": "IStatePort not initialized (no SpacetimeDB backend)" })),
    )
}

/// GET /api/pulse — lightweight multi-project status overview.
///
/// For each registered project, computes a single state word:
///   blocked > decision > active > complete > idle
pub async fn get_pulse(
    State(state): State<SharedState>,
) -> (StatusCode, Json<Value>) {
    let port = match &state.state_port {
        Some(p) => p,
        None => return no_state_port(),
    };

    // ── 1. Get registered projects ───────────────────────
    let projects = port.project_list().await.unwrap_or_default();

    if projects.is_empty() {
        return (StatusCode::OK, Json(json!([])));
    }

    // ── 2. Get pending inbox notifications ───────────────
    let notifications = port.inbox_query("*", None, true).await.unwrap_or_default();

    // ── 3. Get all tasks ─────────────────────────────────
    let tasks = port.swarm_task_list(None).await.unwrap_or_default();

    // ── 4. Get connected agents ──────────────────────────
    let agents = port.hex_agent_list().await.unwrap_or_default();

    // ── 5. Build pulse for each project ──────────────────
    let mut result: Vec<Value> = Vec::new();

    for proj in &projects {
        let pid = &proj.id;

        // Count inbox items (decisions) for this project
        let decision_count = notifications
            .iter()
            .filter(|n| n.agent_id == "*" || n.agent_id.contains(pid.as_str()))
            .count();

        let critical_count = notifications
            .iter()
            .filter(|n| {
                (n.agent_id == "*" || n.agent_id.contains(pid.as_str()))
                    && n.priority >= 2
            })
            .count();

        // Count active/completed tasks (match via swarm naming convention)
        let active_count = tasks
            .iter()
            .filter(|t| t.status == "in_progress" && t.swarm_id.contains(pid.as_str()))
            .count();

        let completed_count = tasks
            .iter()
            .filter(|t| t.status == "completed" && t.swarm_id.contains(pid.as_str()))
            .count();

        // Count connected agents for this project
        let agent_count = agents
            .iter()
            .filter(|a| {
                a.get("project_id")
                    .and_then(|v| v.as_str())
                    .map(|s| s == pid)
                    .unwrap_or(false)
            })
            .count();

        // Determine state: blocked > decision > active > complete > idle
        let pulse_state = if critical_count > 0 {
            "blocked"
        } else if decision_count > 0 {
            "decision"
        } else if active_count > 0 {
            "active"
        } else if completed_count > 0 {
            "complete"
        } else {
            "idle"
        };

        result.push(json!({
            "project_id": pid,
            "name": &proj.name,
            "state": pulse_state,
            "agent_count": agent_count,
            "decision_count": decision_count,
        }));
    }

    // Sort by state severity (blocked first)
    result.sort_by(|a, b| {
        let state_order = |s: &str| match s {
            "blocked" => 4,
            "decision" => 3,
            "active" => 2,
            "complete" => 1,
            _ => 0,
        };
        let sa = a.get("state").and_then(|v| v.as_str()).unwrap_or("idle");
        let sb = b.get("state").and_then(|v| v.as_str()).unwrap_or("idle");
        state_order(sb).cmp(&state_order(sa))
    });

    (StatusCode::OK, Json(json!(result)))
}
