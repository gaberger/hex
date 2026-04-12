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

/// Scaffolded inference dispatcher. Wraps an inference port with Best-of-N
/// generation, compile-gate validation, and error-feedback retries.
pub struct ScaffoldedDispatch {
    inference: Arc<dyn hex_core::ports::inference::IInferencePort>,
    compile_checker: Box<dyn CompileChecker>,
    config: ScaffoldConfig,
    /// Optional frontier adapter for cascading escalation (P4).
    frontier: Option<Arc<dyn hex_core::ports::inference::IInferencePort>>,
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
            tracing::info!("Escalating to frontier model after local failure");
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
    use hex_core::domain::messages::Message;
    use hex_core::ports::inference::mock::MockInferencePort;

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
}
