//! Comprehensive reducer-level tests for the merge-team gate (ADR-2026-05-08-1126 P1).
//!
//! Covers:
//!   - merge_request_open (creation, validation, idempotent re-open)
//!   - merge_request_set_status (allowed transitions, invalid transitions)
//!   - merge_vote_cast (voter validation, verdict validation, size cap, idempotency)
//!   - merge_quorum_policy_set (validation)
//!   - merge_team_init (idempotency)
//!   - merge_decision_tally (all decision paths: voting, approved, rejected,
//!     judge-fail, operator-pass override, operator-fail short-circuit,
//!     impossible-quorum early reject, terminal-state stickiness)
//!
//! Each test uses a unique `worktree_path` (uuid-suffixed) so the suite
//! is parallel-safe against a shared STDB instance.
//!
//! Gated on `HEX_GATE_E2E=1` + nexus + STDB up. Skip otherwise.

use std::time::Duration;

use serde_json::Value;
use uuid::Uuid;

const STDB_HOST_DEFAULT: &str = "http://127.0.0.1:3033";

fn host() -> String {
    std::env::var("HEX_SPACETIMEDB_HOST").unwrap_or_else(|_| STDB_HOST_DEFAULT.to_string())
}

fn db() -> String {
    std::env::var("HEX_STDB_DATABASE")
        .unwrap_or_else(|_| hex_core::stdb_database_for_module("hexflo-coordination").to_string())
}

fn gated() -> bool {
    if std::env::var("HEX_GATE_E2E").as_deref() == Ok("1") {
        true
    } else {
        eprintln!("skipping merge-gate test (set HEX_GATE_E2E=1 + nexus + STDB to run)");
        false
    }
}

fn unique_path(prefix: &str) -> String {
    format!("/tmp/gate-{}-{}", prefix, Uuid::new_v4())
}

async fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("client")
}

/// Call a reducer; returns Ok(()) on HTTP success, Err(body) on failure.
async fn call(reducer: &str, args: Value) -> Result<(), String> {
    let url = format!("{}/v1/database/{}/call/{}", host(), db(), reducer);
    let resp = client()
        .await
        .post(&url)
        .json(&args)
        .send()
        .await
        .map_err(|e| format!("http: {}", e))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("HTTP {}: {}", status, body));
    }
    Ok(())
}

/// SQL query, returns rows.
async fn sql(query: &str) -> Vec<Vec<Value>> {
    let url = format!("{}/v1/database/{}/sql", host(), db());
    let resp = client()
        .await
        .post(&url)
        .header("Content-Type", "text/plain")
        .body(query.to_string())
        .send()
        .await
        .expect("sql http");
    if !resp.status().is_success() {
        return Vec::new();
    }
    let body: Value = resp.json().await.expect("sql json");
    body.as_array()
        .and_then(|a| a.first())
        .and_then(|t| t.get("rows"))
        .and_then(|r| r.as_array())
        .map(|rows| {
            rows.iter()
                .filter_map(|r| r.as_array().cloned())
                .collect()
        })
        .unwrap_or_default()
}

async fn merge_request_status(path: &str) -> Option<String> {
    let safe = path.replace('\'', "''");
    let q = format!(
        "SELECT status FROM merge_request WHERE worktree_path = '{}'",
        safe
    );
    let rows = sql(&q).await;
    rows.first()
        .and_then(|r| r.first())
        .and_then(|v| v.as_str())
        .map(String::from)
}

async fn open_request(path: &str, branch: &str, role: &str) {
    call(
        "merge_request_open",
        serde_json::json!([path, branch, role, "wp-test", "agent-test"]),
    )
    .await
    .expect("merge_request_open");
}

async fn cast(path: &str, voter: &str, verdict: &str, reason: &str) -> Result<(), String> {
    call(
        "merge_vote_cast",
        serde_json::json!([path, voter, verdict, reason]),
    )
    .await
}

async fn tally(path: &str) {
    call("merge_decision_tally", serde_json::json!([path]))
        .await
        .expect("tally");
}

// ─── merge_request_open ───────────────────────────────────────────────

#[tokio::test]
async fn open_creates_row_with_pending_status() {
    if !gated() {
        return;
    }
    let p = unique_path("open-creates");
    open_request(&p, "feat/x", "hex-coder").await;
    assert_eq!(merge_request_status(&p).await.as_deref(), Some("pending"));
}

#[tokio::test]
async fn open_rejects_empty_worktree_path() {
    if !gated() {
        return;
    }
    let r = call(
        "merge_request_open",
        serde_json::json!(["", "branch", "role", "wp", "agent"]),
    )
    .await;
    assert!(r.is_err(), "expected error for empty path");
}

#[tokio::test]
async fn open_rejects_empty_branch() {
    if !gated() {
        return;
    }
    let p = unique_path("open-empty-branch");
    let r = call(
        "merge_request_open",
        serde_json::json!([p, "", "role", "wp", "agent"]),
    )
    .await;
    assert!(r.is_err(), "expected error for empty branch");
}

#[tokio::test]
async fn open_idempotent_resets_to_pending() {
    if !gated() {
        return;
    }
    let p = unique_path("open-idempotent");
    open_request(&p, "feat/x", "hex-coder").await;
    cast(&p, "validation-judge", "fail", "first vote").await.unwrap();
    tally(&p).await;
    assert_eq!(merge_request_status(&p).await.as_deref(), Some("rejected"));
    // Re-open should reset the row to pending. Note: votes persist (operator
    // intent: keep audit trail; tally re-evaluates against the fresh status).
    open_request(&p, "feat/x", "hex-coder").await;
    let new_status = merge_request_status(&p).await;
    assert_eq!(
        new_status.as_deref(),
        Some("pending"),
        "re-open should flip status back to pending"
    );
}

// ─── merge_request_set_status ────────────────────────────────────────

#[tokio::test]
async fn transition_pending_to_voting_allowed() {
    if !gated() {
        return;
    }
    let p = unique_path("trans-pending-voting");
    open_request(&p, "feat/x", "hex-coder").await;
    let r = call(
        "merge_request_set_status",
        serde_json::json!([p, "voting"]),
    )
    .await;
    assert!(r.is_ok(), "{:?}", r);
    assert_eq!(merge_request_status(&p).await.as_deref(), Some("voting"));
}

#[tokio::test]
async fn transition_pending_to_merged_blocked() {
    if !gated() {
        return;
    }
    let p = unique_path("trans-pending-merged");
    open_request(&p, "feat/x", "hex-coder").await;
    let r = call(
        "merge_request_set_status",
        serde_json::json!([p, "merged"]),
    )
    .await;
    assert!(
        r.is_err(),
        "pending→merged should fail (must go through voting+approved first)"
    );
}

#[tokio::test]
async fn transition_voting_to_approved_then_merged() {
    if !gated() {
        return;
    }
    let p = unique_path("trans-full-path");
    open_request(&p, "feat/x", "hex-coder").await;
    call("merge_request_set_status", serde_json::json!([p, "voting"]))
        .await
        .unwrap();
    call(
        "merge_request_set_status",
        serde_json::json!([p, "approved"]),
    )
    .await
    .unwrap();
    call("merge_request_set_status", serde_json::json!([p, "merged"]))
        .await
        .unwrap();
    assert_eq!(merge_request_status(&p).await.as_deref(), Some("merged"));
}

#[tokio::test]
async fn transition_set_status_on_nonexistent_errors() {
    if !gated() {
        return;
    }
    let p = unique_path("nonexistent");
    let r = call(
        "merge_request_set_status",
        serde_json::json!([p, "voting"]),
    )
    .await;
    assert!(r.is_err());
}

// ─── merge_vote_cast ─────────────────────────────────────────────────

#[tokio::test]
async fn cast_happy_path() {
    if !gated() {
        return;
    }
    let p = unique_path("cast-happy");
    open_request(&p, "feat/x", "hex-coder").await;
    cast(&p, "validation-judge", "pass", "looks good").await.unwrap();
    let q = format!(
        "SELECT verdict FROM merge_vote WHERE worktree_path = '{}' AND voter = 'validation-judge'",
        p
    );
    let rows = sql(&q).await;
    assert_eq!(
        rows.first()
            .and_then(|r| r.first())
            .and_then(|v| v.as_str()),
        Some("pass")
    );
}

#[tokio::test]
async fn cast_rejects_unknown_voter() {
    if !gated() {
        return;
    }
    let p = unique_path("cast-bad-voter");
    open_request(&p, "feat/x", "hex-coder").await;
    let r = cast(&p, "random-actor", "pass", "").await;
    assert!(r.is_err());
}

#[tokio::test]
async fn cast_rejects_unknown_verdict() {
    if !gated() {
        return;
    }
    let p = unique_path("cast-bad-verdict");
    open_request(&p, "feat/x", "hex-coder").await;
    let r = cast(&p, "validation-judge", "maybe", "").await;
    assert!(r.is_err());
}

#[tokio::test]
async fn cast_rejects_reason_over_4kb() {
    if !gated() {
        return;
    }
    let p = unique_path("cast-big-reason");
    open_request(&p, "feat/x", "hex-coder").await;
    let big = "x".repeat(5000);
    let r = cast(&p, "validation-judge", "pass", &big).await;
    assert!(r.is_err());
}

#[tokio::test]
async fn cast_rejects_phantom_merge_request() {
    if !gated() {
        return;
    }
    let p = unique_path("cast-phantom");
    let r = cast(&p, "validation-judge", "pass", "").await;
    assert!(r.is_err(), "voting on non-existent request should fail");
}

#[tokio::test]
async fn cast_idempotent_overwrites() {
    if !gated() {
        return;
    }
    let p = unique_path("cast-idempotent");
    open_request(&p, "feat/x", "hex-coder").await;
    cast(&p, "validation-judge", "fail", "v1").await.unwrap();
    cast(&p, "validation-judge", "pass", "v2").await.unwrap();
    let q = format!(
        "SELECT verdict, reason FROM merge_vote WHERE worktree_path = '{}' AND voter = 'validation-judge'",
        p
    );
    let rows = sql(&q).await;
    let (verdict, reason) = rows
        .first()
        .map(|r| {
            (
                r.first().and_then(|v| v.as_str()).unwrap_or("").to_string(),
                r.get(1).and_then(|v| v.as_str()).unwrap_or("").to_string(),
            )
        })
        .unwrap_or_default();
    assert_eq!(verdict, "pass");
    assert_eq!(reason, "v2");
}

// ─── merge_quorum_policy_set ────────────────────────────────────────

#[tokio::test]
async fn policy_set_rejects_zero_min_passes() {
    if !gated() {
        return;
    }
    let r = call(
        "merge_quorum_policy_set",
        serde_json::json!(["nonexistent-pool", 0, true, true]),
    )
    .await;
    assert!(r.is_err());
}

#[tokio::test]
async fn policy_set_rejects_too_many_min_passes() {
    if !gated() {
        return;
    }
    let r = call(
        "merge_quorum_policy_set",
        serde_json::json!(["nonexistent-pool", 99, true, true]),
    )
    .await;
    assert!(r.is_err());
}

#[tokio::test]
async fn merge_team_init_idempotent() {
    if !gated() {
        return;
    }
    // calling twice should not error or create duplicate `*` rows
    call("merge_team_init", serde_json::json!([])).await.unwrap();
    call("merge_team_init", serde_json::json!([])).await.unwrap();
    let rows = sql("SELECT pool_id FROM merge_quorum_policy WHERE pool_id = '*'").await;
    assert_eq!(rows.len(), 1, "exactly one default policy row");
}

// ─── merge_decision_tally ────────────────────────────────────────────

#[tokio::test]
async fn tally_no_votes_stays_voting() {
    if !gated() {
        return;
    }
    let p = unique_path("tally-no-votes");
    open_request(&p, "feat/x", "hex-coder").await;
    call("merge_request_set_status", serde_json::json!([p, "voting"]))
        .await
        .unwrap();
    tally(&p).await;
    assert_eq!(merge_request_status(&p).await.as_deref(), Some("voting"));
}

#[tokio::test]
async fn tally_two_passes_plus_judge_pass_approves() {
    if !gated() {
        return;
    }
    let p = unique_path("tally-2pass");
    open_request(&p, "feat/x", "hex-coder").await;
    cast(&p, "validation-judge", "pass", "").await.unwrap();
    cast(&p, "adversarial-red", "pass", "").await.unwrap();
    tally(&p).await;
    assert_eq!(merge_request_status(&p).await.as_deref(), Some("approved"));
}

#[tokio::test]
async fn tally_judge_fail_rejects_even_with_two_passes() {
    if !gated() {
        return;
    }
    let p = unique_path("tally-judge-fail");
    open_request(&p, "feat/x", "hex-coder").await;
    cast(&p, "validation-judge", "fail", "spec broken").await.unwrap();
    cast(&p, "adversarial-red", "pass", "").await.unwrap();
    cast(&p, "adversarial-blue", "pass", "").await.unwrap();
    tally(&p).await;
    assert_eq!(merge_request_status(&p).await.as_deref(), Some("rejected"));
}

#[tokio::test]
async fn tally_operator_pass_overrides_judge_fail() {
    if !gated() {
        return;
    }
    let p = unique_path("tally-operator-override");
    open_request(&p, "feat/x", "hex-coder").await;
    cast(&p, "validation-judge", "fail", "").await.unwrap();
    cast(&p, "operator", "pass", "I accept the risk").await.unwrap();
    tally(&p).await;
    assert_eq!(merge_request_status(&p).await.as_deref(), Some("approved"));
}

#[tokio::test]
async fn tally_operator_fail_short_circuits() {
    if !gated() {
        return;
    }
    let p = unique_path("tally-operator-fail");
    open_request(&p, "feat/x", "hex-coder").await;
    cast(&p, "validation-judge", "pass", "").await.unwrap();
    cast(&p, "adversarial-red", "pass", "").await.unwrap();
    cast(&p, "operator", "fail", "operator stop").await.unwrap();
    tally(&p).await;
    assert_eq!(merge_request_status(&p).await.as_deref(), Some("rejected"));
}

#[tokio::test]
async fn tally_impossible_quorum_rejects_early() {
    if !gated() {
        return;
    }
    let p = unique_path("tally-impossible");
    open_request(&p, "feat/x", "hex-coder").await;
    // 3 fails out of 4 possible voters → can't reach 2 passes.
    cast(&p, "validation-judge", "fail", "").await.unwrap();
    cast(&p, "adversarial-red", "fail", "").await.unwrap();
    cast(&p, "adversarial-blue", "fail", "").await.unwrap();
    tally(&p).await;
    assert_eq!(merge_request_status(&p).await.as_deref(), Some("rejected"));
}

#[tokio::test]
async fn tally_terminal_states_are_sticky() {
    if !gated() {
        return;
    }
    let p = unique_path("tally-sticky");
    open_request(&p, "feat/x", "hex-coder").await;
    cast(&p, "validation-judge", "fail", "").await.unwrap();
    tally(&p).await;
    assert_eq!(merge_request_status(&p).await.as_deref(), Some("rejected"));
    // Now flip the vote to pass and re-tally — should NOT transition.
    cast(&p, "validation-judge", "pass", "").await.unwrap();
    tally(&p).await;
    assert_eq!(
        merge_request_status(&p).await.as_deref(),
        Some("rejected"),
        "rejected must be sticky even if votes change"
    );
}

#[tokio::test]
async fn tally_partial_passes_stays_voting() {
    if !gated() {
        return;
    }
    let p = unique_path("tally-partial");
    open_request(&p, "feat/x", "hex-coder").await;
    cast(&p, "adversarial-red", "pass", "").await.unwrap();
    // Only 1 pass + no judge vote yet → stays voting (need 2 + judge=pass).
    tally(&p).await;
    assert_eq!(merge_request_status(&p).await.as_deref(), Some("voting"));
}

#[tokio::test]
async fn tally_judge_pass_alone_stays_voting_under_default_policy() {
    if !gated() {
        return;
    }
    let p = unique_path("tally-judge-only");
    open_request(&p, "feat/x", "hex-coder").await;
    cast(&p, "validation-judge", "pass", "").await.unwrap();
    tally(&p).await;
    // Default policy is min_pass_votes=2, so judge=pass alone isn't enough.
    assert_eq!(merge_request_status(&p).await.as_deref(), Some("voting"));
}

#[tokio::test]
async fn tally_on_phantom_merge_request_errors() {
    if !gated() {
        return;
    }
    let p = unique_path("tally-phantom");
    let r = call("merge_decision_tally", serde_json::json!([p])).await;
    assert!(r.is_err());
}
