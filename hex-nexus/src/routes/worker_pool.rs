//! `/api/worker-pool` — runtime consumer-availability gate
//! (ADR-2605190900 §1 + P3.4).
//!
//! One endpoint:
//!   GET /api/worker-pool/check?role=<role>&ttl_secs=<n>
//!     -> { role, ttl_secs, status: "alive"|"degraded"|"none",
//!          worker_count: u32, dispatch_allowed: bool,
//!          oldest_heartbeat_age_secs?: u64 }
//!
//! The dispatcher calls this before publishing a brain-task. On
//! `dispatch_allowed: false` it should refuse to publish + record an
//! event + fire a bounded inbox notification. Today's behavior (the
//! 2026-05-19 postmortem) was to publish into a void and let the
//! lease re-enqueue 30+ times per workplan.
//!
//! Failure semantics: when the worker-pool backend (STDB) is unreachable
//! we return 503 with the worker-pool error in the body. Callers should
//! decide whether to fail-open (proceed with dispatch — risk re-entering
//! the void) or fail-closed (skip dispatch — risk false negatives during
//! STDB hiccups). Recommended pattern: fail-open with a circuit-breaker
//! around 5 consecutive 503s.

use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
};
use hex_core::domain::worker_pool::ConsumerStatus;
use hex_core::ports::worker_pool::IWorkerPoolPort;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::adapters::spacetime_worker_pool::SpacetimeWorkerPoolAdapter;
use crate::state::AppState;

/// Default TTL for "alive" — 60s mirrors the supervisor_tick stale-
/// heartbeat threshold (worker beats every 15s = 4 missed beats).
const DEFAULT_TTL_SECS: u64 = 60;

#[derive(Deserialize)]
pub struct CheckQuery {
    pub role: String,
    #[serde(default)]
    pub ttl_secs: Option<u64>,
}

#[derive(Serialize)]
pub struct CheckResponse {
    pub role: String,
    pub ttl_secs: u64,
    pub status: &'static str,
    pub worker_count: u32,
    pub dispatch_allowed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oldest_heartbeat_age_secs: Option<u64>,
}

fn make_adapter(_state: &AppState) -> SpacetimeWorkerPoolAdapter {
    let host = std::env::var("HEX_SPACETIMEDB_HOST")
        .unwrap_or_else(|_| "http://127.0.0.1:3033".to_string());
    let database = std::env::var("HEX_STDB_DATABASE").unwrap_or_else(|_| "hex".to_string());
    SpacetimeWorkerPoolAdapter::new(host, database)
}

pub async fn check(
    State(state): State<Arc<AppState>>,
    Query(q): Query<CheckQuery>,
) -> Result<Json<CheckResponse>, StatusCode> {
    let ttl_secs = q.ttl_secs.unwrap_or(DEFAULT_TTL_SECS);
    let ttl = Duration::from_secs(ttl_secs);
    let adapter = make_adapter(&state);

    let result = adapter
        .ensure_consumer(&q.role, ttl)
        .await
        .map_err(|e| {
            tracing::warn!(error = %e, role = %q.role, "worker_pool check failed");
            StatusCode::SERVICE_UNAVAILABLE
        })?;

    let response = match result {
        ConsumerStatus::Alive { worker_count } => CheckResponse {
            role: q.role,
            ttl_secs,
            status: "alive",
            worker_count,
            dispatch_allowed: true,
            oldest_heartbeat_age_secs: None,
        },
        ConsumerStatus::Degraded {
            worker_count,
            oldest_heartbeat_age_secs,
        } => CheckResponse {
            role: q.role,
            ttl_secs,
            status: "degraded",
            worker_count,
            dispatch_allowed: false,
            oldest_heartbeat_age_secs: Some(oldest_heartbeat_age_secs),
        },
        ConsumerStatus::None => CheckResponse {
            role: q.role,
            ttl_secs,
            status: "none",
            worker_count: 0,
            dispatch_allowed: false,
            oldest_heartbeat_age_secs: None,
        },
    };

    Ok(Json(response))
}

/// POST /api/worker-process/:id/heartbeat — refresh last_heartbeat on
/// the worker_process row identified by `id`.
///
/// Called by hex-agent on a 15s cadence (when launched with the
/// `HEX_WORKER_PROCESS_ID` env var set by `spawn_local_agent`). The
/// supervisor_tick reaps any row whose last_heartbeat is older than
/// 60s; without this endpoint the row goes stale and the supervisor
/// keeps spawning replacements while the hex-agent process is still
/// alive. Observed 2026-05-21: ~3 hex-agents/pool/min growth → 246
/// processes after ~30 min, with the supervisor unable to detect that
/// the originals were still running.
///
/// Delegates to the WASM reducer `worker_process_heartbeat`. Returns:
///   - 200 + `{ok:true}` on success
///   - 404 if the row id doesn't exist (caller should fall through to
///     `/api/hex-agents/{id}/heartbeat` — the legacy surface still
///     applies to swarm-task agents that weren't spawned via the
///     supervisor)
///   - 503 if STDB is unreachable (transient — caller retries next tick)
pub async fn process_heartbeat(
    State(_state): State<std::sync::Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let host = std::env::var("HEX_SPACETIMEDB_HOST")
        .unwrap_or_else(|_| "http://127.0.0.1:3033".to_string());
    let database = std::env::var("HEX_STDB_DATABASE").unwrap_or_else(|_| "hex".to_string());
    let url = format!("{host}/v1/database/{database}/call/worker_process_heartbeat");

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("http client: {e}")})),
        ))?;

    let resp = client
        .post(&url)
        .json(&json!([id]))
        .send()
        .await
        .map_err(|e| (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": format!("STDB: {e}")})),
        ))?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        // Reducer error "worker_process 'X' not found" → 404
        let lower = body.to_ascii_lowercase();
        if lower.contains("not found") {
            return Err((
                StatusCode::NOT_FOUND,
                Json(json!({"error": "worker_process row not found", "id": id})),
            ));
        }
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": format!("STDB HTTP {status}: {body}")})),
        ));
    }

    Ok(Json(json!({"ok": true, "id": id})))
}
