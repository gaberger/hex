//! REST endpoints for the AIOS morning briefing (ADR-2604131500 P1.4).
//!
//! GET /api/briefing            — full briefing with events + pending decisions
//! GET /api/briefing/decisions  — pending decisions only

use axum::{
    extract::{Query, State},
    Json,
};
use http::StatusCode;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::state::SharedState;

fn no_state_port() -> (StatusCode, Json<Value>) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({ "error": "IStatePort not initialized (no SpacetimeDB backend)" })),
    )
}

#[derive(Debug, Deserialize)]
pub struct BriefingParams {
    /// Optional project filter.
    pub project: Option<String>,
    /// Whether to include pending decisions (default true).
    pub decisions: Option<bool>,
    /// Only include events since this timestamp (ISO 8601 or Unix seconds).
    pub since: Option<String>,
}

/// GET /api/briefing — generate a developer morning briefing.
///
/// Aggregates pending inbox notifications (as decisions) and active swarm
/// tasks per project. Falls back gracefully if data is unavailable.
pub async fn get_briefing(
    State(state): State<SharedState>,
    Query(params): Query<BriefingParams>,
) -> (StatusCode, Json<Value>) {
    let port = match &state.state_port {
        Some(p) => p,
        None => return no_state_port(),
    };

    // ── Fetch projects ───────────────────────────────────
    let projects = port.project_list().await.unwrap_or_default();

    // ── Fetch pending inbox notifications (unacked, all agents) ──
    let include_decisions = params.decisions.unwrap_or(true);
    let notifications = if include_decisions {
        port.inbox_query("*", None, true).await.unwrap_or_default()
    } else {
        Vec::new()
    };

    // ── Fetch active tasks ───────────────────────────────
    let tasks = port.swarm_task_list(None).await.unwrap_or_default();

    // ── Fetch connected agents ───────────────────────────
    let agents = port.hex_agent_list().await.unwrap_or_default();

    // ── Build per-project briefing ───────────────────────
    let mut result_projects: Vec<Value> = Vec::new();

    for proj in &projects {
        let pid = &proj.id;

        // Apply project filter if provided
        if let Some(ref filter) = params.project {
            if pid != filter && proj.name != *filter {
                continue;
            }
        }

        // Pending decisions (inbox notifications for this project's agents)
        let pending_decisions: Vec<Value> = notifications
            .iter()
            .filter(|n| {
                // Match by agent_id containing project info, or broadcast notifications
                n.agent_id == "*" || n.agent_id.contains(pid)
            })
            .map(|n| {
                json!({
                    "id": n.id,
                    "priority": n.priority,
                    "kind": &n.kind,
                    "payload": &n.payload,
                    "created_at": &n.created_at,
                })
            })
            .collect();

        // Active tasks for this project (match via swarm naming convention)
        let project_tasks: Vec<Value> = tasks
            .iter()
            .filter(|t| t.status == "in_progress" && t.swarm_id.contains(pid))
            .map(|t| {
                json!({
                    "id": &t.id,
                    "title": &t.title,
                    "status": &t.status,
                    "agent_id": &t.agent_id,
                })
            })
            .collect();

        // Agent count for this project
        let agent_count = agents
            .iter()
            .filter(|a| {
                a.get("project_id")
                    .and_then(|v| v.as_str())
                    .map(|s| s == pid)
                    .unwrap_or(false)
            })
            .count();

        result_projects.push(json!({
            "project_id": pid,
            "name": &proj.name,
            "events": project_tasks,
            "pending_decisions": pending_decisions,
            "summary": {
                "agent_count": agent_count,
                "event_count": project_tasks.len(),
                "decision_count": pending_decisions.len(),
                "health": 0,
                "spend": 0.0
            }
        }));
    }

    // If project filter was given but no registered project matched, return
    // a single synthetic entry so the caller still gets a useful response.
    if result_projects.is_empty() {
        if let Some(ref filter) = params.project {
            let pending_decisions: Vec<Value> = notifications
                .iter()
                .map(|n| {
                    json!({
                        "id": n.id,
                        "priority": n.priority,
                        "kind": &n.kind,
                        "payload": &n.payload,
                        "created_at": &n.created_at,
                    })
                })
                .collect();

            result_projects.push(json!({
                "project_id": filter,
                "events": [],
                "pending_decisions": pending_decisions,
                "summary": {
                    "agent_count": 0,
                    "event_count": 0,
                    "decision_count": pending_decisions.len(),
                    "health": 0,
                    "spend": 0.0
                }
            }));
        }
    }

    (
        StatusCode::OK,
        Json(json!({
            "projects": result_projects,
            "generated_at": chrono::Utc::now().to_rfc3339(),
        })),
    )
}

/// GET /api/briefing/decisions — pending decisions only.
pub async fn get_decisions(
    State(state): State<SharedState>,
    Query(params): Query<BriefingParams>,
) -> (StatusCode, Json<Value>) {
    let port = match &state.state_port {
        Some(p) => p,
        None => return no_state_port(),
    };

    let notifications = port.inbox_query("*", None, true).await.unwrap_or_default();

    let decisions: Vec<Value> = notifications
        .into_iter()
        .filter(|n| {
            if let Some(ref proj) = params.project {
                n.agent_id == "*" || n.agent_id.contains(proj.as_str())
            } else {
                true
            }
        })
        .map(|n| {
            json!({
                "id": n.id,
                "priority": n.priority,
                "kind": n.kind,
                "payload": n.payload,
                "created_at": n.created_at,
                "acknowledged": n.acknowledged_at.is_some(),
            })
        })
        .collect();

    (StatusCode::OK, Json(json!({ "decisions": decisions })))
}
