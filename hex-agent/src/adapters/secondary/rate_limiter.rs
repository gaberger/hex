use crate::ports::{RateLimitHeaders, RateLimitState};
use crate::ports::rate_limiter::RateLimiterPort;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

/// In-memory rate limiter that tracks per-model consumption.
///
/// Thread-safe via Mutex (low contention — single agent process).
/// State resets automatically when the 60-second window expires.
pub struct RateLimiterAdapter {
    states: Mutex<HashMap<String, RateLimitState>>,
}

impl RateLimiterAdapter {
    pub fn new() -> Self {
        Self {
            states: Mutex::new(HashMap::new()),
        }
    }

    fn ensure_state(states: &mut HashMap<String, RateLimitState>, model: &str) {
        if !states.contains_key(model) {
            states.insert(model.to_string(), RateLimitState::new(model.to_string()));
        }
    }
}

#[async_trait]
impl RateLimiterPort for RateLimiterAdapter {
    async fn should_throttle(
        &self,
        model: &str,
        estimated_input_tokens: u32,
        estimated_output_tokens: u32,
    ) -> Option<Duration> {
        let mut states = self.states.lock().ok()?;
        Self::ensure_state(&mut states, model);
        let state = states.get_mut(model)?;
        state.maybe_reset_window();
        state.should_throttle(estimated_input_tokens, estimated_output_tokens)
    }

    async fn record_usage(&self, model: &str, input_tokens: u32, output_tokens: u32) {
        if let Ok(mut states) = self.states.lock() {
            Self::ensure_state(&mut states, model);
            if let Some(state) = states.get_mut(model) {
                state.record_usage(input_tokens, output_tokens);
            }
        }
    }

    async fn record_rate_limit(&self, model: &str) {
        if let Ok(mut states) = self.states.lock() {
            Self::ensure_state(&mut states, model);
            if let Some(state) = states.get_mut(model) {
                state.record_rate_limit();
            }
        }
    }

    async fn update_from_headers(&self, model: &str, headers: &RateLimitHeaders) {
        if let Ok(mut states) = self.states.lock() {
            Self::ensure_state(&mut states, model);
            if let Some(state) = states.get_mut(model) {
                state.update_limits_from_headers(headers);
            }
        }
    }

    async fn get_state(&self, model: &str) -> Option<RateLimitState> {
        let states = self.states.lock().ok()?;
        states.get(model).cloned()
    }

    async fn peak_utilization(&self) -> f64 {
        let states = match self.states.lock() {
            Ok(s) => s,
            Err(_) => return 0.0,
        };
        states
            .values()
            .map(|s| s.peak_utilization())
            .fold(0.0f64, f64::max)
    }

    async fn recommend_model(&self, candidates: &[String]) -> Option<String> {
        if candidates.is_empty() {
            return None;
        }
        let states = self.states.lock().ok()?;
        candidates
            .iter()
            .min_by(|a, b| {
                let util_a = states
                    .get(a.as_str())
                    .map(|s| s.peak_utilization())
                    .unwrap_or(0.0);
                let util_b = states
                    .get(b.as_str())
                    .map(|s| s.peak_utilization())
                    .unwrap_or(0.0);
                util_a.partial_cmp(&util_b).unwrap_or(std::cmp::Ordering::Equal)
            })
            .cloned()
    }
}

/// No-op rate limiter — never throttles, used when rate limiting is disabled.
pub struct NoopRateLimiter;

#[async_trait]
impl RateLimiterPort for NoopRateLimiter {
    async fn should_throttle(&self, _: &str, _: u32, _: u32) -> Option<Duration> {
        None
    }
    async fn record_usage(&self, _: &str, _: u32, _: u32) {}
    async fn record_rate_limit(&self, _: &str) {}
    async fn update_from_headers(&self, _: &str, _: &RateLimitHeaders) {}
    async fn get_state(&self, _: &str) -> Option<RateLimitState> { None }
    async fn peak_utilization(&self) -> f64 { 0.0 }
    async fn recommend_model(&self, _: &[String]) -> Option<String> { None }
}

impl Default for RateLimiterAdapter {
    fn default() -> Self {
        Self::new()
    }
}
