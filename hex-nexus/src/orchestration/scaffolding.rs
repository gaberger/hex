//! Scaffolded dispatch — Best-of-N with compile gate + error-feedback retry
//! (ADR-2604120202 Phase 2, tasks P3.1 and P3.2).
//!
//! Wraps an `IInferencePort` to generate multiple completions, validate each
//! against a compile gate, and retry with error feedback on failure. The
//! scaffolding is transparent to callers — they see the same `complete()`
//! interface but get dramatically higher first-pass success rates on local
//! models.

use std::sync::Arc;

use async_trait::async_trait;
use hex_core::ports::inference::{InferenceError, InferenceRequest, InferenceResponse};

use crate::remote::transport::TaskTier;

/// Tracks escalation events for observability and tier-reclassification hints.
/// Writes are fire-and-forget — a failed write must never block dispatch.
#[async_trait]
pub trait EscalationTracker: Send + Sync {
    /// Record that a local dispatch succeeded without escalation.
    async fn record_local_success(&self, task_type: &str, model: &str);
    /// Record that a dispatch escalated from local to frontier.
    async fn record_escalation(&self, task_type: &str, model: &str, sample_error: &str);
}

/// Escalation statistics for a single task-type + model combination.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EscalationStats {
    pub task_type: String,
    pub model: String,
    pub local_successes: u64,
    pub escalations: u64,
    pub last_sample_error: Option<String>,
}

impl EscalationStats {
    /// Escalation rate as a fraction in [0.0, 1.0].
    pub fn escalation_rate(&self) -> f64 {
        let total = self.local_successes + self.escalations;
        if total == 0 {
            return 0.0;
        }
        self.escalations as f64 / total as f64
    }
}

/// Result of a scaffolded dispatch attempt.
#[derive(Debug)]
pub enum ScaffoldResult {
    /// Code compiled successfully.
    Success {
        response: InferenceResponse,
        attempt: usize,
        total_attempts: usize,
    },
    /// All attempts + retries failed the compile gate.
    CompileGateFailed {
        total_attempts: usize,
        best_error: String,
        /// Remediation hint when no frontier is available.
        remediation: Option<String>,
    },
}

/// Error from the compile checker.
#[derive(Debug, Clone)]
pub struct CompileError {
    pub stderr: String,
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.stderr)
    }
}

/// Trait for compile-checking generated code. Enables mocked tests without
/// shelling out to `cargo check` / `tsc` / `go build`.
#[async_trait]
pub trait CompileChecker: Send + Sync {
    /// Check if the code compiles. Returns `Ok(())` on success, or
    /// `Err(CompileError)` with stderr on failure.
    async fn check(&self, code: &str) -> Result<(), CompileError>;
}

/// Shell-based compile checker that runs an external command.
pub struct ShellCompileChecker {
    /// Command to run (e.g. "cargo check", "tsc --noEmit", "go build").
    pub command: String,
}

#[async_trait]
impl CompileChecker for ShellCompileChecker {
    async fn check(&self, code: &str) -> Result<(), CompileError> {
        let id = uuid::Uuid::new_v4();
        let tmp_path = std::env::temp_dir().join(format!("hex-scaffold-{id}.rs"));
        std::fs::write(&tmp_path, code).map_err(|e| CompileError {
            stderr: format!("failed to write temp file: {e}"),
        })?;

        let parts: Vec<&str> = self.command.split_whitespace().collect();
        let (cmd, args) = parts.split_first().unwrap_or((&"cargo", &[]));

        let output = tokio::process::Command::new(cmd)
            .args(args)
            .arg(&tmp_path)
            .output()
            .await
            .map_err(|e| CompileError {
                stderr: format!("failed to run compile command: {e}"),
            })?;

        let _ = std::fs::remove_file(&tmp_path);

        if output.status.success() {
            Ok(())
        } else {
            Err(CompileError {
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            })
        }
    }
}

/// Per-tier scaffolding configuration.
#[derive(Debug, Clone)]
pub struct ScaffoldConfig {
    /// Number of completions to generate per tier (Best-of-N).
    /// T1: 1, T2: 3, T2.5: 5, T3: 1 (frontier, no scaffolding needed).
    pub n_for_tier: fn(TaskTier) -> usize,
    /// Maximum error-feedback retries after all N completions fail.
    pub max_retries: usize,
}

impl Default for ScaffoldConfig {
    fn default() -> Self {
        Self {
            n_for_tier: |tier| match tier {
                TaskTier::T1 => 1,
                TaskTier::T2 => 3,
                TaskTier::T2_5 => 5,
                TaskTier::T3 => 1,
            },
            max_retries: 2,
        }
    }
}

/// No-op tracker for tests and environments without HexFlo memory.
pub struct NoopEscalationTracker;

#[async_trait]
impl EscalationTracker for NoopEscalationTracker {
    async fn record_local_success(&self, _task_type: &str, _model: &str) {}
    async fn record_escalation(&self, _task_type: &str, _model: &str, _sample_error: &str) {}
}

/// Tracker that writes escalation/success counts to HexFlo memory via the
/// state adapter. Each event does a read-increment-write cycle. Writes are
/// fire-and-forget — a failed write logs a warning but never blocks dispatch.
pub struct HexFloEscalationTracker {
    state: Arc<dyn crate::ports::state::IHexFloMemoryStatePort>,
}

impl HexFloEscalationTracker {
    pub fn new(state: Arc<dyn crate::ports::state::IHexFloMemoryStatePort>) -> Self {
        Self { state }
    }
}

#[async_trait]
impl EscalationTracker for HexFloEscalationTracker {
    async fn record_local_success(&self, task_type: &str, model: &str) {
        let key = format!("success:{}:{}", task_type, model);
        let count = match self.state.hexflo_memory_retrieve(&key).await {
            Ok(Some(val)) => parse_count(&val).saturating_add(1),
            _ => 1,
        };
        let value = serde_json::json!({
            "count": count,
            "last_seen": chrono::Utc::now().to_rfc3339(),
        })
        .to_string();
        if let Err(e) = self.state.hexflo_memory_store(&key, &value, "global").await {
            tracing::warn!(key, %e, "failed to write success tracking to HexFlo memory");
        }
    }

    async fn record_escalation(&self, task_type: &str, model: &str, sample_error: &str) {
        let key = format!("escalation:{}:{}", task_type, model);
        let count = match self.state.hexflo_memory_retrieve(&key).await {
            Ok(Some(val)) => parse_count(&val).saturating_add(1),
            _ => 1,
        };
        let value = serde_json::json!({
            "count": count,
            "last_escalated": chrono::Utc::now().to_rfc3339(),
            "sample_error": sample_error.chars().take(200).collect::<String>(),
        })
        .to_string();
        if let Err(e) = self.state.hexflo_memory_store(&key, &value, "global").await {
            tracing::warn!(key, %e, "failed to write escalation tracking to HexFlo memory");
        }
    }
}

/// Parse the "count" field from a JSON value string, defaulting to 0.
fn parse_count(val: &str) -> u64 {
    serde_json::from_str::<serde_json::Value>(val)
        .ok()
        .and_then(|v| v.get("count")?.as_u64())
        .unwrap_or(0)
}

/// Scaffolded inference dispatcher. Wraps an inference port with Best-of-N
/// generation, compile-gate validation, and error-feedback retries.
pub struct ScaffoldedDispatch {
    inference: Arc<dyn hex_core::ports::inference::IInferencePort>,
    compile_checker: Box<dyn CompileChecker>,
    config: ScaffoldConfig,
    /// Optional frontier adapter for cascading escalation (P4).
    frontier: Option<Arc<dyn hex_core::ports::inference::IInferencePort>>,
    /// Optional escalation tracker for observability (P4.2).
    tracker: Option<Arc<dyn EscalationTracker>>,
}

impl ScaffoldedDispatch {
    pub fn new(
        inference: Arc<dyn hex_core::ports::inference::IInferencePort>,
        compile_checker: Box<dyn CompileChecker>,
    ) -> Self {
        Self {
            inference,
            compile_checker,
            config: ScaffoldConfig::default(),
            frontier: None,
            tracker: None,
        }
    }

    pub fn with_config(mut self, config: ScaffoldConfig) -> Self {
        self.config = config;
        self
    }

    pub fn with_frontier(
        mut self,
        frontier: Arc<dyn hex_core::ports::inference::IInferencePort>,
    ) -> Self {
        self.frontier = Some(frontier);
        self
    }

    pub fn with_tracker(mut self, tracker: Arc<dyn EscalationTracker>) -> Self {
        self.tracker = Some(tracker);
        self
    }

    /// Dispatch with scaffolding: Best-of-N + compile gate + retry loop.
    pub async fn dispatch(
        &self,
        request: &InferenceRequest,
        tier: TaskTier,
    ) -> Result<ScaffoldResult, InferenceError> {
        let n = (self.config.n_for_tier)(tier);
        let mut best_error = String::new();
        let mut total_attempts = 0;
        let mut current_request = request.clone();

        for retry in 0..=self.config.max_retries {
            // Generate N completions
            for i in 0..n {
                total_attempts += 1;
                let response = self.inference.complete(current_request.clone()).await?;

                // Extract code from response
                let code = extract_code_from_response(&response);

                match self.compile_checker.check(&code).await {
                    Ok(()) => {
                        tracing::info!(
                            attempt = total_attempts,
                            retry,
                            tier = tier.as_str(),
                            "Scaffolded dispatch: compile gate passed"
                        );
                        // P4.2: track local success
                        if let Some(ref tracker) = self.tracker {
                            tracker
                                .record_local_success(tier.as_str(), &request.model)
                                .await;
                        }
                        return Ok(ScaffoldResult::Success {
                            response,
                            attempt: total_attempts,
                            total_attempts,
                        });
                    }
                    Err(e) => {
                        tracing::debug!(
                            attempt = i + 1,
                            retry,
                            stderr_len = e.stderr.len(),
                            "Compile gate failed"
                        );
                        // Keep the shortest error as "best" (most actionable)
                        if best_error.is_empty() || e.stderr.len() < best_error.len() {
                            best_error = e.stderr;
                        }
                    }
                }
            }

            // All N failed — augment prompt with best error for retry
            if retry < self.config.max_retries {
                tracing::info!(
                    retry = retry + 1,
                    max = self.config.max_retries,
                    "All {} completions failed, retrying with error feedback",
                    n
                );
                current_request = augment_with_error(request, &best_error);
            }
        }

        // All attempts + retries exhausted
        tracing::warn!(
            total_attempts,
            tier = tier.as_str(),
            "Scaffolded dispatch: all attempts failed compile gate"
        );

        // Cascading escalation (P4.1): try frontier if available
        if let Some(ref frontier) = self.frontier {
            tracing::warn!(
                tier = tier.as_str(),
                total_attempts,
                best_error_len = best_error.len(),
                "Escalating to frontier adapter after local compile-gate exhaustion"
            );
            // P4.2: track escalation event
            if let Some(ref tracker) = self.tracker {
                tracker
                    .record_escalation(tier.as_str(), &request.model, &best_error)
                    .await;
            }
            // Use the original prompt — not the retry-augmented one — so the
            // frontier model gets a clean context without prior error feedback.
            let response = frontier.complete(request.clone()).await?;
            return Ok(ScaffoldResult::Success {
                response,
                attempt: total_attempts + 1,
                total_attempts: total_attempts + 1,
            });
        }

        Ok(ScaffoldResult::CompileGateFailed {
            total_attempts,
            best_error,
            remediation: Some(
                "No frontier adapter configured. Add a frontier provider with \
                 `hex inference add --tier frontier` or set inference.frontier \
                 in .hex/project.json to enable automatic escalation."
                    .into(),
            ),
        })
    }
}

/// Extract code text from an inference response.
/// Looks for code fence blocks first, then falls back to raw text.
fn extract_code_from_response(response: &InferenceResponse) -> String {
    let text: String = response
        .content
        .iter()
        .filter_map(|block| {
            if let hex_core::domain::messages::ContentBlock::Text { text } = block {
                Some(text.as_str())
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("");

    // Try to extract from ```...``` fences
    if let Some(start) = text.find("```") {
        let after_fence = &text[start + 3..];
        // Skip the language tag (e.g. "rust\n")
        if let Some(newline) = after_fence.find('\n') {
            let code_start = &after_fence[newline + 1..];
            if let Some(end) = code_start.find("```") {
                return code_start[..end].to_string();
            }
        }
    }

    // No fences — return raw text
    text
}

/// Augment an inference request with compiler error feedback for retry.
fn augment_with_error(original: &InferenceRequest, error: &str) -> InferenceRequest {
    let mut req = original.clone();
    // Append error feedback to the last user message
    let error_suffix = format!(
        "\n\nThe previous attempt produced this compiler error:\n```\n{}\n```\nFix the code and return ONLY the corrected version.",
        error.chars().take(500).collect::<String>() // cap error length
    );

    if let Some(last_msg) = req.messages.last_mut() {
        for block in &mut last_msg.content {
            if let hex_core::domain::messages::ContentBlock::Text { text } = block {
                text.push_str(&error_suffix);
                return req;
            }
        }
    }
    req
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_core::domain::messages::{ContentBlock, Message};
    use hex_core::ports::inference::mock::MockInferencePort;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Mock compile checker that accepts/rejects based on content.
    struct MockCompileChecker {
        accept_fn: Box<dyn Fn(&str) -> bool + Send + Sync>,
    }

    #[async_trait]
    impl CompileChecker for MockCompileChecker {
        async fn check(&self, code: &str) -> Result<(), CompileError> {
            if (self.accept_fn)(code) {
                Ok(())
            } else {
                Err(CompileError {
                    stderr: "mock compile error: code rejected".into(),
                })
            }
        }
    }

    /// Compile checker that returns different-length errors, so we can verify
    /// that the "best error" (shortest stderr) is selected.
    struct VariableErrorChecker {
        call_count: AtomicUsize,
        errors: Vec<String>,
    }

    #[async_trait]
    impl CompileChecker for VariableErrorChecker {
        async fn check(&self, _code: &str) -> Result<(), CompileError> {
            let idx = self.call_count.fetch_add(1, Ordering::SeqCst);
            let err = self
                .errors
                .get(idx % self.errors.len())
                .cloned()
                .unwrap_or_else(|| "fallback error".into());
            Err(CompileError { stderr: err })
        }
    }

    /// Mock compile checker that rejects the first `reject_count` checks then
    /// accepts all subsequent ones. Simulates error-feedback retry success.
    struct EventuallyPassingChecker {
        call_count: AtomicUsize,
        reject_count: usize,
    }

    #[async_trait]
    impl CompileChecker for EventuallyPassingChecker {
        async fn check(&self, _code: &str) -> Result<(), CompileError> {
            let n = self.call_count.fetch_add(1, Ordering::SeqCst);
            if n < self.reject_count {
                Err(CompileError {
                    stderr: format!("error on attempt {}", n + 1),
                })
            } else {
                Ok(())
            }
        }
    }

    /// Mock inference port that captures all requests it receives.
    struct CapturingInferencePort {
        response_text: String,
        captured: std::sync::Mutex<Vec<InferenceRequest>>,
    }

    impl CapturingInferencePort {
        fn new(response_text: &str) -> Self {
            Self {
                response_text: response_text.to_string(),
                captured: std::sync::Mutex::new(Vec::new()),
            }
        }

        fn captured_requests(&self) -> Vec<InferenceRequest> {
            self.captured.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl hex_core::ports::inference::IInferencePort for CapturingInferencePort {
        async fn complete(
            &self,
            request: InferenceRequest,
        ) -> Result<InferenceResponse, InferenceError> {
            self.captured.lock().unwrap().push(request);
            Ok(InferenceResponse {
                content: vec![ContentBlock::Text {
                    text: self.response_text.clone(),
                }],
                model_used: "mock".to_string(),
                stop_reason: hex_core::domain::messages::StopReason::EndTurn,
                input_tokens: 0,
                output_tokens: self.response_text.len() as u64,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                latency_ms: 0,
            })
        }

        async fn stream(
            &self,
            _request: InferenceRequest,
        ) -> Result<
            Box<dyn hex_core::ports::inference::futures_stream::Stream<Item = hex_core::ports::inference::StreamChunk> + Send + Unpin>,
            InferenceError,
        > {
            unimplemented!("not needed for scaffolding tests")
        }

        async fn health(
            &self,
        ) -> Result<hex_core::ports::inference::HealthStatus, InferenceError> {
            Ok(hex_core::ports::inference::HealthStatus::Ok {
                models: vec![],
            })
        }

        fn capabilities(&self) -> hex_core::ports::inference::InferenceCapabilities {
            hex_core::ports::inference::InferenceCapabilities {
                models: vec![],
                supports_tool_use: false,
                supports_thinking: false,
                supports_caching: false,
                supports_streaming: false,
                max_context_tokens: 8_192,
                cost_per_mtok_input: 0.0,
                cost_per_mtok_output: 0.0,
            }
        }
    }

    fn make_request() -> InferenceRequest {
        InferenceRequest {
            model: "test".to_string(),
            system_prompt: String::new(),
            messages: vec![Message::user("write fibonacci")],
            tools: vec![],
            max_tokens: 100,
            temperature: 0.2,
            thinking_budget: None,
            cache_control: false,
            priority: hex_core::ports::inference::Priority::Normal,
            grammar: None,
        }
    }

    #[tokio::test]
    async fn success_on_first_attempt() {
        let mock = MockInferencePort::with_response("fn fibonacci(n: u64) -> u64 { 0 }");
        let checker = MockCompileChecker {
            accept_fn: Box::new(|code| code.contains("fibonacci")),
        };
        let dispatch = ScaffoldedDispatch::new(Arc::new(mock), Box::new(checker));

        let result = dispatch.dispatch(&make_request(), TaskTier::T2).await.unwrap();
        match result {
            ScaffoldResult::Success { attempt, .. } => assert_eq!(attempt, 1),
            _ => panic!("expected Success"),
        }
    }

    #[tokio::test]
    async fn all_fail_returns_compile_gate_failed() {
        let mock = MockInferencePort::with_response("bad code");
        let checker = MockCompileChecker {
            accept_fn: Box::new(|_| false), // always reject
        };
        let config = ScaffoldConfig {
            n_for_tier: |_| 2,
            max_retries: 1,
        };
        let dispatch = ScaffoldedDispatch::new(Arc::new(mock), Box::new(checker))
            .with_config(config);

        let result = dispatch.dispatch(&make_request(), TaskTier::T2).await.unwrap();
        match result {
            ScaffoldResult::CompileGateFailed { total_attempts, .. } => {
                // 2 per round * 2 rounds (initial + 1 retry) = 4
                assert_eq!(total_attempts, 4);
            }
            _ => panic!("expected CompileGateFailed"),
        }
    }

    #[tokio::test]
    async fn t1_uses_n_equals_1() {
        let mock = MockInferencePort::with_response("good code");
        let checker = MockCompileChecker {
            accept_fn: Box::new(|_| true),
        };
        let dispatch = ScaffoldedDispatch::new(Arc::new(mock), Box::new(checker));

        let result = dispatch.dispatch(&make_request(), TaskTier::T1).await.unwrap();
        match result {
            ScaffoldResult::Success { total_attempts, .. } => {
                assert_eq!(total_attempts, 1); // N=1 for T1
            }
            _ => panic!("expected Success"),
        }
    }

    // ── P3.2: Error-feedback retry loop tests ─────────────────────────

    #[tokio::test]
    async fn retry_succeeds_on_second_round() {
        // N=2, reject first 2 attempts (round 0), accept on attempt 3 (round 1).
        let mock = MockInferencePort::with_response("fixed code");
        let checker = EventuallyPassingChecker {
            call_count: AtomicUsize::new(0),
            reject_count: 2, // fail first round (2 attempts), pass first of second round
        };
        let config = ScaffoldConfig {
            n_for_tier: |_| 2,
            max_retries: 2,
        };
        let dispatch =
            ScaffoldedDispatch::new(Arc::new(mock), Box::new(checker)).with_config(config);

        let result = dispatch.dispatch(&make_request(), TaskTier::T2).await.unwrap();
        match result {
            ScaffoldResult::Success {
                attempt,
                total_attempts,
                ..
            } => {
                // 2 failed in round 0, then 1st attempt in round 1 passes → attempt 3
                assert_eq!(attempt, 3);
                assert_eq!(total_attempts, 3);
            }
            other => panic!("expected Success, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn error_feedback_augments_prompt_on_retry() {
        // Use a capturing mock to inspect what the inference port receives.
        // N=1, max_retries=1 → round 0 fails, round 1 retries with error in prompt.
        let capturing = Arc::new(CapturingInferencePort::new("bad code"));
        let checker = MockCompileChecker {
            accept_fn: Box::new(|_| false), // always reject
        };
        let config = ScaffoldConfig {
            n_for_tier: |_| 1,
            max_retries: 1,
        };
        let dispatch = ScaffoldedDispatch::new(capturing.clone(), Box::new(checker))
            .with_config(config);

        let _ = dispatch.dispatch(&make_request(), TaskTier::T1).await.unwrap();

        let reqs = capturing.captured_requests();
        // 2 requests total: round 0 (original) + round 1 (augmented)
        assert_eq!(reqs.len(), 2);

        // First request should be the original (no error feedback)
        let first_text = extract_user_text(&reqs[0]);
        assert!(
            !first_text.contains("compiler error"),
            "first request should not contain error feedback"
        );

        // Second request should contain the error feedback suffix
        let second_text = extract_user_text(&reqs[1]);
        assert!(
            second_text.contains("The previous attempt produced this compiler error"),
            "retry request must contain error feedback, got: {second_text}"
        );
        assert!(
            second_text.contains("mock compile error"),
            "retry request must include the actual stderr"
        );
    }

    #[tokio::test]
    async fn best_error_is_shortest_stderr() {
        // Two completions per round with different error lengths; best_error
        // should be the shorter one.
        let mock = MockInferencePort::with_response("bad code");
        let checker = VariableErrorChecker {
            call_count: AtomicUsize::new(0),
            errors: vec![
                "a]long error message that goes on and on".into(),
                "short".into(),
            ],
        };
        let config = ScaffoldConfig {
            n_for_tier: |_| 2,
            max_retries: 0, // no retries — just one round
        };
        let dispatch =
            ScaffoldedDispatch::new(Arc::new(mock), Box::new(checker)).with_config(config);

        let result = dispatch.dispatch(&make_request(), TaskTier::T2).await.unwrap();
        match result {
            ScaffoldResult::CompileGateFailed { best_error, .. } => {
                assert_eq!(best_error, "short");
            }
            other => panic!("expected CompileGateFailed, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn total_attempts_equals_n_times_rounds() {
        // N=3, max_retries=2 → 3 rounds × 3 per round = 9 total.
        let mock = MockInferencePort::with_response("bad code");
        let checker = MockCompileChecker {
            accept_fn: Box::new(|_| false),
        };
        let config = ScaffoldConfig {
            n_for_tier: |_| 3,
            max_retries: 2,
        };
        let dispatch =
            ScaffoldedDispatch::new(Arc::new(mock), Box::new(checker)).with_config(config);

        let result = dispatch.dispatch(&make_request(), TaskTier::T2).await.unwrap();
        match result {
            ScaffoldResult::CompileGateFailed { total_attempts, .. } => {
                assert_eq!(total_attempts, 9); // 3 * (1 + 2)
            }
            other => panic!("expected CompileGateFailed, got {:?}", other),
        }
    }

    // ── P4.1: Frontier escalation tests ────────────────────────────────

    #[tokio::test]
    async fn escalates_to_frontier_on_compile_gate_exhaustion() {
        // Local adapter always produces bad code; frontier produces good code.
        let local = Arc::new(CapturingInferencePort::new("bad code"));
        let frontier = Arc::new(CapturingInferencePort::new("good frontier code"));
        let checker = MockCompileChecker {
            accept_fn: Box::new(|_| false), // always reject (frontier skips gate)
        };
        let config = ScaffoldConfig {
            n_for_tier: |_| 1,
            max_retries: 0,
        };
        let dispatch = ScaffoldedDispatch::new(local.clone(), Box::new(checker))
            .with_config(config)
            .with_frontier(frontier.clone());

        let result = dispatch.dispatch(&make_request(), TaskTier::T2).await.unwrap();
        match result {
            ScaffoldResult::Success { response, .. } => {
                // Should contain frontier's response, not local's
                let text: String = response
                    .content
                    .iter()
                    .filter_map(|b| {
                        if let ContentBlock::Text { text } = b {
                            Some(text.as_str())
                        } else {
                            None
                        }
                    })
                    .collect();
                assert_eq!(text, "good frontier code");
            }
            other => panic!("expected Success from frontier escalation, got {:?}", other),
        }

        // Frontier should receive the original prompt (no error augmentation)
        let frontier_reqs = frontier.captured_requests();
        assert_eq!(frontier_reqs.len(), 1);
        let frontier_text = extract_user_text(&frontier_reqs[0]);
        assert!(
            !frontier_text.contains("compiler error"),
            "frontier must receive original prompt, not error-augmented"
        );
    }

    #[tokio::test]
    async fn no_frontier_returns_compile_gate_failed() {
        // Same setup but WITHOUT a frontier — should fall through to error.
        let local = Arc::new(CapturingInferencePort::new("bad code"));
        let checker = MockCompileChecker {
            accept_fn: Box::new(|_| false),
        };
        let config = ScaffoldConfig {
            n_for_tier: |_| 1,
            max_retries: 0,
        };
        let dispatch =
            ScaffoldedDispatch::new(local, Box::new(checker)).with_config(config);

        let result = dispatch.dispatch(&make_request(), TaskTier::T2).await.unwrap();
        assert!(
            matches!(result, ScaffoldResult::CompileGateFailed { .. }),
            "without frontier, should return CompileGateFailed"
        );
    }

    /// Helper: extract concatenated user-message text from an InferenceRequest.
    fn extract_user_text(req: &InferenceRequest) -> String {
        req.messages
            .iter()
            .flat_map(|m| m.content.iter())
            .filter_map(|block| {
                if let ContentBlock::Text { text } = block {
                    Some(text.as_str())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("")
    }

    // ── P4.2: Escalation tracking tests ───────────────────────────────

    /// In-memory tracker for testing that records all events.
    struct RecordingTracker {
        successes: std::sync::Mutex<Vec<(String, String)>>,
        escalations: std::sync::Mutex<Vec<(String, String, String)>>,
    }

    impl RecordingTracker {
        fn new() -> Self {
            Self {
                successes: std::sync::Mutex::new(Vec::new()),
                escalations: std::sync::Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl EscalationTracker for RecordingTracker {
        async fn record_local_success(&self, task_type: &str, model: &str) {
            self.successes
                .lock()
                .unwrap()
                .push((task_type.to_string(), model.to_string()));
        }
        async fn record_escalation(&self, task_type: &str, model: &str, sample_error: &str) {
            self.escalations.lock().unwrap().push((
                task_type.to_string(),
                model.to_string(),
                sample_error.to_string(),
            ));
        }
    }

    #[tokio::test]
    async fn tracker_records_local_success() {
        let mock = MockInferencePort::with_response("good code");
        let checker = MockCompileChecker {
            accept_fn: Box::new(|_| true),
        };
        let tracker = Arc::new(RecordingTracker::new());
        let dispatch = ScaffoldedDispatch::new(Arc::new(mock), Box::new(checker))
            .with_tracker(tracker.clone());

        let result = dispatch.dispatch(&make_request(), TaskTier::T2).await.unwrap();
        assert!(matches!(result, ScaffoldResult::Success { .. }));

        let successes = tracker.successes.lock().unwrap();
        assert_eq!(successes.len(), 1);
        assert_eq!(successes[0].0, "T2");
        assert_eq!(successes[0].1, "test");
        assert!(tracker.escalations.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn tracker_records_escalation_on_frontier_fallback() {
        let local = Arc::new(CapturingInferencePort::new("bad code"));
        let frontier = Arc::new(CapturingInferencePort::new("frontier code"));
        let checker = MockCompileChecker {
            accept_fn: Box::new(|_| false),
        };
        let tracker = Arc::new(RecordingTracker::new());
        let config = ScaffoldConfig {
            n_for_tier: |_| 1,
            max_retries: 0,
        };
        let dispatch = ScaffoldedDispatch::new(local, Box::new(checker))
            .with_config(config)
            .with_frontier(frontier)
            .with_tracker(tracker.clone());

        let result = dispatch.dispatch(&make_request(), TaskTier::T2).await.unwrap();
        assert!(matches!(result, ScaffoldResult::Success { .. }));

        let escalations = tracker.escalations.lock().unwrap();
        assert_eq!(escalations.len(), 1);
        assert_eq!(escalations[0].0, "T2");
        assert_eq!(escalations[0].1, "test");
        assert!(
            escalations[0].2.contains("mock compile error"),
            "sample_error should contain the compile stderr"
        );
        assert!(tracker.successes.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn tracker_not_called_without_frontier_on_failure() {
        // No frontier, all fail → no tracker calls at all (neither success nor escalation).
        let mock = MockInferencePort::with_response("bad code");
        let checker = MockCompileChecker {
            accept_fn: Box::new(|_| false),
        };
        let tracker = Arc::new(RecordingTracker::new());
        let config = ScaffoldConfig {
            n_for_tier: |_| 1,
            max_retries: 0,
        };
        let dispatch = ScaffoldedDispatch::new(Arc::new(mock), Box::new(checker))
            .with_config(config)
            .with_tracker(tracker.clone());

        let result = dispatch.dispatch(&make_request(), TaskTier::T1).await.unwrap();
        assert!(matches!(result, ScaffoldResult::CompileGateFailed { .. }));

        assert!(tracker.successes.lock().unwrap().is_empty());
        assert!(tracker.escalations.lock().unwrap().is_empty());
    }

    #[test]
    fn parse_count_extracts_count_field() {
        assert_eq!(parse_count(r#"{"count": 5}"#), 5);
        assert_eq!(parse_count(r#"{"count": 0}"#), 0);
        assert_eq!(parse_count(r#"{"other": 3}"#), 0);
        assert_eq!(parse_count("not json"), 0);
        assert_eq!(parse_count(""), 0);
    }

    #[test]
    fn escalation_stats_rate_calculation() {
        let stats = EscalationStats {
            task_type: "T2".into(),
            model: "qwen3:32b".into(),
            local_successes: 7,
            escalations: 3,
            last_sample_error: None,
        };
        let rate = stats.escalation_rate();
        assert!((rate - 0.3).abs() < 0.001);

        let zero = EscalationStats {
            task_type: "T1".into(),
            model: "test".into(),
            local_successes: 0,
            escalations: 0,
            last_sample_error: None,
        };
        assert_eq!(zero.escalation_rate(), 0.0);
    }
}
