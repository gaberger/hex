//! Integration tests for hex-desktop commands using wiremock.
//!
//! These tests spin up a mock HTTP server that simulates the hex-hub API,
//! then exercise the extracted helper functions (hub_get, hub_post, hub_delete)
//! and verify serialization contracts, error handling, and edge cases.

use hex_desktop::commands::{hub_delete, hub_get, hub_post, HubState};
use wiremock::matchers::{method, path, body_json};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Helper: create a HubState pointing at the mock server.
fn mock_hub(server: &MockServer) -> HubState {
    HubState::with_base_url(&server.uri())
}

// ─── list_agents ────────────────────────────────────────────────

#[tokio::test]
async fn list_agents_returns_empty_array() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/agents"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({ "agents": [] })),
        )
        .mount(&server)
        .await;

    let hub = mock_hub(&server);
    let result = hub_get(&hub, "/api/agents").await.unwrap();

    assert_eq!(result["agents"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn list_agents_returns_populated_list() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/agents"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "agents": [
                { "id": "agent-1", "name": "hex-coder", "status": "running" },
                { "id": "agent-2", "name": "planner", "status": "idle" }
            ]
        })))
        .mount(&server)
        .await;

    let hub = mock_hub(&server);
    let result = hub_get(&hub, "/api/agents").await.unwrap();
    let agents = result["agents"].as_array().unwrap();

    assert_eq!(agents.len(), 2);
    assert_eq!(agents[0]["id"], "agent-1");
    assert_eq!(agents[1]["name"], "planner");
}

#[tokio::test]
async fn list_agents_server_error_returns_err() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/agents"))
        .respond_with(
            ResponseTemplate::new(500)
                .set_body_json(serde_json::json!({ "error": "internal server error" })),
        )
        .mount(&server)
        .await;

    let hub = mock_hub(&server);
    let result = hub_get(&hub, "/api/agents").await;

    assert!(result.is_err());
    assert_eq!(result.unwrap_err(), "internal server error");
}

// ─── spawn_agent ────────────────────────────────────────────────

#[tokio::test]
async fn spawn_agent_success() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/agents/spawn"))
        .and(body_json(serde_json::json!({
            "projectDir": "/tmp/my-project",
            "agentName": "hex-coder"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "agent-42",
            "status": "spawned"
        })))
        .mount(&server)
        .await;

    let hub = mock_hub(&server);
    let body = serde_json::json!({
        "projectDir": "/tmp/my-project",
        "agentName": "hex-coder"
    });
    let result = hub_post(&hub, "/api/agents/spawn", &body).await.unwrap();

    assert_eq!(result["id"], "agent-42");
    assert_eq!(result["status"], "spawned");
}

#[tokio::test]
async fn spawn_agent_conflict_returns_error() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/agents/spawn"))
        .respond_with(
            ResponseTemplate::new(409)
                .set_body_json(serde_json::json!({ "error": "agent already exists" })),
        )
        .mount(&server)
        .await;

    let hub = mock_hub(&server);
    let body = serde_json::json!({
        "projectDir": "/tmp/proj",
        "agentName": "hex-coder"
    });
    let result = hub_post(&hub, "/api/agents/spawn", &body).await;

    assert!(result.is_err());
    assert_eq!(result.unwrap_err(), "agent already exists");
}

// ─── kill_agent ─────────────────────────────────────────────────

#[tokio::test]
async fn kill_agent_success() {
    let server = MockServer::start().await;

    Mock::given(method("DELETE"))
        .and(path("/api/agents/agent-42"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "agent-42",
            "status": "terminated"
        })))
        .mount(&server)
        .await;

    let hub = mock_hub(&server);
    let result = hub_delete(&hub, "/api/agents/agent-42").await.unwrap();

    assert_eq!(result["status"], "terminated");
}

#[tokio::test]
async fn kill_agent_not_found() {
    let server = MockServer::start().await;

    Mock::given(method("DELETE"))
        .and(path("/api/agents/nonexistent"))
        .respond_with(
            ResponseTemplate::new(404)
                .set_body_json(serde_json::json!({ "error": "agent not found" })),
        )
        .mount(&server)
        .await;

    let hub = mock_hub(&server);
    let result = hub_delete(&hub, "/api/agents/nonexistent").await;

    assert!(result.is_err());
    assert_eq!(result.unwrap_err(), "agent not found");
}

// ─── error edge cases ───────────────────────────────────────────

#[tokio::test]
async fn error_response_without_error_field_returns_unknown() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/agents"))
        .respond_with(
            ResponseTemplate::new(503).set_body_json(serde_json::json!({ "message": "unavailable" })),
        )
        .mount(&server)
        .await;

    let hub = mock_hub(&server);
    let result = hub_get(&hub, "/api/agents").await;

    assert!(result.is_err());
    assert_eq!(result.unwrap_err(), "Unknown error");
}

#[tokio::test]
async fn connection_refused_returns_http_error() {
    // Point at a port nothing is listening on
    let hub = HubState::with_base_url("http://127.0.0.1:1");
    let result = hub_get(&hub, "/api/agents").await;

    assert!(result.is_err());
    assert!(result.unwrap_err().contains("HTTP request failed"));
}

// ─── HubStatus serialization ────────────────────────────────────

#[test]
fn hub_status_camel_case_serialization() {
    use hex_desktop::commands::HubStatus;

    let status = HubStatus {
        running: true,
        port: 5555,
        version: "1.0.0".to_string(),
        build_hash: "abc123".to_string(),
        uptime_secs: 42,
        active_agents: 3,
    };

    let json = serde_json::to_value(&status).unwrap();

    // Verify camelCase field names (serde rename_all)
    assert_eq!(json["running"], true);
    assert_eq!(json["port"], 5555);
    assert_eq!(json["version"], "1.0.0");
    assert_eq!(json["buildHash"], "abc123");
    assert_eq!(json["uptimeSecs"], 42);
    assert_eq!(json["activeAgents"], 3);

    // Verify NO snake_case leaks
    assert!(json.get("build_hash").is_none());
    assert!(json.get("uptime_secs").is_none());
    assert!(json.get("active_agents").is_none());
}

// ─── HubState construction ──────────────────────────────────────

#[test]
fn hub_state_default_base_url() {
    let state = HubState::new(5555);
    assert_eq!(state.base_url(), "http://127.0.0.1:5555");
}

#[test]
fn hub_state_custom_port() {
    let state = HubState::new(9090);
    assert_eq!(state.base_url(), "http://127.0.0.1:9090");
}

#[test]
fn hub_state_with_override_url() {
    let state = HubState::with_base_url("http://localhost:12345");
    assert_eq!(state.base_url(), "http://localhost:12345");
}
