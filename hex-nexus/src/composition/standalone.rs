//! Standalone composition path — wires AgentManager from (state port +
//! inference adapter + secret resolver + capability token service) without
//! assuming Claude Code is the outer shell.
//!
//! This is the variant that fires when `CLAUDE_SESSION_ID` is unset in the
//! process environment. See `composition/mod.rs` for dispatch, ADR-2604112000
//! for the decision record, and wp-hex-standalone-dispatch P2.1/P3.3 for the
//! task descriptions.
//!
//! ## Default inference provider
//!
//! Per ADR-2604112000 §Decision, Ollama is the reference standalone provider.
//! Upstream callers that don't already have an [`IInferencePort`] to pass in
//! can use [`default_inference_adapter`] to get an
//! [`OllamaInferenceAdapter`] pointed at `OLLAMA_HOST` (or localhost).
//!
//! **Important**: this function does NOT probe health — composition must not
//! block on network at wire time. Callers are responsible for probing
//! `IInferencePort::health()` asynchronously before dispatching work, and
//! surfacing a `HealthStatus::Unreachable` into the doctor / CI gates from
//! wp-hex-standalone-dispatch P5.

use std::sync::Arc;

use hex_core::ports::inference::IInferencePort;

use crate::adapters::inference::OllamaInferenceAdapter;
use crate::composition::{build_agent_manager, CompositionInputs};
use crate::orchestration::agent_manager::AgentManager;
use crate::orchestration::errors::MissingComposition;

/// Build an `AgentManager` without reading any session files.
///
/// Returns a structured [`MissingComposition`] if a prerequisite is absent
/// (inference adapter, HexFlo reachability, or incomplete port wiring). The
/// actual construction is delegated to `build_agent_manager` so the standalone
/// and Claude-integrated paths share everything downstream of agent-identity
/// resolution.
pub fn compose_standalone(
    inputs: CompositionInputs,
) -> Result<AgentManager, MissingComposition> {
    build_agent_manager(inputs)
}

/// Construct the default standalone-mode inference adapter.
///
/// Today this is always an [`OllamaInferenceAdapter`] reading the
/// `OLLAMA_HOST` env var (default `http://localhost:11434`). Swapping
/// providers is future work — see wp-hex-standalone-dispatch P4 for
/// claude_code and future adapters for vLLM / OpenAI-compatible.
///
/// Wire-time contract: this function does **not** make any network calls.
/// Callers must `health()` asynchronously after construction.
pub fn default_inference_adapter() -> Arc<dyn IInferencePort> {
    Arc::new(OllamaInferenceAdapter::new(None))
}
