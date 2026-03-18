use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Classification of a workload for routing to the appropriate API endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkloadClass {
    /// Real-time interactive conversation — uses Messages API
    Interactive,
    /// Non-interactive bulk work — eligible for Batch API (50% cost reduction)
    Batch,
}

impl WorkloadClass {
    /// Classify a task type string into a workload class.
    ///
    /// Batch-eligible tasks: code analysis, bulk summarization, test generation,
    /// documentation generation, dead-code analysis — anything that doesn't need
    /// sub-second latency.
    pub fn classify(task_type: &str) -> Self {
        match task_type {
            "code_analysis" | "summarization" | "test_generation" | "dead_code"
            | "doc_generation" | "batch_summarize" | "bulk_validate" | "ast_summary" => {
                Self::Batch
            }
            _ => Self::Interactive,
        }
    }

    pub fn is_batch(&self) -> bool {
        matches!(self, Self::Batch)
    }
}

/// Extended thinking configuration for Opus/Sonnet models.
///
/// Controls the `thinking.budget_tokens` parameter to prevent output TPM exhaustion
/// when using extended thinking. Without a cap, a single Opus thinking turn can
/// consume 30k+ output tokens, hitting the 80k TPM limit in 2-3 requests.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThinkingConfig {
    /// Whether extended thinking is enabled for this request.
    pub enabled: bool,
    /// Max tokens the model may use for internal reasoning (thinking block).
    /// Must be < max_tokens. Set to 0 to let the model decide.
    pub budget_tokens: u32,
}

impl Default for ThinkingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            budget_tokens: 0,
        }
    }
}

impl ThinkingConfig {
    pub fn with_budget(budget: u32) -> Self {
        Self {
            enabled: budget > 0,
            budget_tokens: budget,
        }
    }
}

/// Options for a single API request — extends the base parameters with
/// caching, thinking, and workload classification.
#[derive(Debug, Clone, Default)]
pub struct ApiRequestOptions {
    /// Enable prompt caching via `cache_control` on system/tool blocks.
    /// Cached reads are free and bypass input TPM limits.
    pub enable_cache: bool,
    /// Extended thinking configuration.
    pub thinking: ThinkingConfig,
    /// Workload classification for routing.
    pub workload: Option<WorkloadClass>,
}

/// Tracks cached vs uncached token consumption for cost analysis.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CacheMetrics {
    /// Tokens served from prompt cache (free, bypasses input TPM)
    pub cache_read_tokens: u64,
    /// Tokens written to prompt cache (charged at 1.25x on first request)
    pub cache_write_tokens: u64,
    /// Tokens that were not cached (standard pricing)
    pub uncached_input_tokens: u64,
    /// Total API calls with caching enabled
    pub cached_requests: u32,
    /// Total API calls without caching
    pub uncached_requests: u32,
}

impl CacheMetrics {
    /// Record a single API response's cache breakdown.
    pub fn record(&mut self, cache_read: u32, cache_write: u32, uncached: u32, was_cached: bool) {
        self.cache_read_tokens += cache_read as u64;
        self.cache_write_tokens += cache_write as u64;
        self.uncached_input_tokens += uncached as u64;
        if was_cached {
            self.cached_requests += 1;
        } else {
            self.uncached_requests += 1;
        }
    }

    /// Estimated cost savings ratio (0.0 = no savings, 1.0 = all cached).
    pub fn savings_ratio(&self) -> f64 {
        let total = self.cache_read_tokens + self.cache_write_tokens + self.uncached_input_tokens;
        if total == 0 {
            return 0.0;
        }
        self.cache_read_tokens as f64 / total as f64
    }
}

/// Per-model rate limit tracking.
///
/// The Anthropic API enforces three independent limits per model:
/// - RPM (requests per minute)
/// - Input TPM (input tokens per minute)
/// - Output TPM (output tokens per minute)
///
/// We track consumed capacity and throttle proactively to avoid 429s.
#[derive(Debug, Clone)]
pub struct RateLimitState {
    /// Model ID this state tracks (e.g. "claude-sonnet-4-6")
    pub model: String,
    /// Requests made in the current window
    pub rpm_used: u32,
    /// RPM limit from last response headers
    pub rpm_limit: u32,
    /// Input tokens consumed in the current window
    pub input_tpm_used: u64,
    /// Input TPM limit from last response headers
    pub input_tpm_limit: u64,
    /// Output tokens consumed in the current window
    pub output_tpm_used: u64,
    /// Output TPM limit from last response headers
    pub output_tpm_limit: u64,
    /// When the current rate window started
    pub window_start: Instant,
    /// Window duration (typically 60 seconds)
    pub window_duration: Duration,
    /// Number of consecutive 429 responses
    pub consecutive_429s: u32,
    /// Current backoff delay for exponential backoff
    pub backoff_ms: u64,
}

impl RateLimitState {
    pub fn new(model: String) -> Self {
        Self {
            model,
            rpm_used: 0,
            rpm_limit: 50,       // Conservative default
            input_tpm_used: 0,
            input_tpm_limit: 40_000,  // Conservative default for Sonnet
            output_tpm_used: 0,
            output_tpm_limit: 8_000,   // Conservative default
            window_start: Instant::now(),
            window_duration: Duration::from_secs(60),
            consecutive_429s: 0,
            backoff_ms: 0,
        }
    }

    /// Reset the window if it has expired.
    pub fn maybe_reset_window(&mut self) {
        if self.window_start.elapsed() >= self.window_duration {
            self.rpm_used = 0;
            self.input_tpm_used = 0;
            self.output_tpm_used = 0;
            self.window_start = Instant::now();
        }
    }

    /// Record a successful request's token consumption.
    pub fn record_usage(&mut self, input_tokens: u32, output_tokens: u32) {
        self.maybe_reset_window();
        self.rpm_used += 1;
        self.input_tpm_used += input_tokens as u64;
        self.output_tpm_used += output_tokens as u64;
        self.consecutive_429s = 0;
        self.backoff_ms = 0;
    }

    /// Record a 429 rate limit response and compute next backoff.
    pub fn record_rate_limit(&mut self) {
        self.consecutive_429s += 1;
        // Exponential backoff: 1s, 2s, 4s, 8s, 16s, max 60s
        self.backoff_ms = std::cmp::min(
            1000 * 2u64.pow(self.consecutive_429s.saturating_sub(1)),
            60_000,
        );
    }

    /// Update limits from API response headers.
    pub fn update_limits_from_headers(&mut self, headers: &RateLimitHeaders) {
        if let Some(rpm) = headers.rpm_limit {
            self.rpm_limit = rpm;
        }
        if let Some(tpm) = headers.input_tpm_limit {
            self.input_tpm_limit = tpm;
        }
        if let Some(tpm) = headers.output_tpm_limit {
            self.output_tpm_limit = tpm;
        }
        if let Some(remaining) = headers.rpm_remaining {
            // Reconcile: if the server says we have more remaining, trust it
            self.rpm_used = self.rpm_limit.saturating_sub(remaining);
        }
    }

    /// Whether we should throttle before sending the next request.
    ///
    /// Returns `Some(delay)` if we should wait, `None` if we can proceed.
    pub fn should_throttle(&self, estimated_input: u32, estimated_output: u32) -> Option<Duration> {
        // In backoff from a recent 429
        if self.backoff_ms > 0 {
            return Some(Duration::from_millis(self.backoff_ms));
        }

        // RPM check — leave 10% headroom
        let rpm_headroom = (self.rpm_limit as f64 * 0.9) as u32;
        if self.rpm_used >= rpm_headroom {
            let remaining = self.window_duration.saturating_sub(self.window_start.elapsed());
            if !remaining.is_zero() {
                return Some(remaining);
            }
        }

        // Input TPM check — leave 15% headroom (most common bottleneck)
        let input_headroom = (self.input_tpm_limit as f64 * 0.85) as u64;
        if self.input_tpm_used + estimated_input as u64 > input_headroom {
            let remaining = self.window_duration.saturating_sub(self.window_start.elapsed());
            if !remaining.is_zero() {
                return Some(remaining);
            }
        }

        // Output TPM check — critical for extended thinking
        let output_headroom = (self.output_tpm_limit as f64 * 0.85) as u64;
        if self.output_tpm_used + estimated_output as u64 > output_headroom {
            let remaining = self.window_duration.saturating_sub(self.window_start.elapsed());
            if !remaining.is_zero() {
                return Some(remaining);
            }
        }

        None
    }

    /// Utilization percentage for the most constrained resource.
    pub fn peak_utilization(&self) -> f64 {
        let rpm_pct = if self.rpm_limit > 0 {
            self.rpm_used as f64 / self.rpm_limit as f64
        } else {
            0.0
        };
        let input_pct = if self.input_tpm_limit > 0 {
            self.input_tpm_used as f64 / self.input_tpm_limit as f64
        } else {
            0.0
        };
        let output_pct = if self.output_tpm_limit > 0 {
            self.output_tpm_used as f64 / self.output_tpm_limit as f64
        } else {
            0.0
        };
        rpm_pct.max(input_pct).max(output_pct)
    }
}

/// Rate limit headers parsed from an Anthropic API response.
#[derive(Debug, Clone, Default)]
pub struct RateLimitHeaders {
    pub rpm_limit: Option<u32>,
    pub rpm_remaining: Option<u32>,
    pub input_tpm_limit: Option<u64>,
    pub input_tpm_remaining: Option<u64>,
    pub output_tpm_limit: Option<u64>,
    pub output_tpm_remaining: Option<u64>,
    pub retry_after_ms: Option<u64>,
}

/// Batch request status — mirrors the Anthropic Batch API lifecycle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BatchStatus {
    /// Request is queued for processing
    InProgress,
    /// All requests have completed
    Ended,
    /// Batch was cancelled
    Cancelled,
    /// Batch expired before completion
    Expired,
}

/// A batch request submitted to the Anthropic Batch API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchRequest {
    /// Unique batch ID returned by the API
    pub batch_id: String,
    /// Number of requests in the batch
    pub request_count: u32,
    /// Current status
    pub status: BatchStatus,
    /// Custom IDs for correlating results back to original requests
    pub custom_ids: Vec<String>,
}

/// Aggregated metrics for the token budget dashboard.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApiMetricsSnapshot {
    /// Cache performance metrics
    pub cache: CacheMetrics,
    /// Per-model rate limit states (model_id → utilization percentage)
    pub rate_limits: HashMap<String, f64>,
    /// Requests routed to real-time vs batch
    pub realtime_requests: u32,
    pub batch_requests: u32,
    /// Total tokens by category
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_cache_read_tokens: u64,
    /// Estimated cost savings from caching + batching
    pub estimated_savings_pct: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workload_classification() {
        assert_eq!(WorkloadClass::classify("code_analysis"), WorkloadClass::Batch);
        assert_eq!(WorkloadClass::classify("summarization"), WorkloadClass::Batch);
        assert_eq!(WorkloadClass::classify("conversation"), WorkloadClass::Interactive);
        assert_eq!(WorkloadClass::classify("unknown"), WorkloadClass::Interactive);
    }

    #[test]
    fn thinking_config_defaults() {
        let cfg = ThinkingConfig::default();
        assert!(!cfg.enabled);
        assert_eq!(cfg.budget_tokens, 0);
    }

    #[test]
    fn thinking_config_with_budget() {
        let cfg = ThinkingConfig::with_budget(10000);
        assert!(cfg.enabled);
        assert_eq!(cfg.budget_tokens, 10000);
    }

    #[test]
    fn cache_metrics_savings_ratio() {
        let mut m = CacheMetrics::default();
        assert_eq!(m.savings_ratio(), 0.0);

        m.record(800, 200, 0, true);
        // 800 cached out of 1000 total = 80%
        assert!((m.savings_ratio() - 0.8).abs() < 0.01);
    }

    #[test]
    fn rate_limit_backoff_exponential() {
        let mut state = RateLimitState::new("test".into());
        state.record_rate_limit();
        assert_eq!(state.backoff_ms, 1000);
        state.record_rate_limit();
        assert_eq!(state.backoff_ms, 2000);
        state.record_rate_limit();
        assert_eq!(state.backoff_ms, 4000);
    }

    #[test]
    fn rate_limit_reset_on_success() {
        let mut state = RateLimitState::new("test".into());
        state.record_rate_limit();
        state.record_rate_limit();
        assert_eq!(state.consecutive_429s, 2);

        state.record_usage(100, 50);
        assert_eq!(state.consecutive_429s, 0);
        assert_eq!(state.backoff_ms, 0);
    }

    #[test]
    fn rate_limit_peak_utilization() {
        let mut state = RateLimitState::new("test".into());
        state.rpm_limit = 100;
        state.rpm_used = 80;
        state.input_tpm_limit = 40_000;
        state.input_tpm_used = 10_000;
        state.output_tpm_limit = 8_000;
        state.output_tpm_used = 2_000;
        // RPM is most constrained at 80%
        assert!((state.peak_utilization() - 0.8).abs() < 0.01);
    }
}
