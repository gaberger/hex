//! Path B execution status — GET /api/workplan/execute/{id}/status
//!
//! Companion to POST /api/workplan/execute (which lives in `orchestration.rs`).
//! Surfaces the terminal state of a workplan execution so that fire-and-forget
//! clients (`hex plan execute` Path B) can poll until completion instead of
//! exiting in <1s with a misleading "dispatched" message.
//!
//! The execution map already lives in `WorkplanExecutor` (see
//! `orchestration::workplan_executor`); this module reuses `get_by_id` so we
//! don't duplicate state. `head_before`/`head_after` are returned as `null`
//! today — git-HEAD tracking will land in a follow-up phase.

use axum::{
    extract::{Path, State},
    Json,
};
use http::StatusCode;
use serde_json::json;

use crate::state::SharedState;

/// GET /api/workplan/execute/{id}/status — terminal-state query for Path B
/// pollers. Returns `{status, result, head_before, head_after}` or 404.
pub async fn get_execution_status(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    let exec = match state.workplan_executor.get() {
        Some(e) => e,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "error": "Workplan executor not initialized" })),
            )
        }
    };

    match exec.get_by_id(&id).await {
        Ok(Some(execution)) => {
            let status_str = execution.status.as_str();
            let result_str: Option<String> = execution
                .result
                .as_ref()
                .map(|v| match v {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                });
            (
                StatusCode::OK,
                Json(json!({
                    "status": status_str,
                    "result": result_str,
                    "head_before": serde_json::Value::Null,
                    "head_after": serde_json::Value::Null,
                })),
            )
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": format!("Execution '{}' not found", id) })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e })),
        ),
    }
}
