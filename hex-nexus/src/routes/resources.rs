//! Resource supervisor dashboard endpoints (ADR-2026-05-08-2200).
//!
//! Read-only views over `process_observation` + `resource_anomaly` plus an
//! ack write shim. The `/proc` walker that populates these tables lives in
//! `orchestration::resource_observer`.

use std::time::Duration;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::state::SharedState;

const STDB_TIMEOUT_SECS: u64 = 5;

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
        .expect("resources http client")
}

async fn sql(query: &str) -> Result<Vec<Vec<Value>>, String> {
    let url = format!("{}/v1/database/{}/sql", stdb_host(), hex_db());
    let resp = http()
        .post(&url)
        .header("Content-Type", "text/plain")
        .body(query.to_string())
        .send()
        .await
        .map_err(|e| format!("http: {}", e))?;
    if !resp.status().is_success() {
        let s = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("HTTP {}: {}", s, body));
    }
    let body: Value = resp.json().await.map_err(|e| format!("json: {}", e))?;
    Ok(body
        .as_array()
        .and_then(|a| a.first())
        .and_then(|t| t.get("rows"))
        .and_then(|r| r.as_array())
        .map(|rows| rows.iter().filter_map(|r| r.as_array().cloned()).collect())
        .unwrap_or_default())
}

async fn call_reducer(reducer: &str, args: Value) -> Result<(), String> {
    let url = format!(
        "{}/v1/database/{}/call/{}",
        stdb_host(),
        hex_db(),
        reducer
    );
    let resp = http()
        .post(&url)
        .json(&args)
        .send()
        .await
        .map_err(|e| format!("http: {}", e))?;
    if !resp.status().is_success() {
        let s = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("HTTP {}: {}", s, body));
    }
    Ok(())
}

// ─── GET /api/resources ──────────────────────────────────────────────

#[derive(Deserialize)]
pub struct ResourcesQuery {
    pub limit: Option<u32>,
}

/// Returns every active process_observation row.
pub async fn list_processes(
    State(_state): State<SharedState>,
    Query(q): Query<ResourcesQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let rows = sql(
        "SELECT pid, host, argv_sha, argv_first, state, ppid, started_micros, rss_kb, cpu_pct, observed_at FROM process_observation",
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e}))))?;

    let mut processes: Vec<Value> = rows
        .into_iter()
        .filter_map(|r| {
            if r.len() < 10 {
                return None;
            }
            Some(json!({
                "pid":             r[0].as_u64().unwrap_or(0),
                "host":            r[1].as_str().unwrap_or(""),
                "argv_sha":        r[2].as_str().unwrap_or(""),
                "argv_first":      r[3].as_str().unwrap_or(""),
                "state":           r[4].as_str().unwrap_or(""),
                "ppid":            r[5].as_u64().unwrap_or(0),
                "started_micros":  r[6].as_i64().unwrap_or(0),
                "rss_kb":          r[7].as_u64().unwrap_or(0),
                "cpu_pct":         r[8].as_f64().unwrap_or(0.0),
                "observed_at":     r[9].as_str().unwrap_or(""),
            }))
        })
        .collect();
    // Sort by RSS descending — operator wants memory hogs at the top.
    processes.sort_by(|a, b| {
        b.get("rss_kb")
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
            .cmp(&a.get("rss_kb").and_then(|v| v.as_u64()).unwrap_or(0))
    });
    if let Some(n) = q.limit {
        processes.truncate(n as usize);
    }
    Ok(Json(json!({ "processes": processes })))
}

// ─── GET /api/resources/anomalies ────────────────────────────────────

#[derive(Deserialize)]
pub struct AnomaliesQuery {
    /// "open" | "all". Defaults to open.
    pub status: Option<String>,
    pub limit: Option<u32>,
}

pub async fn list_anomalies(
    State(_state): State<SharedState>,
    Query(q): Query<AnomaliesQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let rows = sql(
        "SELECT id, detected_at, kind, severity, pids, note, handled, handled_at, handled_by FROM resource_anomaly",
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e}))))?;

    let want_open = q.status.as_deref() != Some("all");

    let mut anomalies: Vec<Value> = rows
        .into_iter()
        .filter_map(|r| {
            if r.len() < 9 {
                return None;
            }
            let handled = r[6].as_bool().unwrap_or(false);
            if want_open && handled {
                return None;
            }
            Some(json!({
                "id":          r[0].as_u64().unwrap_or(0),
                "detected_at": r[1].as_str().unwrap_or(""),
                "kind":        r[2].as_str().unwrap_or(""),
                "severity":    r[3].as_str().unwrap_or(""),
                "pids":        r[4].as_str().unwrap_or(""),
                "note":        r[5].as_str().unwrap_or(""),
                "handled":     handled,
                "handled_at":  r[7].as_str().unwrap_or(""),
                "handled_by":  r[8].as_str().unwrap_or(""),
            }))
        })
        .collect();
    anomalies.sort_by(|a, b| {
        b.get("id")
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
            .cmp(&a.get("id").and_then(|v| v.as_u64()).unwrap_or(0))
    });
    if let Some(n) = q.limit {
        anomalies.truncate(n as usize);
    }
    Ok(Json(json!({ "anomalies": anomalies })))
}

// ─── GET /api/commitments ────────────────────────────────────────────

#[derive(Deserialize)]
pub struct CommitmentsQuery {
    /// "open" | "all" | "overdue". Defaults to all-open-or-overdue.
    pub status: Option<String>,
    pub role: Option<String>,
    pub limit: Option<u32>,
}

pub async fn list_commitments(
    State(_state): State<SharedState>,
    Query(q): Query<CommitmentsQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let rows = sql(
        "SELECT id, role, raw_text, action, deadline_micros, success_artifact, artifact_kind, thread_id, related_msg_id, created_at, status, last_checked, note FROM commitment",
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e}))))?;

    let want = q.status.as_deref().unwrap_or("active");
    let role_filter = q.role.clone();

    let mut commitments: Vec<Value> = rows
        .into_iter()
        .filter_map(|r| {
            if r.len() < 13 {
                return None;
            }
            let role = r[1].as_str().unwrap_or("").to_string();
            let status = r[10].as_str().unwrap_or("").to_string();
            if let Some(ref f) = role_filter {
                if &role != f {
                    return None;
                }
            }
            let keep = match want {
                "all" => true,
                "open" => status == "open",
                "overdue" => status == "overdue",
                "active" | _ => status == "open" || status == "overdue",
            };
            if !keep {
                return None;
            }
            Some(json!({
                "id":               r[0].as_u64().unwrap_or(0),
                "role":             role,
                "raw_text":         r[2].as_str().unwrap_or(""),
                "action":           r[3].as_str().unwrap_or(""),
                "deadline_micros":  r[4].as_i64().unwrap_or(0),
                "success_artifact": r[5].as_str().unwrap_or(""),
                "artifact_kind":    r[6].as_str().unwrap_or(""),
                "thread_id":        r[7].as_str().unwrap_or(""),
                "related_msg_id":   r[8].as_u64().unwrap_or(0),
                "created_at":       r[9].as_str().unwrap_or(""),
                "status":           status,
                "last_checked":     r[11].as_str().unwrap_or(""),
                "note":             r[12].as_str().unwrap_or(""),
            }))
        })
        .collect();
    commitments.sort_by(|a, b| {
        b.get("id")
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
            .cmp(&a.get("id").and_then(|v| v.as_u64()).unwrap_or(0))
    });
    if let Some(n) = q.limit {
        commitments.truncate(n as usize);
    }
    Ok(Json(json!({ "commitments": commitments })))
}

#[derive(Deserialize)]
pub struct SatisfyBody {
    pub id: u64,
    pub evidence: String,
}

pub async fn satisfy_commitment(
    State(_state): State<SharedState>,
    Json(body): Json<SatisfyBody>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    call_reducer("commitment_satisfy", json!([body.id, body.evidence]))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e}))))?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub struct AbandonBody {
    pub id: u64,
    pub reason: String,
}

pub async fn abandon_commitment(
    State(_state): State<SharedState>,
    Json(body): Json<AbandonBody>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    call_reducer("commitment_abandon", json!([body.id, body.reason]))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e}))))?;
    Ok(Json(json!({ "ok": true })))
}

// ─── POST /api/resources/anomalies/ack ───────────────────────────────

#[derive(Deserialize)]
pub struct AckBody {
    pub id: u64,
    pub handled_by: Option<String>,
}

pub async fn ack_anomaly(
    State(_state): State<SharedState>,
    Json(body): Json<AckBody>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let by = body.handled_by.unwrap_or_else(|| "operator".to_string());
    call_reducer("resource_anomaly_ack", json!([body.id, by]))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e}))))?;
    Ok(Json(json!({ "ok": true })))
}
