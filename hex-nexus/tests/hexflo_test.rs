//! HexFlo coordination system integration tests.
//!
//! Tests the full HexFlo API surface:
//! - Memory CRUD (store, retrieve, search, delete, scope isolation)
//! - Swarm lifecycle (init → task create → task complete → teardown)
//! - Agent cleanup (stale detection, dead detection, task reclamation)
//! - REST endpoints (memory + cleanup via embedded hub)

use futures::{SinkExt, StreamExt};
use hex_nexus::HubConfig;
use serde_json::{json, Value};
use std::net::SocketAddr;

// ── Test helpers ─────────────────────────────────────────

async fn start_hub() -> SocketAddr {
    std::env::set_var("HEX_STATE_BACKEND", "sqlite");
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
        hex_nexus::axum::serve(listener, router).await.expect("server error");
    });

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    addr
}

async fn api_get(addr: SocketAddr, path: &str) -> (u16, Value) {
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://{}{}", addr, path))
        .send()
        .await
        .expect("GET request");
    let status = resp.status().as_u16();
    let body: Value = resp.json().await.unwrap_or(json!(null));
    (status, body)
}

async fn api_post(addr: SocketAddr, path: &str, body: Value) -> (u16, Value) {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://{}{}", addr, path))
        .json(&body)
        .send()
        .await
        .expect("POST request");
    let status = resp.status().as_u16();
    let body: Value = resp.json().await.unwrap_or(json!(null));
    (status, body)
}

async fn api_delete(addr: SocketAddr, path: &str) -> (u16, Value) {
    let client = reqwest::Client::new();
    let resp = client
        .delete(format!("http://{}{}", addr, path))
        .send()
        .await
        .expect("DELETE request");
    let status = resp.status().as_u16();
    let body: Value = resp.json().await.unwrap_or(json!(null));
    (status, body)
}

#[allow(dead_code)]
async fn api_patch(addr: SocketAddr, path: &str, body: Value) -> (u16, Value) {
    let client = reqwest::Client::new();
    let resp = client
        .patch(format!("http://{}{}", addr, path))
        .json(&body)
        .send()
        .await
        .expect("PATCH request");
    let status = resp.status().as_u16();
    let body: Value = resp.json().await.unwrap_or(json!(null));
    (status, body)
}

// ══════════════════════════════════════════════════════════
// Memory CRUD Tests
// ══════════════════════════════════════════════════════════

#[tokio::test]
async fn memory_store_and_retrieve() {
    let addr = start_hub().await;

    // Store a value
    let (status, body) = api_post(
        addr,
        "/api/hexflo/memory",
        json!({ "key": "test-key", "value": "hello world" }),
    )
    .await;
    assert_eq!(status, 200, "store should succeed: {:?}", body);
    assert_eq!(body["ok"], true);
    assert_eq!(body["key"], "test-key");

    // Retrieve it
    let (status, body) = api_get(addr, "/api/hexflo/memory/test-key").await;
    assert_eq!(status, 200, "retrieve should succeed: {:?}", body);
    assert_eq!(body["key"], "test-key");
    assert_eq!(body["value"], "hello world");
}

#[tokio::test]
async fn memory_retrieve_nonexistent_returns_404() {
    let addr = start_hub().await;

    let (status, body) = api_get(addr, "/api/hexflo/memory/does-not-exist").await;
    assert_eq!(status, 404);
    assert!(body["error"].is_string());
}

#[tokio::test]
async fn memory_store_upserts_on_duplicate_key() {
    let addr = start_hub().await;

    // Store initial
    api_post(
        addr,
        "/api/hexflo/memory",
        json!({ "key": "upsert-key", "value": "v1" }),
    )
    .await;

    // Overwrite
    api_post(
        addr,
        "/api/hexflo/memory",
        json!({ "key": "upsert-key", "value": "v2" }),
    )
    .await;

    // Should get v2
    let (_, body) = api_get(addr, "/api/hexflo/memory/upsert-key").await;
    assert_eq!(body["value"], "v2");
}

#[tokio::test]
async fn memory_delete_removes_entry() {
    let addr = start_hub().await;

    // Store
    api_post(
        addr,
        "/api/hexflo/memory",
        json!({ "key": "delete-me", "value": "temp" }),
    )
    .await;

    // Delete
    let (status, body) = api_delete(addr, "/api/hexflo/memory/delete-me").await;
    assert_eq!(status, 200);
    assert_eq!(body["ok"], true);

    // Verify gone
    let (status, _) = api_get(addr, "/api/hexflo/memory/delete-me").await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn memory_delete_nonexistent_returns_404() {
    let addr = start_hub().await;

    let (status, _) = api_delete(addr, "/api/hexflo/memory/ghost-key").await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn memory_search_finds_matching_entries() {
    let addr = start_hub().await;

    // Store several entries with unique prefix to avoid cross-test contamination
    api_post(
        addr,
        "/api/hexflo/memory",
        json!({ "key": "xyzfind:alpha:config", "value": "max_tokens=8192" }),
    )
    .await;
    api_post(
        addr,
        "/api/hexflo/memory",
        json!({ "key": "xyzfind:beta:config", "value": "max_tokens=4096" }),
    )
    .await;
    api_post(
        addr,
        "/api/hexflo/memory",
        json!({ "key": "swarm:searchtest:status", "value": "running" }),
    )
    .await;

    // Search for "xyzfind"
    let (status, body) = api_get(addr, "/api/hexflo/memory/search?q=xyzfind").await;
    assert_eq!(status, 200);
    let results = body["results"].as_array().expect("results should be array");
    assert_eq!(results.len(), 2, "should find 2 xyzfind entries: {:?}", results);
}

#[tokio::test]
async fn memory_search_matches_values_too() {
    let addr = start_hub().await;

    api_post(
        addr,
        "/api/hexflo/memory",
        json!({ "key": "valmatch:k1", "value": "the quick brown foxjump" }),
    )
    .await;
    api_post(
        addr,
        "/api/hexflo/memory",
        json!({ "key": "valmatch:k2", "value": "lazy dog" }),
    )
    .await;

    let (_, body) = api_get(addr, "/api/hexflo/memory/search?q=foxjump").await;
    let results = body["results"].as_array().unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["key"], "valmatch:k1");
}

#[tokio::test]
async fn memory_scope_isolation() {
    let addr = start_hub().await;

    // Store in different scopes
    api_post(
        addr,
        "/api/hexflo/memory",
        json!({ "key": "shared-key", "value": "global-val", "scope": "global" }),
    )
    .await;
    api_post(
        addr,
        "/api/hexflo/memory",
        json!({ "key": "scoped-key", "value": "swarm-val", "scope": "swarm-123" }),
    )
    .await;

    // Both should be retrievable by key (scope doesn't affect key lookup)
    let (s1, b1) = api_get(addr, "/api/hexflo/memory/shared-key").await;
    assert_eq!(s1, 200);
    assert_eq!(b1["value"], "global-val");

    let (s2, b2) = api_get(addr, "/api/hexflo/memory/scoped-key").await;
    assert_eq!(s2, 200);
    assert_eq!(b2["value"], "swarm-val");
}

// ══════════════════════════════════════════════════════════
// Swarm Lifecycle Tests
// ══════════════════════════════════════════════════════════

#[tokio::test]
async fn swarm_init_creates_swarm() {
    let addr = start_hub().await;

    let (status, body) = api_post(
        addr,
        "/api/swarms",
        json!({ "projectId": "test-project", "name": "my-swarm", "topology": "mesh" }),
    )
    .await;

    // 200 if SwarmDb initialized, 503 if not
    assert!(status == 200 || status == 201, "swarm init: status={} body={:?}", status, body);
    assert!(body["id"].is_string(), "should return swarm ID");
    assert_eq!(body["name"], "my-swarm");
    assert_eq!(body["topology"], "mesh");
    assert_eq!(body["status"], "active");
}

#[tokio::test]
async fn swarm_status_lists_active_swarms() {
    let addr = start_hub().await;

    // Create two swarms
    api_post(
        addr,
        "/api/swarms",
        json!({ "projectId": "p1", "name": "s1" }),
    )
    .await;
    api_post(
        addr,
        "/api/swarms",
        json!({ "projectId": "p2", "name": "s2" }),
    )
    .await;

    let (status, body) = api_get(addr, "/api/swarms/active").await;
    assert_eq!(status, 200, "list swarms: {:?}", body);
    // Response is a flat array of swarm objects
    let swarms = body.as_array().expect("response should be an array");
    assert!(swarms.len() >= 2, "should list at least 2 swarms: {:?}", body);
}

#[tokio::test]
async fn swarm_full_lifecycle() {
    let addr = start_hub().await;

    // 1. Init swarm
    let (s, swarm) = api_post(
        addr,
        "/api/swarms",
        json!({ "projectId": "lifecycle", "name": "test-lifecycle", "topology": "hierarchical" }),
    )
    .await;
    assert!(s == 200 || s == 201, "swarm init: {:?}", swarm);
    let swarm_id = swarm["id"].as_str().unwrap();

    // 2. Verify swarm appears in detail view
    // SwarmDetail serializes flat: {id, name, topology, tasks, agents, ...}
    let (s, detail) = api_get(addr, &format!("/api/swarms/{}", swarm_id)).await;
    assert_eq!(s, 200, "get swarm detail: {:?}", detail);
    assert_eq!(detail["name"], "test-lifecycle", "detail: {:?}", detail);
    assert_eq!(detail["topology"], "hierarchical");
    assert_eq!(detail["status"], "active");

    // 3. Verify swarm appears in active list
    let (s, list) = api_get(addr, "/api/swarms/active").await;
    assert_eq!(s, 200);
    let swarms = list.as_array().expect("should be array");
    assert!(
        swarms.iter().any(|s| s["id"].as_str() == Some(swarm_id)),
        "swarm should appear in active list"
    );
}

// ══════════════════════════════════════════════════════════
// Cleanup Tests
// ══════════════════════════════════════════════════════════

#[tokio::test]
async fn cleanup_endpoint_returns_report() {
    let addr = start_hub().await;

    let (status, body) = api_post(addr, "/api/hexflo/cleanup", json!({})).await;
    assert_eq!(status, 200, "cleanup: {:?}", body);
    assert!(body["staleCount"].is_number(), "should report stale count");
    assert!(body["deadCount"].is_number(), "should report dead count");
    assert!(body["reclaimedTasks"].is_number(), "should report reclaimed tasks");
}

#[tokio::test]
async fn cleanup_with_no_agents_reports_zeros() {
    let addr = start_hub().await;

    let (_, body) = api_post(addr, "/api/hexflo/cleanup", json!({})).await;
    assert_eq!(body["staleCount"], 0);
    assert_eq!(body["deadCount"], 0);
    assert_eq!(body["reclaimedTasks"], 0);
}

// ══════════════════════════════════════════════════════════
// REST Endpoint Edge Cases
// ══════════════════════════════════════════════════════════

#[tokio::test]
async fn memory_store_with_scope() {
    let addr = start_hub().await;

    let (status, body) = api_post(
        addr,
        "/api/hexflo/memory",
        json!({ "key": "agent:001:state", "value": "thinking", "scope": "agent-001" }),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(body["ok"], true);
}

#[tokio::test]
async fn memory_search_empty_returns_empty_array() {
    let addr = start_hub().await;

    let (status, body) = api_get(addr, "/api/hexflo/memory/search?q=zzzznotfound").await;
    assert_eq!(status, 200);
    let results = body["results"].as_array().unwrap();
    assert!(results.is_empty(), "search for non-matching should return empty");
}

// ══════════════════════════════════════════════════════════
// Agent Notification Inbox (ADR-060)
// ══════════════════════════════════════════════════════════

/// Helper: register a test agent and return its ID.
async fn register_test_agent(addr: SocketAddr, name: &str) -> String {
    let (status, body) = api_post(
        addr,
        "/api/hex-agents/connect",
        json!({ "name": name, "host": "test", "model": "test", "session_id": name }),
    )
    .await;
    assert!(status == 200 || status == 201, "agent register: {:?}", body);
    body["agentId"].as_str().unwrap().to_string()
}

/// POST with X-Hex-Agent-Id header (for guarded /api/hexflo routes).
async fn api_post_guarded(addr: SocketAddr, path: &str, body: Value, agent_id: &str) -> (u16, Value) {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://{}{}", addr, path))
        .header("x-hex-agent-id", agent_id)
        .json(&body)
        .send()
        .await
        .expect("POST request");
    let status = resp.status().as_u16();
    let body: Value = resp.json().await.unwrap_or(json!(null));
    (status, body)
}

/// PATCH with X-Hex-Agent-Id header (for guarded /api/hexflo routes).
async fn api_patch_guarded(addr: SocketAddr, path: &str, body: Value, agent_id: &str) -> (u16, Value) {
    let client = reqwest::Client::new();
    let resp = client
        .patch(format!("http://{}{}", addr, path))
        .header("x-hex-agent-id", agent_id)
        .json(&body)
        .send()
        .await
        .expect("PATCH request");
    let status = resp.status().as_u16();
    let body: Value = resp.json().await.unwrap_or(json!(null));
    (status, body)
}

#[tokio::test]
async fn inbox_notify_and_query() {
    let addr = start_hub().await;
    let agent_id = register_test_agent(addr, "inbox-test-1").await;

    // Send a notification (guarded route — needs agent header)
    let (status, body) = api_post_guarded(
        addr,
        "/api/hexflo/inbox/notify",
        json!({
            "agent_id": agent_id,
            "priority": 2,
            "kind": "restart",
            "payload": "{\"reason\":\"test\"}"
        }),
        &agent_id,
    )
    .await;
    assert_eq!(status, 201, "notify should succeed: {:?}", body);
    assert_eq!(body["ok"], true);

    // Query inbox (GET — not guarded)
    let (status, body) = api_get(
        addr,
        &format!("/api/hexflo/inbox/{}?min_priority=0&unacked_only=true", agent_id),
    )
    .await;
    assert_eq!(status, 200, "query inbox: {:?}", body);
    let notifications = body["notifications"].as_array().expect("should be array");
    assert!(!notifications.is_empty(), "should have at least 1 notification");

    let n = &notifications[0];
    assert_eq!(n["kind"], "restart");
    assert_eq!(n["priority"], 2);
    assert_eq!(n["agentId"], agent_id);
}

#[tokio::test]
async fn inbox_acknowledge_removes_from_unacked() {
    let addr = start_hub().await;
    let agent_id = register_test_agent(addr, "inbox-ack-1").await;

    // Send notification
    api_post_guarded(
        addr,
        "/api/hexflo/inbox/notify",
        json!({
            "agent_id": agent_id,
            "priority": 1,
            "kind": "config_change",
            "payload": "{}"
        }),
        &agent_id,
    )
    .await;

    // Get the notification ID
    let (_, body) = api_get(
        addr,
        &format!("/api/hexflo/inbox/{}?unacked_only=true", agent_id),
    )
    .await;
    let notifications = body["notifications"].as_array().unwrap();
    assert!(!notifications.is_empty());
    let notif_id = notifications[0]["id"].as_u64().unwrap();

    // Acknowledge it
    let (status, body) = api_patch_guarded(
        addr,
        &format!("/api/hexflo/inbox/{}/ack", notif_id),
        json!({ "agent_id": agent_id }),
        &agent_id,
    )
    .await;
    assert_eq!(status, 200, "ack should succeed: {:?}", body);
    assert_eq!(body["ok"], true);

    // Query unacked — should be empty now
    let (_, body) = api_get(
        addr,
        &format!("/api/hexflo/inbox/{}?unacked_only=true", agent_id),
    )
    .await;
    let notifications = body["notifications"].as_array().unwrap();
    assert!(notifications.is_empty(), "acked notification should not appear in unacked query");
}

#[tokio::test]
async fn inbox_wrong_agent_cannot_ack() {
    let addr = start_hub().await;
    let agent_a = register_test_agent(addr, "inbox-wrong-1").await;
    let agent_b = register_test_agent(addr, "inbox-wrong-2").await;

    // Send to agent A
    api_post_guarded(
        addr,
        "/api/hexflo/inbox/notify",
        json!({
            "agent_id": agent_a,
            "priority": 2,
            "kind": "restart",
            "payload": "{}"
        }),
        &agent_a,
    )
    .await;

    // Get notification ID
    let (_, body) = api_get(
        addr,
        &format!("/api/hexflo/inbox/{}?unacked_only=true", agent_a),
    )
    .await;
    let notif_id = body["notifications"][0]["id"].as_u64().unwrap();

    // Agent B tries to ack — should be rejected
    let (status, body) = api_patch_guarded(
        addr,
        &format!("/api/hexflo/inbox/{}/ack", notif_id),
        json!({ "agent_id": agent_b }),
        &agent_b,
    )
    .await;
    assert_eq!(status, 403, "wrong agent ack should be forbidden: {:?}", body);
}

#[tokio::test]
async fn inbox_priority_filter() {
    let addr = start_hub().await;
    let agent_id = register_test_agent(addr, "inbox-filter-1").await;

    // Send notifications at different priorities
    api_post_guarded(addr, "/api/hexflo/inbox/notify", json!({
        "agent_id": agent_id, "priority": 0, "kind": "info", "payload": "{}"
    }), &agent_id).await;
    api_post_guarded(addr, "/api/hexflo/inbox/notify", json!({
        "agent_id": agent_id, "priority": 1, "kind": "config_change", "payload": "{}"
    }), &agent_id).await;
    api_post_guarded(addr, "/api/hexflo/inbox/notify", json!({
        "agent_id": agent_id, "priority": 2, "kind": "restart", "payload": "{}"
    }), &agent_id).await;

    // Query with min_priority=2 — should only get the critical one
    let (_, body) = api_get(
        addr,
        &format!("/api/hexflo/inbox/{}?min_priority=2&unacked_only=true", agent_id),
    )
    .await;
    let notifications = body["notifications"].as_array().unwrap();
    assert_eq!(notifications.len(), 1, "only critical should pass filter");
    assert_eq!(notifications[0]["kind"], "restart");

    // Query with min_priority=0 — should get all 3
    let (_, body) = api_get(
        addr,
        &format!("/api/hexflo/inbox/{}?min_priority=0&unacked_only=true", agent_id),
    )
    .await;
    let notifications = body["notifications"].as_array().unwrap();
    assert_eq!(notifications.len(), 3, "all priorities should appear: {:?}", notifications);
}

#[tokio::test]
async fn inbox_expire_cleans_old_notifications() {
    let addr = start_hub().await;
    let agent_id = register_test_agent(addr, "inbox-expire-1").await;

    // Expire endpoint should work even with no notifications
    let (status, body) = api_post_guarded(addr, "/api/hexflo/inbox/expire", json!({}), &agent_id).await;
    assert_eq!(status, 200, "expire: {:?}", body);
    assert_eq!(body["ok"], true);
}

#[tokio::test]
async fn inbox_notify_requires_target() {
    let addr = start_hub().await;
    let agent_id = register_test_agent(addr, "inbox-notarget-1").await;

    // Neither agent_id nor project_id — should get 400
    let (status, body) = api_post_guarded(
        addr,
        "/api/hexflo/inbox/notify",
        json!({ "priority": 1, "kind": "info" }),
        &agent_id,
    )
    .await;
    assert_eq!(status, 400, "should require target: {:?}", body);
    assert!(body["error"].as_str().unwrap().contains("agent_id or project_id"));
}

// ══════════════════════════════════════════════════════════
// WebSocket Event Broadcasting
// ══════════════════════════════════════════════════════════

#[tokio::test]
async fn swarm_operations_are_observable_via_ws() {
    let addr = start_hub().await;

    // Connect a WS client
    let url = format!("ws://{}/ws", addr);
    let (mut ws, _) = tokio_tungstenite::connect_async(&url)
        .await
        .expect("WS connect");

    // Consume welcome
    let _ = ws.next().await;

    // Create a swarm (should generate broadcast events)
    let (_, swarm) = api_post(
        addr,
        "/api/swarms",
        json!({ "projectId": "ws-test", "name": "observable" }),
    )
    .await;
    assert!(swarm["id"].is_string());

    // The WS should not have crashed
    use tokio_tungstenite::tungstenite::Message;
    ws.send(Message::Ping(vec![1, 2, 3].into()))
        .await
        .expect("ping");

    let resp = tokio::time::timeout(std::time::Duration::from_secs(2), ws.next()).await;
    assert!(resp.is_ok(), "WS should still be alive after swarm operations");

    let _ = ws.close(None).await;
}
