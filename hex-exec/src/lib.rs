//! hex-exec — the single-agent execution loop (ADR-2606071340 P1).
//!
//! The ReAct tool-use loop (`direct_react`), the single-shot executor
//! (`direct_exec`), per-run git-worktree isolation (`direct_workspace`,
//! ADR-2606071323), transcript compression, the tool-use protocol
//! (`simple_agent`), and the curated guarded `tools` library. Depends only on
//! hex-core (ports/types), hex-graph (code-graph context), and hex-git
//! (worktree/commit) — no daemon coupling, so the agent loop is reusable
//! outside hex-nexus.

pub mod local_store;
pub mod adversarial;
pub mod compress;
pub mod direct_exec;
pub mod direct_react;
pub mod direct_workspace;
pub mod resource_governor;
pub mod simple_agent;
pub mod telegram_notifier;
pub mod tools;

/// The repository the tools operate on.
///
/// `HEX_REPO_ROOT`, else the CURRENT DIRECTORY. It used to fall back to
/// `/home/gary/hex-intf` — one contributor's checkout, hardcoded into eleven tools. On any other
/// machine, and in any other project, every `repo_read` and `repo_grep` resolved against a path
/// that does not exist and returned `ok=false`.
///
/// That failure is invisible from the outside. The ReAct loop watched devstral-small-2:24b call
/// `repo_read` thirteen times with perfectly correct arguments, get nothing back each time, and
/// give up — which reads as a model that cannot follow through, not as a tool pointed at a
/// stranger's home directory. hex "installs INTO a target project"; the tools have to look there.
pub fn repo_root() -> String {
    std::env::var("HEX_REPO_ROOT").unwrap_or_else(|_| {
        std::env::current_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| ".".to_string())
    })
}
