/// wp-brain-string-cleanup P2.2 — Smoke test for `hex sched enqueue` output.
///
/// Verifies that `hex sched enqueue shell -- 'true'` prints
/// "enqueued sched task <uuid>" and NOT "enqueued brain task <uuid>".
///
/// Strategy:
///   1. `hex sched enqueue --help` exits 0 — proves the subcommand exists
///      under "sched" (not only under the deprecated "brain" alias).
///   2. Source-level assertion: grep the sched.rs enqueue handler for the
///      exact println! string.  This catches regressions even when nexus
///      is offline (the command errors before printing when it can't reach
///      the nexus REST API, so a live invocation is not hermetic).

use std::process::Command;

/// Locate the hex binary (debug preferred, release fallback, PATH last).
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

#[test]
fn sched_enqueue_help_exists() {
    let output = hex_bin()
        .args(["sched", "enqueue", "--help"])
        .env("HEX_NEXUS_URL", "http://127.0.0.1:1")
        .output()
        .expect("failed to execute hex sched enqueue --help");

    assert!(
        output.status.success(),
        "`hex sched enqueue --help` should exit 0 (subcommand registered). stderr: {}",
        String::from_utf8_lossy(&output.stderr),
    );
}

#[test]
fn enqueue_output_says_sched_not_brain() {
    let sched_rs = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/commands/sched.rs");
    let source = std::fs::read_to_string(&sched_rs)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", sched_rs.display()));

    assert!(
        source.contains(r#"enqueued sched task"#),
        "sched.rs enqueue handler must print 'enqueued sched task', not 'brain task'",
    );
    assert!(
        !source.contains(r#"enqueued brain task"#),
        "sched.rs must NOT contain 'enqueued brain task' — rename to 'sched task' per ADR-2604150000",
    );
}
