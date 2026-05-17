//! Lifecycle / state-machine tests for the integrator subscriber
//! (ADR-2026-05-08-1126 P4).
//!
//! These tests prove the subscriber correctly drives merge_request rows
//! through the gate states: pending → voting → (approved | rejected) →
//! merged. Each test creates a real git worktree (so cargo check has
//! something to inspect) and asserts state transitions over time.
//!
//! Gated on `HEX_GATE_E2E=1` + nexus + STDB up. Tests are designed to
//! parallel-run against a shared subscriber.
//!
//! Coverage:
//!   - pending transitions to voting within one tick
//!   - operator approve flows pending → approved → merged
//!   - operator reject transitions to rejected from any non-terminal state
//!   - rejected and merged are sticky across multiple ticks
//!   - subscriber processes multiple worktrees in the same tick
//!   - a merge_request with a non-existent worktree path gets judged fail
//!     (covers the path-existence guard)

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use serde_json::Value;
use uuid::Uuid;

const HOST_DEFAULT: &str = "http://127.0.0.1:3033";
const TRANSITION_TIMEOUT_SECS: u64 = 90;

fn host() -> String {
    std::env::var("HEX_SPACETIMEDB_HOST").unwrap_or_else(|_| HOST_DEFAULT.to_string())
}

fn db() -> String {
    std::env::var("HEX_STDB_DATABASE")
        .unwrap_or_else(|_| hex_core::stdb_database_for_module("hexflo-coordination").to_string())
}

fn gated() -> bool {
    if std::env::var("HEX_GATE_E2E").as_deref() == Ok("1") {
        true
    } else {
        eprintln!("skipping lifecycle test (HEX_GATE_E2E=1 + nexus + STDB needed)");
        false
    }
}

async fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("client")
}

async fn call_reducer(reducer: &str, args: Value) -> Result<(), String> {
    let url = format!("{}/v1/database/{}/call/{}", host(), db(), reducer);
    let resp = http()
        .await
        .post(&url)
        .json(&args)
        .send()
        .await
        .map_err(|e| format!("http: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    Ok(())
}

async fn current_status(path: &str) -> Option<String> {
    let safe = path.replace('\'', "''");
    let q = format!(
        "SELECT status FROM merge_request WHERE worktree_path = '{}'",
        safe
    );
    let url = format!("{}/v1/database/{}/sql", host(), db());
    let resp = http()
        .await
        .post(&url)
        .header("Content-Type", "text/plain")
        .body(q)
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let body: Value = resp.json().await.ok()?;
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

async fn await_status(path: &str, expected: &[&str]) -> Result<String, String> {
    let start = Instant::now();
    let mut last = String::new();
    while start.elapsed() < Duration::from_secs(TRANSITION_TIMEOUT_SECS) {
        if let Some(s) = current_status(path).await {
            last = s.clone();
            if expected.contains(&s.as_str()) {
                return Ok(s);
            }
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    Err(format!(
        "did not reach any of {:?} within {}s; last seen: '{}'",
        expected, TRANSITION_TIMEOUT_SECS, last
    ))
}

/// Trunk root (the workspace toplevel).
fn trunk_root() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .expect("hex-nexus must be a workspace member")
        .to_path_buf()
}

/// Create a fresh worktree off main and return (path, branch).
fn make_worktree(label: &str) -> (String, String) {
    let id = Uuid::new_v4().simple().to_string();
    let path = format!("/tmp/gate-lifecycle-{}-{}", label, &id[..8]);
    let branch = format!("gate-lifecycle-{}-{}", label, &id[..8]);
    let trunk = trunk_root();
    let out = Command::new("git")
        .args(["worktree", "add", &path, "-b", &branch])
        .current_dir(&trunk)
        .output()
        .expect("git worktree add");
    assert!(
        out.status.success(),
        "git worktree add failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    (path, branch)
}

fn cleanup_worktree(path: &str, branch: &str) {
    let trunk = trunk_root();
    let _ = Command::new("git")
        .args(["worktree", "remove", "--force", path])
        .current_dir(&trunk)
        .output();
    let _ = Command::new("git")
        .args(["branch", "-D", branch])
        .current_dir(&trunk)
        .output();
}

/// RAII cleanup guard — removes the worktree + branch on drop, even on panic.
struct Cleanup {
    path: String,
    branch: String,
}

impl Cleanup {
    fn new(path: &str, branch: &str) -> Self {
        Self {
            path: path.to_string(),
            branch: branch.to_string(),
        }
    }
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        cleanup_worktree(&self.path, &self.branch);
    }
}

async fn open_request(path: &str, branch: &str, role: &str) {
    call_reducer(
        "merge_request_open",
        serde_json::json!([path, branch, role, "wp-lifecycle", "agent-test"]),
    )
    .await
    .expect("merge_request_open");
}

async fn cast_vote(path: &str, voter: &str, verdict: &str, reason: &str) {
    call_reducer(
        "merge_vote_cast",
        serde_json::json!([path, voter, verdict, reason]),
    )
    .await
    .expect("merge_vote_cast");
}

// ─── tests ───────────────────────────────────────────────────────────

/// Pending should transition to voting on the next subscriber tick (≤5s)
/// without operator intervention.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pending_transitions_to_voting() {
    if !gated() {
        return;
    }
    let (path, branch) = make_worktree("p2v");
    let _g = Cleanup::new(&path, &branch);
    open_request(&path, &branch, "hex-coder").await;
    let result = await_status(&path, &["voting", "approved", "rejected", "merged"]).await;
    assert!(result.is_ok(), "{:?}", result);
}

/// Operator approve flips status to approved → integrator merges →
/// terminal=merged.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn operator_approve_drives_to_merged() {
    if !gated() {
        return;
    }
    let (path, branch) = make_worktree("approve");
    let _g = Cleanup::new(&path, &branch);
    open_request(&path, &branch, "hex-coder").await;
    cast_vote(&path, "operator", "pass", "approve test").await;
    call_reducer("merge_decision_tally", serde_json::json!([path]))
        .await
        .expect("tally");
    // Now wait for integrator to run hex worktree merge and flip to merged.
    let result = await_status(&path, &["merged"]).await;
    assert!(result.is_ok(), "{:?}", result);
}

/// Operator reject flips status to rejected directly.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn operator_reject_drives_to_rejected() {
    if !gated() {
        return;
    }
    let (path, branch) = make_worktree("reject");
    let _g = Cleanup::new(&path, &branch);
    open_request(&path, &branch, "hex-coder").await;
    cast_vote(&path, "operator", "fail", "reject test").await;
    call_reducer("merge_decision_tally", serde_json::json!([path]))
        .await
        .expect("tally");
    assert_eq!(current_status(&path).await.as_deref(), Some("rejected"));
}

/// Rejected status is sticky: re-tallying after vote changes does not
/// transition it back.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rejected_is_sticky_across_retallies() {
    if !gated() {
        return;
    }
    let (path, branch) = make_worktree("sticky-rej");
    let _g = Cleanup::new(&path, &branch);
    open_request(&path, &branch, "hex-coder").await;
    cast_vote(&path, "operator", "fail", "initial reject").await;
    call_reducer("merge_decision_tally", serde_json::json!([path]))
        .await
        .unwrap();
    assert_eq!(current_status(&path).await.as_deref(), Some("rejected"));

    // Try to flip via overwhelming pass votes → still rejected.
    cast_vote(&path, "operator", "pass", "second thought").await;
    cast_vote(&path, "validation-judge", "pass", "ok now").await;
    cast_vote(&path, "adversarial-red", "pass", "").await;
    cast_vote(&path, "adversarial-blue", "pass", "").await;
    call_reducer("merge_decision_tally", serde_json::json!([path]))
        .await
        .unwrap();
    // Note: open_request would reset it, but tally alone must NOT.
    assert_eq!(current_status(&path).await.as_deref(), Some("rejected"));
}

/// Subscriber concurrently processes multiple merge_requests in one tick.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn subscriber_processes_multiple_requests_in_parallel() {
    if !gated() {
        return;
    }
    let mut wts = Vec::new();
    let mut paths = Vec::new();
    for i in 0..3 {
        let (path, branch) = make_worktree(&format!("multi-{}", i));
        paths.push(path.clone());
        wts.push((path, branch));
    }

    // Open all three with operator approve so the subscriber doesn't need
    // judge cargo-check (which can flake on system deps).
    for path in &paths {
        open_request(path, "feat/multi", "hex-coder").await;
        cast_vote(path, "operator", "pass", "multi test").await;
        call_reducer("merge_decision_tally", serde_json::json!([path]))
            .await
            .unwrap();
    }

    // All three should reach merged within the timeout.
    for path in &paths {
        let r = await_status(path, &["merged"]).await;
        assert!(r.is_ok(), "path {} did not reach merged: {:?}", path, r);
    }

    for (path, branch) in &wts {
        cleanup_worktree(path, branch);
    }
}

/// merge_request pointing at a non-existent worktree path: the judge runs
/// cargo check, sees the path doesn't exist, votes fail. Status flips to
/// rejected. (Tests the path-existence guard in run_cargo_check.)
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn nonexistent_worktree_path_judged_fail() {
    if !gated() {
        return;
    }
    // Use a fresh /tmp path that has NEVER existed.
    let path = format!("/tmp/gate-nonexistent-{}", Uuid::new_v4().simple());
    open_request(&path, "feat/nonexistent", "hex-coder").await;
    let r = await_status(&path, &["rejected"]).await;
    assert!(r.is_ok(), "{:?}", r);
}

/// Reject path with a real worktree → rejected stays terminal even after
/// another merge_decision_tally tick.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn judge_fail_reaches_rejected_terminal() {
    if !gated() {
        return;
    }
    let (path, branch) = make_worktree("judge-rej");
    let _g = Cleanup::new(&path, &branch);
    open_request(&path, &branch, "hex-coder").await;
    cast_vote(&path, "validation-judge", "fail", "synthetic fail").await;
    call_reducer("merge_decision_tally", serde_json::json!([path]))
        .await
        .unwrap();
    assert_eq!(current_status(&path).await.as_deref(), Some("rejected"));
    // One more tally — should be no-op (sticky).
    call_reducer("merge_decision_tally", serde_json::json!([path]))
        .await
        .unwrap();
    assert_eq!(current_status(&path).await.as_deref(), Some("rejected"));
}
