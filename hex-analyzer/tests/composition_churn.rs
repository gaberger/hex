//! Integration tests for the `--composition-churn` detector.
//!
//! Each test materializes a tempdir, `git init`s it, makes synthetic
//! wiring commits, and writes ADR markdown with whatever Status / Date
//! frontmatter the case requires. Test names start with
//! `architectural_detectors_` so the workplan gate filter
//! (`cargo test -p hex-analyzer architectural_detectors`) picks them up.

use std::fs;
use std::path::Path;
use std::process::Command;

use hex_analyzer::analyzers::composition_churn;

// ── Test fixture helpers ─────────────────────────────────────────────

fn write(root: &Path, rel: &str, contents: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, contents).unwrap();
}

fn run_git(repo: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(repo)
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .status()
        .expect("git invocation");
    assert!(status.success(), "git {args:?} failed");
}

fn init_repo(repo: &Path) {
    run_git(repo, &["init", "-q", "-b", "main"]);
    run_git(repo, &["config", "user.email", "test@example.com"]);
    run_git(repo, &["config", "user.name", "Test"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);
    // Seed commit so the repo has a HEAD; subsequent tests can amend
    // or add wiring files freely.
    write(repo, "README.md", "seed\n");
    run_git(repo, &["add", "README.md"]);
    run_git(repo, &["commit", "-q", "-m", "seed"]);
}

/// Make N commits touching synthetic wiring files. Each commit edits a
/// different wiring file so `--name-only` produces a non-trivial set.
fn make_wiring_commits(repo: &Path, n: usize) {
    let wiring_files = [
        "hex-nexus/src/composition_root.rs",
        "src/lib.rs",
        "src/composition.rs",
        "hex-cli/src/composition_root.rs",
    ];
    for i in 0..n {
        let f = wiring_files[i % wiring_files.len()];
        write(repo, f, &format!("// rev {i}\npub fn wire() {{}}\n"));
        run_git(repo, &["add", f]);
        run_git(repo, &["commit", "-q", "-m", &format!("wire change {i}")]);
    }
}

/// Write an ADR markdown file with the given status + date.
fn write_adr(repo: &Path, name: &str, status: &str, date: &str) {
    let body = format!(
        "# ADR-{name}\n\n**Status:** {status}\n**Date:** {date}\n\n## Context\n\nbody.\n"
    );
    write(repo, &format!("docs/adrs/ADR-{name}.md"), &body);
}

// ── Tests ────────────────────────────────────────────────────────────

#[test]
fn architectural_detectors_composition_churn_silent_when_not_a_git_repo() {
    // No `.git/` → git log fails → detector silently returns no findings
    // rather than blowing up. Important so `hex analyze .` works on
    // freshly scaffolded projects that haven't `git init`ed yet.
    let tmp = tempfile::tempdir().unwrap();
    let report = composition_churn::analyze(tmp.path(), "30d").unwrap();
    assert!(
        report.findings.is_empty(),
        "non-git path should not flag; got {:#?}",
        report.findings
    );
}

#[test]
fn architectural_detectors_composition_churn_silent_when_no_wiring_commits() {
    // Repo exists but no wiring files have been touched in the window.
    // No commits → ratio = 0 → no finding.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    init_repo(root);

    let report = composition_churn::analyze(root, "30d").unwrap();
    assert!(
        report.findings.is_empty(),
        "no wiring churn should not flag; got {:#?}",
        report.findings
    );
}

#[test]
fn architectural_detectors_composition_churn_flags_when_ratio_above_threshold() {
    // 4 wiring commits, 1 accepted ADR in window → ratio 4.0, > 1.5 → flag.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    init_repo(root);
    make_wiring_commits(root, 4);
    // ADR dated today (within any reasonable window).
    write_adr(root, "0001", "Accepted", "2026-04-27");

    let report = composition_churn::analyze(root, "30d").unwrap();
    assert_eq!(report.findings.len(), 1, "{:#?}", report.findings);
    let f = &report.findings[0];
    assert_eq!(f.kind, "composition_churn");
    assert_eq!(f.window, "30d");
    assert_eq!(f.commits_touching_wiring, 4);
    assert_eq!(f.accepted_adrs_in_window, 1);
    assert!(f.ratio >= 4.0 - 1e-6 && f.ratio <= 4.0 + 1e-6, "{}", f.ratio);
    assert!(
        !f.files_touched.is_empty(),
        "files_touched should list wiring paths: {:?}",
        f.files_touched
    );
}

#[test]
fn architectural_detectors_composition_churn_silent_when_ratio_below_threshold() {
    // 3 wiring commits, 3 accepted ADRs → ratio 1.0, < 1.5 → no finding.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    init_repo(root);
    make_wiring_commits(root, 3);
    write_adr(root, "0001", "Accepted", "2026-04-27");
    write_adr(root, "0002", "Accepted", "2026-04-26");
    write_adr(root, "0003", "Accepted", "2026-04-25");

    let report = composition_churn::analyze(root, "30d").unwrap();
    assert!(
        report.findings.is_empty(),
        "ratio 1.0 must not flag; got {:#?}",
        report.findings
    );
}

#[test]
fn architectural_detectors_composition_churn_flags_when_no_adrs_at_all() {
    // Wiring is being rewritten but there are zero accepted ADRs in
    // the window — ratio = +inf → flag. This is the "no decisions
    // recorded" case the detector exists to catch.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    init_repo(root);
    make_wiring_commits(root, 2);

    let report = composition_churn::analyze(root, "30d").unwrap();
    assert_eq!(report.findings.len(), 1, "{:#?}", report.findings);
    let f = &report.findings[0];
    assert_eq!(f.commits_touching_wiring, 2);
    assert_eq!(f.accepted_adrs_in_window, 0);
    assert!(
        f.ratio.is_infinite(),
        "zero ADRs should produce infinite ratio, got {}",
        f.ratio
    );
}

#[test]
fn architectural_detectors_composition_churn_ignores_non_accepted_adrs() {
    // 4 wiring commits + 4 *Proposed* ADRs (none accepted) → still
    // flagged because no accepted decisions cover the churn.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    init_repo(root);
    make_wiring_commits(root, 4);
    write_adr(root, "0001", "Proposed", "2026-04-27");
    write_adr(root, "0002", "Rejected", "2026-04-27");
    write_adr(root, "0003", "Superseded", "2026-04-27");
    write_adr(root, "0004", "Abandoned", "2026-04-27");

    let report = composition_churn::analyze(root, "30d").unwrap();
    assert_eq!(report.findings.len(), 1, "{:#?}", report.findings);
    assert_eq!(report.findings[0].accepted_adrs_in_window, 0);
}

#[test]
fn architectural_detectors_composition_churn_excludes_adrs_outside_window() {
    // 4 wiring commits in the window; the only accepted ADR is dated
    // years ago — it doesn't count, so ratio = 4/0 = inf → flag.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    init_repo(root);
    make_wiring_commits(root, 4);
    write_adr(root, "0001", "Accepted", "2020-01-01");

    let report = composition_churn::analyze(root, "30d").unwrap();
    assert_eq!(report.findings.len(), 1, "{:#?}", report.findings);
    assert_eq!(report.findings[0].accepted_adrs_in_window, 0);
}

#[test]
fn architectural_detectors_composition_churn_files_touched_lists_wiring_paths() {
    // The improver uses files_touched to localize hypotheses. Verify
    // it actually contains the relative paths git reported, not just
    // a count.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    init_repo(root);
    make_wiring_commits(root, 3);

    let report = composition_churn::analyze(root, "30d").unwrap();
    assert_eq!(report.findings.len(), 1);
    let touched = &report.findings[0].files_touched;
    assert!(
        touched.iter().any(|p| p.ends_with("composition_root.rs")),
        "expected composition_root.rs in {touched:?}"
    );
    assert!(
        touched.iter().any(|p| p.ends_with("lib.rs")),
        "expected lib.rs in {touched:?}"
    );
}

#[test]
fn architectural_detectors_composition_churn_envelope_serializes_with_findings_array() {
    // Wire-shape contract for the improver's detector table.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    init_repo(root);
    make_wiring_commits(root, 4);

    let report = composition_churn::analyze(root, "30d").unwrap();
    let json = serde_json::to_value(&report).unwrap();
    let arr = json
        .get("findings")
        .and_then(|v| v.as_array())
        .expect("findings array");
    assert!(!arr.is_empty(), "{json:#?}");

    let f = &arr[0];
    assert_eq!(
        f.get("kind").and_then(|v| v.as_str()),
        Some("composition_churn")
    );
    for field in [
        "window",
        "commits_touching_wiring",
        "accepted_adrs_in_window",
        "ratio",
        "files_touched",
    ] {
        assert!(f.get(field).is_some(), "missing {field}: {f:#?}");
    }
}

#[test]
fn architectural_detectors_composition_churn_handles_unknown_window_unit() {
    // Window argument is operator-supplied — we'd rather error
    // explicitly than silently produce nonsense.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    init_repo(root);
    let err = composition_churn::analyze(root, "30y").unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("unit") || msg.contains("expected"),
        "unhelpful error: {msg}"
    );
}

#[test]
fn architectural_detectors_composition_churn_no_adrs_dir_is_not_an_error() {
    // Project without `docs/adrs/` at all. We should still run, treat
    // accepted-adr-count as 0, and only flag if there's churn.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    init_repo(root);
    make_wiring_commits(root, 2);

    let report = composition_churn::analyze(root, "30d").unwrap();
    assert_eq!(report.findings.len(), 1, "{:#?}", report.findings);
    assert_eq!(report.findings[0].accepted_adrs_in_window, 0);
}
