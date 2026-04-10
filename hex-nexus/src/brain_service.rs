//! Brain self-improvement service — runs as a background service.
//!
//! Periodically tests local models, records outcomes, and updates
//! method scores in SpacetimeDB via the RL engine.

use serde_json::json;
use std::time::Duration;

use crate::state::SharedState;

/// Interval between self-improvement cycles (10 minutes).
const IMPROVEMENT_INTERVAL_SECS: u64 = 600;

/// Timeout for model test requests (30 seconds).
const TEST_TIMEOUT_SECS: u64 = 30;

/// Model to test (configured via env var, defaults to nemotron-mini).
fn test_model() -> String {
    std::env::var("HEX_BRAIN_TEST_MODEL")
        .unwrap_or_else(|_| "nemotron-mini".to_string())
}

/// State key for brain model selection.
fn state_key() -> String {
    "brain:model:selection".to_string()
}

/// Spawns the brain self-improvement service.
///
/// This runs as a background task that:
/// 1. Every 10 minutes, tests the configured local model
/// 2. Records the outcome (success/failure) to RL engine
/// 3. Updates method scores based on outcomes
pub fn spawn(state: SharedState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(IMPROVEMENT_INTERVAL_SECS));

        // Initial delay before first test
        interval.tick().await;

        loop {
            let result = run_improvement_cycle(&state).await;

            match result {
                Ok(outcome) => {
                    tracing::info!(
                        "Brain self-improvement: model={}, outcome={}, reward={:.2}",
                        test_model(),
                        outcome.outcome,
                        outcome.reward
                    );
                }
                Err(e) => {
                    tracing::warn!("Brain self-improvement cycle failed: {}", e);
                }
            }

            interval.tick().await;
        }
    });
}

/// Result of a single improvement cycle.
#[derive(Debug)]
pub struct ImprovementOutcome {
    pub outcome: String,
    pub reward: f64,
}

/// Runs one improvement cycle: test model, record outcome.
async fn run_improvement_cycle(_state: &SharedState) -> Result<ImprovementOutcome, String> {
    let model = test_model();
    let state_key = state_key();

    // Create a simple test prompt
    let prompt = "Write a simple hello world function in TypeScript. Return only the code, no explanation.";

    // Make the inference request via local hex-nexus API
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(TEST_TIMEOUT_SECS))
        .build()
        .map_err(|e| format!("client build error: {}", e))?;

    let nexus_host = std::env::var("HEX_NEXUS_HOST")
        .unwrap_or_else(|_| "http://127.0.0.1:5555".to_string());

    let url = format!("{}/api/inference/complete", nexus_host);

    let body = json!({
        "model": model,
        "messages": [
            {
                "role": "user",
                "content": prompt
            }
        ],
        "max_tokens": 256
    });

    let response = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("request error: {}", e))?;

    let status = response.status();
    let outcome = if status.is_success() {
        "success"
    } else if status.as_u16() == 429 {
        "rate_limited"
    } else {
        "failed"
    };

    // Compute reward based on outcome
    let reward = match outcome {
        "success" => 0.5,
        "rate_limited" => -0.3,
        _ => -0.5,
    };

    // Record to RL engine if outcome is conclusive
    if outcome == "success" || outcome == "rate_limited" || outcome == "failed" {
        if let Err(e) = record_reward_to_rl(&state_key, &format!("model:{}", model), reward).await {
            tracing::warn!("Failed to record reward to RL: {}", e);
        }
    }

    Ok(ImprovementOutcome {
        outcome: outcome.to_string(),
        reward,
    })
}

/// Records a reward to the RL engine reducer.
async fn record_reward_to_rl(
    state_key: &str,
    action: &str,
    reward: f64,
) -> Result<(), String> {
    let stdb_host = std::env::var("HEX_SPACETIMEDB_HOST")
        .unwrap_or_else(|_| "http://127.0.0.1:3033".to_string());

    let url = format!("{}/database/{}/reducer/record_reward/call",
        stdb_host,
        hex_core::STDB_DATABASE_RL
    );

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| format!("client error: {}", e))?;

    let now = chrono::Utc::now().to_rfc3339();

    let payload = json!({
        "state_key": state_key,
        "action": action,
        "reward": reward,
        "next_state_key": state_key,
        "rate_limited": false,
        "openrouter_cost_usd": 0.0,
        "task_type": "inference",
        "timestamp": now
    });

    let response = client
        .post(&url)
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("RL call error: {}", e))?;

    if response.status().is_success() {
        Ok(())
    } else {
        Err(format!("RL returned {}", response.status()))
    }
}