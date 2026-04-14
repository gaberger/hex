//! Brain API routes (ADR-2604102200).
//!
//! GET  /api/brain/status - Service status
//! POST /api/brain/test  - Run a test
//! GET  /api/brain/scores - Get method scores

use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};

use crate::brain_service;
use crate::state::SharedState;

#[derive(Serialize)]
pub struct BrainStatus {
    pub service_enabled: bool,
    pub test_model: String,
    pub interval_secs: u64,
    pub last_test: String,
    /// Pending brain tasks in the queue (from hexflo memory search).
    pub queue_pending: u32,
    /// Seconds since last brain_tick event (null if never). Operators watching
    /// the statusline use this to verify brain is actually iterating.
    pub last_tick_secs_ago: Option<u64>,
}

#[derive(Deserialize)]
pub struct BrainTestRequest {
    pub model: String,
}

#[derive(Serialize)]
pub struct BrainTestResponse {
    pub outcome: String,
    pub reward: f64,
    pub response: String,
}

pub async fn status(State(state): State<SharedState>) -> Json<BrainStatus> {
    let test_model = std::env::var("HEX_BRAIN_TEST_MODEL")
        .unwrap_or_else(|_| "nemotron-mini".to_string());

    let last_test = state
        .brain_last_test
        .read()
        .await
        .clone()
        .unwrap_or_else(|| "never".to_string());

    // Queue depth — count brain-task:* entries whose status is "pending".
    // Best-effort: if the state port isn't configured yet, return 0.
    let queue_pending = if let Some(sp) = state.state_port.as_ref() {
        match sp.hexflo_memory_search("brain-task:").await {
            Ok(entries) => entries
                .iter()
                .filter(|(_key, value)| {
                    serde_json::from_str::<serde_json::Value>(value)
                        .ok()
                        .and_then(|v| v.get("status").and_then(|s| s.as_str()).map(|s| s.to_string()))
                        .as_deref()
                        == Some("pending")
                })
                .count() as u32,
            Err(_) => 0,
        }
    } else {
        0
    };

    Json(BrainStatus {
        service_enabled: true,
        test_model,
        interval_secs: 600,
        last_test,
        queue_pending,
        last_tick_secs_ago: None, // TODO: read from event_adapter once a brain_tick filter exists
    })
}

pub async fn test(
    State(state): State<SharedState>,
    Json(_req): Json<BrainTestRequest>,
) -> Json<BrainTestResponse> {
    // Run a test cycle synchronously
    let result = match brain_service::run_improvement_cycle(&state).await {
        Ok(outcome) => BrainTestResponse {
            outcome: outcome.outcome,
            reward: outcome.reward,
            response: "test completed".to_string(),
        },
        Err(e) => BrainTestResponse {
            outcome: "error".to_string(),
            reward: -0.5,
            response: e,
        },
    };

    // Record the timestamp regardless of outcome — a failed test is still a
    // test. Operators care "when did we last probe?" not "when did we last
    // get a green result." (errors are visible in the response body itself.)
    *state.brain_last_test.write().await = Some(chrono::Utc::now().to_rfc3339());

    Json(result)
}