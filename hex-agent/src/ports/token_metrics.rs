use crate::domain::ApiMetricsSnapshot;
use async_trait::async_trait;

/// Port for exposing token consumption metrics to the hex dashboard.
///
/// Tracks cached vs uncached input, output, batch vs real-time,
/// and rate limit utilization across all models.
#[async_trait]
pub trait TokenMetricsPort: Send + Sync {
    /// Record tokens from a real-time API call.
    async fn record_realtime(
        &self,
        model: &str,
        input_tokens: u32,
        output_tokens: u32,
        cache_read: u32,
        cache_write: u32,
    );

    /// Record tokens from a batch API call.
    async fn record_batch(
        &self,
        model: &str,
        input_tokens: u32,
        output_tokens: u32,
    );

    /// Get the current metrics snapshot for the dashboard.
    async fn snapshot(&self) -> ApiMetricsSnapshot;

    /// Reset all metrics (e.g., at session start).
    async fn reset(&self);
}
