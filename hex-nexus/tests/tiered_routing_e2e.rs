//! End-to-end test: T2 task with mock Ollama (ADR-2604120202, task P5.2).
//!
//! Integration test that wires ScaffoldedDispatch with a sequenced mock
//! inference port (fails on first attempt, succeeds on second) and a mock
//! escalation tracker. Asserts:
//!   (a) Two attempts made
//!   (b) Compile gate checked both attempts
//!   (c) Second attempt's code returned
//!   (d) Escalation NOT triggered (no frontier call)
//!   (e) HexFlo memory records local success (not escalation)

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use hex_core::domain::messages::{ContentBlock, Message, StopReason};
use hex_core::ports::inference::{
    IInferencePort, InferenceCapabilities, InferenceError, InferenceRequest, InferenceResponse,
    ModelInfo, ModelTier, Priority, StreamChunk,
};
use hex_nexus::orchestration::scaffolding::{
    CompileChecker, CompileError, EscalationTracker, ScaffoldConfig, ScaffoldResult,
    ScaffoldedDispatch,
};
use hex_nexus::remote::transport::TaskTier;

// ── Mock inference port (simulates Ollama: fail → succeed) ──

/// Mock inference that returns a different canned response per call.
/// First call returns code that won't compile; second returns valid code.
struct MockOllamaInference {
    responses: Vec<String>,
    call_count: AtomicUsize,
}

impl MockOllamaInference {
    fn new(responses: Vec<&str>) -> Self {
        Self {
            responses: responses.into_iter().map(String::from).collect(),
            call_count: AtomicUsize::new(0),
        }
    }

    fn calls(&self) -> usize {
        self.call_count.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl IInferencePort for MockOllamaInference {
    async fn complete(
        &self,
        request: InferenceRequest,
    ) -> Result<InferenceResponse, InferenceError> {
        let idx = self.call_count.fetch_add(1, Ordering::SeqCst);
        let text = self
            .responses
            .get(idx)
            .cloned()
            .unwrap_or_else(|| format!("unexpected call #{idx}"));

        Ok(InferenceResponse {
            content: vec![ContentBlock::Text { text }],
            model_used: request.model.clone(),
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
        Box<dyn hex_core::ports::inference::futures_stream::Stream<Item = StreamChunk> + Send + Unpin>,
        InferenceError,
    > {
        Err(InferenceError::ProviderUnavailable(
            "stream not implemented in test mock".into(),
        ))
    }

    async fn health(
        &self,
    ) -> Result<hex_core::ports::inference::HealthStatus, InferenceError> {
        Ok(hex_core::ports::inference::HealthStatus::Ok {
            models: vec![],
        })
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities {
            models: vec![ModelInfo {
                id: "qwen2.5-coder:7b".into(),
                provider: "ollama".into(),
                tier: ModelTier::Local,
                context_window: 32_768,
            }],
            supports_tool_use: false,
            supports_thinking: false,
            supports_caching: false,
            supports_streaming: false,
            max_context_tokens: 32_768,
            cost_per_mtok_input: 0.0,
            cost_per_mtok_output: 0.0,
        }
    }
}

// ── Mock compile checker with call tracking ──────────────

/// Compile checker that tracks which code strings were checked.
/// Accepts any code starting with "GOOD", rejects the rest.
struct TrackingCompileChecker {
    checked: Mutex<Vec<String>>,
}

impl TrackingCompileChecker {
    fn new() -> Self {
        Self {
            checked: Mutex::new(Vec::new()),
        }
    }

    fn checked_codes(&self) -> Vec<String> {
        self.checked.lock().unwrap().clone()
    }
}

#[async_trait]
impl CompileChecker for TrackingCompileChecker {
    async fn check(&self, code: &str) -> Result<(), CompileError> {
        self.checked.lock().unwrap().push(code.to_string());
        if code.starts_with("GOOD") {
            Ok(())
        } else {
            Err(CompileError {
                stderr: format!("error[E0308]: expected `u64`, found `&str` in `{}`",
                    &code[..code.len().min(30)]),
            })
        }
    }
}

// ── Mock escalation tracker ──────────────────────────────

#[derive(Debug, Clone)]
#[allow(dead_code)]
enum TrackerEvent {
    LocalSuccess { task_type: String, model: String },
    Escalation { task_type: String, model: String, sample_error: String },
}

struct MockHexFloTracker {
    events: Mutex<Vec<TrackerEvent>>,
}

impl MockHexFloTracker {
    fn new() -> Self {
        Self {
            events: Mutex::new(Vec::new()),
        }
    }

    fn events(&self) -> Vec<TrackerEvent> {
        self.events.lock().unwrap().clone()
    }
}

#[async_trait]
impl EscalationTracker for MockHexFloTracker {
    async fn record_local_success(&self, task_type: &str, model: &str) {
        self.events.lock().unwrap().push(TrackerEvent::LocalSuccess {
            task_type: task_type.to_string(),
            model: model.to_string(),
        });
    }

    async fn record_escalation(&self, task_type: &str, model: &str, sample_error: &str) {
        self.events.lock().unwrap().push(TrackerEvent::Escalation {
            task_type: task_type.to_string(),
            model: model.to_string(),
            sample_error: sample_error.to_string(),
        });
    }
}

// ── Helpers ──────────────────────────────────────────────

fn make_t2_workplan_request() -> InferenceRequest {
    InferenceRequest {
        model: "qwen2.5-coder:7b".into(),
        system_prompt: "You are a Rust code generator.".into(),
        messages: vec![Message::user(
            "Implement a function `parse_config(input: &str) -> Result<Config, ParseError>` \
             that reads a TOML string into a Config struct.",
        )],
        tools: vec![],
        max_tokens: 1024,
        temperature: 0.7,
        thinking_budget: None,
        cache_control: false,
        priority: Priority::Normal,
        grammar: None,
    }
}

/// N=3 for T2 (matches production default), but we stop after first success.
fn n_for_tier_production(tier: TaskTier) -> usize {
    match tier {
        TaskTier::T1 => 1,
        TaskTier::T2 => 3,
        TaskTier::T2_5 => 5,
        TaskTier::T3 => 1,
    }
}

// ══════════════════════════════════════════════════════════
// Main E2E test: T2 task — fail once, succeed on second attempt
// ══════════════════════════════════════════════════════════

#[tokio::test]
async fn t2_e2e_mock_ollama_fail_then_succeed() {
    // Setup: mock Ollama returns bad code first, good code second.
    let ollama = Arc::new(MockOllamaInference::new(vec![
        "BAD: fn parse_config(input: &str) -> Config { unimplemented!() }",
        "GOOD: fn parse_config(input: &str) -> Result<Config, ParseError> { toml::from_str(input) }",
        "BAD: should never be reached",
    ]));

    let checker = TrackingCompileChecker::new();
    let tracker = Arc::new(MockHexFloTracker::new());

    // Wire everything together — no frontier configured (T2 should not need it).
    let dispatch = ScaffoldedDispatch::new(ollama.clone(), Box::new(checker))
        .with_config(ScaffoldConfig {
            n_for_tier: n_for_tier_production,
            max_retries: 1,
        })
        .with_tracker(tracker.clone());

    // Dispatch as T2 (single-function code generation).
    let result = dispatch
        .dispatch(&make_t2_workplan_request(), TaskTier::T2)
        .await
        .expect("dispatch should not return Err");

    // ── Assertion (a): exactly two attempts made ──
    assert_eq!(
        ollama.calls(),
        2,
        "mock Ollama should have been called exactly twice (fail + succeed)"
    );

    match result {
        ScaffoldResult::Success {
            response,
            attempt,
            total_attempts,
        } => {
            // ── Assertion (a) continued: attempt counters ──
            assert_eq!(attempt, 2, "should succeed on attempt 2");
            assert_eq!(total_attempts, 2, "should stop early after success (not exhaust all N=3)");

            // ── Assertion (c): second attempt's code returned ──
            let text = match response.content.first() {
                Some(ContentBlock::Text { text }) => text.as_str(),
                _ => panic!("expected Text content block"),
            };
            assert!(
                text.starts_with("GOOD"),
                "response should be the second (successful) completion, got: {text}"
            );
            assert!(
                text.contains("Result<Config, ParseError>"),
                "returned code should be the correct implementation"
            );
        }
        ScaffoldResult::CompileGateFailed { .. } => {
            panic!("expected Success, got CompileGateFailed — second attempt should have compiled");
        }
    }

    // ── Assertion (d): escalation NOT triggered ──
    // No frontier was configured, and even if it were, dispatch should have
    // succeeded locally on attempt 2 without needing escalation.
    let events = tracker.events();
    for event in &events {
        assert!(
            !matches!(event, TrackerEvent::Escalation { .. }),
            "escalation should NOT have been triggered — local succeeded on attempt 2"
        );
    }

    // ── Assertion (e): HexFlo memory records local success ──
    assert_eq!(events.len(), 1, "tracker should record exactly one event");
    match &events[0] {
        TrackerEvent::LocalSuccess { task_type, model } => {
            assert_eq!(task_type, "T2", "task_type should be T2");
            assert_eq!(model, "qwen2.5-coder:7b", "model should match the request");
        }
        other => panic!("expected LocalSuccess event, got: {:?}", other),
    }
}

// ══════════════════════════════════════════════════════════
// Supplementary: compile gate verified both attempts
// ══════════════════════════════════════════════════════════

/// Separate test that uses a tracking checker to verify assertion (b):
/// the compile gate was invoked for both the failing and succeeding attempts.
#[tokio::test]
async fn t2_e2e_compile_gate_checked_both_attempts() {
    let ollama = Arc::new(MockOllamaInference::new(vec![
        "BAD: fn parse_config() {}",
        "GOOD: fn parse_config(input: &str) -> Result<Config, ParseError> { Ok(Config {}) }",
    ]));

    // Use a checker we can inspect after dispatch.
    let checker = Arc::new(TrackingCompileChecker::new());
    // Clone the Arc for post-dispatch inspection.
    let checker_ref = checker.clone();

    // We need to pass ownership of a CompileChecker to ScaffoldedDispatch.
    // Wrap the Arc in a delegating struct.
    struct DelegatingChecker(Arc<TrackingCompileChecker>);

    #[async_trait]
    impl CompileChecker for DelegatingChecker {
        async fn check(&self, code: &str) -> Result<(), CompileError> {
            self.0.check(code).await
        }
    }

    let dispatch = ScaffoldedDispatch::new(ollama.clone(), Box::new(DelegatingChecker(checker)))
        .with_config(ScaffoldConfig {
            n_for_tier: n_for_tier_production,
            max_retries: 1,
        });

    let result = dispatch
        .dispatch(&make_t2_workplan_request(), TaskTier::T2)
        .await
        .expect("dispatch should not error");

    assert!(
        matches!(result, ScaffoldResult::Success { .. }),
        "should succeed on second attempt"
    );

    // ── Assertion (b): compile gate checked BOTH attempts ──
    let checked = checker_ref.checked_codes();
    assert_eq!(
        checked.len(),
        2,
        "compile gate should have been invoked exactly twice, got {}",
        checked.len()
    );
    assert!(
        checked[0].starts_with("BAD"),
        "first checked code should be the failing attempt"
    );
    assert!(
        checked[1].starts_with("GOOD"),
        "second checked code should be the succeeding attempt"
    );
}

// ══════════════════════════════════════════════════════════
// Edge case: T2 succeeds on first attempt — no retry overhead
// ══════════════════════════════════════════════════════════

#[tokio::test]
async fn t2_e2e_first_attempt_success_no_retry() {
    let ollama = Arc::new(MockOllamaInference::new(vec![
        "GOOD: fn parse_config(input: &str) -> Result<Config, ParseError> { toml::from_str(input) }",
    ]));

    let checker = TrackingCompileChecker::new();
    let tracker = Arc::new(MockHexFloTracker::new());

    let dispatch = ScaffoldedDispatch::new(ollama.clone(), Box::new(checker))
        .with_config(ScaffoldConfig {
            n_for_tier: n_for_tier_production,
            max_retries: 1,
        })
        .with_tracker(tracker.clone());

    let result = dispatch
        .dispatch(&make_t2_workplan_request(), TaskTier::T2)
        .await
        .expect("dispatch should not error");

    match result {
        ScaffoldResult::Success {
            attempt,
            total_attempts,
            ..
        } => {
            assert_eq!(attempt, 1, "should succeed on first attempt");
            assert_eq!(total_attempts, 1, "only one attempt needed");
        }
        ScaffoldResult::CompileGateFailed { .. } => {
            panic!("expected Success on first attempt");
        }
    }

    assert_eq!(ollama.calls(), 1, "only one inference call needed");

    let events = tracker.events();
    assert_eq!(events.len(), 1);
    assert!(
        matches!(&events[0], TrackerEvent::LocalSuccess { task_type, .. } if task_type == "T2"),
        "should record local success for T2"
    );
}
