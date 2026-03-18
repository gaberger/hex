use crate::domain::{RateLimitHeaders, RateLimitState};
use async_trait::async_trait;
use std::time::Duration;

/// Port for tracking and enforcing per-model rate limits.
///
/// The rate limiter sits between the conversation loop and the Anthropic adapter,
/// providing proactive throttling to avoid 429s rather than reacting to them.
#[async_trait]
pub trait RateLimiterPort: Send + Sync {
    /// Check whether a request should be throttled.
    ///
    /// Returns `Some(delay)` if we should wait before sending,
    /// `None` if we can proceed immediately.
    async fn should_throttle(
        &self,
        model: &str,
        estimated_input_tokens: u32,
        estimated_output_tokens: u32,
    ) -> Option<Duration>;

    /// Record a successful request's token consumption.
    async fn record_usage(&self, model: &str, input_tokens: u32, output_tokens: u32);

    /// Record a 429 rate limit response for a model.
    async fn record_rate_limit(&self, model: &str);

    /// Update rate limits from API response headers.
    async fn update_from_headers(&self, model: &str, headers: &RateLimitHeaders);

    /// Get current rate limit state for a model (for dashboard exposure).
    async fn get_state(&self, model: &str) -> Option<RateLimitState>;

    /// Get peak utilization across all tracked models (0.0-1.0).
    async fn peak_utilization(&self) -> f64;

    /// Recommend the best model to use right now based on rate limit headroom.
    /// Returns the model with the most remaining capacity.
    async fn recommend_model(&self, candidates: &[String]) -> Option<String>;
}
