use crate::ports::{ApiMetricsSnapshot, CacheMetrics};
use crate::ports::token_metrics::TokenMetricsPort;
use crate::domain::pricing::{default_pricing, calculate_cost, ModelPricing};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Mutex;

/// In-memory token metrics tracker for the dashboard.
///
/// Aggregates cache hits, batch vs real-time routing, per-model consumption, and USD costs.
pub struct TokenMetricsAdapter {
    inner: Mutex<MetricsState>,
    pricing: HashMap<String, ModelPricing>,
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
    total_cost_usd: f64,
    per_task_cost: HashMap<String, f64>,
}

impl TokenMetricsAdapter {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(MetricsState::default()),
            pricing: default_pricing(),
        }
    }

    pub fn calculate_request_cost(&self, model: &str, input_tokens: u32, output_tokens: u32) -> f64 {
        if let Some(model_pricing) = self.pricing.get(model) {
            calculate_cost(input_tokens, output_tokens, model_pricing)
        } else if let Some(model_pricing) = self.pricing.get(&model.to_lowercase()) {
            calculate_cost(input_tokens, output_tokens, model_pricing)
        } else {
            0.0
        }
    }

    pub fn record_task_cost(&self, task_id: &str, cost: f64) {
        if let Ok(mut state) = self.inner.lock() {
            state.total_cost_usd += cost;
            *state.per_task_cost.entry(task_id.to_string()).or_default() += cost;
        }
    }

    pub fn get_task_cost(&self, task_id: &str) -> f64 {
        if let Ok(state) = self.inner.lock() {
            state.per_task_cost.get(task_id).copied().unwrap_or(0.0)
        } else {
            0.0
        }
    }

    pub fn total_cost_usd(&self) -> f64 {
        if let Ok(state) = self.inner.lock() {
            state.total_cost_usd
        } else {
            0.0
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
        let cost = self.calculate_request_cost(model, input_tokens, output_tokens);
        if let Ok(mut state) = self.inner.lock() {
            state.realtime_requests += 1;
            state.total_input_tokens += input_tokens as u64;
            state.total_output_tokens += output_tokens as u64;
            state.total_cache_read_tokens += cache_read as u64;
            state.total_cost_usd += cost;
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
        let cost = self.calculate_request_cost(model, input_tokens, output_tokens);
        if let Ok(mut state) = self.inner.lock() {
            state.batch_requests += 1;
            state.total_input_tokens += input_tokens as u64;
            state.total_output_tokens += output_tokens as u64;
            state.total_cost_usd += cost;

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
