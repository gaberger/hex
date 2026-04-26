//! Sched self-improvement service — runs as a background service.
//!
//! Periodically tests local models, records outcomes, and updates
//! method scores in SpacetimeDB via the RL engine.

use serde_json::json;
use std::time::Duration;

use crate::state::SharedState;

/// Interval between self-improvement cycles (10 minutes).
const IMPROVEMENT_INTERVAL_SECS: u64 = 600;

/// Interval between substrate promote-orchestrator ticks (30s). Short
/// because a shadow_green ticket sitting un-promoted means traffic is
/// still routing through the old binding when it should have flipped —
/// we want the operator-visible latency between "judge greenlit" and
/// "live binding flipped" to stay under one minute.
const PROMOTE_TICK_INTERVAL_SECS: u64 = 30;

/// Interval between L4 shrinkage-daemon ticks (1 hour). Long because the
/// pressure shrinkage applies is structural-not-urgent — leftover
/// candidate handles waste a small amount of memory but don't affect
/// correctness. The idle window itself is the real shrinkage gate.
const SHRINKAGE_TICK_INTERVAL_SECS: u64 = 3600;

/// Default idle window before an unrouted, unbound handle is shrinkable
/// (24 hours). Operator can override per-deployment by editing the
/// constant or, future-work, via .hex/project.json.
const SHRINKAGE_IDLE_WINDOW_SECS: u64 = 86400;

/// Timeout for model test requests (30 seconds).
const TEST_TIMEOUT_SECS: u64 = 30;

/// Model to test (configured via env var, defaults to nemotron-mini).
fn test_model() -> String {
    std::env::var("HEX_SCHED_TEST_MODEL")
        .unwrap_or_else(|_| "nemotron-mini".to_string())
}

/// State key for sched model selection.
fn state_key() -> String {
    "brain:model:selection".to_string()
}

/// Spawns the sched self-improvement service.
///
/// This runs as a background task that:
/// 1. Every 10 minutes, tests the configured local model
/// 2. Records the outcome (success/failure) to RL engine
/// 3. Updates method scores based on outcomes
///
/// On startup, seeds all known model Q-values so the RL engine has actions
/// to choose from before any rewards are observed.
pub fn spawn(state: SharedState) {
    // Substrate promote-orchestrator tick (ADR-2604261500 P6,
    // wp-substrate-inference-consumer-rewires P5 follow-up). Spawned as
    // its own task with a tight 30s cadence so live-binding flips happen
    // promptly after the judge marks shadow_green. No-op when the
    // substrate isn't wired (inference_runtime_composition + shadow_router
    // both required); short-circuits inside the orchestrator when no
    // shadow_green tickets exist.
    spawn_promote_orchestrator(state.clone());

    // Substrate L4 shrinkage daemon (ADR-2604261311 L4 / ADR-2604261500
    // C6). Hourly tick; evicts handles registered on the shadow router
    // that are not bound, not active shadow candidates, and either never
    // routed or last routed before the idle window. No-op when the
    // substrate isn't wired.
    spawn_shrinkage_daemon(state.clone());

    tokio::spawn(async move {
        // ADR-2604241820: seed Q-values before the loop starts so RL has priors.
        let state_key = state_key();
        if let Err(e) = seed_rl_q_values(&state_key).await {
            tracing::warn!("Failed to seed RL Q-values: {} — RL will explore from empty table", e);
        }

        let mut interval = tokio::time::interval(Duration::from_secs(IMPROVEMENT_INTERVAL_SECS));

        // Initial delay before first test
        interval.tick().await;

        loop {
            let result = run_improvement_cycle(&state).await;

            match result {
                Ok(outcome) => {
                    tracing::info!(
                        "Sched self-improvement: model={}, outcome={}, reward={:.2}",
                        test_model(),
                        outcome.outcome,
                        outcome.reward
                    );
                }
                Err(e) => {
                    tracing::warn!("Sched self-improvement cycle failed: {}", e);
                }
            }

            interval.tick().await;
        }
    });
}

fn spawn_shrinkage_daemon(state: SharedState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(SHRINKAGE_TICK_INTERVAL_SECS));
        interval.tick().await; // initial delay
        let idle = Duration::from_secs(SHRINKAGE_IDLE_WINDOW_SECS);
        loop {
            if let Some(router) = state.inference_shadow_router.as_ref() {
                let daemon = crate::orchestration::shrinkage_daemon::ShrinkageDaemon::new(
                    router.clone(),
                    idle,
                );
                let report = daemon.tick().await;
                if !report.shrunk.is_empty() {
                    tracing::info!(
                        shrunk = ?report.shrunk,
                        "substrate: shrinkage_daemon evicted unrouted handles",
                    );
                }
            }
            interval.tick().await;
        }
    });
}

fn spawn_promote_orchestrator(state: SharedState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(PROMOTE_TICK_INTERVAL_SECS));
        interval.tick().await; // initial delay
        loop {
            let report = match (
                state.swap_ticket_port.as_ref(),
                state.inference_runtime_composition.as_ref(),
                state.inference_shadow_router.as_ref(),
            ) {
                (Some(swap_port), Some(comp), Some(router)) => {
                    // Wire L5 ADR conformance gate (ADR-2604261311 L5 /
                    // ADR-2604261500 C6). The checker reads docs/adrs/
                    // fresh on each tick — operator edits to ADR Status
                    // fields take effect on the next 30s tick without a
                    // nexus restart.
                    let checker = crate::orchestration::adr_conformance::AdrConformanceChecker::new(
                        std::path::PathBuf::from("docs/adrs"),
                    );
                    let orch = crate::orchestration::promote_orchestrator::PromoteOrchestrator::new(
                        swap_port.clone(),
                        comp.clone(),
                        router.clone(),
                    )
                    .with_conformance(checker);
                    Some(orch.tick().await)
                }
                _ => None,
            };
            if let Some(report) = report {
                if !report.promoted.is_empty() {
                    tracing::info!(
                        promoted = ?report.promoted,
                        "substrate: promote_orchestrator flipped live bindings",
                    );
                }
                if !report.skipped_missing_handle.is_empty() {
                    tracing::warn!(
                        skipped = ?report.skipped_missing_handle,
                        "substrate: promote_orchestrator skipped tickets (candidate handle not registered)",
                    );
                }
                if !report.blocked_by_l5.is_empty() {
                    tracing::warn!(
                        blocked = ?report.blocked_by_l5,
                        "substrate: promote_orchestrator blocked tickets (L5 ADR conformance)",
                    );
                }
                for (id, err) in report.errors {
                    tracing::warn!(ticket = %id, error = %err, "substrate: promote_orchestrator error");
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
pub async fn run_improvement_cycle(_state: &SharedState) -> Result<ImprovementOutcome, String> {
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

/// Seed all model Q-values into the RL engine for the given state key.
/// Called once on startup to prime the Q-table with default priors so
/// the RL engine has actions to choose from before any rewards are observed.
async fn seed_rl_q_values(state_key: &str) -> Result<u32, String> {
    let stdb_host = std::env::var("HEX_SPACETIMEDB_HOST")
        .unwrap_or_else(|_| "http://127.0.0.1:3033".to_string());

    let url = format!(
        "{}/database/{}/reducer/seed_model_q_values/call",
        stdb_host,
        hex_core::STDB_DATABASE_RL
    );

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| format!("client error: {}", e))?;

    let payload = json!([state_key]);

    let response = client
        .post(&url)
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("seed RL call error: {}", e))?;

    if response.status().is_success() {
        tracing::info!("Seeded RL Q-values for state '{}'", state_key);
        Ok(1)
    } else {
        Err(format!("seed RL returned {}", response.status()))
    }
}