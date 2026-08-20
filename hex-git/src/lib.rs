//! hex-git — pure git plumbing (status, log, diff, blame, worktree, commit
//! correlation) over libgit2 + the git CLI. No daemon/runtime dependencies, so
//! it is reusable outside hex-nexus (ADR-2606071340 P1). Daemon-coupled adapters
//! (the git poller that pushes to SharedState/websocket, and the swarm-task
//! timeline join) stay in hex-nexus and consume this crate.

pub mod status;
pub mod log;
pub mod diff;
pub mod worktree;
pub mod blame;
pub mod correlation;

use std::path::{Path, PathBuf};

/// Validates that `root_path` is a real git repository and returns its canonical
/// working directory (or the repo path for a bare repo). Returns an error string
/// if invalid.
pub fn validate_repo_path(root_path: &str) -> Result<PathBuf, String> {
    let p = Path::new(root_path);
    if !p.exists() {
        return Err(format!("Path does not exist: {}", root_path));
    }
    git2::Repository::open(p)
        .map(|repo| repo.workdir().unwrap_or(repo.path()).to_path_buf())
        .map_err(|e| format!("Not a git repository: {} ({})", root_path, e))
}
