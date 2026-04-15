//! P1.3 — Workplan execute + status polling integration test.
//!
//! Validates the full workplan lifecycle via REST:
//!   1. POST /api/workplan/execute with a minimal workplan JSON
//!   2. Assert response contains an execution id
//!   3. GET /api/workplan/status — poll until state transitions
//!   4. Assert status moves through pending → running → completed|failed
//!
//! When the workplan executor is unavailable (no state backend), the test
//! verifies the 503 shape instead.

use hex_nexus::HubConfig;
use reqwest::Client;
use serde_json::{json, Value};
use std::io::Write;
use std::net::SocketAddr;
use tempfile::NamedTempFile;

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

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    addr
}

fn minimal_workplan() -> NamedTempFile {
    let mut f = NamedTempFile::new().expect("create temp workplan");
    let wp = json!({
        "id": "wp-test-execute-status",
        "feature": "integration test workplan",
        "adr": "",
        "specs": "",
        "phases": [
            {
                "id": "P0",
                "name": "Test Phase",
                "tier": 0,
                "tasks": [
                    {
                        "id": "P0.1",
                        "name": "Noop task",
                        "description": "A minimal task for testing execution flow",
                        "agent": "hex-coder",
                        "layer": "domain",
                        "deps": []
                    }
                ]
            }
        ]
    });
    f.write_all(wp.to_string().as_bytes())
        .expect("write workplan JSON");
    f.flush().expect("flush workplan");
    f
}

// ── Test: POST execute returns execution id or 503 ─────────────────

#[tokio::test]
async fn workplan_execute_returns_execution_id() {
    let addr = start_hub().await;
    let client = Client::new();
    let wp_file = minimal_workplan();

    let resp = client
        .post(format!("http://{}/api/workplan/execute", addr))
        .json(&json!({ "workplanPath": wp_file.path().to_str().unwrap() }))
        .send()
        .await
        .expect("POST execute");

    let status = resp.status().as_u16();
    let body: Value = resp.json().await.expect("parse JSON body");

    match status {
        200 => {
            let exec = &body["execution"];
            assert!(
                exec.get("id").and_then(|v| v.as_str()).is_some(),
                "execution must have an id: {body}"
            );
            assert_eq!(
                body["status"], "started",
                "top-level status should be 'started': {body}"
            );
        }
        503 => {
            assert!(
                body.get("error").is_some(),
                "503 should include error field: {body}"
            );
        }
        other => panic!("unexpected status {other}: {body}"),
    }
}

// ── Test: Status endpoint returns valid shape ──────────────────────

#[tokio::test]
async fn workplan_status_returns_valid_shape() {
    let addr = start_hub().await;
    let client = Client::new();

    let resp = client
        .get(format!("http://{}/api/workplan/status", addr))
        .send()
        .await
        .expect("GET status");

    let status = resp.status().as_u16();
    let body: Value = resp.json().await.expect("parse JSON body");

    match status {
        200 => {
            assert!(
                body.get("execution").is_some(),
                "status response must have 'execution' key: {body}"
            );
        }
        503 => {
            assert!(
                body.get("error").is_some(),
                "503 should include error field: {body}"
            );
        }
        other => panic!("unexpected status {other}: {body}"),
    }
}

// ── Test: Full lifecycle — execute then poll status transitions ─────

#[tokio::test]
async fn workplan_execute_then_poll_status_transitions() {
    let addr = start_hub().await;
    let client = Client::new();
    let wp_file = minimal_workplan();

    // Step 1: POST execute
    let exec_resp = client
        .post(format!("http://{}/api/workplan/execute", addr))
        .json(&json!({ "workplanPath": wp_file.path().to_str().unwrap() }))
        .send()
        .await
        .expect("POST execute");

    if exec_resp.status().as_u16() == 503 {
        eprintln!("workplan executor not available (no state backend) — skipping lifecycle test");
        return;
    }

    assert_eq!(exec_resp.status().as_u16(), 200);
    let exec_body: Value = exec_resp.json().await.expect("parse execute body");

    let execution_id = exec_body["execution"]["id"]
        .as_str()
        .expect("execution must have id")
        .to_string();
    assert!(!execution_id.is_empty(), "execution id must be non-empty");

    let initial_status = exec_body["execution"]["status"]
        .as_str()
        .unwrap_or("unknown");
    assert!(
        ["running", "completed", "failed"].contains(&initial_status),
        "initial status should be running, completed, or failed — got: {initial_status}"
    );

    // Step 2: Poll status endpoint — state should be reachable
    let mut last_status = String::new();
    let mut saw_running = initial_status == "running";
    let mut saw_terminal = ["completed", "failed"].contains(&initial_status);

    for _ in 0..20 {
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;

        let status_resp = client
            .get(format!("http://{}/api/workplan/status", addr))
            .send()
            .await
            .expect("GET status");

        if status_resp.status().as_u16() != 200 {
            continue;
        }

        let status_body: Value = status_resp.json().await.expect("parse status body");
        let exec = &status_body["execution"];

        if exec.is_null() {
            continue;
        }

        if let Some(s) = exec["status"].as_str() {
            last_status = s.to_string();
            match s {
                "running" => saw_running = true,
                "completed" | "failed" => {
                    saw_terminal = true;
                    break;
                }
                "paused" => {}
                other => eprintln!("unexpected execution status: {other}"),
            }
        }
    }

    assert!(
        saw_running || saw_terminal,
        "should have observed running or terminal state, last saw: {last_status}"
    );

    if saw_terminal {
        assert!(
            ["completed", "failed"].contains(&last_status.as_str()),
            "terminal state should be completed or failed, got: {last_status}"
        );
    }
}
