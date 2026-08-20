//! Git integration for hex-nexus (ADR-044).
//!
//! The pure, stateless git plumbing (status/log/diff/blame/worktree/correlation
//! + `validate_repo_path`) lives in the standalone `hex-git` crate
//! (ADR-2606071340 P1) and is re-exported here so `crate::git::*` consumers are
//! unchanged. The two daemon-coupled adapters stay local: `poller` pushes git
//! state into `SharedState`/the websocket, and `timeline` joins git history with
//! swarm tasks (`SwarmTaskInfo`).

pub use hex_git::{blame, correlation, diff, log, status, validate_repo_path, worktree};

pub mod poller;
pub mod timeline;
