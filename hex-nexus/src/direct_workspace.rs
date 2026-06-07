//! Per-run git-worktree isolation for the autonomous executor (ADR-2606071323).
//!
//! The single-agent executor commits — and may roll back — on every run. Doing
//! that on the operator's checked-out branch is a data-loss race: incident
//! 2026-06-07, an autonomous rollback `git reset` dropped an operator commit
//! (`e8437655`) as collateral. This module confines each *isolated* run to a
//! dedicated worktree on a `hex/auto/<slug>` branch, created as a SIBLING of the
//! repo so it never even appears in the operator's `git status`. Commit,
//! evidence, and any rollback are scoped to that worktree; results flow back via
//! `hex worktree merge` (ADR-2026-04-13-1930), never a raw reset on the shared root.
//!
//! Isolation is the default. Only the interactive operator path (`hex do`, which
//! sends `isolate:false`) commits directly on its own branch — that tree is the
//! operator's to manage (ADR-2606071323 scope).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::git::worktree;

static RUN_SEQ: AtomicU64 = AtomicU64::new(1);

/// A unique, filesystem- and ref-safe slug for one run: `<compact-utc>-<seq>`.
pub fn next_run_slug() -> String {
    let seq = RUN_SEQ.fetch_add(1, Ordering::Relaxed);
    let ts = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
    format!("{ts}-{seq}")
}

/// Where one run reads, edits, runs evidence, and commits.
pub struct RunWorkspace {
    workdir: PathBuf,
    main_root: PathBuf,
    branch: Option<String>,
    isolated: bool,
}

impl RunWorkspace {
    /// Acquire a workspace for a run. When `isolate`, create a dedicated worktree
    /// on a `hex/auto/<slug>` branch placed beside the repo (so it never pollutes
    /// the operator's working tree); otherwise run in the main checkout.
    ///
    /// If isolation is requested but the worktree cannot be created, this returns
    /// `Err` — the caller MUST abort rather than silently fall back to the
    /// operator's tree (that fallback is exactly the race this prevents).
    pub fn acquire(slug: &str, isolate: bool) -> Result<Self, String> {
        let main_root = crate::direct_exec::repo_root();
        if !isolate {
            return Ok(Self {
                workdir: main_root.clone(),
                main_root,
                branch: None,
                isolated: false,
            });
        }
        let branch = format!("hex/auto/{slug}");
        // Siblings of the repo, grouped under one dir — mirrors hex's existing
        // sibling-worktree convention and keeps the main tree's status clean.
        let base = main_root
            .parent()
            .map(|p| p.join(".hex-autoruns"))
            .unwrap_or_else(|| main_root.join(".hex").join("worktrees"));
        let _ = std::fs::create_dir_all(&base);
        let wt_path = base.join(format!("auto-{slug}"));
        worktree::create_worktree(&main_root, &branch, &wt_path)
            .map_err(|e| format!("create isolated worktree: {e}"))?;
        tracing::info!(branch = %branch, path = %wt_path.display(), "autonomous run isolated to worktree");
        Ok(Self {
            workdir: wt_path,
            main_root,
            branch: Some(branch),
            isolated: true,
        })
    }

    pub fn workdir(&self) -> &Path {
        &self.workdir
    }

    /// True when commits should be authored by the factory identity (isolated runs).
    pub fn is_isolated(&self) -> bool {
        self.isolated
    }

    /// P5 guard (ADR-2606071323): an isolated run must NEVER resolve to the
    /// operator's main worktree. Call before any commit on the isolated path.
    pub fn assert_off_operator_tree(&self) -> Result<(), String> {
        if self.isolated && self.workdir == self.main_root {
            return Err(
                "isolated autonomous run resolved to the operator main worktree — refusing to commit"
                    .to_string(),
            );
        }
        Ok(())
    }

    /// Finish the run. On failure the isolated worktree AND its branch are
    /// removed (nothing worth keeping). On success the branch is left in place
    /// holding the commit, for `hex worktree merge` to land and GC. A
    /// non-isolated (operator) workspace is a no-op.
    pub fn finish(self, success: bool) {
        if !self.isolated {
            return;
        }
        let Some(branch) = self.branch.as_deref() else {
            return;
        };
        if success {
            tracing::info!(branch = %branch, "isolated autonomous commit awaiting `hex worktree merge`");
        } else {
            let wt = self.workdir.to_string_lossy().to_string();
            match worktree::remove_worktree(&self.main_root, &wt, true, true) {
                Ok(_) => tracing::info!(branch = %branch, "failed autonomous run: isolated worktree+branch removed"),
                Err(e) => tracing::warn!(branch = %branch, error = %e, "failed to GC isolated worktree"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_are_unique_and_ref_safe() {
        let a = next_run_slug();
        let b = next_run_slug();
        assert_ne!(a, b, "sequential slugs must differ");
        assert!(!a.contains(' ') && !a.contains('~') && !a.contains(':'));
        assert!(a.ends_with(|c: char| c.is_ascii_digit()));
    }

    #[test]
    fn non_isolated_uses_main_root_and_finish_is_noop() {
        let ws = RunWorkspace::acquire("test-slug", false).expect("non-isolated acquire");
        assert_eq!(ws.workdir(), crate::direct_exec::repo_root());
        assert!(!ws.is_isolated());
        assert!(ws.assert_off_operator_tree().is_ok());
        ws.finish(true); // must not panic / touch anything
    }
}
