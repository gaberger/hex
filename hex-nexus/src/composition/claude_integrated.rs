//! Claude-integrated composition path — the preferred branch when Claude
//! Code is the outer shell.
//!
//! Today the agent-identity resolution (reading
//! `~/.hex/sessions/agent-{CLAUDE_SESSION_ID}.json`) still lives upstream in
//! `hex-cli/src/nexus_client.rs`; this module is a thin dispatch target so
//! the compose() probe can branch. A future task may move the session-file
//! resolver into this module — see ADR-2604112000 §Consequences.
//!
//! For P2, the Claude-integrated path shares downstream construction with
//! the standalone path via `build_agent_manager`: every port past the
//! identity source is identical.

use crate::composition::{build_agent_manager, CompositionInputs};
use crate::orchestration::agent_manager::AgentManager;
use crate::orchestration::errors::MissingComposition;

/// Build an `AgentManager` for a process running inside a Claude Code session.
///
/// The current implementation delegates to the shared helper. The variant
/// exists as a separate entry point so the probe in `compose()` has
/// somewhere to dispatch and so the eventual session-file resolver has a
/// natural home.
pub fn compose_claude_integrated(
    inputs: CompositionInputs,
) -> Result<AgentManager, MissingComposition> {
    build_agent_manager(inputs)
}
