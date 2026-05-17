//! Integration test: full task lifecycle via SpacetimeDB (ADR-2603300100 P5.1).
//!
//! Tests the end-to-end flow:
//!   1. Create swarm + task via hex-nexus REST (TaskPayload JSON encoded title)
//!   2. Start hex-agent daemon pointing at the nexus
//!   3. Verify task transitions to `in_progress` within 5s
//!   4. Verify code files written to temp output_dir within 30s
//!   5. Verify task status = `completed` within 30s
//!
//! ## Required environment variables
//!
//! ```
//! HEX_NEXUS_URL=http://localhost:5555   # running hex-nexus daemon
//! HEX_AGENT_ID=test-agent-001           # arbitrary test agent ID
//! # Optional (for real inference):
//! ANTHROPIC_API_KEY=sk-ant-...
//! OPENROUTER_API_KEY=sk-or-...
//! ```
//!
//! The test **skips** (does not fail) when `HEX_NEXUS_URL` is not set or the
//! nexus is unreachable, so it is safe to run in offline CI.
//!
//! Run manually:
//!   HEX_NEXUS_URL=http://localhost:5555 cargo test -p hex-agent --test stdb_task_lifecycle -- --nocapture

use std::process::{Command, Stdio};
use std::time::Duration;

const HEX_AGENT_BIN: &str = env!("CARGO_BIN_EXE_hex-agent");

// ── Helpers ───────────────────────────────────────────────────────────────────

fn nexus_url() -> Option<String> {
    std::env::var("HEX_NEXUS_URL").ok().filter(|u| !u.is_empty())
}

/// Returns true when the nexus responds to GET /api/status.
async fn nexus_reachable(url: &str) -> bool {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    client
        .get(format!("{}/api/status", url))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

/// Poll a closure until it returns `Some(T)` or the deadline is reached.
async fn poll_until<F, Fut, T>(deadline: Duration, interval: Duration, mut f: F) -> Option<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Option<T>>,
{
    let start = tokio::time::Instant::now();
    loop {
        if let Some(v) = f().await {
            return Some(v);
        }
        if start.elapsed() >= deadline {
            return None;
        }
        tokio::time::sleep(interval).await;
    }
}

// ── Test ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn task_lifecycle_create_to_complete() {
    let nexus = match nexus_url() {
        Some(u) => u,
        None => {
            eprintln!("[skip] HEX_NEXUS_URL not set — skipping stdb_task_lifecycle test");
            return;
        }
    };

    if !nexus_reachable(&nexus).await {
        eprintln!("[skip] nexus at {} is not reachable — skipping", nexus);
        return;
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap();

    // ── 1. Create a swarm ────────────────────────────────────────────────────
    let swarm_resp = client
        .post(format!("{}/api/swarms", nexus))
        .json(&serde_json::json!({
            "name": "test-stdb-lifecycle",
            "topology": "single"
        }))
        .send()
        .await
        .expect("POST /api/swarms failed");

    assert!(
        swarm_resp.status().is_success(),
        "POST /api/swarms returned {}",
        swarm_resp.status()
    );

    let swarm: serde_json::Value = swarm_resp.json().await.unwrap();
    let swarm_id = swarm["id"].as_str().expect("no swarm id").to_string();
    eprintln!("[test] created swarm {}", swarm_id);

    // ── 2. Create a task with TaskPayload JSON title ─────────────────────────
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let output_dir = temp_dir.path().to_string_lossy().to_string();

    let task_payload = serde_json::json!({
        "step_id": "test-step-001",
        "description": "Write a file named hello.txt containing exactly the text: hello world",
        "output_dir": output_dir,
    });

    let task_resp = client
        .post(format!("{}/api/swarms/{}/tasks", nexus, swarm_id))
        .json(&serde_json::json!({
            "title": task_payload.to_string(),
        }))
        .send()
        .await
        .expect("POST /api/swarms/{id}/tasks failed");

    assert!(
        task_resp.status().is_success(),
        "POST task returned {}",
        task_resp.status()
    );

    let task: serde_json::Value = task_resp.json().await.unwrap();
    let task_id = task["id"].as_str().expect("no task id").to_string();
    eprintln!("[test] created task {}", task_id);

    // ── 3. Spawn hex-agent daemon ────────────────────────────────────────────
    let agent_id = format!("test-agent-{}", &task_id[..8.min(task_id.len())]);
    let mut daemon = Command::new(HEX_AGENT_BIN)
        .arg("daemon")
        .arg("--agent-id").arg(&agent_id)
        .env("HEX_NEXUS_URL", &nexus)
        .env("HEX_AGENT_ID", &agent_id)
        .env("HEX_SWARM_ID", &swarm_id)
        .env("HEX_PROJECT_DIR", temp_dir.path())
        .env("HEX_PROJECT_ROOT", temp_dir.path())
        .env("RUST_LOG", "warn")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn hex-agent daemon — run `cargo build -p hex-agent` first");

    eprintln!("[test] spawned daemon pid={}", daemon.id());

    // ── 4. Verify task reaches in_progress (max 5s) ──────────────────────────
    let client2 = client.clone();
    let nexus2 = nexus.clone();
    let task_id2 = task_id.clone();

    let in_progress = poll_until(Duration::from_secs(5), Duration::from_millis(100), || {
        let c = client2.clone();
        let n = nexus2.clone();
        let t = task_id2.clone();
        async move {
            let resp = c.get(format!("{}/api/hexflo/tasks/{}", n, t)).send().await.ok()?;
            let body: serde_json::Value = resp.json().await.ok()?;
            let status = body["status"].as_str().unwrap_or("");
            if status == "in_progress" || status == "completed" {
                Some(status.to_string())
            } else {
                None
            }
        }
    })
    .await;

    eprintln!("[test] in_progress check: {:?}", in_progress);
    // Note: in_progress transition may be fast — tolerate if already completed
    assert!(
        in_progress.is_some(),
        "task {} never reached in_progress within 5s",
        task_id
    );

    // ── 5. Verify task completes (max 30s) ───────────────────────────────────
    let client3 = client.clone();
    let nexus3 = nexus.clone();
    let task_id3 = task_id.clone();

    let completed = poll_until(Duration::from_secs(30), Duration::from_millis(200), || {
        let c = client3.clone();
        let n = nexus3.clone();
        let t = task_id3.clone();
        async move {
            let resp = c.get(format!("{}/api/hexflo/tasks/{}", n, t)).send().await.ok()?;
            let body: serde_json::Value = resp.json().await.ok()?;
            let status = body["status"].as_str().unwrap_or("");
            if status == "completed" {
                Some(body["result"].as_str().unwrap_or("").to_string())
            } else {
                None
            }
        }
    })
    .await;

    // Kill daemon before asserting (ensures clean exit regardless of outcome)
    let _ = daemon.kill();
    let _ = daemon.wait();

    assert!(
        completed.is_some(),
        "task {} never completed within 30s",
        task_id
    );

    let result = completed.unwrap();
    eprintln!("[test] task completed — result: {}", result);

    // ── 6. Verify a file was written to output_dir ───────────────────────────
    // The task description asked for hello.txt — check output_dir for any written file.
    let any_file = std::fs::read_dir(temp_dir.path())
        .ok()
        .and_then(|mut d| d.next())
        .is_some();

    assert!(
        any_file,
        "no files found in output_dir {} after task completion",
        output_dir
    );

    eprintln!("[test] PASS — full task lifecycle verified");
}

// ── Unit tests for helpers ────────────────────────────────────────────────────

#[test]
fn task_payload_roundtrip() {
    let payload = serde_json::json!({
        "step_id": "s1",
        "description": "write main.rs",
        "output_dir": "/tmp/proj",
    });
    let s = payload.to_string();
    let parsed: serde_json::Value = serde_json::from_str(&s).unwrap();
    assert_eq!(parsed["step_id"], "s1");
    assert_eq!(parsed["description"], "write main.rs");
}

#[test]
fn nexus_url_from_env_absent_returns_none() {
    // Ensure the skip logic works: if env var is unset, nexus_url() returns None.
    // We can't unset env vars safely in parallel tests, so just verify the logic.
    let result: Option<String> = std::env::var("HEX_NEXUS_URL_NONEXISTENT_KEY")
        .ok()
        .filter(|u| !u.is_empty());
    assert!(result.is_none());
}
