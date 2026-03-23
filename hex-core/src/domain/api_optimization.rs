use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Classification of a workload for routing to the appropriate API endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkloadClass {
    Interactive,
    Batch,
}

impl WorkloadClass {
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
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ThinkingConfig {
    pub enabled: bool,
    pub budget_tokens: u32,
}

impl ThinkingConfig {
    pub fn with_budget(budget: u32) -> Self {
        Self {
            enabled: budget > 0,
            budget_tokens: budget,
        }
    }
}

/// Options for a single API request.
#[derive(Debug, Clone, Default)]
pub struct ApiRequestOptions {
    pub enable_cache: bool,
    pub thinking: ThinkingConfig,
    pub workload: Option<WorkloadClass>,
}

/// Tracks cached vs uncached token consumption for cost analysis.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CacheMetrics {
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub uncached_input_tokens: u64,
    pub cached_requests: u32,
    pub uncached_requests: u32,
}

impl CacheMetrics {
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

    pub fn savings_ratio(&self) -> f64 {
        let total = self.cache_read_tokens + self.cache_write_tokens + self.uncached_input_tokens;
        if total == 0 {
            return 0.0;
        }
        self.cache_read_tokens as f64 / total as f64
    }
}

/// Per-model rate limit tracking.
#[derive(Debug, Clone)]
pub struct RateLimitState {
    pub model: String,
    pub rpm_used: u32,
    pub rpm_limit: u32,
    pub input_tpm_used: u64,
    pub input_tpm_limit: u64,
    pub output_tpm_used: u64,
    pub output_tpm_limit: u64,
    pub window_start: Instant,
    pub window_duration: Duration,
    pub consecutive_429s: u32,
    pub backoff_ms: u64,
}

impl RateLimitState {
    pub fn new(model: String) -> Self {
        Self {
            model,
            rpm_used: 0,
            rpm_limit: 1000,
            input_tpm_used: 0,
            input_tpm_limit: 400_000,
            output_tpm_used: 0,
            output_tpm_limit: 80_000,
            window_start: Instant::now(),
            window_duration: Duration::from_secs(60),
            consecutive_429s: 0,
            backoff_ms: 0,
        }
    }

    pub fn maybe_reset_window(&mut self) {
        if self.window_start.elapsed() >= self.window_duration {
            self.rpm_used = 0;
            self.input_tpm_used = 0;
            self.output_tpm_used = 0;
            self.window_start = Instant::now();
        }
    }

    pub fn record_usage(&mut self, input_tokens: u32, output_tokens: u32) {
        self.maybe_reset_window();
        self.rpm_used += 1;
        self.input_tpm_used += input_tokens as u64;
        self.output_tpm_used += output_tokens as u64;
        self.consecutive_429s = 0;
        self.backoff_ms = 0;
    }

    pub fn record_rate_limit(&mut self) {
        self.consecutive_429s += 1;
        self.backoff_ms = std::cmp::min(
            1000 * 2u64.pow(self.consecutive_429s.saturating_sub(1)),
            60_000,
        );
    }

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
            self.rpm_used = self.rpm_limit.saturating_sub(remaining);
        }
    }

    pub fn should_throttle(&self, estimated_input: u32, estimated_output: u32) -> Option<Duration> {
        if self.backoff_ms > 0 {
            return Some(Duration::from_millis(self.backoff_ms));
        }
        if self.rpm_used == 0 && self.input_tpm_used == 0 && self.output_tpm_used == 0 {
            return None;
        }
        let rpm_headroom = (self.rpm_limit as f64 * 0.9) as u32;
        if self.rpm_used >= rpm_headroom {
            let remaining = self.window_duration.saturating_sub(self.window_start.elapsed());
            if !remaining.is_zero() {
                return Some(remaining);
            }
        }
        let input_headroom = (self.input_tpm_limit as f64 * 0.85) as u64;
        if self.input_tpm_used + estimated_input as u64 > input_headroom {
            let remaining = self.window_duration.saturating_sub(self.window_start.elapsed());
            if !remaining.is_zero() {
                return Some(remaining);
            }
        }
        let output_headroom = (self.output_tpm_limit as f64 * 0.85) as u64;
        if self.output_tpm_used + estimated_output as u64 > output_headroom {
            let remaining = self.window_duration.saturating_sub(self.window_start.elapsed());
            if !remaining.is_zero() {
                return Some(remaining);
            }
        }
        None
    }

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

/// Rate limit headers parsed from an API response.
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

/// Batch request status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BatchStatus {
    InProgress,
    Ended,
    Cancelled,
    Expired,
}

/// A batch request submitted to a Batch API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchRequest {
    pub batch_id: String,
    pub request_count: u32,
    pub status: BatchStatus,
    pub custom_ids: Vec<String>,
}

/// Aggregated metrics for the token budget dashboard.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApiMetricsSnapshot {
    pub cache: CacheMetrics,
    pub rate_limits: HashMap<String, f64>,
    pub realtime_requests: u32,
    pub batch_requests: u32,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_cache_read_tokens: u64,
    pub estimated_savings_pct: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workload_classification() {
        assert_eq!(WorkloadClass::classify("code_analysis"), WorkloadClass::Batch);
        assert_eq!(
            WorkloadClass::classify("conversation"),
            WorkloadClass::Interactive
        );
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
        m.record(800, 200, 0, true);
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
        state.record_usage(100, 50);
        assert_eq!(state.consecutive_429s, 0);
        assert_eq!(state.backoff_ms, 0);
    }
}
