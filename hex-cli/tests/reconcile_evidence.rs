/// ADR-2604142200 Reconcile Evidence Verification — Regression Tests (R1.3)
///
/// Verifies that `hex plan reconcile` does NOT auto-promote tasks whose
/// target files are missing from the filesystem, even when other heuristic
/// signals (identifier grep, cargo check) might fire.
///
/// Two scenarios:
///   1. Partial workplan (wp-partial.json): P1 already done, P2/P3 pending
///      with non-existent files → P2/P3 must stay "needs work".
///   2. Fully-evidenced tasks: files exist in the repo → promotion works.

use std::process::Command;

fn hex_bin() -> Command {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let workspace_root = std::path::Path::new(manifest_dir)
        .parent()
        .expect("hex-cli must be inside workspace");
    let debug_bin = workspace_root.join("target/debug/hex");
    let release_bin = workspace_root.join("target/release/hex");

    let bin_path = if debug_bin.exists() {
        debug_bin
    } else if release_bin.exists() {
        release_bin
    } else {
        return Command::new("hex");
    };

    Command::new(bin_path)
}

fn fixture_path(name: &str) -> String {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    format!("{}/tests/fixtures/reconcile/{}", manifest_dir, name)
}

// ── Test 1: Partial workplan — P2/P3 must NOT auto-promote ──────────────

#[test]
fn reconcile_partial_workplan_leaves_pending_tasks() {
    let output = hex_bin()
        .args(["plan", "reconcile", &fixture_path("wp-partial.json")])
        .output()
        .expect("failed to execute hex plan reconcile");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{}\n{}", stdout, stderr);

    // P1 tasks are already marked done — reconcile should report them as such
    assert!(
        combined.contains("already done") || combined.contains("P1.1"),
        "Expected P1 tasks to appear in output. Got:\n{}",
        combined
    );

    // P2 tasks must NOT be promoted — their files don't exist
    assert!(
        !task_promoted(&combined, "P2.1"),
        "P2.1 was promoted but its files don't exist! Output:\n{}",
        combined
    );
    assert!(
        !task_promoted(&combined, "P2.2"),
        "P2.2 was promoted but its files don't exist! Output:\n{}",
        combined
    );

    // P3 tasks must NOT be promoted — their files don't exist
    assert!(
        !task_promoted(&combined, "P3.1"),
        "P3.1 was promoted but its files don't exist! Output:\n{}",
        combined
    );
    assert!(
        !task_promoted(&combined, "P3.2"),
        "P3.2 was promoted but its files don't exist! Output:\n{}",
        combined
    );

    // Summary should show only 2 of 6 done (the two P1 tasks)
    assert!(
        combined.contains("2/6 steps confirmed done")
            || combined.contains("2/6 tasks confirmed done"),
        "Expected 2/6 done (only P1 tasks). Got:\n{}",
        combined
    );
}

// ── Test 2: --update must NOT mutate pending tasks with missing files ────

#[test]
fn reconcile_update_preserves_pending_when_files_missing() {
    // Copy fixture to a temp file so --update can write without touching the original
    let tmp_dir = std::env::temp_dir().join("hex-reconcile-test");
    let _ = std::fs::create_dir_all(&tmp_dir);
    let tmp_fixture = tmp_dir.join("wp-partial-copy.json");
    std::fs::copy(fixture_path("wp-partial.json"), &tmp_fixture)
        .expect("failed to copy fixture");

    let output = hex_bin()
        .args([
            "plan",
            "reconcile",
            "--update",
            tmp_fixture.to_str().unwrap(),
        ])
        .output()
        .expect("failed to execute hex plan reconcile --update");

    let _stdout = String::from_utf8_lossy(&output.stdout);

    // Re-read the (possibly mutated) fixture and verify P2/P3 are still pending
    let content =
        std::fs::read_to_string(&tmp_fixture).expect("failed to read updated fixture");
    let raw: serde_json::Value =
        serde_json::from_str(&content).expect("fixture is not valid JSON");

    for phase_id in &["P2", "P3"] {
        let phases = raw["phases"].as_array().expect("phases is array");
        let phase = phases
            .iter()
            .find(|p| p["id"].as_str() == Some(phase_id))
            .unwrap_or_else(|| panic!("phase {} not found", phase_id));

        let tasks = phase["tasks"].as_array().expect("tasks is array");
        for task in tasks {
            let tid = task["id"].as_str().unwrap_or("?");
            let status = task["status"].as_str().unwrap_or("?");
            assert_ne!(
                status, "done",
                "Task {} in phase {} was promoted to done despite missing files!",
                tid, phase_id
            );
        }
    }

    // Cleanup
    let _ = std::fs::remove_file(&tmp_fixture);
    let _ = std::fs::remove_dir(&tmp_dir);
}

// ── Test 3: Fully-evidenced workplan — promotion works ──────────────────

#[test]
fn reconcile_promotes_tasks_with_full_evidence() {
    // Build a workplan in a temp file pointing to files that actually exist
    // in the repo, with real ADR and created_at in the distant past so
    // any commit qualifies as "evidence".
    let wp = serde_json::json!({
        "adr": "",
        "created_at": "2020-01-01T00:00:00Z",
        "created_by": "test",
        "description": "Positive-case fixture: all files exist, broad match window.",
        "feature": "Test: full evidence fixture",
        "id": "wp-test-full-evidence",
        "phases": [{
            "id": "P1",
            "name": "Existing files",
            "description": "Tasks pointing to files that definitely exist.",
            "tier": 0,
            "tasks": [{
                "id": "P1.1",
                "name": "Cargo workspace manifest",
                "description": "The root Cargo.toml.",
                "files": ["Cargo.toml"],
                "layer": "domain",
                "agent": "hex-coder",
                "deps": [],
                "status": "pending",
                "strategy_hint": "codegen"
            }]
        }]
    });

    let tmp_dir = std::env::temp_dir().join("hex-reconcile-test-full");
    let _ = std::fs::create_dir_all(&tmp_dir);
    let tmp_fixture = tmp_dir.join("wp-full-evidence.json");
    std::fs::write(&tmp_fixture, serde_json::to_string_pretty(&wp).unwrap())
        .expect("failed to write temp fixture");

    let output = hex_bin()
        .args([
            "plan",
            "reconcile",
            tmp_fixture.to_str().unwrap(),
        ])
        .output()
        .expect("failed to execute hex plan reconcile");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{}\n{}", stdout, stderr);

    // Cargo.toml has been modified by many commits since 2020-01-01.
    // With an empty ADR field, scoped git evidence won't match, but
    // the identifier grep or unscoped heuristics should still fire.
    // At minimum, the task should NOT show as "already done" (it's pending).
    // The important thing: the reconcile machinery runs without error.
    assert!(
        combined.contains("P1.1"),
        "Expected P1.1 in output. Got:\n{}",
        combined
    );

    // With empty ADR and broad created_at, the git evidence check should
    // still find modifications. If the evidence system is working,
    // 1/1 tasks should be confirmed.
    let has_promotion = combined.contains("1/1 steps confirmed done")
        || combined.contains("1/1 tasks confirmed done")
        || task_promoted(&combined, "P1.1");

    assert!(
        has_promotion,
        "Expected P1.1 to be promoted (Cargo.toml exists with git history). Got:\n{}",
        combined
    );

    // Cleanup
    let _ = std::fs::remove_file(&tmp_fixture);
    let _ = std::fs::remove_dir(&tmp_dir);
}

// ── Helpers ─────────────────────────────────────────────────────────────

/// Check if a task ID appears in output with a promotion indicator.
/// Promotion markers: "verified", "git-confirmed", "commit-matched", "done"
/// (but NOT "already done", which means it was pre-marked).
fn task_promoted(output: &str, task_id: &str) -> bool {
    for line in output.lines() {
        if !line.contains(task_id) {
            continue;
        }
        // Skip lines that say "already done" — those are pre-marked, not promoted
        if line.contains("already done") {
            continue;
        }
        // Check for promotion markers
        if line.contains("verified")
            || line.contains("git-confirmed")
            || line.contains("commit-matched")
            || line.contains("done")
        {
            return true;
        }
    }
    false
}
