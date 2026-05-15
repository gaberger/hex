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

use std::collections::{BTreeSet, HashMap};
use std::sync::OnceLock;
use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use regex::Regex;
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

fn chat_db() -> String {
    hex_core::stdb_database_for_module("chat-relay").to_string()
}

fn agent_comms_db() -> String {
    hex_core::stdb_database_for_module("agent-comms").to_string()
}

/// STDB SQL returns timestamp columns in three observed forms:
///   1. An ISO-8601 string (when the column is declared as `String`)
///   2. A JSON object `{"__timestamp_micros_since_unix_epoch__": N}`
///   3. A *string* containing Rust's Debug representation of a
///      Timestamp value: `"Timestamp { __timestamp_micros_since_unix_epoch__: N }"`
///      (the form actually emitted by current STDB SQL when the WASM
///      column type is the native Timestamp).
///
/// Normalize all three to an ISO string the dashboard can `Date.parse`.
fn as_ts_iso(v: &Value) -> String {
    // Form 3 first — the Debug-formatted string. Look for the literal
    // marker and extract the trailing number.
    if let Some(s) = v.as_str() {
        if let Some(rest) = s.strip_prefix("Timestamp { __timestamp_micros_since_unix_epoch__: ") {
            if let Some(num_str) = rest.strip_suffix(" }") {
                if let Ok(micros) = num_str.trim().parse::<i64>() {
                    return micros_to_iso(micros);
                }
            }
        }
        // Otherwise treat as already-ISO.
        return s.to_string();
    }
    if let Some(obj) = v.as_object() {
        if let Some(micros) = obj
            .get("__timestamp_micros_since_unix_epoch__")
            .and_then(|v| v.as_i64())
        {
            return micros_to_iso(micros);
        }
    }
    String::new()
}

/// Compute age in seconds from an ISO-8601 timestamp (normalized output
/// of `as_ts_iso`). Returns 0 on parse failure so the attention feed
/// degrades gracefully rather than disappearing items with unparseable
/// stamps.
fn age_seconds_from_iso(iso: &str) -> u64 {
    if iso.is_empty() { return 0; }
    match chrono::DateTime::parse_from_rfc3339(iso) {
        Ok(dt) => {
            let now = chrono::Utc::now().timestamp();
            let then = dt.timestamp();
            if now > then { (now - then) as u64 } else { 0 }
        }
        Err(_) => 0,
    }
}

fn micros_to_iso(micros: i64) -> String {
    let secs = micros / 1_000_000;
    let nanos = ((micros % 1_000_000) * 1_000) as u32;
    chrono::DateTime::<chrono::Utc>::from_timestamp(secs, nanos)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_default()
}

fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(STDB_TIMEOUT_SECS))
        .build()
        .expect("mission_control http client")
}

/// Match canonical hyphenated ADR IDs (mirror of BS-5 detector in
/// hex-cli::commands::sched::improver::thought_patterns).
fn adr_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"ADR-\d{4}-\d{2}-\d{2}-\d{3,4}").expect("ADR regex"))
}

async fn sql_db(query: &str, db: &str) -> Vec<Vec<Value>> {
    let url = format!("{}/v1/database/{}/sql", stdb_host(), db);
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
    State(state): State<SharedState>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    // Fire all STDB queries in parallel. Events come from in-memory adapter.
    let events_future = state.event_adapter.list_events(None, 200);
    let chat = chat_db();
    let comms = agent_comms_db();
    let (
        executed,
        commitments,
        proposed,
        anomalies,
        personas,
        merge_reqs,
        processes,
        thoughts,
        messages,
        events,
    ) = tokio::join!(
        sql("SELECT id, kind, payload_json, success, error, executed_at, evidence FROM executed_action"),
        sql("SELECT id, role, action, success_artifact, status, created_at FROM commitment"),
        sql("SELECT id, kind, proposed_by, status, twin_verdict, twin_rationale, escalate_reason FROM proposed_action"),
        sql("SELECT id, detected_at, kind, severity, pids, note, handled FROM resource_anomaly"),
        sql("SELECT role, display_name, paused, last_tick_at FROM persona_pool"),
        sql("SELECT worktree_path, branch, status, opened_at FROM merge_request"),
        sql("SELECT pid, argv_first, rss_kb, cpu_pct, state FROM process_observation"),
        sql_db(
            "SELECT thought_id, agent_role, kind, content, related_msg_id, created_at FROM agent_thought",
            &chat,
        ),
        sql_db(
            "SELECT id, from_agent, to_agent, message, timestamp FROM agent_messages",
            &comms,
        ),
        events_future,
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
            // Suppress duplicate_argv from the operator's attention feed.
            // The supervisor emits one of these every tick when two
            // hex-agents share an argv_sha (common during brain-daemon
            // respawn races) — pure noise, not an action item.
            // Rows still land in STDB for audit; just not surfaced here.
            let kind = r.get(2).and_then(|v| v.as_str()).unwrap_or("");
            if kind == "duplicate_argv" {
                return None;
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
    // Dedup by (kind, pids) — the supervisor emits a fresh anomaly
    // every tick that an issue persists, so 15 rss_oversize rows for
    // the same STDB pid become one "STDB at 27 GiB" alert. Most-
    // recent wins (already sorted desc by id).
    {
        let mut seen: BTreeSet<String> = BTreeSet::new();
        open_anomalies.retain(|a| {
            let key = format!(
                "{}|{}",
                a.get("kind").and_then(|v| v.as_str()).unwrap_or(""),
                a.get("pids").and_then(|v| v.as_str()).unwrap_or("")
            );
            seen.insert(key)
        });
    }
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

    // ── Recent thoughts (BS-5 emitter side) ─────────────────────────
    let mut recent_thoughts: Vec<Value> = thoughts
        .iter()
        .filter_map(|r| {
            if r.len() < 6 {
                return None;
            }
            let raw_content = r[3].as_str().unwrap_or("");
            let content_snippet: String = raw_content.chars().take(200).collect();
            Some(json!({
                "thought_id":      r[0].as_u64().unwrap_or(0),
                "agent_role":      r[1].as_str().unwrap_or(""),
                "kind":            r[2].as_str().unwrap_or(""),
                "content":         content_snippet,
                "related_msg_id":  r[4].as_u64().unwrap_or(0),
                "created_at":      as_ts_iso(&r[5]),
            }))
        })
        .collect();
    recent_thoughts.sort_by(|a, b| {
        b.get("thought_id")
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
            .cmp(&a.get("thought_id").and_then(|v| v.as_u64()).unwrap_or(0))
    });
    let total_thoughts = recent_thoughts.len();
    recent_thoughts.truncate(8);

    // ── Active thought patterns (BS-5 consumer side, inline) ─────────
    // Mirror of detect_in() from hex-cli::commands::sched::improver::thought_patterns.
    // We run detection over the last 200 thoughts so a live signal
    // appears in the dashboard without waiting on the sched daemon's
    // tick cadence.
    let mut adr_hits: HashMap<String, (usize, BTreeSet<String>, Vec<u64>)> = HashMap::new();
    let mut frustration_count: usize = 0;
    for row in thoughts.iter().take(200) {
        if row.len() < 6 {
            continue;
        }
        let content = row[3].as_str().unwrap_or("");
        let role = row[1].as_str().unwrap_or("").to_string();
        let kind = row[2].as_str().unwrap_or("");
        let tid = row[0].as_u64().unwrap_or(0);
        if kind == "frustration" {
            frustration_count += 1;
        }
        let mut seen_here = std::collections::HashSet::new();
        for m in adr_re().find_iter(content) {
            let adr_id = m.as_str().to_string();
            if !seen_here.insert(adr_id.clone()) {
                continue;
            }
            let entry = adr_hits
                .entry(adr_id)
                .or_insert_with(|| (0, BTreeSet::new(), Vec::new()));
            entry.0 += 1;
            if !role.is_empty() {
                entry.1.insert(role.clone());
            }
            if entry.2.len() < 5 {
                entry.2.push(tid);
            }
        }
    }
    let mut thought_patterns: Vec<Value> = Vec::new();
    for (adr_id, (count, roles, samples)) in adr_hits {
        if count < 3 || roles.len() < 2 {
            continue;
        }
        let severity = if count >= 5 { "error" } else { "warning" };
        let role_list: Vec<String> = roles.into_iter().collect();
        thought_patterns.push(json!({
            "pattern":             "adr_repetition",
            "scope":               adr_id,
            "severity":            severity,
            "count":               count,
            "mentioning_roles":    role_list,
            "sample_thought_ids":  samples,
        }));
    }
    if frustration_count >= 5 {
        let severity = if frustration_count >= 10 { "error" } else { "warning" };
        thought_patterns.push(json!({
            "pattern":  "frustration_spike",
            "scope":    "kind:frustration",
            "severity": severity,
            "count":    frustration_count,
        }));
    }
    thought_patterns.sort_by(|a, b| {
        let sev = |v: &Value| if v.get("severity").and_then(|s| s.as_str()) == Some("error") { 1 } else { 0 };
        sev(b).cmp(&sev(a))
            .then(b.get("count").and_then(|v| v.as_u64()).unwrap_or(0)
                .cmp(&a.get("count").and_then(|v| v.as_u64()).unwrap_or(0)))
    });

    // ── Recent persona messages ─────────────────────────────────────
    let mut recent_messages: Vec<Value> = messages
        .into_iter()
        .filter_map(|r| {
            if r.len() < 5 {
                return None;
            }
            // agent_messages.to_agent is `Option<String>` — STDB SQL serializes
            // it as a tagged tuple `[tag, value]`: `[0, "cto"]` = Some("cto"),
            // `[1, []]` = None. Some older paths still emit `{"some": "x"}`
            // / `{"none": []}` or a raw string, so we handle all three.
            let to_role = match &r[2] {
                Value::Array(arr) => match arr.first().and_then(|v| v.as_u64()) {
                    Some(0) => arr
                        .get(1)
                        .and_then(|v| v.as_str())
                        .unwrap_or("*")
                        .to_string(),
                    _ => "*".to_string(),
                },
                Value::String(s) => s.clone(),
                Value::Object(o) => o
                    .get("some")
                    .and_then(|v| v.as_str())
                    .unwrap_or("*")
                    .to_string(),
                _ => "*".to_string(),
            };
            let raw = r[3].as_str().unwrap_or("");
            let snippet: String = raw.chars().take(240).collect();
            Some(json!({
                "msg_id":     r[0].as_u64().unwrap_or(0),
                "from_role":  r[1].as_str().unwrap_or(""),
                "to_role":    to_role,
                "message":    snippet,
                "created_at": as_ts_iso(&r[4]),
            }))
        })
        .collect();
    recent_messages.sort_by(|a, b| {
        b.get("msg_id")
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
            .cmp(&a.get("msg_id").and_then(|v| v.as_u64()).unwrap_or(0))
    });
    recent_messages.truncate(8);

    // ── Live system events ──────────────────────────────────────────
    // Filter out the noisy Claude tool-use stream — operators want the
    // autonomous-loop signals (improver_tick, improver_act, loop_notification,
    // brain_*, persona_*) without scrolling through every Bash/Read.
    let high_signal_kinds: &[&str] = &[
        "improver_tick",
        "improver_act",
        "loop_notification",
        "brain_tick",
        "brain_analyze_regression",
        "brain.analyze.regression",
        "persona_reply",
        "thought_journaled",
        "twin_verdict",
        "executor_applied",
    ];
    let live_events: Vec<Value> = events
        .iter()
        .filter(|e| {
            high_signal_kinds
                .iter()
                .any(|k| e.event_type.as_str() == *k)
        })
        .take(20)
        .map(|e| {
            let snippet: String = e
                .input_json
                .as_deref()
                .unwrap_or("")
                .chars()
                .take(220)
                .collect();
            json!({
                "id":         e.id,
                "event_type": e.event_type,
                "created_at": e.created_at,
                "session_id": e.session_id,
                "preview":    snippet,
            })
        })
        .collect();

    // ── Pulse — last activity timestamps for narrative header ────────
    let last_thought_ts = recent_thoughts
        .first()
        .and_then(|t| t.get("created_at"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let last_persona_msg = recent_messages
        .iter()
        .find(|m| m.get("from_role").and_then(|v| v.as_str()) != Some("operator"));
    let last_persona_role = last_persona_msg
        .and_then(|m| m.get("from_role"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let last_persona_msg_ts = last_persona_msg
        .and_then(|m| m.get("created_at"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let last_persona_msg_preview = last_persona_msg
        .and_then(|m| m.get("message"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .chars()
        .take(140)
        .collect::<String>();
    let last_improver_event_ts = live_events
        .iter()
        .find(|e| {
            matches!(
                e.get("event_type").and_then(|v| v.as_str()),
                Some("improver_tick") | Some("improver_act") | Some("loop_notification")
            )
        })
        .and_then(|e| e.get("created_at"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let git_head = git_head_summary();

    // ── Loop health (process freshness via /proc walker observations) ─
    // For each known nexus tokio loop, the latest log signature is the
    // proxy. Cheap implementation: report the schedule rows present
    // (tick is alive if its schedule exists and STDB SQL responded).
    let stdb_alive = !persona_rows.is_empty() || !top_procs.is_empty();

    // ── Attention feed ──────────────────────────────────────────────
    // Operator's single landing surface. Each AttentionItem maps to a
    // P0/P1/P2 lane and carries a `cli_repro` for terminal-first
    // operators. Source rules:
    //   pending_actions (escalate_reason set) → P0 escalation
    //   open_anomalies  (severity=critical)   → P0 resource_anomaly
    //   open_anomalies  (else)                → P1 resource_anomaly
    //   open_merge      (status=voting)       → P1 merge_vote_needed
    //   open_commitments(status=overdue)      → P1 overdue_commitment
    //   recent_executed (last 5 SOP commits)  → P2 autonomous_commit
    // P0 = act now (red). P1 = act soon (amber). P2 = info (blue).
    let mut attention_feed: Vec<Value> = Vec::new();
    for a in pending_actions.iter() {
        let reason = a.get("escalate_reason").and_then(|v| v.as_str()).unwrap_or("");
        if reason.is_empty() { continue; }
        let id = a.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
        let kind_name = a.get("kind").and_then(|v| v.as_str()).unwrap_or("?");
        attention_feed.push(json!({
            "id": format!("escalation-{}", id),
            "priority": 0,
            "kind": "escalation",
            "title": format!("{} action #{} needs operator attention", kind_name, id),
            "subtitle": reason.chars().take(180).collect::<String>(),
            "age_seconds": 0,
            "action_url": "#/commitments",
            "cli_repro": format!("hex ops abandon {}", id),
        }));
    }
    for an in open_anomalies.iter() {
        let id = an.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
        let sev = an.get("severity").and_then(|v| v.as_str()).unwrap_or("");
        let kind_name = an.get("kind").and_then(|v| v.as_str()).unwrap_or("?");
        let note = an.get("note").and_then(|v| v.as_str()).unwrap_or("");
        let priority = if sev == "critical" { 0 } else { 1 };
        attention_feed.push(json!({
            "id": format!("anomaly-{}", id),
            "priority": priority,
            "kind": "resource_anomaly",
            "title": format!("{} ({})", kind_name, sev),
            "subtitle": note.chars().take(180).collect::<String>(),
            "age_seconds": age_seconds_from_iso(an.get("detected_at").and_then(|v| v.as_str()).unwrap_or("")),
            "action_url": "#/resources",
            "cli_repro": "hex resources anomalies".to_string(),
        }));
    }
    for mr in open_merge.iter() {
        if mr.get("status").and_then(|v| v.as_str()) != Some("voting") { continue; }
        let path = mr.get("worktree_path").and_then(|v| v.as_str()).unwrap_or("");
        let branch = mr.get("branch").and_then(|v| v.as_str()).unwrap_or("");
        attention_feed.push(json!({
            "id": format!("merge-{}", branch),
            "priority": 1,
            "kind": "merge_vote_needed",
            "title": format!("Merge vote: {}", branch),
            "subtitle": path.chars().take(180).collect::<String>(),
            "age_seconds": age_seconds_from_iso(mr.get("opened_at").and_then(|v| v.as_str()).unwrap_or("")),
            "action_url": "#/merge-gate",
            "cli_repro": "hex worktree status".to_string(),
        }));
    }
    for c in open_commitments.iter() {
        if c.get("status").and_then(|v| v.as_str()) != Some("overdue") { continue; }
        let id = c.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
        let role = c.get("role").and_then(|v| v.as_str()).unwrap_or("");
        let action = c.get("action").and_then(|v| v.as_str()).unwrap_or("");
        attention_feed.push(json!({
            "id": format!("commitment-{}", id),
            "priority": 1,
            "kind": "overdue_commitment",
            "title": format!("Overdue: {} — {}", role, action.chars().take(60).collect::<String>()),
            "subtitle": format!("commitment #{}", id),
            "age_seconds": age_seconds_from_iso(c.get("created_at").and_then(|v| v.as_str()).unwrap_or("")),
            "action_url": "#/commitments",
            "cli_repro": format!("hex ops abandon {}", id),
        }));
    }
    for ex in recent_executed.iter().take(5) {
        if ex.get("success").and_then(|v| v.as_bool()) != Some(true) { continue; }
        let id = ex.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
        let path = ex.get("path").and_then(|v| v.as_str()).unwrap_or("");
        if path.is_empty() { continue; }
        attention_feed.push(json!({
            "id": format!("autocommit-{}", id),
            "priority": 2,
            "kind": "autonomous_commit",
            "title": format!("Auto-committed: {}", path.rsplit('/').next().unwrap_or(path)),
            "subtitle": format!("action #{} — {}", id, path),
            "age_seconds": age_seconds_from_iso(ex.get("executed_at").and_then(|v| v.as_str()).unwrap_or("")),
            "action_url": "#/activity",
            "cli_repro": format!("git log --grep 'action#{}' -1 -p", id),
        }));
    }
    attention_feed.sort_by(|a, b| {
        let pa = a.get("priority").and_then(|v| v.as_u64()).unwrap_or(9);
        let pb = b.get("priority").and_then(|v| v.as_u64()).unwrap_or(9);
        if pa != pb { return pa.cmp(&pb); }
        let aa = a.get("age_seconds").and_then(|v| v.as_u64()).unwrap_or(0);
        let ab = b.get("age_seconds").and_then(|v| v.as_u64()).unwrap_or(0);
        ab.cmp(&aa) // older = higher in same priority lane
    });
    attention_feed.truncate(40);

    Ok(Json(json!({
        "ts": chrono::Utc::now().to_rfc3339(),
        "stdb_alive": stdb_alive,
        "attention_feed": attention_feed,
        "pulse": {
            "last_thought_ts":         last_thought_ts,
            "last_persona_role":       last_persona_role,
            "last_persona_msg_ts":     last_persona_msg_ts,
            "last_persona_msg_preview": last_persona_msg_preview,
            "last_improver_event_ts":  last_improver_event_ts,
            "total_thoughts_db":       total_thoughts,
            "active_pattern_count":    thought_patterns.len(),
            "git_head":                git_head,
            "autonomous_commits_today": autonomous_commits_today(),
        },
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
        "recent_thoughts": recent_thoughts,
        "thought_patterns": thought_patterns,
        "recent_messages": recent_messages,
        "live_events": live_events,
    })))
}

/// Best-effort `git log -1 --format=...` so the pulse pane can show the
/// most recent commit subject + age. Returns an object with `sha`,
/// `subject`, `age_seconds`. Empty strings if git is unavailable.
/// Count today's autonomous commits on the current branch.
/// Cheap proxy: `git log --since='today 00:00' --grep='Co-Authored-By: hex-autonomous' --oneline | wc -l`.
fn autonomous_commits_today() -> u64 {
    use std::process::Command;
    let output = match Command::new("git")
        .args([
            "log",
            "--since=today 00:00",
            "--grep=Co-Authored-By: hex-autonomous",
            "--pretty=%H",
        ])
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return 0,
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .count() as u64
}

fn git_head_summary() -> Value {

    use std::process::Command;
    let out = Command::new("git")
        .args(["log", "-1", "--format=%H%x00%s%x00%ct"])
        .output();
    let Ok(out) = out else { return json!({"sha": "", "subject": "", "age_seconds": 0}) };
    if !out.status.success() {
        return json!({"sha": "", "subject": "", "age_seconds": 0});
    }
    let s = String::from_utf8_lossy(&out.stdout);
    let parts: Vec<&str> = s.trim_end().splitn(3, '\u{0}').collect();
    if parts.len() < 3 {
        return json!({"sha": "", "subject": "", "age_seconds": 0});
    }
    let sha: String = parts[0].chars().take(12).collect();
    let subject = parts[1].to_string();
    let ts: i64 = parts[2].parse().unwrap_or(0);
    let now = chrono::Utc::now().timestamp();
    let age = (now - ts).max(0);
    json!({"sha": sha, "subject": subject, "age_seconds": age})
}
