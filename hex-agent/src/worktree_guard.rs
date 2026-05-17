//! Worktree-mandatory guard for `hex-agent daemon` (ADR-2605081126 P2.1).
//!
//! No agent writes to trunk. Ever. On startup the daemon refuses to run if
//! its CWD resolves to the trunk (the bare repo root), unless
//! `HEXFLO_WORKTREE_REQUIRED=0` is set as an explicit operator override.
//!
//! Trunk detection uses `git worktree list --porcelain`: the first row is
//! always the main worktree. The CURRENT worktree is `git rev-parse
//! --show-toplevel`. Equality means we're in trunk → refuse.

use std::path::{Path, PathBuf};
use std::process::Command;

const ENV_REQUIRED: &str = "HEXFLO_WORKTREE_REQUIRED";
pub const ENV_WORKTREE_PATH: &str = "HEXFLO_WORKTREE_PATH";

#[derive(Debug)]
pub enum GuardOutcome {
    /// Allowed to proceed; CWD is inside a worktree (or override is active).
    Ok { worktree_path: PathBuf, is_trunk: bool },
    /// Refuse to start — operator must run from a worktree.
    RefuseTrunk { trunk_path: PathBuf },
    /// Couldn't determine git state — caller decides whether to fail-closed
    /// (recommended in production) or fail-open (e.g. running outside a repo).
    Indeterminate { reason: String },
}

/// Inspect the CWD relative to the git trunk and return a verdict.
///
/// HEXFLO_WORKTREE_REQUIRED handling:
///   - unset or "1"/"true" (default) → enforce; trunk = refuse
///   - "0"/"false"                    → bypass; always Ok
///   - any other value                → log, treat as "1"
pub fn check_cwd() -> GuardOutcome {
    let required = match std::env::var(ENV_REQUIRED) {
        Ok(v) if v == "0" || v.eq_ignore_ascii_case("false") => false,
        _ => true,
    };

    let cwd_top = match git_toplevel() {
        Ok(p) => p,
        Err(e) => return GuardOutcome::Indeterminate { reason: e },
    };
    let trunk = match worktree_trunk(&cwd_top) {
        Ok(p) => p,
        Err(e) => return GuardOutcome::Indeterminate { reason: e },
    };
    let is_trunk = paths_equal(&cwd_top, &trunk);

    if is_trunk && required {
        return GuardOutcome::RefuseTrunk { trunk_path: trunk };
    }
    GuardOutcome::Ok {
        worktree_path: cwd_top,
        is_trunk,
    }
}

/// `git rev-parse --show-toplevel` — absolute path of the current worktree.
fn git_toplevel() -> Result<PathBuf, String> {
    let out = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .map_err(|e| format!("git rev-parse: {}", e))?;
    if !out.status.success() {
        return Err(format!(
            "git rev-parse failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        return Err("git rev-parse returned empty path".into());
    }
    Ok(PathBuf::from(s))
}

/// Read `git worktree list --porcelain` and return the path of the trunk
/// (first `worktree` line; subsequent lines are linked worktrees).
fn worktree_trunk(any_worktree: &Path) -> Result<PathBuf, String> {
    let out = Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(any_worktree)
        .output()
        .map_err(|e| format!("git worktree list: {}", e))?;
    if !out.status.success() {
        return Err(format!(
            "git worktree list failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    for line in stdout.lines() {
        if let Some(p) = line.strip_prefix("worktree ") {
            return Ok(PathBuf::from(p));
        }
    }
    Err("no worktree line in `git worktree list --porcelain`".into())
}

/// Path equality with symlink resolution. Bazzite/Fedora-Atomic stores
/// `/home` as a symlink to `/var/home`, so `/home/gary/hex-intf` and
/// `/var/home/gary/hex-intf` are the same trunk and must compare equal.
fn paths_equal(a: &Path, b: &Path) -> bool {
    let ca = std::fs::canonicalize(a).unwrap_or_else(|_| a.to_path_buf());
    let cb = std::fs::canonicalize(b).unwrap_or_else(|_| b.to_path_buf());
    ca == cb
}

/// Operator-friendly error message for the RefuseTrunk case.
pub fn refuse_message(trunk: &Path) -> String {
    format!(
        "hex-agent daemon: refusing to run from trunk ({}). Per ADR-2605081126 \
         agents must run inside a git worktree. Either:\n  \
         1) cd into a worktree and re-run, or\n  \
         2) `hex worktree create feat/<task-id>/<role>` then run from there, or\n  \
         3) Set HEXFLO_WORKTREE_REQUIRED=0 to bypass (operator override only).",
        trunk.display()
    )
}
