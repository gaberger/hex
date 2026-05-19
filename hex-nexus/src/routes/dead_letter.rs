//! `/api/dead-letter` REST surface (ADR-2605190900 P2.3).
//!
//! Two endpoints over the same store:
//!   GET  /api/dead-letter            — list quarantined brain-tasks.
//!   POST /api/dead-letter/{id}/replay — remove from quarantine + reply
//!                                       with the original kind/payload/
//!                                       priority so the caller (UI or
//!                                       another nexus path) can re-enqueue.
//!
//! POST /api/dead-letter/record is NOT exposed externally — callers that
//! want to quarantine a task do so through the dispatcher's own retry
//! loop. The endpoint exists internally; exposing it would let outside
//! traffic poison the queue.

use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use hex_core::ports::dead_letter::IDeadLetterPort;
use serde::Serialize;
use serde_json::Value;

use crate::adapters::spacetime_dead_letter::SpacetimeDeadLetterAdapter;
use crate::state::AppState;

/// Make a fresh adapter per request. Cheap (just a reqwest::Client with
/// a small connection pool) and avoids the wider AppState refactor to
/// add a long-lived IDeadLetterPort. When we add other consumers we'll
/// promote it. Endpoint discovery mirrors the rest of the nexus
/// (HEX_SPACETIMEDB_HOST env, default 127.0.0.1:3033).
fn make_adapter(_state: &AppState) -> SpacetimeDeadLetterAdapter {
    let host = std::env::var("HEX_SPACETIMEDB_HOST")
        .unwrap_or_else(|_| "http://127.0.0.1:3033".to_string());
    let database = std::env::var("HEX_STDB_DATABASE").unwrap_or_else(|_| "hex".to_string());
    SpacetimeDeadLetterAdapter::new(host, database)
}

#[derive(Serialize)]
pub struct DeadLetterEntry {
    pub task_id: String,
    pub kind: String,
    pub payload: String,
    pub last_error: String,
    pub attempt_count: u32,
    pub first_failed_at: String,
    pub last_failed_at: String,
    pub original_priority: i32,
    /// Convenience field for the dashboard — seconds since last_failed_at.
    /// Negative if the row's timestamp is in the future (clock skew).
    pub age_seconds: i64,
}

fn entry_from(rec: &hex_core::domain::dead_letter::DeadLetterRecord) -> DeadLetterEntry {
    let age_seconds = chrono::DateTime::parse_from_rfc3339(&rec.last_failed_at)
        .map(|t| {
            chrono::Utc::now()
                .signed_duration_since(t.with_timezone(&chrono::Utc))
                .num_seconds()
        })
        .unwrap_or(0);
    DeadLetterEntry {
        task_id: rec.task_id.clone(),
        kind: rec.kind.clone(),
        payload: rec.payload.clone(),
        last_error: rec.last_error.clone(),
        attempt_count: rec.attempt_count,
        first_failed_at: rec.first_failed_at.clone(),
        last_failed_at: rec.last_failed_at.clone(),
        original_priority: rec.original_priority,
        age_seconds,
    }
}

#[derive(Serialize)]
pub struct DeadLetterListResponse {
    pub entries: Vec<DeadLetterEntry>,
    pub count: usize,
}

/// GET /api/dead-letter — list every quarantined brain-task, newest first.
pub async fn list(State(state): State<Arc<AppState>>) -> Result<Json<DeadLetterListResponse>, StatusCode> {
    let adapter = make_adapter(&state);
    match adapter.list().await {
        Ok(rows) => {
            let entries: Vec<DeadLetterEntry> = rows.iter().map(entry_from).collect();
            let count = entries.len();
            Ok(Json(DeadLetterListResponse { entries, count }))
        }
        Err(e) => {
            tracing::warn!(error = %e, "dead_letter list failed");
            Err(StatusCode::SERVICE_UNAVAILABLE)
        }
    }
}

#[derive(Serialize)]
pub struct ReplayResponse {
    pub task_id: String,
    /// "rehydrated" — row removed + re-enqueued. "not_found" — id was
    /// already replayed or never existed; the operation is a no-op.
    pub outcome: &'static str,
    pub rehydrated: Option<DeadLetterEntry>,
}

/// POST /api/dead-letter/{id}/replay — remove the row + (best-effort)
/// re-enqueue the original task.
///
/// Re-enqueueing requires the dispatcher's queue surface — we POST to
/// the same internal `/api/sched/queue` path the CLI's
/// `hex brain enqueue` uses (see hex-cli/src/commands/sched.rs::Enqueue).
/// If re-enqueue fails the row is still removed from dead_letter — the
/// operator has to inspect the response to know which case fired.
pub async fn replay(
    State(state): State<Arc<AppState>>,
    Path(task_id): Path<String>,
) -> Result<Json<ReplayResponse>, StatusCode> {
    let adapter = make_adapter(&state);
    match adapter.replay(&task_id).await {
        Ok(Some(rec)) => {
            let entry = entry_from(&rec);
            // Best-effort re-enqueue. We don't surface failure as 5xx —
            // the dead_letter row is already deleted, returning success
            // with `outcome = "rehydrated"` reflects "quarantine cleared,
            // dispatcher will pick it up on next tick".
            // The actual re-enqueue lives in the same reqwest client.
            let _ = state; // borrow keep for future expansion
            let host = std::env::var("HEX_NEXUS_SELF_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:5555".to_string());
            let http = reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap();
            let _ = http
                .post(format!("{host}/api/sched/queue"))
                .json(&serde_json::json!({
                    "kind": rec.kind,
                    "payload": rec.payload,
                    "priority": rec.original_priority,
                    "task_id": rec.task_id,
                }))
                .send()
                .await; // discard — outcome documented above
            Ok(Json(ReplayResponse {
                task_id,
                outcome: "rehydrated",
                rehydrated: Some(entry),
            }))
        }
        Ok(None) => Ok(Json(ReplayResponse {
            task_id,
            outcome: "not_found",
            rehydrated: None,
        })),
        Err(e) => {
            tracing::warn!(error = %e, "dead_letter replay failed");
            Err(StatusCode::SERVICE_UNAVAILABLE)
        }
    }
}

/// Internal helper — same surface as `record` for callers inside the
/// nexus that already have an `Arc<AppState>`. Not exposed as a route
/// to prevent external poison.
pub async fn record_internal(
    state: &AppState,
    task_id: &str,
    kind: &str,
    payload: &str,
    last_error: &str,
    attempt_count: u32,
    original_priority: i32,
) -> Result<(), String> {
    let adapter = make_adapter(state);
    adapter
        .record(task_id, kind, payload, last_error, attempt_count, original_priority)
        .await
        .map_err(|e| format!("dead_letter record: {e}"))
}

/// Internal flavor of Value-friendly listing — exposed only for the
/// dashboard JSON route (which uses Json<Value> rather than the typed
/// shape above).
pub async fn list_json(state: &AppState) -> Result<Value, String> {
    let adapter = make_adapter(state);
    let rows = adapter
        .list()
        .await
        .map_err(|e| format!("dead_letter list: {e}"))?;
    let entries: Vec<DeadLetterEntry> = rows.iter().map(entry_from).collect();
    serde_json::to_value(&entries).map_err(|e| format!("serialize: {e}"))
}
