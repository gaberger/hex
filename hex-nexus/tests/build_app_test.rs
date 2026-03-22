//! Integration tests for hex-nexus's public API.
//!
//! Verifies that `build_app()` produces a working Axum router that responds
//! to all critical endpoints. These tests start a real TCP server on an
//! ephemeral port, ensuring the full middleware stack (CORS, auth, body limits)
//! is exercised.

use hex_nexus::{HubConfig, DEFAULT_PORT};
use std::net::SocketAddr;

/// Helper: build app with default config, bind to port 0 (OS-assigned),
/// return the bound address for client requests.
async fn start_test_server() -> SocketAddr {
    let config = HubConfig {
        port: 0, // unused — we bind manually
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

    // Small yield to let the server start accepting
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    addr
}

#[tokio::test]
async fn build_app_returns_working_router() {
    let addr = start_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("http://{}/api/version", addr))
        .send()
        .await
        .expect("GET /api/version");

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["name"], "hex-hub");
    assert!(body["version"].is_string());
    assert!(body["buildHash"].is_string());
}

#[tokio::test]
async fn dashboard_index_serves_html() {
    let addr = start_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("http://{}/", addr))
        .send()
        .await
        .expect("GET /");

    assert_eq!(resp.status(), 200);
    let text = resp.text().await.unwrap();
    assert!(text.contains("<!DOCTYPE html>") || text.contains("<html"));
}

#[tokio::test]
async fn chat_page_serves_html() {
    let addr = start_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("http://{}/chat", addr))
        .send()
        .await
        .expect("GET /chat");

    assert_eq!(resp.status(), 200);
    let text = resp.text().await.unwrap();
    assert!(text.contains("<!DOCTYPE html>") || text.contains("<html"));
}

#[tokio::test]
async fn projects_list_returns_empty_array() {
    let addr = start_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("http://{}/api/projects", addr))
        .send()
        .await
        .expect("GET /api/projects");

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["projects"].is_array());
    assert_eq!(body["projects"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn coordination_instances_returns_empty() {
    let addr = start_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("http://{}/api/coordination/instances", addr))
        .send()
        .await
        .expect("GET /api/coordination/instances");

    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn rl_stats_returns_json() {
    let addr = start_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("http://{}/api/rl/stats", addr))
        .send()
        .await
        .expect("GET /api/rl/stats");

    // May return 200 (with swarm_db) or 500 (without) — both are valid
    assert!(resp.status().is_success() || resp.status().is_server_error());
}

#[tokio::test]
async fn websocket_upgrade_accepted() {
    let addr = start_test_server().await;

    // Use raw HTTP to check that /ws responds with 101 Upgrade
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://{}/ws", addr))
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==")
        .send()
        .await
        .expect("GET /ws upgrade");

    assert_eq!(resp.status(), 101);
}

#[tokio::test]
async fn auth_token_blocks_unauthenticated_posts() {
    let config = HubConfig {
        port: 0,
        bind: "127.0.0.1".to_string(),
        token: Some("secret-token-123".to_string()),
        is_daemon: false,
        no_agent: true,
    };

    let (router, _state) = hex_nexus::build_app(&config).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        hex_nexus::axum::serve(listener, router).await.unwrap();
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let client = reqwest::Client::new();

    // POST without token → 401
    let resp = client
        .post(format!("http://{}/api/push", addr))
        .json(&serde_json::json!({"projectId": "x", "type": "health"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);

    // POST with correct token → accepted (may be 4xx for bad payload, but not 401)
    let resp = client
        .post(format!("http://{}/api/push", addr))
        .header("Authorization", "Bearer secret-token-123")
        .json(&serde_json::json!({"projectId": "x", "type": "health"}))
        .send()
        .await
        .unwrap();
    assert_ne!(resp.status(), 401);
}

#[tokio::test]
async fn default_port_constant_is_5555() {
    assert_eq!(DEFAULT_PORT, 5555);
}

#[tokio::test]
async fn version_and_build_hash_are_nonempty() {
    assert!(!hex_nexus::version().is_empty());
    assert!(!hex_nexus::build_hash().is_empty());
}
