//! Standalone composition path — wires AgentManager from (state port +
//! inference adapter + secret resolver + capability token service) without
//! assuming Claude Code is the outer shell.
//!
//! This is the variant that fires when `CLAUDE_SESSION_ID` is unset in the
//! process environment. See `composition/mod.rs` for dispatch, ADR-2604112000
//! for the decision record, and wp-hex-standalone-dispatch P2.1 for the
//! task description.

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
