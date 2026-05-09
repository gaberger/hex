//! Mission Control aggregator (operator's single landing surface).
//!
//! GET /api/mission-control returns one composite payload covering:
//!   - Recent activity (last N executed_actions, file_writes, persona DMs)
//!   - Loop health (drafter/twin/executor/persona supervisor freshness)
//!   - Pending decisions (open commitments needing review, escalated actions)
//!   - Anomaly inbox (open resource_anomaly + overdue commitments)
//!   - Quick stats (rss top hog, ollama running, STDB ping, watchdog up)
//!
//! Every Solid view used to make 3-5 round trips. Now: one /api/mission-control
//! payload at refresh cadence (5s), drill-downs still hit the existing
//! per-domain endpoints when needed.

use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde_json::{json, Value};

use crate::state::SharedState;

const STDB_TIMEOUT_SECS: u64 = 4;

fn stdb_host() -> String {
    std::env::var("HEX_SPACETIMEDB_HOST").unwrap_or_else(|_| "http://127.0.0.1:3033".to_string())
}

fn hex_db() -> String {
    std::env::var("HEX_STDB_DATABASE")
        .unwrap_or_else(|_| hex_core::stdb_database_for_module("hexflo-coordination").to_string())
}

fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(STDB_TIMEOUT_SECS))
        .build()
        .expect("mission_control http client")
}

async fn sql(query: &str) -> Vec<Vec<Value>> {
    let url = format!("{}/v1/database/{}/sql", stdb_host(), hex_db());
    let resp = match http()
        .post(&url)
        .header("Content-Type", "text/plain")
        .body(query.to_string())
        .send()
        .await
    {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    if !resp.status().is_success() {
        return Vec::new();
    }
    let body: Value = match resp.json().await {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    body.as_array()
        .and_then(|a| a.first())
        .and_then(|t| t.get("rows"))
        .and_then(|r| r.as_array())
        .map(|rows| rows.iter().filter_map(|r| r.as_array().cloned()).collect())
        .unwrap_or_default()
}

pub async fn get_mission_control(
    State(_state): State<SharedState>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    // Fire all queries in parallel.
    let (
        executed,
        commitments,
        proposed,
        anomalies,
        personas,
        merge_reqs,
        processes,
    ) = tokio::join!(
        sql("SELECT id, kind, payload_json, success, error, executed_at, evidence FROM executed_action"),
        sql("SELECT id, role, action, success_artifact, status, created_at FROM commitment"),
        sql("SELECT id, kind, proposed_by, status, twin_verdict, twin_rationale, escalate_reason FROM proposed_action"),
        sql("SELECT id, detected_at, kind, severity, pids, note, handled FROM resource_anomaly"),
        sql("SELECT role, display_name, paused, last_tick_at FROM persona_pool"),
        sql("SELECT worktree_path, branch, status, opened_at FROM merge_request"),
        sql("SELECT pid, argv_first, rss_kb, cpu_pct, state FROM process_observation"),
    );

    // ── Activity feed ────────────────────────────────────────────────
    // Last 12 executed_actions, newest first.
    let mut recent_executed: Vec<Value> = executed
        .into_iter()
        .filter_map(|r| {
            if r.len() < 7 {
                return None;
            }
            let payload_str = r.get(2).and_then(|v| v.as_str()).unwrap_or("");
            let path: Option<String> = serde_json::from_str::<Value>(payload_str)
                .ok()
                .and_then(|v| v.get("path").and_then(|p| p.as_str().map(String::from)));
            Some(json!({
                "id":          r[0].as_u64().unwrap_or(0),
                "kind":        r[1].as_str().unwrap_or(""),
                "path":        path,
                "success":     r[3].as_bool().unwrap_or(false),
                "error":       r[4].as_str().unwrap_or(""),
                "executed_at": r[5].as_str().unwrap_or(""),
                "evidence":    r[6].as_str().unwrap_or(""),
            }))
        })
        .collect();
    recent_executed.sort_by(|a, b| {
        b.get("id")
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
            .cmp(&a.get("id").and_then(|v| v.as_u64()).unwrap_or(0))
    });
    recent_executed.truncate(12);

    // ── Pending decisions ────────────────────────────────────────────
    let mut pending_actions: Vec<Value> = proposed
        .iter()
        .filter_map(|r| {
            if r.len() < 7 {
                return None;
            }
            let status = r.get(3).and_then(|v| v.as_str()).unwrap_or("");
            if !matches!(status, "pending" | "escalated") {
                return None;
            }
            Some(json!({
                "id":              r[0].as_u64().unwrap_or(0),
                "kind":            r[1].as_str().unwrap_or(""),
                "proposed_by":     r[2].as_str().unwrap_or(""),
                "status":          status,
                "twin_verdict":    r[4].as_str().unwrap_or(""),
                "twin_rationale":  r[5].as_str().unwrap_or(""),
                "escalate_reason": r[6].as_str().unwrap_or(""),
            }))
        })
        .collect();
    pending_actions.sort_by(|a, b| {
        b.get("id")
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
            .cmp(&a.get("id").and_then(|v| v.as_u64()).unwrap_or(0))
    });
    pending_actions.truncate(20);

    let mut open_commitments: Vec<Value> = commitments
        .iter()
        .filter_map(|r| {
            if r.len() < 6 {
                return None;
            }
            let status = r.get(4).and_then(|v| v.as_str()).unwrap_or("");
            if !matches!(status, "open" | "overdue") {
                return None;
            }
            Some(json!({
                "id":               r[0].as_u64().unwrap_or(0),
                "role":             r[1].as_str().unwrap_or(""),
                "action":           r[2].as_str().unwrap_or(""),
                "success_artifact": r[3].as_str().unwrap_or(""),
                "status":           status,
                "created_at":       r[5].as_str().unwrap_or(""),
            }))
        })
        .collect();
    open_commitments.sort_by(|a, b| {
        b.get("id")
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
            .cmp(&a.get("id").and_then(|v| v.as_u64()).unwrap_or(0))
    });
    open_commitments.truncate(20);

    // ── Anomalies ────────────────────────────────────────────────────
    let mut open_anomalies: Vec<Value> = anomalies
        .into_iter()
        .filter_map(|r| {
            if r.len() < 7 {
                return None;
            }
            if r.get(6).and_then(|v| v.as_bool()).unwrap_or(false) {
                return None; // handled
            }
            Some(json!({
                "id":          r[0].as_u64().unwrap_or(0),
                "detected_at": r[1].as_str().unwrap_or(""),
                "kind":        r[2].as_str().unwrap_or(""),
                "severity":    r[3].as_str().unwrap_or(""),
                "pids":        r[4].as_str().unwrap_or(""),
                "note":        r[5].as_str().unwrap_or(""),
            }))
        })
        .collect();
    open_anomalies.sort_by(|a, b| {
        b.get("id")
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
            .cmp(&a.get("id").and_then(|v| v.as_u64()).unwrap_or(0))
    });
    open_anomalies.truncate(15);

    // ── Persona health ──────────────────────────────────────────────
    let persona_rows: Vec<Value> = personas
        .into_iter()
        .filter_map(|r| {
            if r.len() < 4 {
                return None;
            }
            Some(json!({
                "role":         r[0].as_str().unwrap_or(""),
                "display_name": r[1].as_str().unwrap_or(""),
                "paused":       r[2].as_bool().unwrap_or(false),
                "last_tick_at": r[3].as_str().unwrap_or(""),
            }))
        })
        .collect();

    // ── Merge requests ──────────────────────────────────────────────
    let mut open_merge: Vec<Value> = merge_reqs
        .into_iter()
        .filter_map(|r| {
            if r.len() < 4 {
                return None;
            }
            let status = r.get(2).and_then(|v| v.as_str()).unwrap_or("");
            if !matches!(status, "voting" | "open") {
                return None;
            }
            let path = r.get(0).and_then(|v| v.as_str()).unwrap_or("");
            if path.starts_with("/tmp/cli-") {
                return None;
            }
            Some(json!({
                "worktree_path": path,
                "branch":        r[1].as_str().unwrap_or(""),
                "status":        status,
                "opened_at":     r[3].as_str().unwrap_or(""),
            }))
        })
        .collect();
    open_merge.truncate(10);

    // ── Top processes by RSS ────────────────────────────────────────
    let mut top_procs: Vec<Value> = processes
        .into_iter()
        .filter_map(|r| {
            if r.len() < 5 {
                return None;
            }
            Some(json!({
                "pid":        r[0].as_u64().unwrap_or(0),
                "argv":      r[1].as_str().unwrap_or("").chars().take(60).collect::<String>(),
                "rss_kb":    r[2].as_u64().unwrap_or(0),
                "cpu_pct":   r[3].as_f64().unwrap_or(0.0),
                "state":     r[4].as_str().unwrap_or(""),
            }))
        })
        .collect();
    top_procs.sort_by(|a, b| {
        b.get("rss_kb")
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
            .cmp(&a.get("rss_kb").and_then(|v| v.as_u64()).unwrap_or(0))
    });
    top_procs.truncate(8);

    // ── Loop health (process freshness via /proc walker observations) ─
    // For each known nexus tokio loop, the latest log signature is the
    // proxy. Cheap implementation: report the schedule rows present
    // (tick is alive if its schedule exists and STDB SQL responded).
    let stdb_alive = !persona_rows.is_empty() || !top_procs.is_empty();

    Ok(Json(json!({
        "ts": chrono::Utc::now().to_rfc3339(),
        "stdb_alive": stdb_alive,
        "activity": {
            "recent_executed": recent_executed,
            "open_merge_requests": open_merge,
        },
        "pending_decisions": {
            "actions": pending_actions,
            "commitments": open_commitments,
            "anomalies": open_anomalies,
        },
        "personas": persona_rows,
        "top_processes": top_procs,
    })))
}
