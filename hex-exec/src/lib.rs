//! hex-exec — the single-agent execution loop (ADR-2606071340 P1).
//!
//! The ReAct tool-use loop (`direct_react`), the single-shot executor
//! (`direct_exec`), per-run git-worktree isolation (`direct_workspace`,
//! ADR-2606071323), transcript compression, the tool-use protocol
//! (`simple_agent`), and the curated guarded `tools` library. Depends only on
//! hex-core (ports/types), hex-graph (code-graph context), and hex-git
//! (worktree/commit) — no daemon coupling, so the agent loop is reusable
//! outside hex-nexus.

pub mod compress;
pub mod direct_exec;
pub mod direct_react;
pub mod direct_workspace;
pub mod resource_governor;
pub mod simple_agent;
pub mod telegram_notifier;
pub mod tools;
