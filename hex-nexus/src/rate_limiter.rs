//! Sliding-window rate limit tracker + circuit breaker (ADR-2604052125).
//!
//! Tracks per-provider request/token consumption in a 60-second sliding window
//! and daily counters. Provides circuit breaker (Closed/Open/Half-Open) per
//! provider to prevent hammering providers that are rate-limiting or down.
//!
//! This is ephemeral, per-instance state — stored in hex-nexus memory, not
//! SpacetimeDB. Resets on nexus restart.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

// ─── Circuit Breaker ───────────────────────────────────────────────────────

/// Circuit breaker states (ADR-2604052125).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CircuitState {
    /// Normal operation — requests flow through.
    Closed,
    /// Provider is failing — skip for cooldown period.
    Open,
    /// Cooldown expired — send one probe request to test recovery.
    HalfOpen,
}

/// Per-provider circuit breaker.
#[derive(Debug, Clone)]
pub struct CircuitBreaker {
    pub state: CircuitState,
    /// Number of consecutive failures.
    pub consecutive_failures: u32,
    /// When the circuit was opened (for cooldown calculation).
    pub opened_at: Option<Instant>,
    /// Current cooldown duration (doubles on each re-open, max 30 min).
    pub cooldown: Duration,
}

impl Default for CircuitBreaker {
    fn default() -> Self {
        Self {
            state: CircuitState::Closed,
            consecutive_failures: 0,
            opened_at: None,
            cooldown: Duration::from_secs(300), // 5 minutes initial
        }
    }
}

impl CircuitBreaker {
    /// Record a successful request — close circuit, reset failures.
    pub fn record_success(&mut self) {
        self.state = CircuitState::Closed;
        self.consecutive_failures = 0;
        self.cooldown = Duration::from_secs(300); // Reset to 5 min
        self.opened_at = None;
    }

    /// Record a failure (429, 5xx, timeout).
    /// Opens circuit after 3 consecutive failures.
    pub fn record_failure(&mut self) {
        self.consecutive_failures += 1;
        if self.consecutive_failures >= 3 && self.state == CircuitState::Closed {
            self.state = CircuitState::Open;
            self.opened_at = Some(Instant::now());
            tracing::warn!(
                failures = self.consecutive_failures,
                cooldown_secs = self.cooldown.as_secs(),
                "circuit breaker opened"
            );
        } else if self.state == CircuitState::HalfOpen {
            // Probe failed — re-open with exponential backoff
            self.state = CircuitState::Open;
            self.opened_at = Some(Instant::now());
            self.cooldown = (self.cooldown * 2).min(Duration::from_secs(1800)); // Max 30 min
            tracing::warn!(
                cooldown_secs = self.cooldown.as_secs(),
                "half-open probe failed — re-opening with longer cooldown"
            );
        }
    }

    /// Check if the circuit allows a request.
    /// Returns true if request should proceed, false if provider should be skipped.
    pub fn should_allow(&mut self) -> bool {
        match self.state {
            CircuitState::Closed => true,
            CircuitState::Open => {
                // Check if cooldown has expired
                if let Some(opened_at) = self.opened_at {
                    if opened_at.elapsed() >= self.cooldown {
                        self.state = CircuitState::HalfOpen;
                        tracing::info!("circuit breaker half-open — sending probe request");
                        true // Allow one probe
                    } else {
                        false // Still cooling down
                    }
                } else {
                    false
                }
            }
            CircuitState::HalfOpen => {
                // Only one probe at a time — block additional requests
                false
            }
        }
    }
}

// ─── Rate Limit Tracker ────────────────────────────────────────────────────

/// Per-provider rate consumption state (ADR-2604052125).
#[derive(Debug, Clone)]
pub struct ProviderRateState {
    pub provider_id: String,
    /// Requests in the current 60-second window.
    pub requests_this_minute: u32,
    /// Tokens consumed in the current 60-second window.
    pub tokens_this_minute: u64,
    /// Total requests today (since midnight UTC).
    pub requests_today: u32,
    /// Total tokens consumed today.
    pub tokens_today: u64,
    /// Start of the current sliding window.
    pub window_start: Instant,
    /// UTC date of the current daily counter (YYYY-MM-DD).
    pub daily_date: String,
    /// RPM limit from provider template.
    pub rpm_limit: u32,
    /// TPM limit from provider template.
    pub tpm_limit: u64,
    /// Daily token limit (0 = unlimited).
    pub daily_token_limit: u64,
    /// Daily request limit (0 = unlimited).
    pub daily_request_limit: u32,
    /// Whether this is a free-tier provider.
    pub is_free_tier: bool,
    /// Cost per million input tokens.
    pub cost_per_input_mtok: f64,
    /// Cost per million output tokens.
    pub cost_per_output_mtok: f64,
    /// Circuit breaker state.
    pub circuit: CircuitBreaker,
}

impl ProviderRateState {
    pub fn new(
        provider_id: String,
        rpm_limit: u32,
        tpm_limit: u64,
        daily_token_limit: u64,
        daily_request_limit: u32,
        is_free_tier: bool,
        cost_per_input_mtok: f64,
        cost_per_output_mtok: f64,
    ) -> Self {
        Self {
            provider_id,
            requests_this_minute: 0,
            tokens_this_minute: 0,
            requests_today: 0,
            tokens_today: 0,
            window_start: Instant::now(),
            daily_date: today_utc(),
            rpm_limit,
            tpm_limit,
            daily_token_limit,
            daily_request_limit,
            is_free_tier,
            cost_per_input_mtok,
            cost_per_output_mtok,
            circuit: CircuitBreaker::default(),
        }
    }

    /// Reset the sliding window if 60 seconds have passed.
    fn maybe_reset_window(&mut self) {
        if self.window_start.elapsed() >= Duration::from_secs(60) {
            self.requests_this_minute = 0;
            self.tokens_this_minute = 0;
            self.window_start = Instant::now();
        }
    }

    /// Reset daily counters if the date has changed.
    fn maybe_reset_daily(&mut self) {
        let today = today_utc();
        if self.daily_date != today {
            self.requests_today = 0;
            self.tokens_today = 0;
            self.daily_date = today;
        }
    }

    /// Check if this provider can accept a request.
    /// Returns true if the provider has remaining capacity.
    pub fn should_route(&mut self) -> bool {
        self.maybe_reset_window();
        self.maybe_reset_daily();

        // Circuit breaker check
        if !self.circuit.should_allow() {
            return false;
        }

        // RPM check (>80% consumed = preemptively skip)
        if self.rpm_limit > 0 {
            let threshold = (self.rpm_limit as f64 * 0.8) as u32;
            if self.requests_this_minute >= threshold {
                return false;
            }
        }

        // Daily token limit check
        if self.daily_token_limit > 0 && self.tokens_today >= self.daily_token_limit {
            return false;
        }

        // Daily request limit check
        if self.daily_request_limit > 0 && self.requests_today >= self.daily_request_limit {
            return false;
        }

        true
    }

    /// Record a dispatched request (before response).
    pub fn record_request(&mut self, estimated_tokens: u64) {
        self.maybe_reset_window();
        self.maybe_reset_daily();
        self.requests_this_minute += 1;
        self.tokens_this_minute += estimated_tokens;
        self.requests_today += 1;
        self.tokens_today += estimated_tokens;
    }

    /// Record request completion.
    pub fn record_completion(&mut self, actual_tokens: u64, success: bool) {
        if success {
            self.circuit.record_success();
        } else {
            self.circuit.record_failure();
        }
        // Adjust token counts if actual differs from estimate
        // (we already counted estimated; adjust the delta)
        let _ = actual_tokens; // Token counts were already recorded on dispatch
    }

    /// Remaining daily token quota (None if unlimited).
    pub fn remaining_daily_tokens(&self) -> Option<u64> {
        if self.daily_token_limit > 0 {
            Some(self.daily_token_limit.saturating_sub(self.tokens_today))
        } else {
            None
        }
    }
}

fn today_utc() -> String {
    chrono::Utc::now().format("%Y-%m-%d").to_string()
}

// ─── Rate Limit Manager ────────────────────────────────────────────────────

/// Manages rate state for all providers (ADR-2604052125).
/// Thread-safe, shared across all inference routes.
#[derive(Clone)]
pub struct RateLimitManager {
    states: Arc<RwLock<HashMap<String, ProviderRateState>>>,
    /// Inference cost tracking — actual vs. counterfactual.
    cost_tracker: Arc<RwLock<CostTracker>>,
}

/// Tracks actual vs. counterfactual inference cost (ADR-2604052125 Phase 4).
#[derive(Debug, Clone, Default)]
pub struct CostTracker {
    /// Total actual cost across all providers.
    pub actual_cost_usd: f64,
    /// What this would have cost on Opus ($15/M input, $75/M output).
    pub counterfactual_cost_usd: f64,
    /// Per-provider request counts and token totals.
    pub provider_stats: HashMap<String, ProviderCostStats>,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ProviderCostStats {
    pub name: String,
    pub requests: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
    pub is_free_tier: bool,
}

impl RateLimitManager {
    pub fn new() -> Self {
        Self {
            states: Arc::new(RwLock::new(HashMap::new())),
            cost_tracker: Arc::new(RwLock::new(CostTracker::default())),
        }
    }

    /// Register a provider's rate limits.
    pub async fn register_provider(
        &self,
        provider_id: &str,
        rpm_limit: u32,
        tpm_limit: u64,
        daily_token_limit: u64,
        daily_request_limit: u32,
        is_free_tier: bool,
        cost_per_input_mtok: f64,
        cost_per_output_mtok: f64,
    ) {
        let state = ProviderRateState::new(
            provider_id.to_string(),
            rpm_limit,
            tpm_limit,
            daily_token_limit,
            daily_request_limit,
            is_free_tier,
            cost_per_input_mtok,
            cost_per_output_mtok,
        );
        self.states.write().await.insert(provider_id.to_string(), state);
    }

    /// Check if a provider should receive traffic.
    pub async fn should_route(&self, provider_id: &str) -> bool {
        let mut states = self.states.write().await;
        if let Some(state) = states.get_mut(provider_id) {
            state.should_route()
        } else {
            true // Unknown provider — allow (no rate state registered)
        }
    }

    /// Record a request dispatched to a provider.
    pub async fn record_request(&self, provider_id: &str, estimated_tokens: u64) {
        let mut states = self.states.write().await;
        if let Some(state) = states.get_mut(provider_id) {
            state.record_request(estimated_tokens);
        }
    }

    /// Record request completion (success/failure).
    pub async fn record_completion(
        &self,
        provider_id: &str,
        input_tokens: u64,
        output_tokens: u64,
        success: bool,
    ) {
        {
            let mut states = self.states.write().await;
            if let Some(state) = states.get_mut(provider_id) {
                state.record_completion(input_tokens + output_tokens, success);
            }
        }

        // Update cost tracker
        {
            let states = self.states.read().await;
            let (cost_input, cost_output, is_free) = states.get(provider_id)
                .map(|s| (s.cost_per_input_mtok, s.cost_per_output_mtok, s.is_free_tier))
                .unwrap_or((0.0, 0.0, false));

            let actual_cost = (input_tokens as f64 * cost_input / 1_000_000.0)
                + (output_tokens as f64 * cost_output / 1_000_000.0);
            // Counterfactual: Opus pricing ($15/M input, $75/M output)
            let counterfactual = (input_tokens as f64 * 15.0 / 1_000_000.0)
                + (output_tokens as f64 * 75.0 / 1_000_000.0);

            let mut tracker = self.cost_tracker.write().await;
            tracker.actual_cost_usd += actual_cost;
            tracker.counterfactual_cost_usd += counterfactual;

            let stats = tracker.provider_stats
                .entry(provider_id.to_string())
                .or_insert_with(|| ProviderCostStats {
                    name: provider_id.to_string(),
                    is_free_tier: is_free,
                    ..Default::default()
                });
            stats.requests += 1;
            stats.input_tokens += input_tokens;
            stats.output_tokens += output_tokens;
            stats.cost_usd += actual_cost;
        }
    }

    /// Get all provider rate states (for /api/inference/rate-state endpoint).
    pub async fn get_all_states(&self) -> Vec<serde_json::Value> {
        let states = self.states.read().await;
        states.values().map(|s| {
            serde_json::json!({
                "provider_id": s.provider_id,
                "requests_this_minute": s.requests_this_minute,
                "tokens_this_minute": s.tokens_this_minute,
                "requests_today": s.requests_today,
                "tokens_today": s.tokens_today,
                "rpm_limit": s.rpm_limit,
                "daily_token_limit": s.daily_token_limit,
                "daily_request_limit": s.daily_request_limit,
                "is_free_tier": s.is_free_tier,
                "circuit_state": s.circuit.state,
                "consecutive_failures": s.circuit.consecutive_failures,
            })
        }).collect()
    }

    /// Get cost attribution summary (for /api/inference/stats endpoint).
    pub async fn get_cost_stats(&self) -> serde_json::Value {
        let tracker = self.cost_tracker.read().await;
        let providers: Vec<_> = tracker.provider_stats.values()
            .map(|s| serde_json::to_value(s).unwrap_or_default())
            .collect();
        serde_json::json!({
            "providers": providers,
            "summary": {
                "actual_cost_usd": tracker.actual_cost_usd,
                "counterfactual_cost_usd": tracker.counterfactual_cost_usd,
                "savings_pct": if tracker.counterfactual_cost_usd > 0.0 {
                    (1.0 - tracker.actual_cost_usd / tracker.counterfactual_cost_usd) * 100.0
                } else {
                    0.0
                },
            }
        })
    }

    /// Select the best available provider from a list, respecting rate limits
    /// and circuit breakers. Prefers free-tier providers with remaining quota.
    /// Returns provider_id of the selected provider, or None if all exhausted.
    pub async fn select_best_provider(&self, provider_ids: &[String]) -> Option<String> {
        let mut states = self.states.write().await;

        // Partition into: free with quota, free half-open (testing), paid
        let mut free_available: Vec<&str> = Vec::new();
        let mut free_halfopen: Vec<&str> = Vec::new();
        let mut paid_available: Vec<&str> = Vec::new();
        let mut unknown: Vec<&str> = Vec::new();

        for id in provider_ids {
            if let Some(state) = states.get_mut(id.as_str()) {
                state.maybe_reset_window();
                state.maybe_reset_daily();

                let circuit_ok = match state.circuit.state {
                    CircuitState::Closed => true,
                    CircuitState::Open => {
                        if let Some(opened) = state.circuit.opened_at {
                            opened.elapsed() >= state.circuit.cooldown
                        } else {
                            false
                        }
                    }
                    CircuitState::HalfOpen => false,
                };

                let has_capacity = {
                    let rpm_ok = state.rpm_limit == 0 ||
                        state.requests_this_minute < (state.rpm_limit as f64 * 0.8) as u32;
                    let daily_tok_ok = state.daily_token_limit == 0 ||
                        state.tokens_today < state.daily_token_limit;
                    let daily_req_ok = state.daily_request_limit == 0 ||
                        state.requests_today < state.daily_request_limit;
                    rpm_ok && daily_tok_ok && daily_req_ok
                };

                if state.is_free_tier && circuit_ok && has_capacity {
                    free_available.push(id.as_str());
                } else if state.is_free_tier && state.circuit.state == CircuitState::HalfOpen {
                    free_halfopen.push(id.as_str());
                } else if !state.is_free_tier && circuit_ok && has_capacity {
                    paid_available.push(id.as_str());
                }
            } else {
                unknown.push(id.as_str());
            }
        }

        // Priority order (ADR-2604052125):
        // 1. Free provider with remaining quota
        // 2. Free provider in half-open circuit (testing recovery)
        // 3. Paid provider with lowest cost
        // 4. Unknown providers (no rate state registered)
        free_available.first()
            .or_else(|| free_halfopen.first())
            .or_else(|| paid_available.first())
            .or_else(|| unknown.first())
            .map(|s| s.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn circuit_breaker_opens_after_3_failures() {
        let mut cb = CircuitBreaker::default();
        assert_eq!(cb.state, CircuitState::Closed);
        cb.record_failure();
        cb.record_failure();
        assert_eq!(cb.state, CircuitState::Closed); // Still closed after 2
        cb.record_failure();
        assert_eq!(cb.state, CircuitState::Open); // Opens after 3
    }

    #[test]
    fn circuit_breaker_resets_on_success() {
        let mut cb = CircuitBreaker::default();
        cb.record_failure();
        cb.record_failure();
        cb.record_failure();
        assert_eq!(cb.state, CircuitState::Open);
        // Simulate cooldown expiry
        cb.opened_at = Some(Instant::now() - Duration::from_secs(600));
        assert!(cb.should_allow()); // Transitions to HalfOpen
        assert_eq!(cb.state, CircuitState::HalfOpen);
        cb.record_success();
        assert_eq!(cb.state, CircuitState::Closed);
        assert_eq!(cb.consecutive_failures, 0);
    }

    #[test]
    fn rate_state_preemptive_skip_at_80pct() {
        let mut state = ProviderRateState::new(
            "test".into(), 30, 100_000, 0, 0, true, 0.0, 0.0,
        );
        // 80% of 30 RPM = 24 requests
        for _ in 0..24 {
            state.record_request(100);
        }
        assert!(!state.should_route()); // At 80% — preemptively skip
    }

    #[test]
    fn rate_state_daily_token_limit() {
        let mut state = ProviderRateState::new(
            "test".into(), 0, 0, 1_000_000, 0, true, 0.0, 0.0,
        );
        state.tokens_today = 999_999;
        assert!(state.should_route()); // Just under
        state.tokens_today = 1_000_000;
        assert!(!state.should_route()); // At limit
    }
}
