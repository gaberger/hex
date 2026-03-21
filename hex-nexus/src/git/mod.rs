//! Git integration for hex-nexus (ADR-044).
//!
//! Provides stateless, project-scoped git operations using the `git2` crate
//! for read operations and git CLI for worktree management.

pub mod status;
pub mod log;
pub mod diff;
pub mod worktree;
pub mod poller;
pub mod correlation;
pub mod blame;
pub mod timeline;

use std::path::{Path, PathBuf};

/// Validates that `repo_path` is a real git repository and returns
/// the canonical path. Returns an error string if invalid.
pub fn validate_repo_path(root_path: &str) -> Result<PathBuf, String> {
    let p = Path::new(root_path);
    if !p.exists() {
        return Err(format!("Path does not exist: {}", root_path));
    }
    // Verify it contains a .git directory or is a bare repo
    git2::Repository::open(p)
        .map(|repo| repo.workdir().unwrap_or(repo.path()).to_path_buf())
        .map_err(|e| format!("Not a git repository: {} ({})", root_path, e))
}
