//! Strict-JSON classifier adapter — secondary adapter that wraps an
//! `IInferencePort` and turns raw LLM completions into a validated
//! [`ClassifierResponse`] (ADR-2026-05-17-2030 Phase 3, workplan
//! wp-sop-pipeline-redesign-phase-1 P3.1).
//!
//! ## Role in the pipeline
//!
//! P1 generated the pure types ([`classifier_types`]) and the pure parser
//! ([`classifier_parser`]). Both are zero-I/O — they only know how to
//! validate a `&str`. This file is the **first piece** of the pipeline that
//! actually talks to inference: it sends a strict-JSON system prompt, reads
//! the raw response, runs it through [`SerdeJsonClassifierParser::parse`],
//! and — crucially — retries on `MalformedJson` up to a **2-reparse retry
//! budget**.
//!
//! ## Retry semantics (P3.1 contract)
//!
//! The reparse budget is exactly **2 additional attempts** after the first
//! attempt fails with [`InvariantError::MalformedJson`]. That gives a worst
//! case of 3 inference calls per `classify()`:
//!
//! ```text
//!   attempt 1: malformed → retry budget = 2 left → reprompt with error
//!   attempt 2: malformed → retry budget = 1 left → reprompt with error
//!   attempt 3: malformed → retry budget = 0 left → return MalformedJson(_)
//! ```
//!
//! **Other [`InvariantError`] variants do NOT consume the budget — they
//! escalate immediately.** Re-prompting cannot change the operator-direct
//! identity check (`DecisionNotAllowedForOperator`) and the model already
//! told us which decision it picked, so the missing-field violation
//! (`MissingRequiredField`) is a contract bug rather than a transient parse
//! issue. Both surface straight to the caller so the operator inbox / SOP
//! escalation path (P5) can pick them up.
//!
//! ## Strict-JSON prompting
//!
//! The system prompt is fixed and emphasizes:
//!
//!   1. Output a single JSON object — no prose, no markdown fences.
//!   2. `decision` must be one of the snake_case enum values.
//!   3. Per-decision required fields (mirrors `classifier_parser` invariants).
//!
//! Even with this prompt, smaller models routinely wrap the JSON in
//! ` ```json …``` ` fences. That's fine — the parser strips fences before
//! `serde_json::from_str` so we don't burn a retry on cosmetic packaging.
//!
//! ## What this adapter does NOT do
//!
//! - It does not score or route the classifier output — that's P4 (`sop_executor`).
//! - It does not write to STDB — that's P2's `classifier_response` table writer.
//! - It does not pick a model — the caller picks the `model` string and tier
//!   knob upstream (typically the persona's classifier-tier pin).

use std::sync::Arc;

use hex_core::domain::messages::{ContentBlock, Message};
use hex_core::ports::inference::{
    IInferencePort, InferenceRequest, Priority,
};

use super::classifier_parser::{ClassifierParser, InvariantError, SerdeJsonClassifierParser};
use super::classifier_types::ClassifierResponse;

/// Reparse retry budget — the number of *additional* attempts after the
/// first malformed response. Worst case is 1 + `REPARSE_BUDGET` inference
/// calls per `classify()`.
const REPARSE_BUDGET: u8 = 2;

/// Default token cap for a classifier completion. The output is a small
/// JSON object — even the largest `tool_plan` is < 1k tokens — so capping
/// here keeps both cost (CPO spec) and latency bounded.
const DEFAULT_MAX_TOKENS: u32 = 1_024;

/// Default sampling temperature. Classifier output is meant to be
/// deterministic relative to its inputs; a low temp + retries-on-malformed
/// is cheaper than a high temp + ensembling.
const DEFAULT_TEMPERATURE: f32 = 0.1;

/// The strict-JSON system prompt. Kept as a `const` rather than templated
/// because every classifier call uses the same schema — if a persona needs
/// a different schema, it gets its own adapter.
const STRICT_JSON_SYSTEM_PROMPT: &str = "You are a persona inbox classifier. Your job is to read ONE inbound message and \
respond with a single JSON object describing how this persona should act on it.

OUTPUT FORMAT — strict:
- Respond with exactly one JSON object. No prose. No markdown. No code fences. No commentary.
- Top-level keys: `decision` (required), `cost_usd` (required, number, you may use 0).
- Optional keys depending on `decision`:
  - `tool_plan`: array of {\"tool\": string, \"intent\": string} — required if decision = accept.
  - `reason`: string — required if decision = defer or reject.
  - `target_persona`: string — required if decision = route.
  - `question`: string — required if decision = clarify.
  - `tool_spec`: JSON object — required if decision = request_tool.

DECISION VOCABULARY (snake_case, exactly one of):
  - `accept`        — this persona will act on the ask now.
  - `defer`         — busy / blocked; not now. Forbidden on from=operator traffic.
  - `route`         — forward to a different persona named in `target_persona`.
  - `clarify`       — need more information; ask `question`.
  - `reject`        — refuse. Forbidden on from=operator traffic.
  - `request_tool`  — need a new tool; describe it in `tool_spec`.

CONSTRAINTS:
- If the message is from=operator, you MUST NOT pick `defer` or `reject`.
- Omit optional keys you don't need. Do not include null values.

Return ONLY the JSON object.";

/// Strict-JSON classifier adapter.
///
/// Wraps an `Arc<dyn IInferencePort>` so it can be shared across
/// concurrent persona dispatch tasks without per-call cloning of a heavy
/// HTTP client. The parser is injected for testability — production wires
/// the default [`SerdeJsonClassifierParser`].
pub struct StrictJsonClassifierAdapter {
    inference: Arc<dyn IInferencePort>,
    parser: Arc<dyn ClassifierParser>,
    model: String,
    max_tokens: u32,
    temperature: f32,
}

impl StrictJsonClassifierAdapter {
    /// Construct an adapter with the default parser
    /// ([`SerdeJsonClassifierParser`]) and default token / temperature
    /// knobs. `model` is the model id the wrapped inference port should
    /// route to (e.g. `qwen2.5-coder:14b`).
    pub fn new(inference: Arc<dyn IInferencePort>, model: impl Into<String>) -> Self {
        Self {
            inference,
            parser: Arc::new(SerdeJsonClassifierParser::new()),
            model: model.into(),
            max_tokens: DEFAULT_MAX_TOKENS,
            temperature: DEFAULT_TEMPERATURE,
        }
    }

    /// Construct an adapter with an injected parser. Used by the unit
    /// tests to mock parse outcomes deterministically.
    pub fn with_parser(
        inference: Arc<dyn IInferencePort>,
        parser: Arc<dyn ClassifierParser>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            inference,
            parser,
            model: model.into(),
            max_tokens: DEFAULT_MAX_TOKENS,
            temperature: DEFAULT_TEMPERATURE,
        }
    }

    /// Override the per-call token cap (default `DEFAULT_MAX_TOKENS`).
    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = max_tokens;
        self
    }

    /// Override the sampling temperature (default `DEFAULT_TEMPERATURE`).
    pub fn with_temperature(mut self, temperature: f32) -> Self {
        self.temperature = temperature;
        self
    }

    /// Classify an inbound persona message.
    ///
    /// `operator_message` is the raw inbound text the persona is being
    /// asked to act on; it becomes the user turn in the inference request.
    /// `from_operator` is the upstream identity check — `true` means the
    /// message came directly from the human operator, which forbids
    /// `defer` and `reject` decisions (enforced by the parser).
    ///
    /// On success returns a fully-validated [`ClassifierResponse`].
    ///
    /// On failure returns the first non-retryable [`InvariantError`] (any
    /// variant other than `MalformedJson`) or — if the reparse budget is
    /// exhausted — the last [`InvariantError::MalformedJson`] observed.
    pub async fn classify(
        &self,
        operator_message: &str,
        from_operator: bool,
    ) -> Result<ClassifierResponse, InvariantError> {
        self.classify_with_attempts(STRICT_JSON_SYSTEM_PROMPT, operator_message, from_operator)
            .await
            .map(|(resp, _attempts)| resp)
    }

    /// Same retry contract as [`Self::classify`] but takes an explicit
    /// persona-flavoured `system_prompt` and reports how many inference
    /// attempts were consumed. Used by `org_responder` (P4.1) so the
    /// `classifier_response` STDB row captures the attempt count alongside
    /// the parsed decision, and so each persona can ship its own
    /// JSON-schema system prompt instead of the generic
    /// [`STRICT_JSON_SYSTEM_PROMPT`].
    ///
    /// The reparse-hint suffix is appended to the supplied `system_prompt`
    /// verbatim — callers should ensure their prompt already declares the
    /// strict-JSON output contract documented in the constant above.
    pub async fn classify_with_attempts(
        &self,
        system_prompt: &str,
        user_msg: &str,
        from_operator: bool,
    ) -> Result<(ClassifierResponse, u32), InvariantError> {
        // The user turn carries the inbound message plus a from=operator
        // annotation so the model has the same identity signal the parser
        // will enforce. We keep this rendering centralized so tests can
        // verify the inference adapter receives a deterministic prompt.
        let user_text = render_user_turn(user_msg, from_operator);

        let mut last_malformed: Option<InvariantError> = None;
        // attempt = 0 is the first try; attempts 1..=REPARSE_BUDGET are reparse retries.
        for attempt in 0..=REPARSE_BUDGET {
            let request =
                self.build_request(system_prompt, &user_text, last_malformed.as_ref());
            let response = match self.inference.complete(request).await {
                Ok(r) => r,
                Err(e) => {
                    // Inference transport failures are mapped onto MalformedJson
                    // so the reparse budget governs them the same way — a
                    // transient 5xx or network blip burns the same retry slot
                    // as a malformed completion. Non-transient failures
                    // (UnknownProvider etc.) also surface here; the caller's
                    // escalation path is identical regardless.
                    last_malformed = Some(InvariantError::MalformedJson(format!(
                        "inference call failed on attempt {}: {}",
                        attempt + 1,
                        e
                    )));
                    continue;
                }
            };

            let raw = extract_text(&response.content);
            match self.parser.parse(&raw, from_operator) {
                Ok(resp) => return Ok((resp, attempt as u32 + 1)),
                // Schema invariants are not retryable — escalate immediately.
                // attempts=1 because the model produced a parseable JSON object,
                // it's just one whose decision violates the contract; we burned
                // exactly one inference call to learn that.
                Err(e @ InvariantError::DecisionNotAllowedForOperator(_)) => {
                    return Err(e)
                }
                Err(e @ InvariantError::MissingRequiredField { .. }) => return Err(e),
                // MalformedJson consumes a retry slot and feeds the error back
                // into the next prompt so the model has a hint about what
                // went wrong.
                Err(e @ InvariantError::MalformedJson(_)) => {
                    last_malformed = Some(e);
                    continue;
                }
            }
        }

        // Budget exhausted — return the last MalformedJson we saw. The loop
        // guarantees we ran at least once, so `last_malformed` is `Some`.
        Err(last_malformed.unwrap_or_else(|| {
            InvariantError::MalformedJson(
                "classifier loop exited without producing a result".to_string(),
            )
        }))
    }

    /// Build the per-attempt inference request. When `last_error` is
    /// `Some` (reparse pass), the system prompt gets a trailing
    /// "previous-attempt failed with X" hint so the model knows what to
    /// fix. `base_system_prompt` is the caller-supplied (persona-flavoured)
    /// prompt; the reparse hint is appended to it verbatim.
    fn build_request(
        &self,
        base_system_prompt: &str,
        user_text: &str,
        last_error: Option<&InvariantError>,
    ) -> InferenceRequest {
        let system_prompt = match last_error {
            None => base_system_prompt.to_string(),
            Some(InvariantError::MalformedJson(msg)) => format!(
                "{base_system_prompt}\n\nPREVIOUS ATTEMPT FAILED — your last response could not be parsed as JSON:\n{msg}\n\nReturn ONLY a valid JSON object this time. No prose, no fences, no commentary."
            ),
            Some(other) => format!(
                "{base_system_prompt}\n\nPREVIOUS ATTEMPT FAILED: {other}"
            ),
        };

        InferenceRequest {
            model: self.model.clone(),
            system_prompt,
            messages: vec![Message::user(user_text)],
            tools: vec![],
            max_tokens: self.max_tokens,
            temperature: self.temperature,
            thinking_budget: None,
            cache_control: false,
            priority: Priority::Normal,
            grammar: None,
        }
    }
}

/// Render the user turn deterministically so tests and prod see the same shape.
fn render_user_turn(operator_message: &str, from_operator: bool) -> String {
    let from = if from_operator { "operator" } else { "peer" };
    format!("from={from}\n\n{operator_message}")
}

/// Extract concatenated text from an `InferenceResponse.content` vec. Non-text
/// blocks (tool_use, tool_result) are skipped — the classifier prompt
/// explicitly forbids tool calls, so any non-text content is treated as
/// noise.
fn extract_text(blocks: &[ContentBlock]) -> String {
    blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestration::classifier_types::ClassifierDecision;
    use async_trait::async_trait;
    use hex_core::domain::messages::StopReason;
    use hex_core::ports::inference::{
        futures_stream, HealthStatus, InferenceCapabilities, InferenceError, InferenceResponse,
        ModelInfo, ModelTier, StreamChunk,
    };
    use std::sync::Mutex;

    // -----------------------------------------------------------------------
    // Scripted mock inference port
    //
    // hex-core ships `MockInferencePort` for the simple "always return text X"
    // case, but the reparse-budget tests need a port that returns a
    // *different* response on each call. We roll a tiny scripted port here:
    // the constructor takes a Vec of canned responses and pop_front()s one
    // per `complete` invocation. The system_prompt observed on each call is
    // captured so a test can assert the reparse hint was appended.
    // -----------------------------------------------------------------------

    struct ScriptedInference {
        responses: Mutex<std::collections::VecDeque<Result<String, InferenceError>>>,
        observed_system_prompts: Mutex<Vec<String>>,
    }

    impl ScriptedInference {
        fn new(responses: Vec<Result<String, InferenceError>>) -> Arc<Self> {
            Arc::new(Self {
                responses: Mutex::new(responses.into()),
                observed_system_prompts: Mutex::new(Vec::new()),
            })
        }

        fn observed_prompts(&self) -> Vec<String> {
            self.observed_system_prompts.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl IInferencePort for ScriptedInference {
        async fn complete(
            &self,
            request: InferenceRequest,
        ) -> Result<InferenceResponse, InferenceError> {
            self.observed_system_prompts
                .lock()
                .unwrap()
                .push(request.system_prompt.clone());
            let next = self
                .responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("ScriptedInference ran out of canned responses");
            match next {
                Ok(text) => Ok(InferenceResponse {
                    content: vec![ContentBlock::Text { text }],
                    model_used: "scripted".to_string(),
                    stop_reason: StopReason::EndTurn,
                    input_tokens: 0,
                    output_tokens: 0,
                    cache_read_tokens: 0,
                    cache_write_tokens: 0,
                    latency_ms: 0,
                }),
                Err(e) => Err(e),
            }
        }

        async fn stream(
            &self,
            _request: InferenceRequest,
        ) -> Result<
            Box<dyn futures_stream::Stream<Item = StreamChunk> + Send + Unpin>,
            InferenceError,
        > {
            unimplemented!("classifier adapter uses complete(), not stream()")
        }

        async fn health(&self) -> Result<HealthStatus, InferenceError> {
            Ok(HealthStatus::Ok { models: vec![] })
        }

        fn capabilities(&self) -> InferenceCapabilities {
            InferenceCapabilities {
                models: vec![ModelInfo {
                    id: "scripted".to_string(),
                    provider: "scripted".to_string(),
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

    // Canonical happy-path classifier JSON used in multiple tests.
    const VALID_ACCEPT_JSON: &str = r#"{"decision":"accept","tool_plan":[{"tool":"repo_grep","intent":"find call sites"}],"cost_usd":0.0012}"#;
    const VALID_CLARIFY_JSON: &str = r#"{"decision":"clarify","question":"which workplan?","cost_usd":0.0008}"#;

    // -----------------------------------------------------------------------
    // Happy path
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn happy_path_returns_parsed_response_in_one_call() {
        let inference = ScriptedInference::new(vec![Ok(VALID_ACCEPT_JSON.to_string())]);
        let adapter = StrictJsonClassifierAdapter::new(inference.clone(), "scripted");

        let out = adapter
            .classify("please run cargo check", true)
            .await
            .expect("classifier should accept valid JSON on first try");

        assert!(matches!(out.decision, ClassifierDecision::Accept));
        assert_eq!(out.tool_plan.as_ref().unwrap().len(), 1);
        // Exactly one inference call — no retries on happy path.
        assert_eq!(inference.observed_prompts().len(), 1);
    }

    #[tokio::test]
    async fn happy_path_clarify_from_operator() {
        let inference = ScriptedInference::new(vec![Ok(VALID_CLARIFY_JSON.to_string())]);
        let adapter = StrictJsonClassifierAdapter::new(inference, "scripted");

        let out = adapter
            .classify("the workplan thing", true)
            .await
            .expect("clarify is allowed from operator");
        assert!(matches!(out.decision, ClassifierDecision::Clarify));
    }

    #[tokio::test]
    async fn happy_path_strips_markdown_fences() {
        let fenced = format!("```json\n{}\n```", VALID_ACCEPT_JSON);
        let inference = ScriptedInference::new(vec![Ok(fenced)]);
        let adapter = StrictJsonClassifierAdapter::new(inference.clone(), "scripted");
        let out = adapter.classify("ship it", true).await.expect("fenced ok");
        assert!(matches!(out.decision, ClassifierDecision::Accept));
        // Still only one inference call — fence-stripping does not burn a retry.
        assert_eq!(inference.observed_prompts().len(), 1);
    }

    // -----------------------------------------------------------------------
    // Reparse retry path — the main P3.1 contract
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn malformed_then_valid_succeeds_within_budget() {
        // Attempt 1: malformed prose. Attempt 2: valid JSON.
        let inference = ScriptedInference::new(vec![
            Ok("I think we should accept this".to_string()),
            Ok(VALID_ACCEPT_JSON.to_string()),
        ]);
        let adapter = StrictJsonClassifierAdapter::new(inference.clone(), "scripted");

        let out = adapter
            .classify("ship the fix", true)
            .await
            .expect("retry should succeed after malformed attempt");
        assert!(matches!(out.decision, ClassifierDecision::Accept));

        let prompts = inference.observed_prompts();
        assert_eq!(prompts.len(), 2, "expected exactly one retry");
        // Attempt 1 has the base prompt; attempt 2 has the reparse hint.
        assert!(!prompts[0].contains("PREVIOUS ATTEMPT FAILED"));
        assert!(prompts[1].contains("PREVIOUS ATTEMPT FAILED"));
    }

    #[tokio::test]
    async fn two_malformed_then_valid_succeeds_at_budget_edge() {
        // Attempts 1+2: malformed. Attempt 3: valid (this is the last allowed
        // try — `REPARSE_BUDGET=2` means up to 1 + 2 = 3 attempts total).
        let inference = ScriptedInference::new(vec![
            Ok("nope, just words".to_string()),
            Ok(r#"{decision: "accept"}"#.to_string()), // still bad — unquoted key
            Ok(VALID_ACCEPT_JSON.to_string()),
        ]);
        let adapter = StrictJsonClassifierAdapter::new(inference.clone(), "scripted");

        let out = adapter
            .classify("third time lucky", true)
            .await
            .expect("third attempt should land");
        assert!(matches!(out.decision, ClassifierDecision::Accept));
        assert_eq!(inference.observed_prompts().len(), 3);
    }

    #[tokio::test]
    async fn three_malformed_exhausts_budget_and_returns_last_malformed() {
        // All 3 attempts malformed → budget exhausted → return last
        // MalformedJson. The fourth would never be made.
        let inference = ScriptedInference::new(vec![
            Ok("first bad".to_string()),
            Ok("second bad".to_string()),
            Ok("third bad".to_string()),
        ]);
        let adapter = StrictJsonClassifierAdapter::new(inference.clone(), "scripted");

        let err = adapter
            .classify("hopeless", true)
            .await
            .expect_err("exhausted budget should error");
        assert!(matches!(err, InvariantError::MalformedJson(_)));
        assert_eq!(inference.observed_prompts().len(), 3);
    }

    // -----------------------------------------------------------------------
    // Non-retryable invariants — must NOT consume the retry budget
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn operator_invariant_violation_escalates_immediately() {
        // Defer from operator — schema violation, not retryable. Even though
        // there are more responses queued, the adapter must NOT call inference
        // a second time.
        let inference = ScriptedInference::new(vec![
            Ok(r#"{"decision":"defer","reason":"later","cost_usd":0.0}"#.to_string()),
            // Sentinel that would panic ScriptedInference if it were popped
            // (because the test would now over-read the deque). It's also
            // an Err so we catch any accidental progress.
            Err(InferenceError::ProviderUnavailable("should not be called".into())),
        ]);
        let adapter = StrictJsonClassifierAdapter::new(inference.clone(), "scripted");

        let err = adapter
            .classify("do something later", true)
            .await
            .expect_err("operator+defer must escalate");
        match err {
            InvariantError::DecisionNotAllowedForOperator(ClassifierDecision::Defer) => {}
            other => panic!("expected DecisionNotAllowedForOperator(Defer), got {other:?}"),
        }
        // Exactly one inference call — no retry on schema violations.
        assert_eq!(inference.observed_prompts().len(), 1);
    }

    #[tokio::test]
    async fn missing_required_field_escalates_immediately() {
        // Accept without tool_plan — required-field violation. Same rule:
        // do not retry.
        let inference = ScriptedInference::new(vec![
            Ok(r#"{"decision":"accept","cost_usd":0.0}"#.to_string()),
            Err(InferenceError::ProviderUnavailable("should not be called".into())),
        ]);
        let adapter = StrictJsonClassifierAdapter::new(inference.clone(), "scripted");

        let err = adapter
            .classify("ship it", false)
            .await
            .expect_err("missing tool_plan must escalate");
        match err {
            InvariantError::MissingRequiredField {
                decision: ClassifierDecision::Accept,
                field: "tool_plan",
            } => {}
            other => panic!("expected MissingRequiredField, got {other:?}"),
        }
        assert_eq!(inference.observed_prompts().len(), 1);
    }

    // -----------------------------------------------------------------------
    // Transport-failure handling — inference errors burn retry slots
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn transport_failure_consumes_retry_slot_then_succeeds() {
        let inference = ScriptedInference::new(vec![
            Err(InferenceError::ProviderUnavailable("transient blip".into())),
            Ok(VALID_ACCEPT_JSON.to_string()),
        ]);
        let adapter = StrictJsonClassifierAdapter::new(inference.clone(), "scripted");

        let out = adapter
            .classify("retry after blip", true)
            .await
            .expect("should recover after transient failure");
        assert!(matches!(out.decision, ClassifierDecision::Accept));
        assert_eq!(inference.observed_prompts().len(), 2);
    }

    #[tokio::test]
    async fn transport_failure_three_times_exhausts_budget() {
        let inference = ScriptedInference::new(vec![
            Err(InferenceError::ProviderUnavailable("blip 1".into())),
            Err(InferenceError::ProviderUnavailable("blip 2".into())),
            Err(InferenceError::ProviderUnavailable("blip 3".into())),
        ]);
        let adapter = StrictJsonClassifierAdapter::new(inference.clone(), "scripted");

        let err = adapter
            .classify("flaky network", true)
            .await
            .expect_err("three transport failures must exhaust budget");
        assert!(matches!(err, InvariantError::MalformedJson(_)));
        assert_eq!(inference.observed_prompts().len(), 3);
    }

    // -----------------------------------------------------------------------
    // Prompt rendering
    // -----------------------------------------------------------------------

    #[test]
    fn render_user_turn_includes_from_operator_marker() {
        let s = render_user_turn("hello", true);
        assert!(s.starts_with("from=operator\n\n"));
        assert!(s.contains("hello"));
    }

    #[test]
    fn render_user_turn_marks_peer_traffic() {
        let s = render_user_turn("hello", false);
        assert!(s.starts_with("from=peer\n\n"));
    }

    #[test]
    fn extract_text_concatenates_text_blocks_and_skips_non_text() {
        use serde_json::json;
        let blocks = vec![
            ContentBlock::Text { text: "abc".into() },
            ContentBlock::ToolUse {
                id: "t1".into(),
                name: "noop".into(),
                input: json!({}),
            },
            ContentBlock::Text { text: "def".into() },
        ];
        assert_eq!(extract_text(&blocks), "abcdef");
    }

    #[tokio::test]
    async fn builder_overrides_apply_to_request() {
        let inference = ScriptedInference::new(vec![Ok(VALID_ACCEPT_JSON.to_string())]);
        let adapter = StrictJsonClassifierAdapter::new(inference.clone(), "scripted")
            .with_max_tokens(42)
            .with_temperature(0.7);
        // The adapter's `build_request` is private; round-trip via classify
        // and assert via the observed-prompts hook on the mock. We cannot
        // observe max_tokens directly through the mock, but we can at least
        // confirm classify still works with the overrides.
        let _ = adapter
            .classify("ok", true)
            .await
            .expect("overrides must not break classify");
        assert_eq!(inference.observed_prompts().len(), 1);
    }
}
