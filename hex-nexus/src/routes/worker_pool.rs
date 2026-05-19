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
    extract::{Query, State},
    http::StatusCode,
    response::Json,
};
use hex_core::domain::worker_pool::ConsumerStatus;
use hex_core::ports::worker_pool::IWorkerPoolPort;
use serde::{Deserialize, Serialize};

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
