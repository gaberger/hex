use crate::domain::{ApiMetricsSnapshot, CacheMetrics};
use crate::ports::token_metrics::TokenMetricsPort;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Mutex;

/// In-memory token metrics tracker for the dashboard.
///
/// Aggregates cache hits, batch vs real-time routing, and per-model consumption.
pub struct TokenMetricsAdapter {
    inner: Mutex<MetricsState>,
}

#[derive(Default)]
struct MetricsState {
    cache: CacheMetrics,
    realtime_requests: u32,
    batch_requests: u32,
    total_input_tokens: u64,
    total_output_tokens: u64,
    total_cache_read_tokens: u64,
    per_model_input: HashMap<String, u64>,
    per_model_output: HashMap<String, u64>,
}

impl TokenMetricsAdapter {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(MetricsState::default()),
        }
    }
}

impl Default for TokenMetricsAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl TokenMetricsPort for TokenMetricsAdapter {
    async fn record_realtime(
        &self,
        model: &str,
        input_tokens: u32,
        output_tokens: u32,
        cache_read: u32,
        cache_write: u32,
    ) {
        if let Ok(mut state) = self.inner.lock() {
            state.realtime_requests += 1;
            state.total_input_tokens += input_tokens as u64;
            state.total_output_tokens += output_tokens as u64;
            state.total_cache_read_tokens += cache_read as u64;
            state.cache.record(cache_read, cache_write, input_tokens.saturating_sub(cache_read), cache_read > 0);

            *state.per_model_input.entry(model.to_string()).or_default() += input_tokens as u64;
            *state.per_model_output.entry(model.to_string()).or_default() += output_tokens as u64;
        }
    }

    async fn record_batch(
        &self,
        model: &str,
        input_tokens: u32,
        output_tokens: u32,
    ) {
        if let Ok(mut state) = self.inner.lock() {
            state.batch_requests += 1;
            state.total_input_tokens += input_tokens as u64;
            state.total_output_tokens += output_tokens as u64;

            *state.per_model_input.entry(model.to_string()).or_default() += input_tokens as u64;
            *state.per_model_output.entry(model.to_string()).or_default() += output_tokens as u64;
        }
    }

    async fn snapshot(&self) -> ApiMetricsSnapshot {
        let state = match self.inner.lock() {
            Ok(s) => s,
            Err(_) => return ApiMetricsSnapshot::default(),
        };

        // Estimated savings: cache savings + batch discount (50%)
        let cache_savings = state.cache.savings_ratio();
        let batch_discount = if state.realtime_requests + state.batch_requests > 0 {
            (state.batch_requests as f64 / (state.realtime_requests + state.batch_requests) as f64) * 0.5
        } else {
            0.0
        };

        ApiMetricsSnapshot {
            cache: state.cache.clone(),
            rate_limits: HashMap::new(), // Filled by the caller from RateLimiterPort
            realtime_requests: state.realtime_requests,
            batch_requests: state.batch_requests,
            total_input_tokens: state.total_input_tokens,
            total_output_tokens: state.total_output_tokens,
            total_cache_read_tokens: state.total_cache_read_tokens,
            estimated_savings_pct: cache_savings * 0.7 + batch_discount * 0.3, // weighted
        }
    }

    async fn reset(&self) {
        if let Ok(mut state) = self.inner.lock() {
            *state = MetricsState::default();
        }
    }
}
