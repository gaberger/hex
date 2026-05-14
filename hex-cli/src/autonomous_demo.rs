//! Autonomous-write proof — `hex-cli/src/autonomous_demo.rs`.
//!
//! Written end-to-end via the hex autonomous SOP loop:
//!   POST /api/database/hex/call/proposed_action_open
//!     -> twin_reviewer (source-guard accepts operator-passthrough)
//!     -> action_executor::execute_file_write
//!     -> cargo_check gate (this file compiles, gate passes)
//!     -> action_executor::git_commit_executed_file
//!     -> commit on main with Co-Authored-By: hex-autonomous
//!
//! No persona LLM, no operator-Claude in the per-step loop. The bytes
//! in this file came from the operator's literal-content body sent
//! once via curl, the rest of the chain ran on its own.
//!
//! See ADR-2605141135 (hex-as-hermes-harness roadmap) for context;
//! commit `f33c7a37` for the original spec smoke; this file for the
//! source-guard exception proof.

#![allow(dead_code)]

/// Marker function — its existence in `git log` is the proof. The
/// function itself is never called; the file is not referenced from
/// `main.rs` or any other module, so cargo treats it as dead source.
pub fn marker() -> &'static str {
    "hex-as-hermes autonomous source-write proof — 2026-05-14"
}
