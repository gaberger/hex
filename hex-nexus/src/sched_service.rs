//! Sched self-improvement service — runs as a background service.
//!
//! Periodically tests local models, records outcomes, and updates
//! method scores in SpacetimeDB via the RL engine.

use rand::Rng;
use serde_json::json;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use crate::state::SharedState;
use crate::routes::inference::{inference_complete, InferenceCompleteRequest};

/// Shared lock that serializes the autopilot and RL self-improvement loops
/// against the same Ollama backend. They use different models
/// (qwen2.5-coder:32b ≈ 19.8 GB vs nemotron-mini ≈ 2.7 GB) which together
/// exceed typical local GPU VRAM, so Ollama evicts whichever model is
/// least-recently-used. Without this lock, the autopilot's qwen load
/// during a tick can evict nemotron mid-RL-cycle and the RL inference
/// times out at 30s waiting for a reload (observed 2026-04-27 17:53).
/// Mutex contention is negligible: autopilot ticks every 60min, RL every
/// 10min, both calls take seconds.
fn sched_inference_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

/// Exploration rate for ε-greedy model selection in run_improvement_cycle.
/// 0.30 means: 30% of cycles probe a random local model, 70% probe whichever
/// has the highest current Q-value. The RL engine's own select_action is
/// pure argmax (despite its EPSILON constant), so exploration has to happen
/// here at the caller — otherwise every cycle re-tests the same winner and
/// the loop never learns about other models.
const SCHED_EPSILON: f64 = 0.30;

/// Interval between self-improvement cycles (10 minutes).
pub const IMPROVEMENT_INTERVAL_SECS: u64 = 600;

/// Interval between substrate promote-orchestrator ticks (30s). Short
/// because a shadow_green ticket sitting un-promoted means traffic is
/// still routing through the old binding when it should have flipped —
/// we want the operator-visible latency between "judge greenlit" and
/// "live binding flipped" to stay under one minute.
pub const PROMOTE_TICK_INTERVAL_SECS: u64 = 30;

/// Interval between L4 shrinkage-daemon ticks (1 hour). Long because the
/// pressure shrinkage applies is structural-not-urgent — leftover
/// candidate handles waste a small amount of memory but don't affect
/// correctness. The idle window itself is the real shrinkage gate.
pub const SHRINKAGE_TICK_INTERVAL_SECS: u64 = 3600;

/// Default idle window before an unrouted, unbound handle is shrinkable
/// (24 hours). Operator can override per-deployment by editing the
/// constant or, future-work, via .hex/project.json.
const SHRINKAGE_IDLE_WINDOW_SECS: u64 = 86400;

/// SubstrateAutopilot tick interval (1 hour). Slow because each tick
/// makes an inference call against the live `default-inference` binding
/// — frequent ticks would consume real model budget on substrate
/// self-reflection. Operator can lower it for active development /
/// raise it for production-stable.
pub const AUTOPILOT_TICK_INTERVAL_SECS: u64 = 3600;

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

    // SubstrateAutopilot — the substrate uses itself to recommend its
    // own next improvements (ADR-2604261500 closing-the-loop). Hourly
    // tick reads substrate state, calls the live inference binding, and
    // logs a typed Recommendation. Read-only today; future revision
    // gates auto-propose behind operator opt-in. No-op when inference
    // port or swap_ticket port aren't wired.
    spawn_substrate_autopilot(state.clone());

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
            *state.last_improvement_tick.write().await = Some(chrono::Utc::now().to_rfc3339());
            let result = run_improvement_cycle(&state, None).await;

            match result {
                Ok(outcome) => {
                    tracing::info!(
                        "Sched self-improvement: model={}, outcome={}, reward={:.2}",
                        outcome.model,
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

fn spawn_substrate_autopilot(state: SharedState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(AUTOPILOT_TICK_INTERVAL_SECS));
        interval.tick().await; // initial delay
        loop {
            *state.last_autopilot_tick.write().await = Some(chrono::Utc::now().to_rfc3339());
            if let (Some(swap_port), Some(inference)) = (
                state.swap_ticket_port.as_ref(),
                state.inference_port.as_ref(),
            ) {
                let model = std::env::var("HEX_SUBSTRATE_AUTOPILOT_MODEL")
                    .unwrap_or_else(|_| "qwen2.5-coder:32b".into());
                let pilot = crate::orchestration::substrate_autopilot::SubstrateAutopilot::new(
                    swap_port.clone(),
                    inference.clone(),
                    model,
                );
                // Serialize against the RL self-improvement loop on the same
                // Ollama backend. See sched_inference_lock() docs.
                let _inf_guard = sched_inference_lock().lock().await;
                let report = pilot.tick().await;
                drop(_inf_guard);
                use crate::orchestration::substrate_autopilot::Recommendation;
                match &report.recommendation {
                    Some(Recommendation::NoAction) => {
                        tracing::info!("substrate_autopilot: NO-ACTION (substrate looks healthy)");
                    }
                    Some(Recommendation::Recommend { text }) => {
                        tracing::info!(recommendation = %text, "substrate_autopilot: RECOMMEND");
                    }
                    Some(Recommendation::ProposeSwap { json }) => {
                        tracing::info!(spec = %json, "substrate_autopilot: PROPOSE-SWAP (logged; auto-dispatch is a follow-up)");
                    }
                    Some(Recommendation::Abstain { reason }) => {
                        tracing::warn!(reason = %reason, "substrate_autopilot: abstained");
                    }
                    None => {}
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
            *state.last_shrinkage_tick.write().await = Some(chrono::Utc::now().to_rfc3339());
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
            *state.last_promote_tick.write().await = Some(chrono::Utc::now().to_rfc3339());
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
    pub model: String,
    pub outcome: String,
    pub reward: f64,
}

/// Runs one improvement cycle: pick a model (RL-driven by default, override
/// optional), exercise it, compute a quality-weighted reward, persist to RL.
///
/// `model_override`:
///   - `Some(name)` — operator forced this model (e.g. POST /api/sched/test
///     with explicit `"model"`). Acts as a manual probe.
///   - `None` — autonomous cycle. ε-greedy over the local Ollama models in
///     the rl_q_entry table; RL engine learns which performs best.
pub async fn run_improvement_cycle(
    state: &SharedState,
    model_override: Option<&str>,
) -> Result<ImprovementOutcome, String> {
    let state_key = state_key();
    let stdb_host = std::env::var("HEX_SPACETIMEDB_HOST")
        .unwrap_or_else(|_| "http://127.0.0.1:3033".to_string());

    let (model, selection_mode) = match model_override {
        Some(m) if !m.is_empty() && m != "auto" => (m.to_string(), "override"),
        _ => match select_model_for_cycle(&stdb_host, &state_key).await {
            Ok((m, mode)) => (m, mode),
            Err(e) => {
                tracing::warn!("RL model selection failed ({}); falling back to env default", e);
                (test_model(), "fallback")
            }
        },
    };

    let prompt = "Write a simple hello world function in TypeScript. Return only the code, no explanation.";
    let req = InferenceCompleteRequest {
        model: Some(model.clone()),
        messages: vec![json!({ "role": "user", "content": prompt })],
        system: None,
        max_tokens: 256,
        tools: None,
    };

    // In-process call (ADR-2604270145): bypass HTTP self-loopback to avoid
    // transient TCP-level transport errors that surfaced as
    // "error sending request for url (http://127.0.0.1:5555/...)" when the
    // local connection pool churned. Same SharedState, same handler logic;
    // empty HeaderMap because the cycle has no upstream caller context.
    let started = Instant::now();
    // Acquire the shared sched-inference lock. See sched_inference_lock() docs.
    let _inf_guard = sched_inference_lock().lock().await;
    let inference_fut = inference_complete(
        axum::extract::State(state.clone()),
        axum::http::HeaderMap::new(),
        axum::Json(req),
    );
    let (status, axum::Json(response_body)) =
        match tokio::time::timeout(Duration::from_secs(TEST_TIMEOUT_SECS), inference_fut).await {
            Ok(pair) => pair,
            Err(_) => return Err(format!("inference_complete timed out after {}s", TEST_TIMEOUT_SECS)),
        };
    let elapsed_ms = started.elapsed().as_millis() as u64;

    // Pull output_tokens out of the body when we can — it's the input to the
    // quality bonus. Best-effort; missing field just collapses to 0.
    let output_tokens: u64 = if status.is_success() {
        response_body.get("output_tokens").and_then(|n| n.as_u64()).unwrap_or(0)
    } else {
        0
    };

    let outcome = if status.is_success() {
        "success"
    } else if status.as_u16() == 429 {
        "rate_limited"
    } else {
        "failed"
    };

    // Differentiated reward (calibrated 2026-04-27 after the v1 quality-bonus +
    // linear-latency formula saturated symmetrically and produced flat 0.5):
    //   base             : ±0.5 by outcome
    //   +compliance bonus: +0.3 only if output is in [10, 60] tokens. The probe
    //                      prompt says "return only the code, no explanation";
    //                      verbose answers earn nothing rather than getting
    //                      a saturating bonus that masked latency cost.
    //   −latency penalty : log-shaped, never saturates inside operator range.
    //                      1s→0.0, 3.16s→0.2, 10s→0.4, 31.6s→0.6, 100s→0.8.
    // Net for "ideal" success: 0.5 + 0.3 − ~0.15 ≈ +0.65.
    // Net for "verbose+slow": 0.5 + 0.0 − ~0.55 ≈ −0.05  → RL learns: avoid.
    let (base, compliance_bonus, latency_penalty) = match outcome {
        "success" => {
            let c = if (10..=60).contains(&output_tokens) { 0.3 } else { 0.0 };
            let l = (elapsed_ms as f64 / 1000.0).log10().max(0.0).min(1.5) * 0.4;
            (0.5, c, l)
        }
        "rate_limited" => (-0.3, 0.0, 0.0),
        _ => (-0.5, 0.0, 0.0),
    };
    let reward = base + compliance_bonus - latency_penalty;

    if let Err(e) = record_reward_to_rl(&state_key, &format!("model:{}", model), reward).await {
        tracing::warn!("Failed to record reward to RL: {}", e);
    }

    tracing::info!(
        model = %model,
        selection = %selection_mode,
        outcome = %outcome,
        elapsed_ms = elapsed_ms,
        output_tokens = output_tokens,
        base = base,
        compliance_bonus = compliance_bonus,
        latency_penalty = latency_penalty,
        reward = reward,
        "Sched improvement cycle: detail"
    );

    // Re-query the Q-table after the reward landed so we observe the
    // leader as it stands *now*. If a non-incumbent has overtaken the
    // current leader, that's the headline signal — log it loudly and
    // surface in /api/sched/status.
    if let Ok(new_leader) = current_q_leader(&stdb_host, &state_key).await {
        let prior = state.rl_leader.read().await.clone();
        if prior.as_deref() != Some(new_leader.as_str()) {
            let when = chrono::Utc::now().to_rfc3339();
            tracing::info!(
                from = %prior.as_deref().unwrap_or("(none)"),
                to = %new_leader,
                at = %when,
                "RL leader CHANGED"
            );
            *state.rl_leader.write().await = Some(new_leader.clone());
            *state.rl_leader_changed_at.write().await = Some(when);
        } else if prior.is_none() {
            *state.rl_leader.write().await = Some(new_leader);
            *state.rl_leader_changed_at.write().await = Some(chrono::Utc::now().to_rfc3339());
        }
    }

    Ok(ImprovementOutcome { model: model.clone(), outcome: outcome.to_string(), reward })
}

/// Return the current Q-leader (model name without the `model:` prefix) for
/// `state_key`, restricted to the local-Ollama subset that the cycle actually
/// exercises. Errors out silently — leadership is observability, not load-bearing.
async fn current_q_leader(stdb_host: &str, state_key: &str) -> Result<String, String> {
    let sql = format!(
        "SELECT action, q_value FROM rl_q_entry WHERE state_key = '{}'",
        state_key.replace('\'', "''")
    );
    let url = format!("{}/v1/database/{}/sql", stdb_host, hex_core::STDB_DATABASE_RL);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| format!("client error: {}", e))?;
    let resp = client.post(&url).header("content-type", "text/plain").body(sql)
        .send().await.map_err(|e| format!("STDB SQL error: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("STDB SQL returned {}", resp.status()));
    }
    let parsed: Vec<serde_json::Value> = resp.json().await
        .map_err(|e| format!("STDB SQL parse: {}", e))?;
    let rows = parsed.into_iter().next()
        .and_then(|v| v.get("rows").cloned())
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();
    let local_models = locally_served_models().await.unwrap_or_default();
    rows.iter()
        .filter_map(|r| {
            let arr = r.as_array()?;
            let action = arr.first()?.as_str()?.to_string();
            let q = arr.get(1)?.as_f64()?;
            let model = action.strip_prefix("model:")?.to_string();
            if !local_models.contains(&model) {
                return None;
            }
            Some((model, q))
        })
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(m, _)| m)
        .ok_or_else(|| "no local candidates".into())
}

/// Set of model names served by a registered Ollama / openai-compat endpoint.
/// Empty set is "endpoints unreachable" — caller decides whether to treat
/// that as a hard error or fall back. Used by both the cycle's selector and
/// the leader query so they share a single source of truth for what's
/// actually probeable here.
async fn locally_served_models() -> Result<std::collections::HashSet<String>, String> {
    let nexus_host = std::env::var("HEX_NEXUS_HOST")
        .unwrap_or_else(|_| "http://127.0.0.1:5555".to_string());
    let url = format!("{}/api/inference/endpoints", nexus_host);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| format!("client error: {}", e))?;
    let resp = client.get(&url).send().await
        .map_err(|e| format!("endpoints fetch: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("endpoints returned {}", resp.status()));
    }
    let raw: serde_json::Value = resp.json().await
        .map_err(|e| format!("endpoints parse: {}", e))?;
    // Endpoint route returns either a bare JSON array or `{ "endpoints": [...] }`
    // depending on which handler version is wired (the secrets::list_inference
    // surface uses the wrapper shape). Accept both.
    let endpoints = raw.as_array()
        .or_else(|| raw.get("endpoints").and_then(|v| v.as_array()))
        .ok_or_else(|| "endpoints response neither array nor {endpoints:[...]}".to_string())?;
    Ok(endpoints.iter()
        .filter(|e| matches!(
            e.get("provider").and_then(|v| v.as_str()),
            Some("ollama") | Some("openai_compat")
        ))
        .filter_map(|e| e.get("model").and_then(|v| v.as_str()).map(String::from))
        .collect())
}

/// Pick a model action for the next improvement cycle.
/// ε-greedy over the locally-registered subset of `rl_q_entry` rows for
/// `state_key`. The candidate set is the *intersection* of:
///   1. Q-table actions for this state, and
///   2. Models actually served by a registered Ollama / openai-compat endpoint.
/// Without (2) the cycle would happily explore seeded cloud aliases (opus,
/// sonnet, …) — Ollama would 404, the inference router would silently fall
/// through to the OpenRouter free-model chain, and we'd record a fabricated
/// reward against the wrong model action while paying real OR cents.
///   - With probability SCHED_EPSILON, sample uniformly (explore).
///   - Otherwise pick the action with the highest Q-value (exploit).
/// Returns `(model_name_without_prefix, "explore"|"exploit")`.
async fn select_model_for_cycle(
    stdb_host: &str,
    state_key: &str,
) -> Result<(String, &'static str), String> {
    // 1. Q-table actions for this state.
    let sql = format!(
        "SELECT action, q_value FROM rl_q_entry WHERE state_key = '{}'",
        state_key.replace('\'', "''")
    );
    let url = format!("{}/v1/database/{}/sql", stdb_host, hex_core::STDB_DATABASE_RL);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| format!("client error: {}", e))?;
    let resp = client.post(&url).header("content-type", "text/plain").body(sql)
        .send().await.map_err(|e| format!("STDB SQL error: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("STDB SQL returned {}", resp.status()));
    }
    let parsed: Vec<serde_json::Value> = resp.json().await
        .map_err(|e| format!("STDB SQL parse: {}", e))?;
    let rows = parsed.into_iter().next()
        .and_then(|v| v.get("rows").cloned())
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();

    // 2. Models actually served locally.
    let local_models = locally_served_models().await?;

    // Intersection.
    let candidates: Vec<(String, f64)> = rows.iter().filter_map(|r| {
        let arr = r.as_array()?;
        let action = arr.first()?.as_str()?.to_string();
        let q = arr.get(1)?.as_f64()?;
        let model = action.strip_prefix("model:")?;
        if local_models.contains(model) {
            Some((model.to_string(), q))
        } else {
            None
        }
    }).collect();

    if candidates.is_empty() {
        return Err("no local model candidates intersect rl_q_entry".into());
    }

    let explore = rand::thread_rng().gen_bool(SCHED_EPSILON);
    let pick = if explore {
        let idx = rand::thread_rng().gen_range(0..candidates.len());
        candidates[idx].0.clone()
    } else {
        candidates.iter()
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|c| c.0.clone())
            .ok_or_else(|| "no max in candidates".to_string())?
    };
    Ok((pick, if explore { "explore" } else { "exploit" }))
}

/// Records a reward to the RL engine reducer.
async fn record_reward_to_rl(
    state_key: &str,
    action: &str,
    reward: f64,
) -> Result<(), String> {
    let stdb_host = std::env::var("HEX_SPACETIMEDB_HOST")
        .unwrap_or_else(|_| "http://127.0.0.1:3033".to_string());

    // SpacetimeDB v2 reducer-call shape: POST /v1/database/{db}/call/{reducer}
    // with a JSON array of positional args matching the reducer signature
    // (ctx is implicit). record_reward takes 6 args; the previous payload
    // sent 8 named fields (extra task_type/timestamp) at the legacy URL,
    // so every cycle was 404'ing and the RL table never updated.
    let url = format!("{}/v1/database/{}/call/record_reward",
        stdb_host,
        hex_core::STDB_DATABASE_RL
    );

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| format!("client error: {}", e))?;

    let payload = json!([
        state_key,
        action,
        reward,
        state_key,   // next_state_key
        false,       // rate_limited
        0.0,         // openrouter_cost_usd
    ]);

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
        "{}/v1/database/{}/call/seed_model_q_values",
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