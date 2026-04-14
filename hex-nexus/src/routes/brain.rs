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

    Json(BrainStatus {
        service_enabled: true,
        test_model,
        interval_secs: 600,
        last_test,
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