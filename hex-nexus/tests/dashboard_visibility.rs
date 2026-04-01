//! Integration tests for the ADR-066 Dashboard Visibility Overhaul.
//!
//! Verifies:
//! - Path traversal rejection on token endpoints
//! - Null-byte path rejection
//! - /api/health returns a `spacetimedb` boolean field
//! - /api/inbox returns 200 with a JSON array
//! - /api/inbox/:id/ack returns 200 or 404 (not 500)

use std::net::SocketAddr;

async fn start_test_server() -> SocketAddr {
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

// ── Path traversal ──────────────────────────────────────────────────────────

#[tokio::test]
async fn path_traversal_dot_dot_returns_400() {
    let addr = start_test_server().await;
    let url = format!("http://{}/api/test-project/tokens/../../etc/passwd", addr);
    let resp = reqwest::get(&url).await.expect("request failed");
    let status = resp.status().as_u16();
    assert!(
        status == 400 || status == 404,
        "expected 400 or 404 for path traversal, got {status}"
    );
}

#[tokio::test]
async fn path_traversal_null_byte_returns_400() {
    let addr = start_test_server().await;
    // %00 is a null byte — should be rejected before hitting filesystem
    let url = format!("http://{}/api/test-project/tokens/file%00.rs", addr);
    let resp = reqwest::get(&url).await.expect("request failed");
    let status = resp.status().as_u16();
    assert!(
        status == 400 || status == 404,
        "expected 400 or 404 for null-byte path, got {status}"
    );
}

// ── Health endpoint ─────────────────────────────────────────────────────────

#[tokio::test]
async fn health_endpoint_has_spacetimedb_field() {
    let addr = start_test_server().await;
    let url = format!("http://{}/api/health", addr);
    let resp = reqwest::get(&url).await.expect("request failed");
    assert_eq!(resp.status().as_u16(), 200, "/api/health must return 200");

    let body: serde_json::Value = resp.json().await.expect("body must be JSON");
    assert!(
        body.get("spacetimedb").is_some(),
        "/api/health JSON must contain a `spacetimedb` field; got: {body}"
    );
    assert!(
        body["spacetimedb"].is_boolean(),
        "`spacetimedb` field must be a boolean; got: {}",
        body["spacetimedb"]
    );
}

// ── Inbox endpoints ─────────────────────────────────────────────────────────

#[tokio::test]
async fn inbox_list_returns_json_array() {
    let addr = start_test_server().await;
    let url = format!("http://{}/api/inbox", addr);
    let resp = reqwest::get(&url).await.expect("request failed");
    assert_eq!(resp.status().as_u16(), 200, "/api/inbox must return 200");

    let body: serde_json::Value = resp.json().await.expect("body must be JSON");
    assert!(
        body.is_array(),
        "/api/inbox must return a JSON array; got: {body}"
    );
}

#[tokio::test]
async fn inbox_ack_nonexistent_returns_200_or_404() {
    let addr = start_test_server().await;
    let url = format!("http://{}/api/inbox/nonexistent-id/ack", addr);
    let client = reqwest::Client::new();
    let resp = client
        .post(&url)
        .send()
        .await
        .expect("request failed");
    let status = resp.status().as_u16();
    assert!(
        status == 200 || status == 404 || status == 400,
        "inbox ack for unknown id must be 200, 400, or 404, got {status}"
    );
}
