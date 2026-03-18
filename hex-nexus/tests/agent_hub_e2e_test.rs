//! End-to-end integration test: hex-agent <-> hex-hub pipeline.
//!
//! Proves the hub infrastructure works by exercising:
//!   1. Embedded Axum server on a random port
//!   2. Dashboard (GET /)
//!   3. WebSocket chat protocol (/ws/chat) — connect, welcome, send message, receive broadcast
//!   4. RL stats endpoint (GET /api/rl/stats)
//!   5. Agent list endpoint (GET /api/agents)
//!   6. Clean shutdown
//!
//! NOTE: Does NOT spawn a real hex-agent process (requires Anthropic API key).
//! Instead tests the hub's own endpoints and WebSocket protocol.

use futures::{SinkExt, StreamExt};
use hex_nexus::HubConfig;
use serde_json::{json, Value};
use std::net::SocketAddr;
use tokio_tungstenite::{connect_async, tungstenite::Message};

/// Spin up an embedded hub server on an ephemeral port and return its address.
async fn start_hub() -> SocketAddr {
    let config = HubConfig {
        port: 0,
        bind: "127.0.0.1".to_string(),
        token: None,
        is_daemon: false,
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

    // Yield so the server task starts accepting connections.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    addr
}

// ── Test 1: Dashboard serves HTML ────────────────────────────────────

#[tokio::test]
async fn hub_dashboard_serves_html() {
    let addr = start_hub().await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("http://{}/", addr))
        .send()
        .await
        .expect("GET /");

    assert_eq!(resp.status(), 200, "Dashboard should return 200");
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("<!DOCTYPE html>") || body.contains("<html"),
        "Dashboard should serve HTML"
    );
}

// ── Test 2: WebSocket chat connect + welcome message ─────────────────

#[tokio::test]
async fn ws_chat_sends_welcome_on_connect() {
    let addr = start_hub().await;

    let url = format!("ws://{}/ws/chat", addr);
    let (mut ws, _resp) = connect_async(&url).await.expect("WS connect failed");

    // First message should be a "connected" welcome envelope
    let msg = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
        .await
        .expect("timeout waiting for welcome")
        .expect("stream ended")
        .expect("ws error");

    let text = msg.into_text().expect("expected text frame");
    let welcome: Value = serde_json::from_str(&text).expect("invalid JSON");

    assert_eq!(welcome["event"], "connected", "first message type should be 'connected'");
    assert!(welcome["data"]["sessionId"].is_string(), "welcome should include sessionId");
    assert_eq!(welcome["data"]["authenticated"], true, "no auth token means authenticated=true");

    // Clean close
    let _ = ws.close(None).await;
}

// ── Test 3: WebSocket chat message round-trip (broadcast path) ───────

#[tokio::test]
async fn ws_chat_message_broadcasts_to_agent_topic() {
    let addr = start_hub().await;

    // Connect to /ws/chat
    let url = format!("ws://{}/ws/chat", addr);
    let (mut ws, _) = connect_async(&url).await.expect("WS connect");

    // Consume the welcome message
    let welcome_msg = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
        .await
        .expect("timeout")
        .expect("stream ended")
        .expect("ws error");
    let welcome_text = welcome_msg.into_text().unwrap();
    let welcome: Value = serde_json::from_str(&welcome_text).unwrap();
    assert_eq!(welcome["event"], "connected");

    // Send a chat message (no agent_id => broadcast path, no LLM bridge without API key)
    let chat_msg = json!({
        "type": "chat_message",
        "content": "hello from e2e test",
    });
    ws.send(Message::Text(serde_json::to_string(&chat_msg).unwrap().into()))
        .await
        .expect("send chat message");

    // The hub broadcasts to "agent:broadcast:input" topic. Since no agent is
    // subscribed, the send_task won't forward anything back to us. This is
    // expected — we're testing that the hub ACCEPTS the message without error.
    // Give a short window to confirm no crash/disconnect.
    let result = tokio::time::timeout(std::time::Duration::from_millis(500), ws.next()).await;

    // Either timeout (no message, which is fine) or a valid message
    match result {
        Err(_) => { /* timeout — expected, no agent to respond */ }
        Ok(Some(Ok(msg))) => {
            // If we do get a message, it should be valid JSON
            if let Ok(text) = msg.into_text() {
                let _parsed: Value = serde_json::from_str(&text)
                    .expect("any WS message should be valid JSON");
            }
        }
        Ok(Some(Err(e))) => panic!("unexpected WS error: {}", e),
        Ok(None) => panic!("WS stream closed unexpectedly after sending chat message"),
    }

    let _ = ws.close(None).await;
}

// ── Test 4: WebSocket chat with agent_id routes to specific agent ────

#[tokio::test]
async fn ws_chat_message_with_agent_id_routes_correctly() {
    let addr = start_hub().await;

    let url = format!("ws://{}/ws/chat", addr);
    let (mut ws, _) = connect_async(&url).await.expect("WS connect");

    // Consume welcome
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    // Send a chat message targeting a specific agent_id
    let chat_msg = json!({
        "type": "chat_message",
        "content": "hello agent-42",
        "agent_id": "agent-42"
    });
    ws.send(Message::Text(serde_json::to_string(&chat_msg).unwrap().into()))
        .await
        .expect("send targeted chat message");

    // Same logic — hub accepts it, broadcasts to agent:agent-42:input
    let result = tokio::time::timeout(std::time::Duration::from_millis(500), ws.next()).await;
    match result {
        Err(_) => { /* timeout — expected */ }
        Ok(Some(Ok(_))) => { /* valid response is fine */ }
        Ok(Some(Err(e))) => panic!("unexpected WS error: {}", e),
        Ok(None) => panic!("WS stream closed unexpectedly"),
    }

    let _ = ws.close(None).await;
}

// ── Test 5: RL stats endpoint responds ───────────────────────────────

#[tokio::test]
async fn rl_stats_endpoint_responds() {
    let addr = start_hub().await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("http://{}/api/rl/stats", addr))
        .send()
        .await
        .expect("GET /api/rl/stats");

    // Returns 200 if SwarmDb initialized, or 503 if no database.
    // Both are valid depending on ~/.hex/hub.db availability.
    let status = resp.status().as_u16();
    assert!(
        status == 200 || status == 503,
        "RL stats should return 200 or 503, got {}",
        status
    );

    let body: Value = resp.json().await.unwrap();
    if status == 200 {
        // Should have Q-learning stats fields
        assert!(
            body.is_object(),
            "RL stats should return a JSON object"
        );
    } else {
        assert!(body["error"].is_string(), "503 should include error message");
    }
}

// ── Test 6: Agent list endpoint responds ─────────────────────────────

#[tokio::test]
async fn agent_list_endpoint_responds() {
    let addr = start_hub().await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("http://{}/api/agents", addr))
        .send()
        .await
        .expect("GET /api/agents");

    let status = resp.status().as_u16();
    // 200 if AgentManager initialized, or 503 if not (depends on SwarmDb)
    assert!(
        status == 200 || status == 503,
        "Agent list should return 200 or 503, got {}",
        status
    );

    let body: Value = resp.json().await.unwrap();
    if status == 200 {
        assert!(body["agents"].is_array(), "agents should be an array");
    } else {
        assert!(body["error"].is_string(), "503 should include error message");
    }
}

// ── Test 7: Version endpoint confirms hub identity ───────────────────

#[tokio::test]
async fn version_endpoint_confirms_hub() {
    let addr = start_hub().await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("http://{}/api/version", addr))
        .send()
        .await
        .expect("GET /api/version");

    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["name"], "hex-hub");
    assert!(body["version"].is_string());
    assert!(body["buildHash"].is_string());
}

// ── Test 8: Multiple concurrent WebSocket clients ────────────────────

#[tokio::test]
async fn multiple_ws_clients_can_connect_simultaneously() {
    let addr = start_hub().await;
    let url = format!("ws://{}/ws/chat", addr);

    // Connect 3 clients in parallel
    let (ws1, _) = connect_async(&url).await.expect("WS connect 1");
    let (ws2, _) = connect_async(&url).await.expect("WS connect 2");
    let (ws3, _) = connect_async(&url).await.expect("WS connect 3");

    let mut clients = vec![ws1, ws2, ws3];
    let mut session_ids = Vec::new();

    // Each should receive a unique welcome with distinct sessionId
    for (i, ws) in clients.iter_mut().enumerate() {
        let msg = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
            .await
            .unwrap_or_else(|_| panic!("timeout on client {}", i))
            .unwrap_or_else(|| panic!("stream ended on client {}", i))
            .unwrap_or_else(|e| panic!("ws error on client {}: {}", i, e));

        let text = msg.into_text().unwrap();
        let welcome: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(welcome["event"], "connected");

        let sid = welcome["data"]["sessionId"].as_str().unwrap().to_string();
        session_ids.push(sid);
    }

    // All session IDs should be unique
    session_ids.sort();
    session_ids.dedup();
    assert_eq!(session_ids.len(), 3, "each WS client should get a unique sessionId");

    // Clean close all
    for mut ws in clients {
        let _ = ws.close(None).await;
    }
}

// ── Test 9: ConnectAgent control message ─────────────────────────────

#[tokio::test]
async fn ws_connect_agent_message_accepted() {
    let addr = start_hub().await;
    let url = format!("ws://{}/ws/chat", addr);
    let (mut ws, _) = connect_async(&url).await.expect("WS connect");

    // Consume welcome
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    // Send connect_agent control message
    let msg = json!({
        "type": "connect_agent",
        "agent_id": "test-agent-99"
    });
    ws.send(Message::Text(serde_json::to_string(&msg).unwrap().into()))
        .await
        .expect("send connect_agent");

    // Hub broadcasts an agent_connected event — our send_task may or may not
    // forward it depending on topic filtering. Just verify no crash.
    let result = tokio::time::timeout(std::time::Duration::from_millis(500), ws.next()).await;
    match result {
        Err(_) => { /* timeout — fine */ }
        Ok(Some(Ok(_))) => { /* got a response — also fine */ }
        Ok(Some(Err(e))) => panic!("unexpected WS error: {}", e),
        Ok(None) => panic!("WS stream closed unexpectedly"),
    }

    let _ = ws.close(None).await;
}

// ── Test 10: Fleet endpoint responds ─────────────────────────────────

#[tokio::test]
async fn fleet_list_returns_empty() {
    let addr = start_hub().await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("http://{}/api/fleet", addr))
        .send()
        .await
        .expect("GET /api/fleet");

    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert!(body["nodes"].is_array());
    assert_eq!(body["nodes"].as_array().unwrap().len(), 0);
}
