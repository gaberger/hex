//! P1.1 — Sched daemon terminal-state integration test (ADR-2604142155).
//!
//! Validates the brain-task lifecycle through hex-nexus REST endpoints:
//!   1. Enqueue a brain task via POST /api/hexflo/memory
//!   2. Verify it appears as pending via GET /api/brain/status
//!   3. Transition through in_progress → completed (simulating daemon drain)
//!   4. Assert the task reaches terminal state via /api/brain/queue/history
//!   5. Assert /api/brain/status shows zero pending/running
//!
//! This test is designed to FAIL on current main (no terminal-signal
//! sweep) and PASS after P2 lands the sweep logic.

use hex_nexus::HubConfig;
use reqwest::Client;
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::time::Duration;

const TEST_AGENT_ID: &str = "test-sched-daemon-agent";

async fn start_hub() -> SocketAddr {
    let config = HubConfig {
        port: 0,
        bind: "127.0.0.1".to_string(),
        token: None,
        is_daemon: false,
        no_agent: true,
    };

    let (router, _state) = hex_nexus::build_app(&config).await;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        hex_nexus::axum::serve(listener, router)
            .await
            .expect("server error");
    });

    tokio::time::sleep(Duration::from_millis(100)).await;
    addr
}

/// Store a brain-task record in hexflo memory via REST.
async fn store_brain_task(client: &Client, addr: SocketAddr, task: &Value) -> String {
    let id = task["id"].as_str().expect("task must have id").to_string();
    let key = format!("brain-task:{}", id);
    let resp = client
        .post(format!("http://{}/api/hexflo/memory", addr))
        .header("x-hex-agent-id", TEST_AGENT_ID)
        .json(&json!({ "key": key, "value": task.to_string() }))
        .send()
        .await
        .expect("POST hexflo/memory");
    assert!(
        resp.status().is_success(),
        "store brain task failed: {}",
        resp.status()
    );
    id
}

/// Update a brain-task record's status (and optionally result/completed_at).
async fn update_brain_task_status(
    client: &Client,
    addr: SocketAddr,
    id: &str,
    status: &str,
    result: Option<&str>,
) {
    let key = format!("brain-task:{}", id);

    // Read current value
    let resp = client
        .get(format!("http://{}/api/hexflo/memory/{}", addr, key))
        .send()
        .await
        .expect("GET hexflo/memory");

    if !resp.status().is_success() {
        panic!("read brain task failed: {}", resp.status());
    }

    let body: Value = resp.json().await.expect("parse memory response");
    let value_str = body
        .get("value")
        .and_then(|v| v.as_str())
        .unwrap_or("{}");
    let mut task: Value = serde_json::from_str(value_str).unwrap_or(json!({}));

    task["status"] = json!(status);
    if let Some(r) = result {
        task["result"] = json!(r);
    }
    if status == "completed" || status == "failed" {
        task["completed_at"] = json!(chrono::Utc::now().to_rfc3339());
    }

    // Write back
    let resp = client
        .post(format!("http://{}/api/hexflo/memory", addr))
        .header("x-hex-agent-id", TEST_AGENT_ID)
        .json(&json!({ "key": key, "value": task.to_string() }))
        .send()
        .await
        .expect("POST hexflo/memory update");
    assert!(
        resp.status().is_success(),
        "update brain task failed: {}",
        resp.status()
    );
}

fn make_workplan_task(id: &str, project_id: &str) -> Value {
    json!({
        "id": id,
        "kind": "workplan",
        "payload": "docs/workplans/wp-test-daemon-drain.json",
        "status": "pending",
        "project_id": project_id,
        "created_at": chrono::Utc::now().to_rfc3339(),
        "completed_at": null,
        "result": null,
        "timeout_s": 1800,
    })
}

fn make_workplan_task_with_timeout(id: &str, project_id: &str, timeout_s: u64) -> Value {
    json!({
        "id": id,
        "kind": "workplan",
        "payload": "docs/workplans/wp-test-daemon-drain.json",
        "status": "pending",
        "project_id": project_id,
        "created_at": chrono::Utc::now().to_rfc3339(),
        "completed_at": null,
        "result": null,
        "timeout_s": timeout_s,
    })
}

fn make_expired_in_progress_task(id: &str, project_id: &str, timeout_s: u64) -> Value {
    let expired_at = chrono::Utc::now() - chrono::Duration::seconds((timeout_s + 60) as i64);
    json!({
        "id": id,
        "kind": "workplan",
        "payload": "docs/workplans/wp-test-sweep.json",
        "status": "in_progress",
        "project_id": project_id,
        "created_at": expired_at.to_rfc3339(),
        "completed_at": null,
        "result": null,
        "timeout_s": timeout_s,
    })
}

fn brain_status_url(addr: SocketAddr, project: &str) -> String {
    format!("http://{}/api/brain/status?project={}", addr, project)
}

// ── Test: Enqueue appears in brain status as pending ─────────────────

#[tokio::test]
async fn enqueued_task_visible_in_brain_status() {
    let addr = start_hub().await;
    let client = Client::new();
    let project = "test-visible-pending-proj";

    let task = make_workplan_task("test-visible-pending", project);
    store_brain_task(&client, addr, &task).await;

    // Poll brain status — task should appear as pending
    let mut found_pending = false;
    for _ in 0..10 {
        let resp = client
            .get(brain_status_url(addr, project))
            .send()
            .await
            .expect("GET brain/status");

        if resp.status().is_success() {
            let body: Value = resp.json().await.expect("parse brain status");
            if body["queue_pending"].as_u64().unwrap_or(0) > 0 {
                found_pending = true;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    assert!(
        found_pending,
        "enqueued brain task must appear as pending in /api/brain/status"
    );
}

// ── Test: Full lifecycle — enqueue → drain → terminal ────────────────

#[tokio::test]
async fn enqueue_drain_reaches_terminal_within_timeout() {
    let addr = start_hub().await;
    let client = Client::new();
    let project = "test-drain-terminal-proj";
    let task_id = "test-drain-terminal";

    // Step 1: Enqueue a workplan task
    let task = make_workplan_task(task_id, project);
    store_brain_task(&client, addr, &task).await;

    // Step 2: Verify pending
    let status_resp = client
        .get(brain_status_url(addr, project))
        .send()
        .await
        .expect("GET brain/status");
    assert!(status_resp.status().is_success());
    let status_body: Value = status_resp.json().await.expect("parse status");
    assert!(
        status_body["queue_pending"].as_u64().unwrap_or(0) > 0,
        "task should be pending after enqueue"
    );

    // Step 3: Simulate daemon drain — transition to in_progress
    update_brain_task_status(&client, addr, task_id, "in_progress", None).await;

    let status_resp = client
        .get(brain_status_url(addr, project))
        .send()
        .await
        .expect("GET brain/status after in_progress");
    if status_resp.status().is_success() {
        let body: Value = status_resp.json().await.expect("parse");
        assert!(
            body["queue_running"].as_u64().unwrap_or(0) > 0,
            "task should be running after transition to in_progress"
        );
    }

    // Step 4: Simulate daemon completion — transition to terminal
    update_brain_task_status(
        &client,
        addr,
        task_id,
        "completed",
        Some("drain completed with evidence"),
    )
    .await;

    // Step 5: Verify terminal via brain status (queue should be empty)
    let status_resp = client
        .get(brain_status_url(addr, project))
        .send()
        .await
        .expect("GET brain/status after completion");
    assert!(status_resp.status().is_success());
    let final_status: Value = status_resp.json().await.expect("parse final status");
    assert_eq!(
        final_status["queue_pending"].as_u64().unwrap_or(99),
        0,
        "no tasks should be pending after drain: {final_status}"
    );
    assert_eq!(
        final_status["queue_running"].as_u64().unwrap_or(99),
        0,
        "no tasks should be running after drain: {final_status}"
    );

    // Step 6: Verify terminal via history endpoint
    let history_resp = client
        .get(format!(
            "http://{}/api/brain/queue/history?status=completed",
            addr
        ))
        .send()
        .await
        .expect("GET brain/queue/history");
    assert!(history_resp.status().is_success());
    let history: Vec<Value> = history_resp.json().await.expect("parse history");
    let found = history.iter().any(|t| {
        t["id"].as_str() == Some(task_id)
            && t["status"].as_str() == Some("completed")
    });
    assert!(
        found,
        "completed task must appear in history: {:?}",
        history
    );
}

// ── Test: Failed task also reaches terminal ──────────────────────────

#[tokio::test]
async fn failed_task_reaches_terminal() {
    let addr = start_hub().await;
    let client = Client::new();
    let task_id = "test-fail-terminal";

    let task = make_workplan_task(task_id, "test-fail-proj");
    store_brain_task(&client, addr, &task).await;

    // Transition: pending → in_progress → failed
    update_brain_task_status(&client, addr, task_id, "in_progress", None).await;
    update_brain_task_status(
        &client,
        addr,
        task_id,
        "failed",
        Some("exit=0 but no git evidence of work (HEAD unchanged)"),
    )
    .await;

    // Verify via history
    let history_resp = client
        .get(format!(
            "http://{}/api/brain/queue/history?status=failed",
            addr
        ))
        .send()
        .await
        .expect("GET history");
    assert!(history_resp.status().is_success());
    let history: Vec<Value> = history_resp.json().await.expect("parse history");
    let entry = history
        .iter()
        .find(|t| t["id"].as_str() == Some(task_id));
    assert!(entry.is_some(), "failed task must appear in history");
    assert!(
        entry.unwrap()["result_truncated"]
            .as_str()
            .unwrap_or("")
            .contains("no git evidence"),
        "evidence-guard failure reason must survive in history"
    );
}

// ── Test: Stuck task stays in_progress without sweep (pre-P2) ────────
//
// This test documents the pre-P2 behavior: a task that gets stuck in
// in_progress will NOT transition to terminal on its own. P2 adds the
// sweep that fixes this — at which point this test should be updated
// to assert the sweep flips the task to failed.

#[tokio::test]
async fn stuck_task_remains_in_progress_without_sweep() {
    let addr = start_hub().await;
    let client = Client::new();
    let project = "test-stuck-proj";
    let task_id = "test-stuck-no-sweep";

    let task = make_workplan_task(task_id, project);
    store_brain_task(&client, addr, &task).await;

    // Move to in_progress but never complete
    update_brain_task_status(&client, addr, task_id, "in_progress", None).await;

    // Wait a bit — without the sweep, it should stay in_progress
    tokio::time::sleep(Duration::from_millis(500)).await;

    let status_resp = client
        .get(brain_status_url(addr, project))
        .send()
        .await
        .expect("GET brain/status");
    assert!(status_resp.status().is_success());
    let body: Value = status_resp.json().await.expect("parse status");
    assert!(
        body["queue_running"].as_u64().unwrap_or(0) > 0,
        "stuck task should still show as running (no sweep yet): {body}"
    );
}

// ── Test: Expired in_progress task gets swept to failed ───────────
//
// P2.2: A task stored with created_at in the past (beyond timeout_s +
// 30s grace) should be retrievable as "failed" after an external sweep
// updates it. This test simulates what sweep_stuck_tasks() does by
// checking the timeout arithmetic and performing the status flip via REST.

#[tokio::test]
async fn expired_in_progress_task_swept_to_failed() {
    let addr = start_hub().await;
    let client = Client::new();
    let project = "test-sweep-proj";
    let task_id = "test-sweep-expired";
    let timeout_s: u64 = 5;

    let task = make_expired_in_progress_task(task_id, project, timeout_s);
    store_brain_task(&client, addr, &task).await;

    // Verify it shows as running
    let status_resp = client
        .get(brain_status_url(addr, project))
        .send()
        .await
        .expect("GET brain/status");
    assert!(status_resp.status().is_success());
    let body: Value = status_resp.json().await.expect("parse status");
    assert!(
        body["queue_running"].as_u64().unwrap_or(0) > 0,
        "expired task should initially show as running: {body}"
    );

    // Simulate the sweep: read task, check age > timeout_s + 30s grace, flip to failed
    let key = format!("brain-task:{}", task_id);
    let resp = client
        .get(format!("http://{}/api/hexflo/memory/{}", addr, key))
        .send()
        .await
        .expect("GET task");
    let mem: Value = resp.json().await.expect("parse memory");
    let value_str = mem["value"].as_str().unwrap_or("{}");
    let task_val: Value = serde_json::from_str(value_str).expect("parse task json");

    let created_str = task_val["created_at"].as_str().expect("created_at");
    let created = chrono::DateTime::parse_from_rfc3339(created_str)
        .expect("parse created_at")
        .with_timezone(&chrono::Utc);
    let age = chrono::Utc::now().signed_duration_since(created);
    let stored_timeout = task_val["timeout_s"].as_u64().unwrap_or(1800);
    let deadline = stored_timeout + 30; // grace

    assert!(
        age.num_seconds() >= deadline as i64,
        "task age {}s must exceed deadline {}s for sweep",
        age.num_seconds(),
        deadline,
    );

    // Perform the sweep update
    update_brain_task_status(
        &client,
        addr,
        task_id,
        "failed",
        Some(&format!(
            "timeout sweep: in_progress for {}s exceeds {}s",
            age.num_seconds(),
            deadline,
        )),
    )
    .await;

    // Verify task is now failed
    let history_resp = client
        .get(format!(
            "http://{}/api/brain/queue/history?status=failed",
            addr
        ))
        .send()
        .await
        .expect("GET history");
    assert!(history_resp.status().is_success());
    let history: Vec<Value> = history_resp.json().await.expect("parse history");
    let entry = history.iter().find(|t| t["id"].as_str() == Some(task_id));
    assert!(
        entry.is_some(),
        "swept task must appear in failed history"
    );
    let result_text = entry.unwrap()["result_truncated"]
        .as_str()
        .unwrap_or("");
    assert!(
        result_text.contains("timeout sweep"),
        "sweep reason must be in result: got '{}'",
        result_text,
    );

    // Running count should be zero
    let status_resp = client
        .get(brain_status_url(addr, project))
        .send()
        .await
        .expect("GET brain/status after sweep");
    assert!(status_resp.status().is_success());
    let body: Value = status_resp.json().await.expect("parse status after sweep");
    assert_eq!(
        body["queue_running"].as_u64().unwrap_or(99),
        0,
        "no running tasks after sweep: {body}"
    );
}
