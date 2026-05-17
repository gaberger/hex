//! Integration tests for ADR-2026-05-08-1126 P2.1 — `hex-agent daemon` refuses to
//! run from trunk and proceeds inside a worktree.
//!
//! These tests shell out to the built `hex-agent` binary because the guard
//! exits the process directly via `std::process::exit(2)`; in-process
//! testing would terminate the test runner. The trade-off is that this
//! suite runs in `cargo test --release` only after a build.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

/// Path to the built hex-agent binary (workspace target dir).
fn agent_binary() -> PathBuf {
    // Prefer the per-crate target dir, fall back to the workspace dir.
    let candidates = [
        "../target/x86_64-unknown-linux-gnu/release/hex-agent",
        "../target/release/hex-agent",
        "target/x86_64-unknown-linux-gnu/release/hex-agent",
        "target/release/hex-agent",
    ];
    for c in &candidates {
        let p = PathBuf::from(c);
        if p.exists() {
            return p
                .canonicalize()
                .unwrap_or_else(|_| p);
        }
    }
    panic!(
        "hex-agent binary not found in any of: {:?}. Run `cargo build -p hex-agent --release` first.",
        candidates
    );
}

/// Trunk path (the workspace root).
fn trunk_root() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // hex-agent/Cargo.toml → workspace root is the parent.
    manifest
        .parent()
        .expect("hex-agent must be a workspace member")
        .to_path_buf()
}

/// BS-1: daemon refuses to start from trunk.
#[test]
fn bs1_daemon_refuses_trunk() {
    let trunk = trunk_root();
    let bin = agent_binary();

    let output = Command::new(&bin)
        .arg("daemon")
        .arg("--agent-id")
        .arg("test-trunk-refuse")
        .current_dir(&trunk)
        .env_remove("HEXFLO_WORKTREE_REQUIRED")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // Bound the process so a guard regression doesn't hang the test.
        .spawn()
        .and_then(|mut child| {
            // Give the guard ~5 s to fire; it should exit much faster.
            std::thread::sleep(Duration::from_secs(5));
            // If it hasn't exited, the guard failed and the daemon is
            // running for real. Kill it and fail.
            match child.try_wait()? {
                Some(status) => Ok((status, child.wait_with_output()?)),
                None => {
                    let _ = child.kill();
                    let out = child.wait_with_output()?;
                    Ok((out.status, out))
                }
            }
        })
        .expect("failed to run hex-agent daemon");
    let (status, full) = output;

    let stderr = String::from_utf8_lossy(&full.stderr);
    assert!(
        stderr.contains("refusing to run from trunk"),
        "expected refuse message in stderr; got:\n{}",
        stderr
    );
    // Exit code 2 is the documented "guard refused" code.
    assert_eq!(
        status.code(),
        Some(2),
        "expected exit code 2; got {:?}",
        status.code()
    );
}

/// Operator override: HEXFLO_WORKTREE_REQUIRED=0 lets the daemon proceed
/// even from trunk, with a logged warning.
#[test]
fn bs1_override_bypasses_guard() {
    let trunk = trunk_root();
    let bin = agent_binary();

    let mut child = Command::new(&bin)
        .arg("daemon")
        .arg("--agent-id")
        .arg("test-bypass")
        .current_dir(&trunk)
        .env("HEXFLO_WORKTREE_REQUIRED", "0")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn");

    // The daemon proceeds past the guard, then waits on inference adapter
    // discovery (90 s). 3 s is enough to confirm it didn't exit immediately.
    std::thread::sleep(Duration::from_secs(3));
    let alive = matches!(child.try_wait().expect("try_wait"), None);
    let _ = child.kill();
    let out = child.wait_with_output().expect("wait_with_output");
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(
        alive || stderr.contains("override active"),
        "expected daemon to proceed past guard with override; stderr:\n{}",
        stderr
    );
    assert!(
        stderr.contains("override active") || stderr.contains("worktree-guard"),
        "expected override or worktree-guard log; stderr:\n{}",
        stderr
    );
}

/// Worktree path: daemon starts cleanly when CWD is a real worktree.
#[test]
fn bs1_worktree_passes() {
    let trunk = trunk_root();
    let bin = agent_binary();
    let wt = PathBuf::from(format!("/tmp/hex-guard-test-{}", std::process::id()));
    let branch = format!("test-guard-{}", std::process::id());

    // Create a fresh worktree.
    let setup = Command::new("git")
        .args([
            "worktree",
            "add",
            wt.to_str().unwrap(),
            "-b",
            &branch,
        ])
        .current_dir(&trunk)
        .output()
        .expect("git worktree add");
    assert!(
        setup.status.success(),
        "git worktree add failed: {}",
        String::from_utf8_lossy(&setup.stderr)
    );

    let result = Command::new(&bin)
        .arg("daemon")
        .arg("--agent-id")
        .arg("test-from-worktree")
        .current_dir(&wt)
        .env_remove("HEXFLO_WORKTREE_REQUIRED")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();

    let stderr_out = match result {
        Ok(mut child) => {
            std::thread::sleep(Duration::from_secs(3));
            let _ = child.kill();
            let out = child.wait_with_output().expect("wait_with_output");
            String::from_utf8_lossy(&out.stderr).to_string()
        }
        Err(e) => format!("spawn error: {}", e),
    };

    // Cleanup before assertions so test failures don't leave the worktree behind.
    let _ = Command::new("git")
        .args(["worktree", "remove", "--force", wt.to_str().unwrap()])
        .current_dir(&trunk)
        .output();
    let _ = Command::new("git")
        .args(["branch", "-D", &branch])
        .current_dir(&trunk)
        .output();

    assert!(
        stderr_out.contains("worktree-guard OK"),
        "expected 'worktree-guard OK' in stderr; got:\n{}",
        stderr_out
    );
    assert!(
        !stderr_out.contains("refusing to run from trunk"),
        "daemon refused trunk while inside a worktree; stderr:\n{}",
        stderr_out
    );
}
