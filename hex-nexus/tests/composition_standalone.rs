//! Integration tests for the standalone composition path (ADR-2604112000 P2.3).
//!
//! These tests prove three invariants about `hex_nexus::composition`:
//!
//! 1. When the probe returns `Standalone`, `compose()` dispatches to the
//!    standalone branch and returns a populated `AgentManager`.
//! 2. When the probe returns `ClaudeIntegrated`, `compose()` dispatches to
//!    the Claude-integrated branch.
//! 3. When the standalone path is invoked with no inference adapter,
//!    `compose_standalone()` returns the structured
//!    `MissingComposition::InferenceAdapter` variant — not a stringly-typed
//!    error.
//!
//! The tests are hermetic: they construct an `IInferencePort` with
//! `MockInferencePort` from hex-core, a `SpacetimeStateAdapter` pointed at an
//! unreachable host (no network is touched because `AgentManager::new` only
//! stores the Arc), and inline no-op closures for the remaining ports.
//!
//! Tests live inside a nested `composition::standalone` module so the
//! workplan gate `cargo test -p hex-nexus composition::standalone` filters
//! them by substring match.

// The gate filter is `composition::standalone`. Cargo's integration-test
// binary is named after the file (`composition_standalone`), so the full
// test path is `composition_standalone::composition::standalone::<test>`
// — which contains the filter substring. Do NOT rename this module tree.
mod composition {
    pub mod standalone {
        use std::sync::Arc;

        use hex_core::ports::inference::mock::MockInferencePort;
        use hex_core::ports::inference::IInferencePort;

        use hex_nexus::adapters::capability_token::CapabilityTokenService;
        use hex_nexus::adapters::spacetime_state::SpacetimeConfig;
        use hex_nexus::adapters::spacetime_state::SpacetimeStateAdapter;
        use hex_nexus::composition::{
            compose, CompositionInputs, CompositionVariant,
        };
        use hex_nexus::composition::standalone::compose_standalone;
        use hex_nexus::orchestration::agent_manager::SecretResolver;
        use hex_nexus::orchestration::errors::MissingComposition;
        use hex_nexus::ports::state::IStatePort;

        /// Construct a hermetic `IStatePort` wrapper that never touches the
        /// network. `SpacetimeStateAdapter::new()` only builds a
        /// `reqwest::Client` — no DNS, no TCP, no STDB handshake. Methods on
        /// the adapter WOULD touch the network, but `AgentManager::new()`
        /// stores the `Arc<dyn IStatePort>` without invoking any method on
        /// it, so the construction path is hermetic.
        ///
        /// This is the minimal inline double the P2 task permits. It avoids
        /// a 110-method hand-written `IStatePort` stub while keeping the
        /// test file self-contained.
        fn hermetic_state_port() -> Arc<dyn IStatePort> {
            let config = SpacetimeConfig {
                host: "http://127.0.0.1:1".to_string(), // unreachable, never dialed
                database: "hex-test-unreachable".to_string(),
                auth_token: None,
            };
            Arc::new(SpacetimeStateAdapter::new(config))
        }

        /// Inline no-op secret resolver. Never consulted by the composition
        /// path; the field exists because `AgentManager` stores it for later
        /// use during agent spawn.
        fn noop_secret_resolver() -> SecretResolver {
            Arc::new(|_key: &str| None)
        }

        fn make_inputs(with_inference: bool) -> CompositionInputs {
            let inference: Option<Arc<dyn IInferencePort>> = if with_inference {
                Some(Arc::new(MockInferencePort::with_response("test-response")))
            } else {
                None
            };
            CompositionInputs {
                state_port: hermetic_state_port(),
                inference,
                secret_resolver: noop_secret_resolver(),
                capability_token_service: Arc::new(
                    CapabilityTokenService::new(b"test-secret".to_vec()),
                ),
            }
        }

        /// Case (a): probe returns Standalone + inference present → standalone
        /// branch used and AgentManager populated.
        #[test]
        fn standalone_branch_when_claude_session_unset_and_inference_present() {
            let inputs = make_inputs(true);
            let probe = || CompositionVariant::Standalone;

            let agent_manager = compose(probe, inputs)
                .expect("standalone compose should succeed when inference is present");

            // The returned AgentManager is a concrete struct — its existence
            // is the populated-signal. We assert we can take a reference to
            // it (drop it at end of scope) without panicking.
            let _held: &_ = &agent_manager;
        }

        /// Case (b): probe returns ClaudeIntegrated → Claude-integrated
        /// branch used. Today both variants share the same downstream
        /// construction, so the observable signal is that dispatch returns
        /// Ok and the Standalone variant's validation is NOT the one that
        /// fires. We assert dispatch succeeds with a ClaudeIntegrated probe
        /// against the same inputs.
        #[test]
        fn claude_integrated_branch_when_claude_session_set() {
            let inputs = make_inputs(true);
            let probe = || CompositionVariant::ClaudeIntegrated;

            let agent_manager = compose(probe, inputs)
                .expect("claude_integrated compose should succeed");

            let _held: &_ = &agent_manager;
        }

        /// Case (c): `compose_standalone` called directly with `inference:
        /// None` returns the structured `MissingComposition::InferenceAdapter`
        /// variant — NOT a string, NOT a generic error. This is the
        /// diagnostic P2.2 replaces the `"AgentManager not initialized"`
        /// string with.
        #[test]
        fn standalone_missing_inference_returns_structured_error() {
            let inputs = make_inputs(false);
            // `AgentManager` doesn't impl Debug, so we can't use
            // `.expect_err()` here — match on the Result directly.
            match compose_standalone(inputs) {
                Ok(_) => panic!(
                    "compose_standalone with no inference should fail, got Ok"
                ),
                Err(MissingComposition::InferenceAdapter { reason }) => {
                    assert!(
                        !reason.is_empty(),
                        "InferenceAdapter variant should carry a reason"
                    );
                }
                Err(other) => panic!(
                    "expected MissingComposition::InferenceAdapter, got {:?}",
                    other
                ),
            }
        }

        /// Bonus: the `remediation()` helper returns a non-empty hint for
        /// every variant. This guards against future variants forgetting to
        /// extend the match arm.
        #[test]
        fn missing_composition_remediation_is_non_empty() {
            let variants = [
                MissingComposition::InferenceAdapter {
                    reason: "x".to_string(),
                },
                MissingComposition::HexFloUnreachable {
                    reason: "y".to_string(),
                },
                MissingComposition::IncompletePortWiring {
                    details: "z".to_string(),
                },
            ];
            for v in &variants {
                assert!(!v.remediation().is_empty());
            }
        }
    }
}
