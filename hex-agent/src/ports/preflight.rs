use async_trait::async_trait;

/// Port for lightweight preflight checks using a cheap model (Haiku).
///
/// These run before the main reasoning model to:
/// 1. Verify API connectivity and quota (fail fast)
/// 2. Classify whether user input is a new topic (trigger compaction)
#[async_trait]
pub trait PreflightPort: Send + Sync {
    /// Verify API connectivity and quota before the first turn.
    /// Sends a minimal request to Haiku (~50 tokens) to confirm the key works.
    async fn check_quota(&self) -> Result<(), PreflightError>;

    /// Classify whether user input represents a new conversation topic.
    ///
    /// Uses Haiku to compare the recent context with the new input.
    /// Returns `true` if context should be compacted before processing.
    async fn is_new_topic(
        &self,
        recent_context: &str,
        new_input: &str,
    ) -> Result<bool, PreflightError>;
}

#[derive(Debug, thiserror::Error)]
pub enum PreflightError {
    #[error("API key invalid or expired")]
    AuthFailed,
    #[error("Account quota exhausted")]
    QuotaExhausted,
    #[error("Rate limited — retry after {retry_after_ms}ms")]
    RateLimited { retry_after_ms: u64 },
    #[error("API unreachable: {0}")]
    Unreachable(String),
    #[error("Classification failed: {0}")]
    ClassificationFailed(String),
}
