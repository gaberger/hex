//! Tests for the chat WebSocket interface.
//!
//! These tests verify:
//! - Chat WS upgrade and welcome message
//! - Inbound message routing (chat_message, connect_agent, spawn_agent)
//! - LLM bridge topic structure
//! - Auth gating on spawn

use serde_json::json;
use std::net::SocketAddr;

async fn start_test_server_with_token(token: Option<String>) -> SocketAddr {
    let config = hex_nexus::HubConfig {
        port: 0,
        bind: "127.0.0.1".to_string(),
        token,
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

#[tokio::test]
async fn chat_ws_upgrade_sends_welcome() {
    let addr = start_test_server_with_token(None).await;

    let url = format!("ws://{}/ws/chat", addr);
    let (mut ws, _resp) = tokio_tungstenite::connect_async(&url)
        .await
        .expect("WS connect");

    use futures::StreamExt;
    let msg = ws
        .next()
        .await
        .expect("should receive welcome")
        .expect("valid message");

    let text = msg.to_text().unwrap();
    let parsed: serde_json::Value = serde_json::from_str(text).unwrap();

    // WsEnvelope format: {topic, event, data}
    assert_eq!(parsed["event"], "connected");
    assert!(parsed["data"]["sessionId"].is_string());
    assert_eq!(parsed["data"]["authenticated"], true); // no token = always authed
}

#[tokio::test]
async fn chat_ws_with_token_shows_unauthenticated() {
    let addr = start_test_server_with_token(Some("my-secret".to_string())).await;

    // Connect WITHOUT providing the token
    let url = format!("ws://{}/ws/chat", addr);
    let (mut ws, _resp) = tokio_tungstenite::connect_async(&url)
        .await
        .expect("WS connect");

    use futures::StreamExt;
    let msg = ws
        .next()
        .await
        .expect("should receive welcome")
        .expect("valid message");

    let parsed: serde_json::Value = serde_json::from_str(msg.to_text().unwrap()).unwrap();
    assert_eq!(parsed["data"]["authenticated"], false);
}

#[tokio::test]
async fn chat_ws_with_correct_token_is_authenticated() {
    let addr = start_test_server_with_token(Some("my-secret".to_string())).await;

    let url = format!("ws://{}/ws/chat?token=my-secret", addr);
    let (mut ws, _resp) = tokio_tungstenite::connect_async(&url)
        .await
        .expect("WS connect");

    use futures::StreamExt;
    let msg = ws
        .next()
        .await
        .expect("should receive welcome")
        .expect("valid message");

    let parsed: serde_json::Value = serde_json::from_str(msg.to_text().unwrap()).unwrap();
    assert_eq!(parsed["data"]["authenticated"], true);
}

#[tokio::test]
async fn chat_ws_agent_id_param_forwarded() {
    let addr = start_test_server_with_token(None).await;

    let url = format!("ws://{}/ws/chat?agent_id=agent-42", addr);
    let (mut ws, _resp) = tokio_tungstenite::connect_async(&url)
        .await
        .expect("WS connect");

    use futures::StreamExt;
    let msg = ws
        .next()
        .await
        .expect("should receive welcome")
        .expect("valid message");

    let parsed: serde_json::Value = serde_json::from_str(msg.to_text().unwrap()).unwrap();
    assert_eq!(parsed["data"]["agentId"], "agent-42");
}

#[tokio::test]
async fn chat_message_broadcast_reaches_general_ws() {
    let addr = start_test_server_with_token(None).await;

    // Connect a general WS subscriber that watches all agent:* topics
    let ws_url = format!("ws://{}/ws", addr);
    let (mut general_ws, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .expect("general WS connect");

    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    // Consume the welcome message
    let _ = general_ws.next().await;

    // Subscribe to agent:broadcast:* on the general WS
    let sub_msg = json!({"type": "subscribe", "topic": "agent:broadcast:*"}).to_string();
    general_ws.send(Message::Text(sub_msg.into())).await.unwrap();

    // Give the server time to register the subscription before the sender fires
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Connect a chat client with an explicit agent_id.
    // This forces the server to route the message to the broadcast channel
    // (agent route) rather than the LLM bridge, regardless of whether
    // ANTHROPIC_API_KEY is set in the test environment.
    let chat_url = format!("ws://{}/ws/chat?agent_id=broadcast", addr);
    let (mut chat_ws, _) = tokio_tungstenite::connect_async(&chat_url)
        .await
        .expect("chat WS connect");

    // Consume welcome
    let _ = chat_ws.next().await;

    // Send a chat message — with agent_id=broadcast on the URL, has_agent=true
    // so the server skips the LLM bridge and publishes to agent:broadcast:input.
    let chat_msg = json!({
        "type": "chat_message",
        "content": "Hello from test",
    })
    .to_string();
    chat_ws.send(Message::Text(chat_msg.into())).await.unwrap();

    // The general WS should receive the broadcast
    let received = tokio::time::timeout(std::time::Duration::from_secs(2), general_ws.next()).await;

    assert!(
        received.is_ok(),
        "general WS should receive the broadcast within 2s"
    );
}

#[tokio::test]
async fn chat_ws_invalid_json_ignored() {
    let addr = start_test_server_with_token(None).await;

    let url = format!("ws://{}/ws/chat", addr);
    let (mut ws, _) = tokio_tungstenite::connect_async(&url)
        .await
        .expect("WS connect");

    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    // Consume welcome
    let _ = ws.next().await;

    // Send garbage JSON — should not crash the connection
    ws.send(Message::Text("not valid json!!!".into()))
        .await
        .unwrap();

    // Send a valid ping to confirm connection is still alive
    ws.send(Message::Ping(vec![1, 2, 3].into()))
        .await
        .unwrap();

    // Should get pong back (connection still open)
    let resp = tokio::time::timeout(std::time::Duration::from_secs(2), ws.next()).await;
    assert!(resp.is_ok(), "connection should still be alive after invalid JSON");
}

#[tokio::test]
async fn chat_ws_llm_bridge_disabled_without_api_key() {
    // Temporarily clear API key so the LLM bridge reports disabled.
    struct EnvGuard(Option<String>);
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.0 {
                Some(v) => unsafe { std::env::set_var("ANTHROPIC_API_KEY", v) },
                None => unsafe { std::env::remove_var("ANTHROPIC_API_KEY") },
            }
        }
    }
    let old = std::env::var("ANTHROPIC_API_KEY").ok();
    unsafe { std::env::remove_var("ANTHROPIC_API_KEY") };
    let _guard = EnvGuard(old);

    let addr = start_test_server_with_token(None).await;

    let url = format!("ws://{}/ws/chat", addr);
    let (mut ws, _) = tokio_tungstenite::connect_async(&url)
        .await
        .expect("WS connect");

    use futures::StreamExt;
    let msg = ws
        .next()
        .await
        .expect("welcome")
        .expect("valid");

    let parsed: serde_json::Value = serde_json::from_str(msg.to_text().unwrap()).unwrap();
    // Without ANTHROPIC_API_KEY set, LLM bridge should be disabled
    assert_eq!(parsed["data"]["llmBridge"], false);
}
