//! REST endpoints for the AIOS morning briefing (ADR-2604131500 P1.4).
//!
//! GET /api/briefing            — full briefing with events + pending decisions
//!     ?limit=N                 — max events per project (default 5)
//!     ?summary=true            — counts only, no event bodies
//!     ?since=<ISO8601|unix>    — filter events after timestamp
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
    /// Mark retrieved briefing events as seen (default false).
    pub seen: Option<bool>,
    /// Maximum number of events per project (default 5).
    pub limit: Option<usize>,
    /// When true, return only counts (no event bodies). Default false.
    pub summary: Option<bool>,
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

    // ── Fetch briefing events from HexFlo memory ────────
    let briefing_memory = port
        .hexflo_memory_search("briefing:")
        .await
        .unwrap_or_default();

    // Parse since threshold (supports ISO 8601 and Unix seconds).
    let since_threshold = params.since.as_deref().and_then(|s| {
        chrono::DateTime::parse_from_rfc3339(s)
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .ok()
            .or_else(|| {
                s.parse::<i64>().ok().and_then(|ts| {
                    chrono::DateTime::from_timestamp(ts, 0)
                })
            })
    });

    // Convert memory entries to structured events, applying `since` filter.
    // Events stored by workplan_executor/coordination use: severity, category, title, body, created_at
    let briefing_events: Vec<(String, Value)> = briefing_memory
        .into_iter()
        .filter_map(|(key, value)| {
            let parsed: Value = serde_json::from_str(&value).unwrap_or_else(|_| {
                json!({ "raw": value })
            });

            // Apply since filter: check created_at (primary) or timestamp (legacy).
            if let Some(threshold) = &since_threshold {
                let event_ts = parsed
                    .get("created_at")
                    .or_else(|| parsed.get("timestamp"))
                    .and_then(|v| v.as_str())
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.with_timezone(&chrono::Utc));
                if let Some(ts) = event_ts {
                    if ts < *threshold {
                        return None;
                    }
                }
            }

            let project_id = parsed
                .get("project_id")
                .and_then(|v| v.as_str())
                .unwrap_or("*")
                .to_string();

            // Map stored fields to a CLI-friendly format:
            // CLI BriefEvent expects: id, title, status, agent_id
            let severity = parsed.get("severity").and_then(|v| v.as_str()).unwrap_or("nominal");
            let title = parsed
                .get("title")
                .and_then(|v| v.as_str())
                .or_else(|| parsed.get("summary").and_then(|v| v.as_str()))
                .unwrap_or("event")
                .to_string();

            // Map severity to a status the CLI can render
            let status = match severity {
                "critical" => "critical",
                "decision" => "decision",
                "notable" => "completed",
                _ => "completed",
            };

            Some((project_id, json!({
                "id": key,
                "title": title,
                "status": status,
                "agent_id": "",
                "severity": severity,
                "category": parsed.get("category").unwrap_or(&json!("general")),
                "body": parsed.get("body").unwrap_or(&json!("")),
                "created_at": parsed.get("created_at").unwrap_or(&json!("")),
            })))
        })
        .collect();

    let mark_seen = params.seen.unwrap_or(false);
    let event_limit = params.limit.unwrap_or(5);
    let summary_only = params.summary.unwrap_or(false);

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

        // Briefing events for this project (from HexFlo memory)
        let project_events: Vec<&Value> = briefing_events
            .iter()
            .filter(|(proj_id, _)| proj_id == pid || proj_id == "*")
            .map(|(_, ev)| ev)
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

        // Merge active tasks + briefing events into a single events array
        // so the CLI's BriefEvent struct can parse both uniformly.
        let mut all_events: Vec<Value> = project_tasks;
        for ev in &project_events {
            all_events.push((*ev).clone());
        }

        // Apply pagination: track total count, then truncate to limit.
        let total_event_count = all_events.len();
        let truncated = total_event_count > event_limit;

        // In summary mode, omit event bodies entirely; otherwise truncate to limit.
        let events_payload = if summary_only {
            json!([])
        } else {
            all_events.truncate(event_limit);
            json!(all_events)
        };

        result_projects.push(json!({
            "project_id": pid,
            "name": &proj.name,
            "events": events_payload,
            "pending_decisions": pending_decisions,
            "summary": {
                "agent_count": agent_count,
                "event_count": total_event_count,
                "decision_count": pending_decisions.len(),
                "truncated": truncated,
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

            // Briefing events matching the filter or broadcast
            let fallback_events: Vec<Value> = briefing_events
                .iter()
                .filter(|(proj_id, _)| proj_id == filter || proj_id == "*")
                .map(|(_, ev)| ev.clone())
                .collect();

            let total_event_count = fallback_events.len();
            let truncated = total_event_count > event_limit;

            let events_payload = if summary_only {
                json!([])
            } else {
                let limited: Vec<&Value> = fallback_events.iter().take(event_limit).collect();
                json!(limited)
            };

            result_projects.push(json!({
                "project_id": filter,
                "active_tasks": [],
                "events": events_payload,
                "pending_decisions": pending_decisions,
                "summary": {
                    "agent_count": 0,
                    "task_count": 0,
                    "event_count": total_event_count,
                    "decision_count": pending_decisions.len(),
                    "truncated": truncated,
                    "health": 0,
                    "spend": 0.0
                }
            }));
        }
    }

    // ── Mark events as seen ─────────────────────────────
    if mark_seen && !briefing_events.is_empty() {
        let now = chrono::Utc::now().to_rfc3339();
        for (_, ev) in &briefing_events {
            if let Some(key) = ev.get("key").and_then(|k| k.as_str()) {
                let seen_key = format!("briefing:seen:{}", key.trim_start_matches("briefing:"));
                // Fire-and-forget: best-effort mark as seen.
                let _ = port
                    .hexflo_memory_store(&seen_key, &now, "global")
                    .await;
            }
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
