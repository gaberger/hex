//! Path B P1.3 — integration test for the GET /api/workplan/execute/{id}/status
//! endpoint contract that hex-cli's polling loop depends on (P1.2).
//!
//! Why a stub router and not the real `WorkplanExecutor`?
//! Constructing a real `WorkplanExecutor` requires SpacetimeDB connections,
//! state ports, hexflo coordination, etc. — wiring all of that just to validate
//! HTTP shape would be architecture-as-test. Instead this file pins the wire
//! contract: POST returns `execution_id`, GET returns `status` transitions and
//! 404s on unknown ids, with the exact JSON keys hex-cli expects. If the
//! production handler in `routes/workplan.rs` ever drifts from this shape, the
//! CLI would silently break — keeping the contract anchored here is the goal.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::{Path, State},
    routing::{get, post},
    Json, Router,
};
use http::StatusCode;
use serde_json::{json, Value};
use tokio::sync::RwLock;

#[derive(Clone, Default)]
struct StubState {
    /// execution_id -> (status, optional result)
    executions: Arc<RwLock<HashMap<String, (String, Option<String>)>>>,
}

impl StubState {
    async fn insert_running(&self, id: &str) {
        self.executions
            .write()
            .await
            .insert(id.to_string(), ("running".to_string(), None));
    }

    async fn set_completed(&self, id: &str, result: &str) {
        self.executions
            .write()
            .await
            .insert(id.to_string(), ("completed".to_string(), Some(result.to_string())));
    }
}

async fn stub_post_execute(
    State(state): State<StubState>,
    Json(_body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let id = uuid::Uuid::new_v4().to_string();
    state.insert_running(&id).await;
    (
        StatusCode::OK,
        Json(json!({
            "execution_id": id,
            "status": "started",
        })),
    )
}

async fn stub_get_status(
    State(state): State<StubState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<Value>) {
    let map = state.executions.read().await;
    match map.get(&id) {
        Some((status, result)) => (
            StatusCode::OK,
            Json(json!({
                "status": status,
                "result": result,
                "head_before": Value::Null,
                "head_after": Value::Null,
            })),
        ),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": format!("Execution '{}' not found", id) })),
        ),
    }
}

fn build_app(state: StubState) -> Router {
    Router::new()
        .route("/api/workplan/execute", post(stub_post_execute))
        .route(
            "/api/workplan/execute/{id}/status",
            get(stub_get_status),
        )
        .with_state(state)
}

/// Spin up the router on an ephemeral port and return its base URL + a handle
/// the test can use to mutate stub state. The server runs as a background task
/// for the duration of the test.
async fn spawn_app() -> (String, StubState) {
    let state = StubState::default();
    let app = build_app(state.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local_addr");
    let base = format!("http://{}", addr);

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    // Yield so the server task gets scheduled before the test makes its first
    // request. Without this the very first request can race the bind.
    tokio::time::sleep(Duration::from_millis(20)).await;

    (base, state)
}

#[tokio::test]
async fn post_execute_returns_execution_id() {
    let (base, _state) = spawn_app().await;
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/api/workplan/execute", base))
        .json(&json!({ "workplanPath": "/tmp/wp.json" }))
        .send()
        .await
        .expect("POST execute");

    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let body: Value = resp.json().await.expect("json body");
    let id = body
        .get("execution_id")
        .and_then(|v| v.as_str())
        .expect("execution_id missing");
    assert!(!id.is_empty(), "execution_id must be non-empty");
}

#[tokio::test]
async fn status_immediately_after_post_is_running() {
    let (base, _state) = spawn_app().await;
    let client = reqwest::Client::new();
    let resp: Value = client
        .post(format!("{}/api/workplan/execute", base))
        .json(&json!({ "workplanPath": "/tmp/wp.json" }))
        .send()
        .await
        .expect("POST")
        .json()
        .await
        .expect("json");
    let id = resp["execution_id"].as_str().unwrap().to_string();

    let status: Value = client
        .get(format!("{}/api/workplan/execute/{}/status", base, id))
        .send()
        .await
        .expect("GET status")
        .json()
        .await
        .expect("json");

    assert_eq!(status["status"], "running");
    // head_before / head_after are explicitly null in this phase — pin that.
    assert!(status["head_before"].is_null());
    assert!(status["head_after"].is_null());
}

#[tokio::test]
async fn status_transitions_to_completed() {
    let (base, state) = spawn_app().await;
    let client = reqwest::Client::new();
    let resp: Value = client
        .post(format!("{}/api/workplan/execute", base))
        .json(&json!({ "workplanPath": "/tmp/wp.json" }))
        .send()
        .await
        .expect("POST")
        .json()
        .await
        .expect("json");
    let id = resp["execution_id"].as_str().unwrap().to_string();

    // Simulate an async executor flipping the state after a brief delay.
    let state_for_task = state.clone();
    let id_for_task = id.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        state_for_task.set_completed(&id_for_task, "ok").await;
    });

    // Poll until completed or timeout (mirrors the CLI's poll loop).
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        if std::time::Instant::now() > deadline {
            panic!("status never transitioned to completed within 2s");
        }
        let status: Value = client
            .get(format!("{}/api/workplan/execute/{}/status", base, id))
            .send()
            .await
            .expect("GET")
            .json()
            .await
            .expect("json");
        if status["status"] == "completed" {
            assert_eq!(status["result"], "ok");
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn status_for_unknown_id_is_404() {
    let (base, _state) = spawn_app().await;
    let client = reqwest::Client::new();
    let bogus = uuid::Uuid::new_v4().to_string();
    let resp = client
        .get(format!("{}/api/workplan/execute/{}/status", base, bogus))
        .send()
        .await
        .expect("GET");
    assert_eq!(resp.status(), reqwest::StatusCode::NOT_FOUND);
}
