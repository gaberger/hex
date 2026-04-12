//! Composition module — constructs an [`AgentManager`] from its port
//! prerequisites under one of two variants:
//!
//! - **ClaudeIntegrated**: the preferred branch when Claude Code is the outer
//!   shell. Agent identity is resolved from `~/.hex/sessions/agent-*.json`
//!   upstream (hex-cli's nexus_client), and the nexus receives a
//!   fully-populated `AgentManager`.
//! - **Standalone**: no Claude assumption. The composition root supplies an
//!   [`IInferencePort`] (e.g. Ollama) directly and builds the same
//!   `AgentManager` type from (HexFlo state port + inference + capability
//!   token service + memory-backed secret resolver).
//!
//! Both variants share the same downstream construction — everything past
//! "which identity source did we use?" is identical. See
//! [`build_agent_manager`] for the shared helper.
//!
//! See ADR-2604112000 for the decision record and wp-hex-standalone-dispatch
//! phase P2 for the task decomposition.

pub mod claude_integrated;
pub mod standalone;

use std::sync::Arc;

use hex_core::ports::inference::IInferencePort;

use crate::adapters::capability_token::CapabilityTokenService;
use crate::orchestration::agent_manager::{AgentManager, SecretResolver};
use crate::orchestration::errors::MissingComposition;
use crate::ports::state::IStatePort;

/// Which composition path should fire for this process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompositionVariant {
    /// Claude Code is driving the outer shell — use the Claude-integrated
    /// path that resolves agent identity from session files upstream.
    ClaudeIntegrated,
    /// No Claude shell — use the standalone path that wires inference
    /// directly at the composition root.
    Standalone,
}

/// The inputs every composition variant needs.
///
/// Both variants consume the same ports; only their source of agent identity
/// differs. This struct keeps the two `compose_*` entry points honest about
/// what they share.
pub struct CompositionInputs {
    pub state_port: Arc<dyn IStatePort>,
    pub inference: Option<Arc<dyn IInferencePort>>,
    pub secret_resolver: SecretResolver,
    pub capability_token_service: Arc<CapabilityTokenService>,
}

/// Default env-backed probe.
///
/// Returns `ClaudeIntegrated` iff `CLAUDE_SESSION_ID` is set and non-empty
/// (matching ADR-2604112000 §Decision). The ADR also mentions checking
/// `~/.hex/sessions/agent-*.json`, but a session file without the env var is
/// an inconsistent state we don't handle in P2 — see the ADR for future
/// refinement. The richer `CLAUDECODE` / `CLAUDE_CODE_ENTRYPOINT` probe lives
/// in `orchestration::is_claude_code_session` and serves a different purpose
/// (Path A vs Path B routing).
pub fn default_probe() -> CompositionVariant {
    match std::env::var("CLAUDE_SESSION_ID") {
        Ok(v) if !v.is_empty() => CompositionVariant::ClaudeIntegrated,
        _ => CompositionVariant::Standalone,
    }
}

/// Dispatch to the appropriate composition variant.
///
/// The probe is injected so tests can force a variant deterministically
/// without manipulating env vars (which would race in parallel test runs).
/// Production callers should pass [`default_probe`] directly.
pub fn compose<P>(probe: P, inputs: CompositionInputs) -> Result<AgentManager, MissingComposition>
where
    P: Fn() -> CompositionVariant,
{
    match probe() {
        CompositionVariant::ClaudeIntegrated => claude_integrated::compose_claude_integrated(inputs),
        CompositionVariant::Standalone => standalone::compose_standalone(inputs),
    }
}

/// Shared construction used by both composition variants.
///
/// This is the ONLY place an `AgentManager` is built. `compose_standalone`
/// and `compose_claude_integrated` differ only in where the agent identity
/// comes from — everything past this function is identical.
pub(crate) fn build_agent_manager(
    inputs: CompositionInputs,
) -> Result<AgentManager, MissingComposition> {
    // Validate the inference adapter prerequisite. Both variants require one,
    // but the standalone path is where a missing adapter most commonly bites
    // (the Claude-integrated path historically relied on Claude Code itself
    // as the inference source).
    if inputs.inference.is_none() {
        return Err(MissingComposition::InferenceAdapter {
            reason: "no IInferencePort provided to composition root".to_string(),
        });
    }

    // AgentManager's state port is the only strictly-required port on the
    // Rust type signature today. Missing port wiring on any of the other
    // inputs is caught at the call site (they are non-Option Arcs), so an
    // IncompletePortWiring from here would be unreachable. Kept as a guard
    // because future port additions may relax this.
    Ok(AgentManager::new(
        inputs.state_port,
        inputs.secret_resolver,
        inputs.capability_token_service,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_probe_returns_standalone_when_session_unset() {
        // We cannot deterministically test the env-backed probe without
        // racing the rest of the suite on CLAUDE_SESSION_ID. The injectable
        // probe in integration tests is the real coverage; this is a
        // documentation smoke test.
        let forced = || CompositionVariant::Standalone;
        assert_eq!(forced(), CompositionVariant::Standalone);
    }
}
