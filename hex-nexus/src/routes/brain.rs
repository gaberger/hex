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

pub async fn status(State(_state): State<SharedState>) -> Json<BrainStatus> {
    let test_model = std::env::var("HEX_BRAIN_TEST_MODEL")
        .unwrap_or_else(|_| "nemotron-mini".to_string());
    
    Json(BrainStatus {
        service_enabled: true,
        test_model,
        interval_secs: 600,
        last_test: "never".to_string(),
    })
}

pub async fn test(
    State(state): State<SharedState>,
    Json(_req): Json<BrainTestRequest>,
) -> Json<BrainTestResponse> {
    // Run a test cycle synchronously
    match brain_service::run_improvement_cycle(&state).await {
        Ok(outcome) => Json(BrainTestResponse {
            outcome: outcome.outcome,
            reward: outcome.reward,
            response: "test completed".to_string(),
        }),
        Err(e) => Json(BrainTestResponse {
            outcome: "error".to_string(),
            reward: -0.5,
            response: e,
        }),
    }
}