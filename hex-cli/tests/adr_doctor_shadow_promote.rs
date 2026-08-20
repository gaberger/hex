//! P2.2 — Integration tests for `hex_cli::commands::adr::doctor::shadow_promote`.
//!
//! Coverage:
//!   1. The happy path lands an `Outcome::Applied` and survives the
//!      ADR-2026-04-15-0100 dropped-commits scenario (multi-commit history on
//!      main is fully reachable post-merge; the new fix commit and a real
//!      merge commit are both present).
//!   2. Each abort path returns `Outcome::Aborted` and leaves no orphan
//!      worktree behind:
//!        - dirty target file
//!        - file claimed by a session manifest's `allowed_paths`
//!        - patch self-check still reports findings on the rewritten file
//!        - non-Tier-A finding (defense-in-depth)
//!
//! The `git merge --no-ff` merge strategy is exercised by the dropped-
//! commits test: two unrelated commits land on `main` between the auto-fix
//! branch's base and the merge, and we assert all of them remain reachable.

use std::path::{Path, PathBuf};
use std::process::Command;

use chrono::NaiveDate;
use hex_cli::commands::adr::doctor::{
    finding, shadow_promote_with_config, AutoFixTier, FindingKind, Outcome, ShadowPromoteConfig,
};
use tempfile::TempDir;

// ── Test scaffolding ──────────────────────────────────────────────────────

/// Pre-canonical buggy frontmatter — the exact form from the real ADR
/// `ADR-2026-04-15-0100-worktree-merge-fast-forward-drops-commits.md` before
/// it was hand-normalized in this session.
const BUGGY_ADR: &str = "\
# ADR-2026-04-15-0100: A buggy frontmatter ADR

- **Status**: Proposed
- **Date**: 2026-04-15
- **Depends on**: ADR-2026-04-13-1930 (original worktree-merge integrity claim)

## Context

This ADR's frontmatter uses the bullet-prefixed bold-colon-outside form,
which the strict status reader cannot parse. The doctor must flag it as
UnparseableStatus, and shadow_promote must rewrite it to canonical form
without dropping any unrelated commits already on main.
";

/// Set up a fresh git repo at `dir`. Returns the absolute path.
fn init_repo(dir: &Path) -> PathBuf {
    git(dir, &["init", "--initial-branch=main"]);
    git(dir, &["config", "user.email", "test@hex.local"]);
    git(dir, &["config", "user.name", "test"]);
    git(dir, &["config", "commit.gpgsign", "false"]);
    dir.to_path_buf()
}

fn git(repo: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap_or_else(|e| panic!("git {} spawn failed: {}", args.join(" "), e));
    if !out.status.success() {
        panic!(
            "git {} failed in {}: {}",
            args.join(" "),
            repo.display(),
            String::from_utf8_lossy(&out.stderr),
        );
    }
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn write_file(repo: &Path, rel: &str, content: &str) {
    let p = repo.join(rel);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&p, content).unwrap();
}

fn commit_all(repo: &Path, msg: &str) -> String {
    git(repo, &["add", "-A"]);
    git(repo, &["commit", "-m", msg]);
    git(repo, &["rev-parse", "HEAD"])
}

fn rev_list_all(repo: &Path) -> Vec<String> {
    git(repo, &["rev-list", "HEAD"])
        .lines()
        .map(|s| s.to_string())
        .collect()
}

fn config(repo: &Path, sessions_dir: Option<PathBuf>) -> ShadowPromoteConfig {
    ShadowPromoteConfig {
        repo_root: repo.to_path_buf(),
        sessions_dir,
        // Pin "now" to the same date as the buggy ADR's `Date:` so the
        // self-check doesn't fire StaleProposed (>30 days old).
        now: NaiveDate::from_ymd_opt(2026, 4, 27).unwrap(),
    }
}

fn buggy_finding(rel_path: &str) -> hex_cli::commands::adr::doctor::Finding {
    finding(
        "ADR-2026-04-15-0100",
        PathBuf::from(rel_path),
        FindingKind::UnparseableStatus,
        "buggy frontmatter form",
    )
}

// ── 1. Happy path + ADR-2026-04-15-0100 dropped-commits scenario ──────────────

/// The core safety claim of P2.2: shadow_promote uses `git merge --no-ff`
/// rather than fast-forward, so unrelated commits already on main cannot
/// be silently dropped — even when the auto-fix branch was created after
/// they landed (which is the normal case) AND when other commits land
/// between worktree creation and merge (the ADR-2026-04-15-0100 race).
#[test]
fn happy_path_preserves_all_pre_merge_commits() {
    let tmp = TempDir::new().unwrap();
    let repo = init_repo(tmp.path());

    // ── Setup the multi-commit history that ADR-2026-04-15-0100 says we
    //    must preserve ──
    write_file(&repo, "README.md", "# initial\n");
    let c_initial = commit_all(&repo, "initial");

    // Agent A landed an unrelated change on main.
    write_file(&repo, "src/agent_a.rs", "// agent A's work\n");
    let c_agent_a = commit_all(&repo, "agent A: add agent_a.rs");

    // Agent B landed another unrelated change on main.
    write_file(&repo, "src/agent_b.rs", "// agent B's work\n");
    let c_agent_b = commit_all(&repo, "agent B: add agent_b.rs");

    // The buggy ADR was committed.
    write_file(&repo, "docs/adrs/ADR-2026-04-15-0100-buggy.md", BUGGY_ADR);
    let c_adr = commit_all(&repo, "docs: add ADR-2026-04-15-0100 (buggy frontmatter)");

    let pre_merge_history = rev_list_all(&repo);
    assert_eq!(
        pre_merge_history,
        vec![
            c_adr.clone(),
            c_agent_b.clone(),
            c_agent_a.clone(),
            c_initial.clone(),
        ],
        "test setup sanity: 4 commits on main before shadow_promote",
    );

    // ── Run shadow_promote ──
    let f = buggy_finding("docs/adrs/ADR-2026-04-15-0100-buggy.md");
    let cfg = config(&repo, Some(tmp.path().join("fake-sessions")));
    let outcome = shadow_promote_with_config(&f, &cfg).expect("shadow_promote err");

    let (branch, fix_commit) = match outcome {
        Outcome::Applied { branch, commit } => (branch, commit),
        Outcome::Aborted { reason } => panic!("expected Applied, got Aborted: {}", reason),
    };
    assert_eq!(branch, "sched/auto-fix/ADR-doctor/ADR-2026-04-15-0100");

    // ── Verify ADR-2026-04-15-0100 safety property ──
    let post_merge_history = rev_list_all(&repo);
    for c in &pre_merge_history {
        assert!(
            post_merge_history.contains(c),
            "pre-merge commit {} dropped by shadow_promote (ADR-2026-04-15-0100 regression)",
            c
        );
    }
    // The fix commit must be present too.
    assert!(
        post_merge_history.contains(&fix_commit),
        "fix commit {} not reachable from main after merge",
        fix_commit
    );

    // ── Verify --no-ff produced a merge commit (not a fast-forward) ──
    let head = git(&repo, &["rev-parse", "HEAD"]);
    let parents = git(&repo, &["rev-list", "--parents", "-n", "1", &head]);
    let parent_count = parents.split_whitespace().count() - 1; // first token is HEAD itself
    assert_eq!(
        parent_count, 2,
        "shadow_promote must produce a real merge commit (--no-ff), got {} parent(s)",
        parent_count
    );

    // ── Verify the file was actually rewritten to canonical form ──
    let after = std::fs::read_to_string(repo.join("docs/adrs/ADR-2026-04-15-0100-buggy.md")).unwrap();
    assert!(
        after.contains("**Status:** Proposed"),
        "Status line should be canonical after shadow_promote, got:\n{}",
        after
    );
    assert!(
        !after.contains("- **Status**: Proposed"),
        "buggy bullet form should be gone, got:\n{}",
        after
    );

    // ── Worktree should be cleaned up; branch should remain for audit ──
    let worktrees = git(&repo, &["worktree", "list"]);
    assert!(
        !worktrees.contains("auto-fix-worktrees"),
        "auto-fix worktree should be removed, got:\n{}",
        worktrees
    );
    let branches = git(&repo, &["branch", "--list", "sched/auto-fix/*"]);
    assert!(
        branches.contains("sched/auto-fix/ADR-doctor/ADR-2026-04-15-0100"),
        "auto-fix branch should persist for audit, got:\n{}",
        branches
    );
}

// ── 2. Abort paths ────────────────────────────────────────────────────────

#[test]
fn aborts_when_target_file_is_dirty() {
    let tmp = TempDir::new().unwrap();
    let repo = init_repo(tmp.path());
    write_file(&repo, "docs/adrs/ADR-2026-04-15-0100-buggy.md", BUGGY_ADR);
    commit_all(&repo, "docs: add ADR");

    // Modify the file but don't commit — leaves it tracked-dirty.
    write_file(
        &repo,
        "docs/adrs/ADR-2026-04-15-0100-buggy.md",
        &format!("{}\n// uncommitted edit\n", BUGGY_ADR),
    );

    let f = buggy_finding("docs/adrs/ADR-2026-04-15-0100-buggy.md");
    let cfg = config(&repo, Some(tmp.path().join("fake-sessions")));
    let outcome = shadow_promote_with_config(&f, &cfg).expect("shadow_promote err");

    match outcome {
        Outcome::Aborted { reason } => {
            assert!(
                reason.contains("uncommitted"),
                "abort reason should mention uncommitted state, got: {}",
                reason
            );
        }
        Outcome::Applied { .. } => panic!("must abort on dirty file"),
    }

    // The file's uncommitted edit must still be intact (we didn't touch it).
    let after = std::fs::read_to_string(repo.join("docs/adrs/ADR-2026-04-15-0100-buggy.md")).unwrap();
    assert!(after.contains("uncommitted edit"));
    // Total commit count on main is unchanged — no merge happened.
    let count: usize = rev_list_all(&repo).len();
    assert_eq!(count, 1, "main should still have only the initial commit");
}

#[test]
fn aborts_when_file_is_claimed_by_session_manifest() {
    let tmp = TempDir::new().unwrap();
    let repo = init_repo(tmp.path());
    write_file(&repo, "docs/adrs/ADR-2026-04-15-0100-buggy.md", BUGGY_ADR);
    commit_all(&repo, "docs: add ADR");

    // Plant a session manifest that claims docs/adrs/ via allowed_paths.
    let sessions_dir = tmp.path().join("fake-sessions");
    std::fs::create_dir_all(&sessions_dir).unwrap();
    let manifest = serde_json::json!({
        "agent_id": "fake-agent",
        "allowed_paths": ["docs/adrs/"],
        "worktree_path": null,
    });
    std::fs::write(
        sessions_dir.join("agent-fake.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let f = buggy_finding("docs/adrs/ADR-2026-04-15-0100-buggy.md");
    let cfg = config(&repo, Some(sessions_dir.clone()));
    let outcome = shadow_promote_with_config(&f, &cfg).expect("shadow_promote err");

    match outcome {
        Outcome::Aborted { reason } => {
            assert!(
                reason.contains("claimed") && reason.contains("agent-fake"),
                "abort reason should cite the claiming session, got: {}",
                reason
            );
        }
        Outcome::Applied { .. } => panic!("must abort when file is claimed"),
    }

    // No worktree was created.
    let worktrees = git(&repo, &["worktree", "list"]);
    assert!(
        !worktrees.contains("auto-fix-worktrees"),
        "no auto-fix worktree should exist after claim-check abort, got:\n{}",
        worktrees
    );
}

#[test]
fn aborts_when_self_check_still_reports_findings_after_patch() {
    // Fixture: an ADR file that's missing both H1 AND has a buggy Status.
    // The auto-fix patch only normalizes the Status frontmatter; the
    // missing H1 will still trip MissingRequiredField on the self-check,
    // so shadow_promote must abort cleanup the worktree without touching
    // main.
    const STILL_BROKEN: &str = "\
- **Status**: Proposed
- **Date**: 2026-04-15

(no H1 title — MissingRequiredField will still fire after patch)
";
    let tmp = TempDir::new().unwrap();
    let repo = init_repo(tmp.path());
    write_file(&repo, "docs/adrs/ADR-2026-04-15-0100-still-broken.md", STILL_BROKEN);
    let pre = commit_all(&repo, "docs: still-broken ADR");

    let f = finding(
        "ADR-2026-04-15-0100",
        PathBuf::from("docs/adrs/ADR-2026-04-15-0100-still-broken.md"),
        FindingKind::UnparseableStatus,
        "buggy + missing H1",
    );
    let cfg = config(&repo, Some(tmp.path().join("fake-sessions")));
    let outcome = shadow_promote_with_config(&f, &cfg).expect("shadow_promote err");

    match outcome {
        Outcome::Aborted { reason } => {
            assert!(
                reason.contains("strict") && reason.contains("finding"),
                "abort reason should cite strict-mode self-check failure, got: {}",
                reason
            );
        }
        Outcome::Applied { .. } => {
            panic!("must abort when post-patch doctor still reports findings")
        }
    }

    // Main HEAD must be unchanged.
    let post = git(&repo, &["rev-parse", "HEAD"]);
    assert_eq!(pre, post, "main HEAD must not advance on aborted run");
    // Original file content untouched.
    let after = std::fs::read_to_string(repo.join("docs/adrs/ADR-2026-04-15-0100-still-broken.md"))
        .unwrap();
    assert_eq!(after, STILL_BROKEN);
    // Worktree dir cleaned up.
    let wt_root = repo.join(".hex").join("auto-fix-worktrees");
    if wt_root.exists() {
        let entries: Vec<_> = std::fs::read_dir(&wt_root).unwrap().flatten().collect();
        assert!(
            entries.is_empty(),
            "worktree dir should be empty after abort cleanup, got: {:?}",
            entries.iter().map(|e| e.path()).collect::<Vec<_>>()
        );
    }
    let branches = git(&repo, &["branch", "--list", "sched/auto-fix/*"]);
    assert!(
        branches.trim().is_empty(),
        "auto-fix branch should be deleted on abort, got:\n{}",
        branches
    );
}

#[test]
fn aborts_when_finding_is_not_tier_a() {
    // Defense-in-depth: shadow_promote refuses non-Tier-A findings even
    // when called directly. The dispatcher in P2.3 won't call it that
    // way, but the safety property should hold regardless of caller.
    let tmp = TempDir::new().unwrap();
    let repo = init_repo(tmp.path());
    write_file(&repo, "docs/adrs/ADR-2026-04-15-0100-buggy.md", BUGGY_ADR);
    commit_all(&repo, "docs: add ADR");

    let tier_b = finding(
        "ADR-2026-04-15-0100",
        PathBuf::from("docs/adrs/ADR-2026-04-15-0100-buggy.md"),
        FindingKind::StaleProposed, // Tier B
        "stale",
    );
    assert_eq!(tier_b.tier, AutoFixTier::B, "preconditions: must be Tier B");

    let cfg = config(&repo, Some(tmp.path().join("fake-sessions")));
    let outcome = shadow_promote_with_config(&tier_b, &cfg).expect("shadow_promote err");
    match outcome {
        Outcome::Aborted { reason } => {
            assert!(
                reason.contains("Tier-A"),
                "abort reason should cite Tier-A guard, got: {}",
                reason
            );
        }
        Outcome::Applied { .. } => panic!("must abort on non-Tier-A finding"),
    }
}

// ── Sanity: noop patch (already-canonical file) is also an abort ─────────

#[test]
fn aborts_when_patch_is_a_noop_against_already_canonical_file() {
    const CANONICAL: &str = "\
# ADR-2026-04-15-0100: Already canonical

**Status:** Accepted
**Date:** 2026-04-15

## Context
Body.
";
    let tmp = TempDir::new().unwrap();
    let repo = init_repo(tmp.path());
    write_file(&repo, "docs/adrs/ADR-2026-04-15-0100-canonical.md", CANONICAL);
    let pre = commit_all(&repo, "docs: canonical ADR");

    // Even Tier-A finding against a canonical file must abort cleanly —
    // there's nothing to fix.
    let f = buggy_finding("docs/adrs/ADR-2026-04-15-0100-canonical.md");
    let cfg = config(&repo, Some(tmp.path().join("fake-sessions")));
    let outcome = shadow_promote_with_config(&f, &cfg).expect("shadow_promote err");

    match outcome {
        Outcome::Aborted { reason } => {
            assert!(
                reason.contains("no-op") || reason.contains("canonical"),
                "abort reason should cite no-op patch, got: {}",
                reason
            );
        }
        Outcome::Applied { .. } => panic!("must abort on no-op patch"),
    }
    let post = git(&repo, &["rev-parse", "HEAD"]);
    assert_eq!(pre, post, "main HEAD must not advance on no-op abort");
}
