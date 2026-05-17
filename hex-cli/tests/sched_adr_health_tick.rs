//! P3.2 — Integration test for the sched daemon's `tick_adr_health` handler.
//!
//! Constructs a hermetic ADR registry with one Tier-A finding
//! (`UnparseableStatus`) and one Tier-C finding (`MissingRequiredField`),
//! drives the orchestrator the daemon would run on every tick, and asserts
//! the four behaviors that ADR-2604270800 §1a requires:
//!
//!   (a) Tier-A file is mutated AND merged back to `main` via shadow-promote.
//!   (b) The `adr_doctor_tick` sched event is emitted with both findings
//!       reflected in the count payload.
//!   (c) A P1 inbox notification is queued for the Tier-C finding.
//!   (d) The Tier-C file is NOT mutated — Tier-C is judgment-call only.
//!
//! A second test confirms that `tick_adr_health` is registered in the
//! daemon tick loop in the position the workplan calls for: after the
//! workplan reconcile (`validate(true).await`) and before swarm cleanup
//! (`sweep_stuck_tasks().await`).

use std::path::{Path, PathBuf};
use std::process::Command;

use chrono::NaiveDate;
use hex_cli::commands::adr::doctor::{
    self, AutoFixTier, FindingKind, ShadowPromoteConfig,
};
use hex_cli::commands::sched::tick_adr_health_actions;
use tempfile::TempDir;

// ── Fixtures ─────────────────────────────────────────────────────────────

/// Tier-A buggy frontmatter: bullet-prefixed bold-colon-outside form fails
/// the strict status reader, so `detect_unparseable_status` fires. The
/// shadow-promote auto-fix patch normalizes it to canonical
/// `**Status:** Proposed`.
const TIER_A_BUGGY: &str = "\
# ADR-2604280001: Tier-A fixture (buggy status)

- **Status**: Proposed
- **Date**: 2026-04-25

## Context

UnparseableStatus → Tier-A → shadow-promote → merged.
";

/// Tier-C fixture: `**Status:** Accepted` parses fine, but the H1 is
/// missing → `detect_missing_required_field` fires with MissingRequiredField,
/// which the rule table maps to Tier-C (severity Error). The doctor must
/// notify but never mutate.
const TIER_C_FIXTURE: &str = "\
**Status:** Accepted
**Date:** 2026-04-25

No H1 title above — MissingRequiredField fires here. Tier-C: notify only.
";

// ── Test scaffolding ─────────────────────────────────────────────────────

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

fn config(repo: &Path, sessions_dir: PathBuf) -> ShadowPromoteConfig {
    ShadowPromoteConfig {
        repo_root: repo.to_path_buf(),
        sessions_dir: Some(sessions_dir),
        // Pin "now" so the post-patch self-check doesn't fire StaleProposed
        // (>30 days old) against the just-rewritten Tier-A fixture.
        now: NaiveDate::from_ymd_opt(2026, 4, 27).unwrap(),
    }
}

// ── (a)–(d): orchestrator routes Tier-A and Tier-C correctly ─────────────

#[tokio::test]
async fn one_tick_routes_tier_a_via_shadow_promote_and_tier_c_via_p1_notify() {
    let tmp = TempDir::new().unwrap();
    let repo = init_repo(tmp.path());
    let sessions_dir = tmp.path().join("fake-sessions");

    let tier_a_rel = "docs/adrs/ADR-2604280001-tier-a.md";
    let tier_c_rel = "docs/adrs/ADR-2604280002-tier-c.md";
    write_file(&repo, tier_a_rel, TIER_A_BUGGY);
    write_file(&repo, tier_c_rel, TIER_C_FIXTURE);
    commit_all(&repo, "docs: plant Tier-A and Tier-C ADR fixtures");

    let pre_tier_a = std::fs::read_to_string(repo.join(tier_a_rel)).unwrap();
    let pre_tier_c = std::fs::read_to_string(repo.join(tier_c_rel)).unwrap();

    // Build findings the same way `doctor::run` would, but over the planted
    // corpus — relative paths so shadow_promote resolves against repo_root.
    let adrs = vec![
        (PathBuf::from(tier_a_rel), TIER_A_BUGGY.to_string()),
        (PathBuf::from(tier_c_rel), TIER_C_FIXTURE.to_string()),
    ];
    let now = NaiveDate::from_ymd_opt(2026, 4, 27).unwrap();
    let findings = doctor::scan(&adrs, now);

    // Sanity: the corpus produces exactly the two findings we set up.
    let tier_a: Vec<_> = findings.iter().filter(|f| f.tier == AutoFixTier::A).collect();
    let tier_c: Vec<_> = findings.iter().filter(|f| f.tier == AutoFixTier::C).collect();
    assert_eq!(
        tier_a.len(),
        1,
        "fixture must produce exactly one Tier-A finding, got {} (all findings: {:?})",
        tier_a.len(),
        findings,
    );
    assert_eq!(
        tier_c.len(),
        1,
        "fixture must produce exactly one Tier-C finding, got {} (all findings: {:?})",
        tier_c.len(),
        findings,
    );
    assert_eq!(tier_a[0].kind, FindingKind::UnparseableStatus);
    assert_eq!(tier_c[0].kind, FindingKind::MissingRequiredField);

    let cfg = config(&repo, sessions_dir);
    let result = tick_adr_health_actions(&findings, Some(&cfg)).await;

    // ── (a) Tier-A file mutated AND merged back to main ────────────────
    let post_tier_a = std::fs::read_to_string(repo.join(tier_a_rel)).unwrap();
    assert_ne!(
        pre_tier_a, post_tier_a,
        "Tier-A file must be rewritten by shadow-promote",
    );
    assert!(
        post_tier_a.contains("**Status:** Proposed"),
        "Tier-A file should be canonical after shadow-promote, got:\n{}",
        post_tier_a,
    );
    assert!(
        !post_tier_a.contains("- **Status**: Proposed"),
        "buggy bullet form must be gone, got:\n{}",
        post_tier_a,
    );
    // Confirm it landed via a real merge commit (not fast-forward) — the
    // ADR-2604150100 safety property of shadow-promote.
    let head = git(&repo, &["rev-parse", "HEAD"]);
    let parents = git(&repo, &["rev-list", "--parents", "-n", "1", &head]);
    let parent_count = parents.split_whitespace().count() - 1;
    assert_eq!(
        parent_count, 2,
        "Tier-A merge must be --no-ff (two parents on HEAD), got {} parent(s)",
        parent_count,
    );

    // ── (b) sched event emitted with both findings ─────────────────────
    assert_eq!(result.event.event_type, "adr_doctor_tick");
    assert_eq!(result.event.payload["total"], 2);
    assert_eq!(result.event.payload["tier_a"], 1);
    assert_eq!(result.event.payload["tier_b"], 0);
    assert_eq!(result.event.payload["tier_c"], 1);
    // Both findings have severity Error per the rule table.
    assert_eq!(result.event.payload["errors"], 2);
    assert_eq!(result.event.payload["warnings"], 0);

    // ── (c) Tier-C inbox entry created at P1 ────────────────────────────
    let tier_c_notifs: Vec<_> = result
        .notifications
        .iter()
        .filter(|n| n.body["tier"] == "C")
        .collect();
    assert_eq!(
        tier_c_notifs.len(),
        1,
        "exactly one Tier-C notification expected, got {}: {:#?}",
        tier_c_notifs.len(),
        result.notifications,
    );
    let tier_c_notif = tier_c_notifs[0];
    assert_eq!(tier_c_notif.kind, "adr.doctor.notify");
    assert_eq!(
        tier_c_notif.priority, 1,
        "Tier-C must use P1 (operator interrupt), got priority={}",
        tier_c_notif.priority,
    );
    assert_eq!(tier_c_notif.body["adr_id"], "ADR-2604280002");
    assert_eq!(tier_c_notif.body["kind"], "MissingRequiredField");

    // Tier-A success path also queues an `adr.doctor.applied` P3 entry.
    let applied: Vec<_> = result
        .notifications
        .iter()
        .filter(|n| n.kind == "adr.doctor.applied")
        .collect();
    assert_eq!(applied.len(), 1, "Tier-A success must queue exactly one applied notification");
    assert_eq!(applied[0].priority, 3);
    assert_eq!(applied[0].body["adr_id"], "ADR-2604280001");

    // ── (d) Tier-C file NOT mutated ─────────────────────────────────────
    let post_tier_c = std::fs::read_to_string(repo.join(tier_c_rel)).unwrap();
    assert_eq!(
        pre_tier_c, post_tier_c,
        "Tier-C file must be byte-identical before and after the tick — \
         tick_adr_health is forbidden from mutating Tier-C ADRs",
    );
}

// ── Tick is registered in the daemon loop ────────────────────────────────
//
// The daemon loop body is large and exercising it end-to-end requires nexus
// + a long-running tokio task. Source-level assertion catches the most
// likely regression — somebody removes the call site or moves it out of
// position. Pairs with the orchestrator-level test above to cover both
// "is it called?" and "does it do the right thing when called?".

#[test]
fn tick_adr_health_is_registered_after_reconcile_and_before_swarm_cleanup() {
    let sched_rs = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/commands/sched.rs");
    let source = std::fs::read_to_string(&sched_rs)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", sched_rs.display()));

    // The call site exists.
    assert!(
        source.contains("tick_adr_health(&state).await"),
        "daemon loop must call tick_adr_health(&state).await",
    );

    // Position: the call must appear after `validate(true).await` (the
    // workplan reconcile pass) and before `sweep_stuck_tasks().await` (the
    // swarm-cleanup pass).
    let reconcile_pos = source
        .find("validate(true).await")
        .expect("daemon loop must contain validate(true).await (workplan reconcile)");
    let tick_pos = source
        .find("tick_adr_health(&state).await")
        .expect("daemon loop must contain tick_adr_health call (asserted above)");
    let cleanup_pos = source
        .find("sweep_stuck_tasks().await")
        .expect("daemon loop must contain sweep_stuck_tasks().await (swarm cleanup)");

    assert!(
        reconcile_pos < tick_pos,
        "tick_adr_health must be registered AFTER workplan reconcile \
         (validate at byte {}, tick at byte {})",
        reconcile_pos,
        tick_pos,
    );
    assert!(
        tick_pos < cleanup_pos,
        "tick_adr_health must be registered BEFORE swarm cleanup \
         (tick at byte {}, sweep_stuck_tasks at byte {})",
        tick_pos,
        cleanup_pos,
    );
}
