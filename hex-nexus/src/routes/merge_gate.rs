//! Merge gate dashboard endpoints (ADR-2026-05-08-1126 P5 / dashboard).
//!
//! Read-only operator views over the STDB merge_request / merge_vote /
//! persona_pool / persona_health / agent_thought tables. Plus thin write
//! shims for `approve` and `reject` that mirror the `hex worktree` CLI.
//!
//! All queries hit STDB SQL via the shared reqwest client. STDB SQL has
//! quirks (no ORDER BY DESC on some tables, Sum-type filters, etc.) so
//! we filter / sort in Rust on the response side.

use std::time::Duration;

use axum::extract::{Path, Query, State};
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

fn chat_db() -> String {
    std::env::var("HEX_CHAT_STDB_DATABASE")
        .unwrap_or_else(|_| hex_core::stdb_database_for_module("chat-relay").to_string())
}

fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(STDB_TIMEOUT_SECS))
        .build()
        .expect("merge_gate http client")
}

async fn sql(database: &str, query: &str) -> Result<Vec<Vec<Value>>, String> {
    let url = format!("{}/v1/database/{}/sql", stdb_host(), database);
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
        .map(|rows| {
            rows.iter()
                .filter_map(|r| r.as_array().cloned())
                .collect()
        })
        .unwrap_or_default())
}

async fn call_reducer(database: &str, reducer: &str, args: Value) -> Result<(), String> {
    let url = format!("{}/v1/database/{}/call/{}", stdb_host(), database, reducer);
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

// ─── GET /api/merge/requests ─────────────────────────────────────────

/// Returns every merge_request with its current vote tally.
pub async fn list_merge_requests(
    State(_state): State<SharedState>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let req_rows = sql(
        &hex_db(),
        "SELECT worktree_path, branch, role, opened_at, status, related_workplan, agent_id FROM merge_request",
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e}))))?;

    // Drop CLI-test fixtures (worktree_path under /tmp/cli-*). They aren't
    // real merge-team work — they're left over from `hex worktree approve|reject`
    // integration tests and just confuse the dashboard.
    let req_rows: Vec<_> = req_rows
        .into_iter()
        .filter(|r| {
            r.first()
                .and_then(|s| s.as_str())
                .map(|p| !p.starts_with("/tmp/cli-"))
                .unwrap_or(true)
        })
        .collect();

    let vote_rows = sql(
        &hex_db(),
        "SELECT worktree_path, voter, verdict, reason, voted_at FROM merge_vote",
    )
    .await
    .unwrap_or_default();

    let mut requests: Vec<Value> = req_rows
        .into_iter()
        .filter_map(|r| {
            if r.len() < 7 {
                return None;
            }
            let path = r[0].as_str()?.to_string();
            let votes: Vec<Value> = vote_rows
                .iter()
                .filter(|v| v.first().and_then(|s| s.as_str()) == Some(path.as_str()))
                .map(|v| {
                    json!({
                        "voter":    v.get(1).and_then(|x| x.as_str()).unwrap_or(""),
                        "verdict":  v.get(2).and_then(|x| x.as_str()).unwrap_or(""),
                        "reason":   v.get(3).and_then(|x| x.as_str()).unwrap_or(""),
                        "voted_at": v.get(4).and_then(|x| x.as_str()).unwrap_or(""),
                    })
                })
                .collect();
            Some(json!({
                "worktree_path":    path,
                "branch":           r[1].as_str().unwrap_or(""),
                "role":             r[2].as_str().unwrap_or(""),
                "opened_at":        r[3].as_str().unwrap_or(""),
                "status":           r[4].as_str().unwrap_or(""),
                "related_workplan": r[5].as_str().unwrap_or(""),
                "agent_id":         r[6].as_str().unwrap_or(""),
                "votes":            votes,
            }))
        })
        .collect();
    requests.sort_by(|a, b| {
        a.get("worktree_path")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .cmp(b.get("worktree_path").and_then(|v| v.as_str()).unwrap_or(""))
    });
    Ok(Json(json!({ "requests": requests })))
}

// ─── POST /api/merge/approve ─────────────────────────────────────────

#[derive(Deserialize)]
pub struct ApproveBody {
    pub worktree_path: String,
    pub reason: Option<String>,
}

pub async fn approve_merge_request(
    State(_state): State<SharedState>,
    Json(body): Json<ApproveBody>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let reason = body.reason.unwrap_or_else(|| "operator approval".to_string());
    call_reducer(
        &hex_db(),
        "merge_vote_cast",
        json!([body.worktree_path, "operator", "pass", reason]),
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e}))))?;
    let _ = call_reducer(
        &hex_db(),
        "merge_decision_tally",
        json!([body.worktree_path]),
    )
    .await;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub struct RejectBody {
    pub worktree_path: String,
    pub reason: String,
}

pub async fn reject_merge_request(
    State(_state): State<SharedState>,
    Json(body): Json<RejectBody>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    if body.reason.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "reason is required"})),
        ));
    }
    call_reducer(
        &hex_db(),
        "merge_vote_cast",
        json!([body.worktree_path, "operator", "fail", body.reason]),
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e}))))?;
    let _ = call_reducer(
        &hex_db(),
        "merge_decision_tally",
        json!([body.worktree_path]),
    )
    .await;
    Ok(Json(json!({ "ok": true })))
}

// ─── GET /api/merge/personas ─────────────────────────────────────────

/// Persona supervisor + health joined view. One row per persona pool with
/// its pause state, last_tick_at age, and current health (recent_failures,
/// banned_until).
pub async fn list_personas(
    State(_state): State<SharedState>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let pool_rows = sql(
        &hex_db(),
        "SELECT role, display_name, tier, paused, last_tick_at FROM persona_pool",
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e}))))?;
    let health_rows = sql(
        &hex_db(),
        "SELECT role, recent_failures, last_failure_at, last_failure_model, last_failure_status, banned_until FROM persona_health",
    )
    .await
    .unwrap_or_default();

    // 2026-05-22 fix: STDB returns `last_tick_at` as the Rust Debug
    // formatting of its Timestamp struct (e.g.
    // `Timestamp { __timestamp_micros_since_unix_epoch__: 1779491821453421 }`).
    // Without parsing here, the dashboard receives a string it cannot format
    // as a date AND a null health (when persona_health is empty), then
    // renders every exec as "shutdown" — even when the supervisor is
    // healthily ticking once a minute (which it is, see persona_pool's fresh
    // last_tick_at micros above). Two fixes baked in here:
    //
    //   1. Parse the Timestamp Debug string → ISO-8601 RFC-3339 for
    //      `last_tick_at`. Empty string if unparseable so the dashboard
    //      shows "—" rather than a malformed date.
    //   2. Derive a `health.status` (`alive` / `stale` / `paused` /
    //      `banned`) from last_tick_at freshness + persona_health when
    //      present, so the dashboard renders execs as alive whenever the
    //      supervisor IS ticking, regardless of whether persona_health
    //      has been populated for them yet.
    fn parse_stdb_timestamp_micros(s: &str) -> Option<i64> {
        // Match `Timestamp { __timestamp_micros_since_unix_epoch__: <N> }`.
        let key = "__timestamp_micros_since_unix_epoch__:";
        let i = s.find(key)?;
        let tail = &s[i + key.len()..];
        let digits: String = tail
            .chars()
            .skip_while(|c| c.is_whitespace())
            .take_while(|c| c.is_ascii_digit() || *c == '-')
            .collect();
        digits.parse::<i64>().ok()
    }
    fn micros_to_rfc3339(micros: i64) -> String {
        let secs = micros / 1_000_000;
        let nsec = ((micros % 1_000_000) * 1_000) as u32;
        chrono::DateTime::<chrono::Utc>::from_timestamp(secs, nsec)
            .map(|dt| dt.to_rfc3339())
            .unwrap_or_default()
    }
    let now_micros = chrono::Utc::now().timestamp_micros();
    // 60-second freshness gate matches the persona supervisor's 25s tick
    // plus ~2x slack — anything older than 60s is genuinely stale.
    const STALE_AFTER_MICROS: i64 = 60 * 1_000_000;

    let personas: Vec<Value> = pool_rows
        .into_iter()
        .filter_map(|p| {
            if p.len() < 5 {
                return None;
            }
            let role = p[0].as_str()?.to_string();
            let paused = p[3].as_bool().unwrap_or(false);
            let last_tick_raw = p[4].as_str().unwrap_or("");
            let last_tick_micros = parse_stdb_timestamp_micros(last_tick_raw);
            let last_tick_iso = last_tick_micros
                .map(micros_to_rfc3339)
                .unwrap_or_default();
            let last_tick_age_secs = last_tick_micros
                .map(|m| (now_micros - m) / 1_000_000)
                .unwrap_or(i64::MAX);
            let is_stale = last_tick_age_secs > STALE_AFTER_MICROS / 1_000_000;

            let h = health_rows
                .iter()
                .find(|h| h.first().and_then(|s| s.as_str()) == Some(role.as_str()));
            let banned_until = h
                .and_then(|hh| hh.get(5).and_then(|x| x.as_str()))
                .unwrap_or("");
            let recent_failures = h
                .and_then(|hh| hh.get(1).and_then(|x| x.as_u64()))
                .unwrap_or(0);

            // Derived status — the field the dashboard should render off.
            let status = if !banned_until.is_empty() {
                "banned"
            } else if paused {
                "paused"
            } else if is_stale {
                "stale"
            } else {
                "alive"
            };

            let health = json!({
                "status":              status,
                "recent_failures":     recent_failures,
                "last_failure_at":     h.and_then(|hh| hh.get(2).and_then(|x| x.as_str())).unwrap_or(""),
                "last_failure_model":  h.and_then(|hh| hh.get(3).and_then(|x| x.as_str())).unwrap_or(""),
                "last_failure_status": h.and_then(|hh| hh.get(4).and_then(|x| x.as_u64())).unwrap_or(0),
                "banned_until":        banned_until,
            });

            Some(json!({
                "role":              role,
                "display_name":      p[1].as_str().unwrap_or(""),
                "tier":              p[2].as_str().unwrap_or(""),
                "paused":            paused,
                "last_tick_at":      last_tick_iso,
                "last_tick_age_secs": last_tick_age_secs.max(0),
                "status":            status,
                "health":            health,
            }))
        })
        .collect();
    Ok(Json(json!({ "personas": personas })))
}

// ─── GET /api/merge/thoughts ─────────────────────────────────────────

#[derive(Deserialize)]
pub struct ThoughtsQuery {
    pub role: Option<String>,
    pub kind: Option<String>,
    pub limit: Option<u32>,
}

/// Returns recent agent_thought rows, optionally filtered by role / kind.
pub async fn list_thoughts(
    State(_state): State<SharedState>,
    Query(q): Query<ThoughtsQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let rows = sql(
        &chat_db(),
        "SELECT thought_id, agent_role, kind, content, related_msg_id, related_task_id, confidence, created_at FROM agent_thought",
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e}))))?;

    let limit = q.limit.unwrap_or(100) as usize;
    let mut thoughts: Vec<Value> = rows
        .into_iter()
        .filter_map(|r| {
            if r.len() < 8 {
                return None;
            }
            let role = r[1].as_str().unwrap_or("").to_string();
            let kind = r[2].as_str().unwrap_or("").to_string();
            if let Some(ref filter) = q.role {
                if &role != filter {
                    return None;
                }
            }
            if let Some(ref filter) = q.kind {
                if &kind != filter {
                    return None;
                }
            }
            Some(json!({
                "thought_id":      r[0].as_u64().unwrap_or(0),
                "agent_role":      role,
                "kind":            kind,
                "content":         r[3].as_str().unwrap_or(""),
                "related_msg_id":  r[4].as_u64().unwrap_or(0),
                "related_task_id": r[5].as_str().unwrap_or(""),
                "confidence":      r[6].as_f64().unwrap_or(0.0),
                "created_at":      r[7].as_str().unwrap_or(""),
            }))
        })
        .collect();
    thoughts.sort_by(|a, b| {
        b.get("thought_id")
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
            .cmp(&a.get("thought_id").and_then(|v| v.as_u64()).unwrap_or(0))
    });
    thoughts.truncate(limit);
    Ok(Json(json!({ "thoughts": thoughts })))
}

// ─── GET /api/merge/persona-events ───────────────────────────────────

pub async fn list_persona_events(
    State(_state): State<SharedState>,
    Path(role): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let rows = sql(
        &hex_db(),
        &format!(
            "SELECT id, ts, kind, role, payload FROM persona_event WHERE role = '{}'",
            role.replace('\'', "''")
        ),
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e}))))?;
    let mut events: Vec<Value> = rows
        .into_iter()
        .filter_map(|r| {
            if r.len() < 5 {
                return None;
            }
            Some(json!({
                "id":      r[0].as_u64().unwrap_or(0),
                "ts":      r[1].as_str().unwrap_or(""),
                "kind":    r[2].as_str().unwrap_or(""),
                "role":    r[3].as_str().unwrap_or(""),
                "payload": r[4].as_str().unwrap_or(""),
            }))
        })
        .collect();
    events.sort_by(|a, b| {
        b.get("id")
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
            .cmp(&a.get("id").and_then(|v| v.as_u64()).unwrap_or(0))
    });
    events.truncate(50);
    Ok(Json(json!({ "events": events })))
}
