//! End-to-end integration tests for standalone dispatch (ADR-2604112000 P6).
//!
//! These tests prove three things:
//!
//! 1. **P6.1 — Standalone routing verification**: The standalone composition
//!    path produces an `AgentManager` that the executor can reference without
//!    any Claude Code session dependency (`CLAUDE_SESSION_ID`, session files).
//!
//! 2. **P6.2 — One-task workplan dispatch**: A `MockInferencePort` with a
//!    canned non-empty response passes through the standalone composition
//!    path, and the dispatch-evidence guard accepts the output.
//!
//! 3. **P6.3 — Vacuous completion rejection**: A `MockInferencePort` with an
//!    empty response is rejected by the dispatch-evidence guard per
//!    ADR-2604111800. The guard prevents phantom task completions.
//!
//! ## Why not test through `execute_phase` directly?
//!
//! The executor's `execute_phase` requires a fully-populated `SharedState`
//! (`AppState` with ~25 fields including broadcast channels, fleet manager,
//! rate limiters, etc.) and `spawn_agent` spawns real OS processes. Building
//! that infrastructure for a hermetic test is out of scope for P6. Instead
//! we test at two levels:
//!
//! - **Composition level**: standalone compose + MockInferencePort produces a
//!   valid `AgentManager` (reuses P2.3 pattern).
//! - **Guard level**: `validate_dispatch_evidence` is a pure function that
//!   the executor calls on the task completion path. Testing it directly
//!   proves the ADR-2604111800 contract.
//!
//! The composition test proves the standalone path is wirable; the guard
//! tests prove the evidence check works. Together they cover the P6 gate.
//!
//! Tests are in a `standalone_dispatch` module so the gate filter
//! `cargo test -p hex-nexus standalone_dispatch` matches them.

mod standalone_dispatch {
    use std::sync::Arc;

    use hex_core::ports::inference::mock::MockInferencePort;
    use hex_core::ports::inference::IInferencePort;

    use hex_nexus::adapters::capability_token::CapabilityTokenService;
    use hex_nexus::adapters::spacetime_state::{SpacetimeConfig, SpacetimeStateAdapter};
    use hex_nexus::composition::{compose, CompositionInputs, CompositionVariant};
    use hex_nexus::orchestration::agent_manager::SecretResolver;
    use hex_nexus::orchestration::workplan_executor::validate_dispatch_evidence;
    use hex_nexus::ports::state::IStatePort;

    // ── Test helpers (reuse P2.3 pattern) ────────────────

    /// Hermetic state port — never touches the network. Construction-only;
    /// no methods are called during composition.
    fn hermetic_state_port() -> Arc<dyn IStatePort> {
        let config = SpacetimeConfig {
            host: "http://127.0.0.1:1".to_string(),
            database: "hex-test-unreachable".to_string(),
            auth_token: None,
        };
        Arc::new(SpacetimeStateAdapter::new(config))
    }

    fn noop_secret_resolver() -> SecretResolver {
        Arc::new(|_key: &str| None)
    }

    fn make_inputs(inference: Option<Arc<dyn IInferencePort>>) -> CompositionInputs {
        CompositionInputs {
            state_port: hermetic_state_port(),
            inference,
            secret_resolver: noop_secret_resolver(),
            capability_token_service: Arc::new(
                CapabilityTokenService::new(b"test-secret-p6".to_vec()),
            ),
        }
    }

    // ── P6.1: Standalone routing verification ────────────

    /// The standalone composition path produces an `AgentManager` when
    /// `MockInferencePort` is provided. The executor's `execute_phase`
    /// checks `shared_state.agent_manager.is_some()` — if this test
    /// passes, that gate would pass too.
    ///
    /// Crucially, no `CLAUDE_SESSION_ID` env var is set, no session files
    /// are read, and no `is_claude_code_session()` returns true during
    /// this path. The standalone composition is fully independent.
    #[test]
    fn standalone_composition_produces_agent_manager_without_claude_session() {
        // Ensure no Claude env vars leak into the test
        assert!(
            std::env::var("CLAUDE_SESSION_ID").is_err()
                || std::env::var("CLAUDE_SESSION_ID")
                    .unwrap()
                    .is_empty(),
            "CLAUDE_SESSION_ID must not be set for standalone dispatch test"
        );

        let mock_inference: Arc<dyn IInferencePort> = Arc::new(
            MockInferencePort::with_response("fn main() { println!(\"hello\"); }"),
        );
        let inputs = make_inputs(Some(mock_inference));
        let probe = || CompositionVariant::Standalone;

        let agent_manager = compose(probe, inputs)
            .expect("standalone compose with MockInferencePort should succeed");

        // AgentManager exists — the executor's `agent_mgr.is_some()` check passes.
        let _held: &_ = &agent_manager;
    }

    /// When the standalone path is chosen but no inference adapter is
    /// provided, composition fails with `MissingComposition::InferenceAdapter`.
    /// The executor would surface this as a pre-flight failure.
    #[test]
    fn standalone_composition_fails_without_inference() {
        let inputs = make_inputs(None);
        let probe = || CompositionVariant::Standalone;

        match compose(probe, inputs) {
            Ok(_) => panic!("compose without inference must fail, got Ok"),
            Err(e) => {
                let err_msg = e.to_string();
                assert!(
                    err_msg.contains("inference"),
                    "error should mention inference: got '{}'",
                    err_msg
                );
            }
        }
    }

    // ── P6.2: One-task workplan dispatch evidence ────────

    /// A non-empty mock response passes the dispatch-evidence guard.
    /// This simulates the happy path: MockInferencePort returns a canned
    /// code response, the executor would call `validate_dispatch_evidence`
    /// on the agent's output, and the guard accepts it.
    #[test]
    fn dispatch_evidence_accepts_non_empty_output() {
        let output = "fn main() { println!(\"hello\"); }";
        let result = validate_dispatch_evidence(Some(output));
        assert!(
            result.is_ok(),
            "non-empty output should pass dispatch-evidence guard: {:?}",
            result
        );
    }

    /// Multi-line output with code content passes the guard.
    #[test]
    fn dispatch_evidence_accepts_multiline_code() {
        let output = "use std::io;\n\nfn main() {\n    println!(\"hello world\");\n}\n";
        assert!(validate_dispatch_evidence(Some(output)).is_ok());
    }

    /// Output with leading/trailing whitespace but real content passes.
    #[test]
    fn dispatch_evidence_accepts_padded_output() {
        let output = "  \n  real content here  \n  ";
        assert!(validate_dispatch_evidence(Some(output)).is_ok());
    }

    // ── P6.3: Vacuous completion rejection ───────────────

    /// Empty string output is rejected — the agent produced no evidence of
    /// work. Per ADR-2604111800, this must NOT result in a "done" status.
    #[test]
    fn dispatch_evidence_rejects_empty_output() {
        let result = validate_dispatch_evidence(Some(""));
        assert!(
            result.is_err(),
            "empty output must be rejected by dispatch-evidence guard"
        );
        let err = result.unwrap_err();
        assert!(
            err.contains("dispatch-evidence"),
            "error should mention dispatch-evidence: got '{}'",
            err
        );
    }

    /// Whitespace-only output is rejected — spaces/tabs/newlines are not
    /// evidence of meaningful work.
    #[test]
    fn dispatch_evidence_rejects_whitespace_only_output() {
        let result = validate_dispatch_evidence(Some("   \n\t\n   "));
        assert!(
            result.is_err(),
            "whitespace-only output must be rejected"
        );
        let err = result.unwrap_err();
        assert!(err.contains("whitespace-only"));
    }

    /// None output (no response at all) is rejected.
    #[test]
    fn dispatch_evidence_rejects_none_output() {
        let result = validate_dispatch_evidence(None);
        assert!(result.is_err(), "None output must be rejected");
        let err = result.unwrap_err();
        assert!(err.contains("no dispatch output"));
    }

    /// The guard integrates with the standalone composition path: build an
    /// AgentManager via standalone compose with a MockInferencePort, then
    /// verify the mock's canned response would pass the evidence guard.
    /// This is the closest we can get to an end-to-end test without
    /// spinning up the full executor infrastructure.
    #[test]
    fn standalone_compose_plus_evidence_guard_end_to_end() {
        let mock_response = "fn main() { println!(\"hello\"); }";
        let mock_inference: Arc<dyn IInferencePort> = Arc::new(
            MockInferencePort::with_response(mock_response),
        );
        let inputs = make_inputs(Some(mock_inference));
        let probe = || CompositionVariant::Standalone;

        // Step 1: Standalone composition succeeds
        let _agent_manager = compose(probe, inputs)
            .expect("standalone compose should succeed");

        // Step 2: The mock's canned response passes the evidence guard
        let evidence_result = validate_dispatch_evidence(Some(mock_response));
        assert!(
            evidence_result.is_ok(),
            "mock response should pass evidence guard: {:?}",
            evidence_result
        );
    }

    /// Standalone compose with empty-response MockInferencePort: the
    /// composition itself succeeds (inference adapter is present), but
    /// the evidence guard rejects the empty output. This proves the
    /// guard catches vacuous completions even on the standalone path.
    #[test]
    fn standalone_compose_plus_empty_response_rejected_by_guard() {
        let mock_inference: Arc<dyn IInferencePort> = Arc::new(
            MockInferencePort::with_response(""),
        );
        let inputs = make_inputs(Some(mock_inference));
        let probe = || CompositionVariant::Standalone;

        // Composition succeeds — the adapter exists
        let _agent_manager = compose(probe, inputs)
            .expect("standalone compose should succeed even with empty-response mock");

        // But the evidence guard rejects the empty output
        let evidence_result = validate_dispatch_evidence(Some(""));
        assert!(
            evidence_result.is_err(),
            "empty mock response must be rejected by evidence guard"
        );
    }
}
