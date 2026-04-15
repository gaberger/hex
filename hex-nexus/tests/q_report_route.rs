//! Integration tests for GET /api/inference/q-report.
//!
//! Uses httpmock to stand in for SpacetimeDB's SQL endpoint so we can
//! seed deterministic q-entries + experiences without a live STDB.

use httpmock::prelude::*;
use serde_json::json;
use std::net::SocketAddr;

/// SpacetimeDB SQL HTTP response wire format used by `parse_stdb_response`.
fn stdb_response(columns: &[&str], rows: Vec<Vec<serde_json::Value>>) -> serde_json::Value {
    let elements: Vec<serde_json::Value> = columns
        .iter()
        .map(|c| json!({"name": {"some": c}}))
        .collect();
    json!([{ "schema": { "elements": elements }, "rows": rows }])
}

/// Build the 5 q-entry fixture rows.
fn q_entry_rows() -> Vec<Vec<serde_json::Value>> {
    vec![
        // (state_key, action, q_value, visit_count, last_updated)
        vec![json!("scaffold_init"),     json!("qwen3:4b"),              json!(0.72), json!(120), json!("2026-04-15T10:00:00Z")],
        vec![json!("codegen_adapter"),   json!("qwen2.5-coder:32b"),    json!(0.85), json!(95),  json!("2026-04-14T08:00:00Z")],
        vec![json!("inference_router"),  json!("devstral-small-2:24b"),  json!(0.91), json!(40),  json!("2026-04-13T12:00:00Z")],
        vec![json!("codegen_test"),      json!("claude-sonnet-4-20250514"), json!(0.95), json!(200), json!("2026-04-15T14:00:00Z")],
        vec![json!("transform_format"), json!("qwen3:4b"),              json!(0.60), json!(30),  json!("2026-04-10T06:00:00Z")],
    ]
}

/// Build the 10 experience fixture rows — 2 per q-entry, all within last 7 days.
fn experience_rows() -> Vec<Vec<serde_json::Value>> {
    vec![
        // (state_key, action, reward, timestamp)
        vec![json!("scaffold_init"),    json!("qwen3:4b"),              json!(0.80), json!("2026-04-14T09:00:00Z")],
        vec![json!("scaffold_init"),    json!("qwen3:4b"),              json!(0.70), json!("2026-04-13T09:00:00Z")],
        vec![json!("codegen_adapter"),  json!("qwen2.5-coder:32b"),    json!(0.90), json!("2026-04-13T10:00:00Z")],
        vec![json!("codegen_adapter"),  json!("qwen2.5-coder:32b"),    json!(0.80), json!("2026-04-12T10:00:00Z")],
        vec![json!("inference_router"), json!("devstral-small-2:24b"),  json!(0.95), json!("2026-04-12T14:00:00Z")],
        vec![json!("inference_router"), json!("devstral-small-2:24b"),  json!(0.85), json!("2026-04-11T14:00:00Z")],
        vec![json!("codegen_test"),     json!("claude-sonnet-4-20250514"), json!(0.98), json!("2026-04-14T16:00:00Z")],
        vec![json!("codegen_test"),     json!("claude-sonnet-4-20250514"), json!(0.92), json!("2026-04-13T16:00:00Z")],
        vec![json!("transform_format"),json!("qwen3:4b"),              json!(0.65), json!("2026-04-09T08:00:00Z")],
        vec![json!("transform_format"),json!("qwen3:4b"),              json!(0.55), json!("2026-04-08T08:00:00Z")],
    ]
}

fn q_entry_stdb_body() -> serde_json::Value {
    stdb_response(
        &["state_key", "action", "q_value", "visit_count", "last_updated"],
        q_entry_rows(),
    )
}

fn experience_stdb_body() -> serde_json::Value {
    stdb_response(
        &["state_key", "action", "reward", "timestamp"],
        experience_rows(),
    )
}

/// Start the hex-nexus test server with SPACETIMEDB_HOST pointed at the mock.
async fn start_test_server(mock_host: &str) -> SocketAddr {
    unsafe { std::env::set_var("SPACETIMEDB_HOST", mock_host) };

    let config = hex_nexus::HubConfig {
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

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    addr
}

/// Mock both STDB SQL queries (rl_q_entry and rl_experience) and return
/// the appropriate fixture based on the SQL body.
fn mock_stdb_sql(server: &MockServer) {
    server.mock(|when, then| {
        when.method(POST)
            .path_contains("/v1/database/rl-engine/sql")
            .body_contains("rl_q_entry");
        then.status(200)
            .header("content-type", "application/json")
            .json_body(q_entry_stdb_body());
    });

    server.mock(|when, then| {
        when.method(POST)
            .path_contains("/v1/database/rl-engine/sql")
            .body_contains("rl_experience");
        then.status(200)
            .header("content-type", "application/json")
            .json_body(experience_stdb_body());
    });
}

// ── Tests ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn q_report_returns_all_entries_unfiltered() {
    let server = MockServer::start();
    mock_stdb_sql(&server);
    let addr = start_test_server(&server.base_url()).await;

    let resp: serde_json::Value = reqwest::get(format!("http://{}/api/inference/q-report", addr))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(resp["ok"], true);
    assert_eq!(resp["count"], 5);
    assert_eq!(resp["sort"], "visits");
    assert_eq!(resp["entries"].as_array().unwrap().len(), 5);
}

#[tokio::test]
async fn q_report_default_sort_is_visits_descending() {
    let server = MockServer::start();
    mock_stdb_sql(&server);
    let addr = start_test_server(&server.base_url()).await;

    let resp: serde_json::Value = reqwest::get(format!("http://{}/api/inference/q-report", addr))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let entries = resp["entries"].as_array().unwrap();
    let visits: Vec<u64> = entries
        .iter()
        .map(|e| e["visit_count"].as_u64().unwrap())
        .collect();
    // codegen_test:200, scaffold_init:120, codegen_adapter:95, inference_router:40, transform_format:30
    assert_eq!(visits, vec![200, 120, 95, 40, 30]);
}

#[tokio::test]
async fn q_report_sort_by_q_value() {
    let server = MockServer::start();
    mock_stdb_sql(&server);
    let addr = start_test_server(&server.base_url()).await;

    let resp: serde_json::Value =
        reqwest::get(format!("http://{}/api/inference/q-report?sort=q", addr))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();

    assert_eq!(resp["sort"], "q");
    let entries = resp["entries"].as_array().unwrap();
    let q_vals: Vec<f64> = entries
        .iter()
        .map(|e| e["q_value"].as_f64().unwrap())
        .collect();
    // 0.95, 0.91, 0.85, 0.72, 0.60 (descending)
    assert_eq!(q_vals, vec![0.95, 0.91, 0.85, 0.72, 0.60]);
}

#[tokio::test]
async fn q_report_sort_by_recency() {
    let server = MockServer::start();
    mock_stdb_sql(&server);
    let addr = start_test_server(&server.base_url()).await;

    let resp: serde_json::Value =
        reqwest::get(format!("http://{}/api/inference/q-report?sort=recency", addr))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();

    assert_eq!(resp["sort"], "recency");
    let entries = resp["entries"].as_array().unwrap();
    let keys: Vec<&str> = entries
        .iter()
        .map(|e| e["state_key"].as_str().unwrap())
        .collect();
    // Most recent first: codegen_test (14:00), scaffold_init (10:00 on 4/15),
    // codegen_adapter (08:00 on 4/14), inference_router (12:00 on 4/13), transform_format (06:00 on 4/10)
    assert_eq!(
        keys,
        vec![
            "codegen_test",
            "scaffold_init",
            "codegen_adapter",
            "inference_router",
            "transform_format"
        ]
    );
}

#[tokio::test]
async fn q_report_filter_by_tier() {
    let server = MockServer::start();
    mock_stdb_sql(&server);
    let addr = start_test_server(&server.base_url()).await;

    // T1 = qwen3:4b → scaffold_init + transform_format
    let resp: serde_json::Value =
        reqwest::get(format!("http://{}/api/inference/q-report?tier=t1", addr))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();

    assert_eq!(resp["count"], 2);
    let entries = resp["entries"].as_array().unwrap();
    for e in entries {
        assert_eq!(e["tier"], "t1");
    }

    // T3 = claude → codegen_test only
    let resp: serde_json::Value =
        reqwest::get(format!("http://{}/api/inference/q-report?tier=t3", addr))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();

    assert_eq!(resp["count"], 1);
    assert_eq!(resp["entries"][0]["state_key"], "codegen_test");
}

#[tokio::test]
async fn q_report_filter_by_task_type() {
    let server = MockServer::start();
    mock_stdb_sql(&server);
    let addr = start_test_server(&server.base_url()).await;

    // task_type is first segment before '_': "codegen" matches codegen_adapter + codegen_test
    let resp: serde_json::Value =
        reqwest::get(format!(
            "http://{}/api/inference/q-report?task_type=codegen",
            addr
        ))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(resp["count"], 2);
    let entries = resp["entries"].as_array().unwrap();
    for e in entries {
        assert_eq!(e["task_type"], "codegen");
    }
}

#[tokio::test]
async fn q_report_filter_by_model_substring() {
    let server = MockServer::start();
    mock_stdb_sql(&server);
    let addr = start_test_server(&server.base_url()).await;

    let resp: serde_json::Value =
        reqwest::get(format!(
            "http://{}/api/inference/q-report?model=devstral",
            addr
        ))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(resp["count"], 1);
    assert_eq!(resp["entries"][0]["action"], "devstral-small-2:24b");
    assert_eq!(resp["entries"][0]["tier"], "t2.5");
}

#[tokio::test]
async fn q_report_limit_truncates() {
    let server = MockServer::start();
    mock_stdb_sql(&server);
    let addr = start_test_server(&server.base_url()).await;

    let resp: serde_json::Value =
        reqwest::get(format!("http://{}/api/inference/q-report?limit=2", addr))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();

    assert_eq!(resp["count"], 2);
    assert_eq!(resp["entries"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn q_report_trend_7d_computed_correctly() {
    let server = MockServer::start();
    mock_stdb_sql(&server);
    let addr = start_test_server(&server.base_url()).await;

    let resp: serde_json::Value =
        reqwest::get(format!(
            "http://{}/api/inference/q-report?sort=q&limit=50",
            addr
        ))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let entries = resp["entries"].as_array().unwrap();

    // codegen_test: q=0.95, experiences rewards=[0.98, 0.92], mean=0.95, trend=0.95-0.95=0.0
    let codegen_test = entries.iter().find(|e| e["state_key"] == "codegen_test").unwrap();
    assert_eq!(codegen_test["trend_7d"], 0.0);

    // scaffold_init: q=0.72, experiences rewards=[0.80, 0.70], mean=0.75, trend=0.75-0.72=0.03
    let scaffold = entries.iter().find(|e| e["state_key"] == "scaffold_init").unwrap();
    assert_eq!(scaffold["trend_7d"], 0.03);

    // codegen_adapter: q=0.85, experiences rewards=[0.90, 0.80], mean=0.85, trend=0.85-0.85=0.0
    let codegen_adapter = entries.iter().find(|e| e["state_key"] == "codegen_adapter").unwrap();
    assert_eq!(codegen_adapter["trend_7d"], 0.0);

    // inference_router: q=0.91, experiences rewards=[0.95, 0.85], mean=0.90, trend=0.90-0.91=-0.01
    let inference = entries.iter().find(|e| e["state_key"] == "inference_router").unwrap();
    assert_eq!(inference["trend_7d"], -0.01);
}

#[tokio::test]
async fn q_report_tier_inference_correctness() {
    let server = MockServer::start();
    mock_stdb_sql(&server);
    let addr = start_test_server(&server.base_url()).await;

    let resp: serde_json::Value = reqwest::get(format!("http://{}/api/inference/q-report", addr))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let entries = resp["entries"].as_array().unwrap();

    let tier_of = |key: &str| -> &str {
        entries
            .iter()
            .find(|e| e["state_key"].as_str().unwrap() == key)
            .unwrap()["tier"]
            .as_str()
            .unwrap()
    };

    assert_eq!(tier_of("scaffold_init"), "t1");
    assert_eq!(tier_of("codegen_adapter"), "t2");
    assert_eq!(tier_of("inference_router"), "t2.5");
    assert_eq!(tier_of("codegen_test"), "t3");
    assert_eq!(tier_of("transform_format"), "t1");
}

#[tokio::test]
async fn q_report_combined_filters() {
    let server = MockServer::start();
    mock_stdb_sql(&server);
    let addr = start_test_server(&server.base_url()).await;

    // tier=t1 AND task_type=scaffold → only scaffold_init
    let resp: serde_json::Value = reqwest::get(format!(
        "http://{}/api/inference/q-report?tier=t1&task_type=scaffold",
        addr
    ))
    .await
    .unwrap()
    .json()
    .await
    .unwrap();

    assert_eq!(resp["count"], 1);
    assert_eq!(resp["entries"][0]["state_key"], "scaffold_init");
    assert_eq!(resp["entries"][0]["tier"], "t1");
    assert_eq!(resp["entries"][0]["task_type"], "scaffold");
}
