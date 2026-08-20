//! SOP run telemetry — `/api/org/sop/{active,recent,runs}`.
//!
//! Reads from the in-memory ring buffer inside
//! `orchestration::sop_executor`. Each SOP run started by the responder
//! lands here so the dashboard can show what's actually happening behind
//! the persona Confirm rows.
//!
//! Buffer is capped (~200 runs) and clears on nexus restart — durability
//! is a non-goal; the dashboard wants real-time visibility, not history.

use axum::{
    extract::Query,
    http::StatusCode,
    response::Json,
};
use serde::{Deserialize, Serialize};

use crate::orchestration::sop_executor::{self, SopRunRecord};

#[derive(Deserialize)]
pub struct LimitQuery {
    pub limit: Option<usize>,
}

#[derive(Serialize)]
pub struct RunsResponse {
    pub runs: Vec<SopRunRecord>,
    pub count: usize,
}

/// GET /api/org/sop/active — currently in-flight SOP runs (no completed_at).
pub async fn list_active() -> Result<Json<RunsResponse>, StatusCode> {
    let runs = sop_executor::active_runs().await;
    let count = runs.len();
    Ok(Json(RunsResponse { runs, count }))
}

/// GET /api/org/sop/recent?limit=25 — most recent runs (default 25, cap 200).
pub async fn list_recent(Query(q): Query<LimitQuery>) -> Result<Json<RunsResponse>, StatusCode> {
    let limit = q.limit.unwrap_or(25).min(200);
    let runs = sop_executor::recent_runs(limit).await;
    let count = runs.len();
    Ok(Json(RunsResponse { runs, count }))
}

/// GET /api/org/sop/runs — full ring buffer snapshot.
pub async fn list_all() -> Result<Json<RunsResponse>, StatusCode> {
    let runs = sop_executor::all_runs().await;
    let count = runs.len();
    Ok(Json(RunsResponse { runs, count }))
}
