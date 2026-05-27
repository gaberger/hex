//! Concrete `IAgentTool` implementations for the SOP agent loop
//! (wp-sop-agent-loop P1.2-P1.5).
//!
//! v1 surface:
//! - `repo_read`  (P1.2) — fetch file contents bounded by the policy allowlist
//! - `repo_grep`  (P1.3) — ripgrep wrapper (lands in P1.3)
//! - `cargo_check` (P1.4) — compilation probe (lands in P1.4)
//! - `code_patch_propose` (P1.5) — TERMINAL action (lands in P1.5)

pub mod repo_read;

pub use repo_read::RepoReadTool;
