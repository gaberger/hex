//! `hex worktree status / approve / reject` CLI tests (ADR-2026-05-08-1126 P5).
//!
//! Spawns the built `hex` binary as a subprocess and asserts on exit code,
//! stdout, stderr. Like the daemon-worktree-required tests, this suite
//! shells out so the CLI surface is exercised the same way an operator
//! exercises it.
//!
//! Gated on `HEX_GATE_E2E=1` — needs nexus + STDB up to talk to. Skips
//! cleanly otherwise so `cargo test` stays fast in offline mode.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

fn gated() -> bool {
    if std::env::var("HEX_GATE_E2E").as_deref() == Ok("1") {
        true
    } else {
        eprintln!("skipping (HEX_GATE_E2E=1 + nexus + STDB needed)");
        false
    }
}

fn hex_binary() -> PathBuf {
    // Use Cargo's CARGO_BIN_EXE_* env var so we exercise whichever
    // profile the test harness built (debug for `cargo test`, release
    // for `cargo test --release`). Hard-coded path lookups failed on
    // CI which only builds debug. Same fix as
    // hex-agent/tests/daemon_worktree_required.rs.
    PathBuf::from(env!("CARGO_BIN_EXE_hex"))
}

fn unique_path(label: &str) -> String {
    format!(
        "/tmp/cli-{}-{}",
        label,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_micros()
    )
}

/// Open a merge_request via STDB call (HTTP), returning the worktree path.
async fn seed_request(path: &str, branch: &str) {
    let host = std::env::var("HEX_SPACETIMEDB_HOST")
        .unwrap_or_else(|_| "http://127.0.0.1:3033".into());
    let db = std::env::var("HEX_STDB_DATABASE")
        .unwrap_or_else(|_| hex_core::stdb_database_for_module("hexflo-coordination").to_string());
    let url = format!("{}/v1/database/{}/call/merge_request_open", host, db);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("client");
    let resp = client
        .post(&url)
        .json(&serde_json::json!([path, branch, "hex-coder", "wp-cli-test", "agent-cli"]))
        .send()
        .await
        .expect("seed http");
    assert!(resp.status().is_success(), "seed merge_request: HTTP {}", resp.status());
}

async fn status_of(path: &str) -> Option<String> {
    let host = std::env::var("HEX_SPACETIMEDB_HOST")
        .unwrap_or_else(|_| "http://127.0.0.1:3033".into());
    let db = std::env::var("HEX_STDB_DATABASE")
        .unwrap_or_else(|_| hex_core::stdb_database_for_module("hexflo-coordination").to_string());
    let q = format!(
        "SELECT status FROM merge_request WHERE worktree_path = '{}'",
        path.replace('\'', "''")
    );
    let url = format!("{}/v1/database/{}/sql", host, db);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("client");
    let resp = client
        .post(&url)
        .header("Content-Type", "text/plain")
        .body(q)
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let body: serde_json::Value = resp.json().await.ok()?;
    body.as_array()?
        .first()?
        .get("rows")?
        .as_array()?
        .first()?
        .as_array()?
        .first()?
        .as_str()
        .map(String::from)
}

// ─── status ──────────────────────────────────────────────────────────

#[test]
fn status_runs_and_exits_zero() {
    if !gated() {
        return;
    }
    let out = Command::new(hex_binary())
        .args(["worktree", "status"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn");
    assert!(
        out.status.success(),
        "exit code {:?}; stderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn status_includes_seeded_request() {
    if !gated() {
        return;
    }
    let path = unique_path("status-includes");
    seed_request(&path, "feat/cli-status").await;
    let out = Command::new(hex_binary())
        .args(["worktree", "status"])
        .output()
        .expect("spawn");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(&path),
        "status output should mention seeded path:\n{}",
        stdout
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn status_json_emits_parseable() {
    if !gated() {
        return;
    }
    let path = unique_path("status-json");
    seed_request(&path, "feat/cli-json").await;
    let out = Command::new(hex_binary())
        .args(["worktree", "status", "--json"])
        .output()
        .expect("spawn");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    let requests = parsed
        .get("requests")
        .and_then(|r| r.as_array())
        .expect("requests array");
    assert!(
        requests.iter().any(|r| r.get("worktree_path").and_then(|p| p.as_str()) == Some(path.as_str())),
        "JSON output should include seeded path"
    );
}

// ─── approve ─────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn approve_records_operator_pass_and_flips_status() {
    if !gated() {
        return;
    }
    let path = unique_path("approve-flip");
    seed_request(&path, "feat/cli-approve").await;
    let out = Command::new(hex_binary())
        .args([
            "worktree", "approve", &path, "--reason", "cli test approve",
        ])
        .output()
        .expect("spawn");
    assert!(
        out.status.success(),
        "approve exited {:?}; stderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Operator override recorded"), "expected override line, got:\n{}", stdout);
    let s = status_of(&path).await;
    assert!(
        matches!(s.as_deref(), Some("approved") | Some("merged")),
        "status should be approved or merged; got {:?}",
        s
    );
}

// ─── reject ──────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reject_records_operator_fail_and_flips_status() {
    if !gated() {
        return;
    }
    let path = unique_path("reject-flip");
    seed_request(&path, "feat/cli-reject").await;
    let out = Command::new(hex_binary())
        .args(["worktree", "reject", &path, "deliberate cli rejection"])
        .output()
        .expect("spawn");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Rejected"), "expected reject line:\n{}", stdout);
    assert_eq!(status_of(&path).await.as_deref(), Some("rejected"));
}

#[test]
fn reject_requires_reason_argument() {
    if !gated() {
        return;
    }
    let out = Command::new(hex_binary())
        .args(["worktree", "reject", "/tmp/nonexistent"])
        // intentionally no reason arg
        .output()
        .expect("spawn");
    assert!(!out.status.success(), "reject without reason should fail");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.to_lowercase().contains("reason") || stderr.contains("required"),
        "stderr should explain missing reason; got:\n{}",
        stderr
    );
}

// ─── help works without nexus ────────────────────────────────────────

#[test]
fn help_lists_status_approve_reject() {
    let out = Command::new(hex_binary())
        .args(["worktree", "--help"])
        .output()
        .expect("spawn");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("status"), "help should list 'status'");
    assert!(stdout.contains("approve"), "help should list 'approve'");
    assert!(stdout.contains("reject"), "help should list 'reject'");
}
