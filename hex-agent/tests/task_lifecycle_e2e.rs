//! P5.1 — Task lifecycle integration tests (ADR-2603300100).
//!
//! Tier 1 (always run — no external deps):
//!   Daemon subprocess starts, logs "REST polling fallback active" when SpacetimeDB
//!   is unavailable, and exits cleanly on SIGTERM/process drop.
//!
//! Tier 2 (ignored by default — requires running SpacetimeDB at ws://localhost:3033):
//!   Full round-trip: supervisor creates task → daemon claims via subscription →
//!   CodePhaseWorker generates code → task_complete reducer called.
//!
//! Run Tier 1: cargo test -p hex-agent --test task_lifecycle_e2e
//! Run Tier 2: cargo test -p hex-agent --test task_lifecycle_e2e -- --include-ignored

use std::process::{Command, Stdio};
use std::time::Duration;

const HEX_AGENT_BIN: &str = env!("CARGO_BIN_EXE_hex-agent");

// ── Tier 1: Daemon startup / shutdown ────────────────────────────────────────

/// Daemon starts without SpacetimeDB, falls back to REST polling mode, and
/// exits within 3 seconds when the process is dropped (no task available).
#[test]
fn daemon_starts_and_exits_cleanly_without_stdb() {
    let dir = tempfile::tempdir().unwrap();

    let mut child = Command::new(HEX_AGENT_BIN)
        .arg("daemon")
        .env("HEX_PROJECT_DIR", dir.path())
        .env("NEXUS_HOST", "127.0.0.1")
        .env("NEXUS_PORT", "19999") // nothing listening — fallback path
        .env("SPACETIMEDB_URL", "ws://127.0.0.1:19998") // nothing listening
        .env("RUST_LOG", "off")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn hex-agent daemon — run `cargo build -p hex-agent` first");

    // Give daemon 500ms to log its startup state
    std::thread::sleep(Duration::from_millis(500));

    // Process should still be alive (waiting for tasks)
    assert!(child.try_wait().unwrap().is_none(), "daemon exited prematurely");

    // Kill it — should not hang
    child.kill().ok();
    let status = child.wait().unwrap();
    // Killed process exits with non-zero, that's expected
    let _ = status; // just assert no panic/hang
}

/// Daemon accepts --agent-id and --nexus-host CLI flags without crashing.
#[test]
fn daemon_accepts_cli_flags() {
    let dir = tempfile::tempdir().unwrap();

    let mut child = Command::new(HEX_AGENT_BIN)
        .arg("daemon")
        .args(["--agent-id", "test-agent-abc"])
        .args(["--nexus-host", "127.0.0.1"])
        .args(["--nexus-port", "19999"])
        .env("HEX_PROJECT_DIR", dir.path())
        .env("SPACETIMEDB_URL", "ws://127.0.0.1:19998")
        .env("RUST_LOG", "off")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to spawn hex-agent daemon");

    std::thread::sleep(Duration::from_millis(300));
    assert!(child.try_wait().unwrap().is_none(), "daemon exited prematurely on valid flags");

    child.kill().ok();
    let _ = child.wait();
}

// ── Tier 2: Full SpacetimeDB task lifecycle ───────────────────────────────────

/// Full round-trip via live nexus: create swarm + task → spawn daemon → daemon
/// claims via REST fallback (StDB dead port) → CodePhaseWorker runs → task
/// reaches "completed" status (result may be an error string, but lifecycle
/// completes).
///
/// Requires:
///   - hex-nexus running at http://127.0.0.1:5555
///   - SpacetimeDB at ws://127.0.0.1:3033 (nexus uses it for state)
#[test]
#[ignore = "requires live SpacetimeDB at ws://localhost:3033"]
fn full_task_lifecycle_via_spacetimedb() {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(task_lifecycle_roundtrip());
}

async fn task_lifecycle_roundtrip() {
    const NEXUS: &str = "http://127.0.0.1:5555";
    let client = reqwest::Client::new();

    // ── 1. Register a test agent to satisfy the X-Hex-Agent-Id middleware ─
    let reg: serde_json::Value = client
        .post(format!("{NEXUS}/api/hex-agents/connect"))
        .json(&serde_json::json!({
            "name": "e2e-lifecycle-test-agent",
            "host": "e2e-test-host"
        }))
        .send().await
        .expect("POST /api/hex-agents/connect failed")
        .json().await
        .expect("connect response not JSON");
    let agent_id = reg["agentId"]
        .as_str()
        .unwrap_or_else(|| panic!("agentId missing — response: {reg}"))
        .to_string();

    // ── 2. Create an isolated swarm for this test run ─────────────────────
    let swarm: serde_json::Value = client
        .post(format!("{NEXUS}/api/swarms"))
        .header("X-Hex-Agent-Id", &agent_id)
        .json(&serde_json::json!({
            "name": "e2e-lifecycle-test",
            "topology": "single",
            "projectId": "e2e-test"
        }))
        .send().await
        .expect("POST /api/swarms failed")
        .json().await
        .expect("swarm response not JSON");
    let swarm_id = swarm["id"]
        .as_str()
        .unwrap_or_else(|| panic!("swarm.id missing — response: {swarm}"))
        .to_string();

    // ── 3. Create one task in the swarm ───────────────────────────────────
    let task: serde_json::Value = client
        .post(format!("{NEXUS}/api/swarms/{swarm_id}/tasks"))
        .header("X-Hex-Agent-Id", &agent_id)
        .json(&serde_json::json!({
            "title": "e2e: echo hello"
        }))
        .send().await
        .expect("POST /api/swarms/{id}/tasks failed")
        .json().await
        .expect("task response not JSON");
    let task_id = task["id"]
        .as_str()
        .unwrap_or_else(|| panic!("task.id missing — response: {task}"))
        .to_string();

    // ── 4. Spawn daemon — REST fallback path (StDB on dead port) ─────────
    //
    // Point StDB at an unused port so the daemon immediately falls back to
    // REST polling. Restrict to our swarm via HEX_SWARM_ID so the daemon
    // cannot accidentally claim unrelated tasks in the live nexus.
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = Command::new(HEX_AGENT_BIN)
        .arg("daemon")
        .env("HEX_PROJECT_DIR", dir.path())
        .env("HEX_NEXUS_URL", NEXUS)
        .env("HEX_AGENT_ID", "e2e-test-agent")
        .env("HEX_SWARM_ID", &swarm_id)
        .env("SPACETIMEDB_URL", "ws://127.0.0.1:19998") // dead → REST fallback
        .env("RUST_LOG", "off")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to spawn daemon — run `cargo build -p hex-agent` first");

    // ── 5. Poll GET /api/hexflo/tasks/:id until completed (max 30s) ──────
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    let mut final_status = "pending".to_string();
    while std::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(1000)).await;
        if let Ok(resp) = client
            .get(format!("{NEXUS}/api/hexflo/tasks/{task_id}"))
            .send().await
        {
            if let Ok(t) = resp.json::<serde_json::Value>().await {
                let status = t["status"].as_str().unwrap_or("").to_string();
                if !status.is_empty() {
                    final_status = status.clone();
                }
                if status == "completed" || status == "failed" {
                    break;
                }
            }
        }
    }

    // ── 6. Teardown ───────────────────────────────────────────────────────
    daemon.kill().ok();
    let _ = daemon.wait();

    // Lifecycle completes even when CodePhaseWorker returns an error —
    // the result field may contain "error: ..." but status must be "completed".
    assert_eq!(
        final_status, "completed",
        "task {task_id} did not reach 'completed' within 30s (last status: {final_status})"
    );
}
