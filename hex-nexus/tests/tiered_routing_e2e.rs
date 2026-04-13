//! End-to-end integration test for T2 scaffolded dispatch (ADR-2604120202 P5.2).
//!
//! Verifies the full ScaffoldedDispatch path with a mock inference port that
//! fails on the first call (returns code that doesn't compile) and succeeds on
//! the second (returns valid code), plus a mock compile checker that mirrors
//! this sequence. Asserts:
//!
//! - Two inference attempts were made before success.
//! - The second attempt's response was returned.
//! - Escalation to a frontier model was NOT triggered.
//! - The overall result is `ScaffoldResult::Success`.

mod tiered_routing_e2e {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use async_trait::async_trait;
    use hex_core::domain::messages::{ContentBlock, Message, StopReason};
    use hex_core::ports::inference::{
        futures_stream, HealthStatus, IInferencePort, InferenceCapabilities, InferenceError,
        InferenceRequest, InferenceResponse, ModelInfo, ModelTier, StreamChunk,
    };
    use hex_nexus::orchestration::scaffolding::{
        CompileChecker, CompileError, ScaffoldConfig, ScaffoldResult, ScaffoldedDispatch,
    };
    use hex_nexus::remote::transport::TaskTier;

    // ── Sequenced mock inference port ───────────────────────

    /// Mock inference port that returns different responses on successive calls.
    /// First call returns "bad code", second call returns "fn valid() {}".
    struct SequencedInferencePort {
        call_count: AtomicUsize,
        responses: Vec<String>,
    }

    impl SequencedInferencePort {
        fn new(responses: Vec<String>) -> Self {
            Self {
                call_count: AtomicUsize::new(0),
                responses,
            }
        }

        fn total_calls(&self) -> usize {
            self.call_count.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl IInferencePort for SequencedInferencePort {
        async fn complete(
            &self,
            _request: InferenceRequest,
        ) -> Result<InferenceResponse, InferenceError> {
            let idx = self.call_count.fetch_add(1, Ordering::SeqCst);
            let text = self
                .responses
                .get(idx)
                .cloned()
                .unwrap_or_else(|| self.responses.last().cloned().unwrap_or_default());

            Ok(InferenceResponse {
                content: vec![ContentBlock::Text { text: text.clone() }],
                model_used: "mock-sequenced".to_string(),
                stop_reason: StopReason::EndTurn,
                input_tokens: 0,
                output_tokens: text.len() as u64,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                latency_ms: 0,
            })
        }

        async fn stream(
            &self,
            _request: InferenceRequest,
        ) -> Result<
            Box<dyn futures_stream::Stream<Item = StreamChunk> + Send + Unpin>,
            InferenceError,
        > {
            Err(InferenceError::ProviderUnavailable(
                "stream not implemented in test mock".into(),
            ))
        }

        async fn health(&self) -> Result<HealthStatus, InferenceError> {
            Ok(HealthStatus::Ok { models: vec![] })
        }

        fn capabilities(&self) -> InferenceCapabilities {
            InferenceCapabilities {
                models: vec![ModelInfo {
                    id: "mock-sequenced".to_string(),
                    provider: "test".to_string(),
                    tier: ModelTier::Local,
                    context_window: 8_192,
                }],
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

    // ── Sequenced mock compile checker ──────────────────────

    /// Mock compile checker that fails on the first call and succeeds on the
    /// second, simulating the real scenario where the first LLM output has
    /// compile errors and the retry (with error feedback) produces valid code.
    struct SequencedCompileChecker {
        call_count: AtomicUsize,
    }

    impl SequencedCompileChecker {
        fn new() -> Self {
            Self {
                call_count: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl CompileChecker for SequencedCompileChecker {
        async fn check(&self, _code: &str) -> Result<(), CompileError> {
            let idx = self.call_count.fetch_add(1, Ordering::SeqCst);
            if idx == 0 {
                Err(CompileError {
                    stderr: "error[E0308]: mismatched types\n  --> src/main.rs:3:5"
                        .to_string(),
                })
            } else {
                Ok(())
            }
        }
    }

    // ── Tracking frontier mock (should NOT be called) ───────

    /// Mock frontier port that records whether it was called. Used to assert
    /// that escalation does NOT happen when the local model succeeds on retry.
    struct TrackingFrontierPort {
        called: AtomicUsize,
    }

    impl TrackingFrontierPort {
        fn new() -> Self {
            Self {
                called: AtomicUsize::new(0),
            }
        }

        fn was_called(&self) -> bool {
            self.called.load(Ordering::SeqCst) > 0
        }
    }

    #[async_trait]
    impl IInferencePort for TrackingFrontierPort {
        async fn complete(
            &self,
            _request: InferenceRequest,
        ) -> Result<InferenceResponse, InferenceError> {
            self.called.fetch_add(1, Ordering::SeqCst);
            Ok(InferenceResponse {
                content: vec![ContentBlock::Text {
                    text: "frontier fallback".to_string(),
                }],
                model_used: "frontier-mock".to_string(),
                stop_reason: StopReason::EndTurn,
                input_tokens: 0,
                output_tokens: 0,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                latency_ms: 0,
            })
        }

        async fn stream(
            &self,
            _request: InferenceRequest,
        ) -> Result<
            Box<dyn futures_stream::Stream<Item = StreamChunk> + Send + Unpin>,
            InferenceError,
        > {
            Err(InferenceError::ProviderUnavailable("not impl".into()))
        }

        async fn health(&self) -> Result<HealthStatus, InferenceError> {
            Ok(HealthStatus::Ok { models: vec![] })
        }

        fn capabilities(&self) -> InferenceCapabilities {
            InferenceCapabilities {
                models: vec![],
                supports_tool_use: false,
                supports_thinking: false,
                supports_caching: false,
                supports_streaming: false,
                max_context_tokens: 0,
                cost_per_mtok_input: 0.0,
                cost_per_mtok_output: 0.0,
            }
        }
    }

    // ── Helper ──────────────────────────────────────────────

    fn make_t2_request() -> InferenceRequest {
        InferenceRequest {
            model: "test-t2".to_string(),
            system_prompt: "You are a Rust code generator.".to_string(),
            messages: vec![Message::user("write a fibonacci function in Rust")],
            tools: vec![],
            max_tokens: 512,
            temperature: 0.2,
            thinking_budget: None,
            cache_control: false,
            priority: hex_core::ports::inference::Priority::Normal,
            grammar: None,
        }
    }

    // ── Tests ───────────────────────────────────────────────

    /// Core E2E test: T2 task with Best-of-N=1 (to make sequencing
    /// deterministic), first attempt fails compile gate, second succeeds.
    /// Verifies: (a) two attempts, (b) second response returned, (c) no
    /// escalation, (d) result is Ok(Success).
    #[tokio::test]
    async fn t2_scaffolded_dispatch_retries_on_compile_failure() {
        let inference = Arc::new(SequencedInferencePort::new(vec![
            "let x: u32 = \"not a number\";".to_string(), // bad code
            "fn valid() -> u64 { 42 }".to_string(),       // good code
        ]));
        let checker = SequencedCompileChecker::new();
        let frontier = Arc::new(TrackingFrontierPort::new());

        // N=1 per tier so we get exactly one completion per round.
        // max_retries=2 gives us room for the retry with error feedback.
        let config = ScaffoldConfig {
            n_for_tier: |_| 1,
            max_retries: 2,
        };

        let dispatch = ScaffoldedDispatch::new(
            inference.clone() as Arc<dyn IInferencePort>,
            Box::new(checker),
        )
        .with_config(config)
        .with_frontier(frontier.clone() as Arc<dyn IInferencePort>);

        let request = make_t2_request();
        let result = dispatch
            .dispatch(&request, TaskTier::T2)
            .await
            .expect("dispatch should not return Err");

        // (a) Two inference calls were made (one failed, one succeeded).
        assert_eq!(
            inference.total_calls(),
            2,
            "expected exactly 2 inference calls"
        );

        // (b) The successful response contains the valid code.
        match &result {
            ScaffoldResult::Success { response, .. } => {
                let text = response
                    .content
                    .iter()
                    .filter_map(|b| {
                        if let ContentBlock::Text { text } = b {
                            Some(text.as_str())
                        } else {
                            None
                        }
                    })
                    .collect::<String>();
                assert!(
                    text.contains("fn valid()"),
                    "expected valid code in response, got: {text}"
                );
            }
            ScaffoldResult::CompileGateFailed { .. } => {
                panic!("expected Success, got CompileGateFailed");
            }
        }

        // (c) Escalation was NOT triggered.
        assert!(
            !frontier.was_called(),
            "frontier should NOT have been called — local retry succeeded"
        );

        // (d) Result is Success with attempt=2.
        match result {
            ScaffoldResult::Success {
                attempt,
                total_attempts,
                ..
            } => {
                assert_eq!(attempt, 2, "success should be on attempt 2");
                assert_eq!(total_attempts, 2, "total attempts should be 2");
            }
            _ => unreachable!(),
        }
    }

    /// Verify that when all attempts exhaust (both N completions and retries),
    /// and a frontier is configured, the frontier IS called (escalation path).
    #[tokio::test]
    async fn t2_escalates_to_frontier_when_all_local_attempts_fail() {
        // Inference always returns bad code.
        let inference = Arc::new(SequencedInferencePort::new(vec![
            "bad code forever".to_string(),
        ]));
        // Compile checker always rejects.
        struct AlwaysRejectChecker;
        #[async_trait]
        impl CompileChecker for AlwaysRejectChecker {
            async fn check(&self, _code: &str) -> Result<(), CompileError> {
                Err(CompileError {
                    stderr: "error: everything is wrong".to_string(),
                })
            }
        }

        let frontier = Arc::new(TrackingFrontierPort::new());

        let config = ScaffoldConfig {
            n_for_tier: |_| 1,
            max_retries: 1,
        };

        let dispatch = ScaffoldedDispatch::new(
            inference.clone() as Arc<dyn IInferencePort>,
            Box::new(AlwaysRejectChecker),
        )
        .with_config(config)
        .with_frontier(frontier.clone() as Arc<dyn IInferencePort>);

        let result = dispatch
            .dispatch(&make_t2_request(), TaskTier::T2)
            .await
            .expect("dispatch should not return Err");

        // Frontier WAS called (escalation).
        assert!(
            frontier.was_called(),
            "frontier should have been called after all local attempts failed"
        );

        // Result should be Success (from frontier).
        match result {
            ScaffoldResult::Success { response, .. } => {
                let text = response
                    .content
                    .iter()
                    .filter_map(|b| {
                        if let ContentBlock::Text { text } = b {
                            Some(text.as_str())
                        } else {
                            None
                        }
                    })
                    .collect::<String>();
                assert!(
                    text.contains("frontier fallback"),
                    "expected frontier response, got: {text}"
                );
            }
            ScaffoldResult::CompileGateFailed { .. } => {
                panic!("expected Success from frontier escalation");
            }
        }
    }

    /// Without a frontier configured, exhausting all attempts returns
    /// `CompileGateFailed` (no escalation path).
    #[tokio::test]
    async fn t2_without_frontier_returns_compile_gate_failed() {
        let inference = Arc::new(SequencedInferencePort::new(vec![
            "bad code".to_string(),
        ]));

        struct AlwaysRejectChecker;
        #[async_trait]
        impl CompileChecker for AlwaysRejectChecker {
            async fn check(&self, _code: &str) -> Result<(), CompileError> {
                Err(CompileError {
                    stderr: "error[E0599]: no method named `foo`".to_string(),
                })
            }
        }

        let config = ScaffoldConfig {
            n_for_tier: |_| 1,
            max_retries: 0,
        };

        let dispatch = ScaffoldedDispatch::new(
            inference.clone() as Arc<dyn IInferencePort>,
            Box::new(AlwaysRejectChecker),
        )
        .with_config(config);
        // No .with_frontier() — escalation path absent.

        let result = dispatch
            .dispatch(&make_t2_request(), TaskTier::T2)
            .await
            .expect("dispatch should not return Err");

        match result {
            ScaffoldResult::CompileGateFailed {
                total_attempts,
                best_error,
            } => {
                assert_eq!(total_attempts, 1, "N=1, retries=0 → 1 attempt");
                assert!(
                    best_error.contains("E0599"),
                    "best_error should contain the compiler error: {best_error}"
                );
            }
            ScaffoldResult::Success { .. } => {
                panic!("expected CompileGateFailed without frontier");
            }
        }
    }
}
