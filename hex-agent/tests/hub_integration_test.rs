//! Integration test: hex-agent HubClientAdapter ↔ mock hub WebSocket server.
//!
//! Validates the full agent→hub protocol: connect, register, send events,
//! receive chat messages, and graceful disconnect.

use futures::{SinkExt, StreamExt};
use hex_agent::ports::hub::{HubClientPort, HubMessage};
use hex_agent::adapters::secondary::hub_client::HubClientAdapter;
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message as WsMessage;

/// Spin up a minimal WebSocket server that echoes agent messages back
/// and sends a ChatMessage on connect.
async fn start_mock_hub() -> (SocketAddr, tokio::task::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let handle = tokio::spawn(async move {
        let mut received: Vec<String> = Vec::new();

        if let Ok((stream, _)) = listener.accept().await {
            let ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            let (mut sink, mut stream) = ws.split();

            // Send a chat message to the agent
            let chat = serde_json::json!({
                "type": "chat_message",
                "content": "Hello from hub"
            });
            sink.send(WsMessage::Text(serde_json::to_string(&chat).unwrap().into()))
                .await
                .ok();

            // Collect messages from agent until it disconnects
            while let Some(Ok(msg)) = stream.next().await {
                match msg {
                    WsMessage::Text(text) => {
                        received.push(text.to_string());
                    }
                    WsMessage::Close(_) => break,
                    _ => {}
                }
            }
        }

        received
    });

    (addr, handle)
}

#[tokio::test]
async fn agent_connects_and_registers_with_hub() {
    let (addr, hub_handle) = start_mock_hub().await;

    let client = HubClientAdapter::new();
    let url = format!("http://127.0.0.1:{}", addr.port());

    // Connect
    client.connect(&url, "test-token").await.unwrap();
    assert!(client.is_connected());

    // Send registration
    client
        .send(HubMessage::Register {
            agent_id: "agent-1".into(),
            agent_name: "test-agent".into(),
            project_dir: "/tmp/test".into(),
        })
        .await
        .unwrap();

    // Receive the chat message from the mock hub
    let msg = client.recv().await.unwrap();
    match msg {
        HubMessage::ChatMessage { content } => {
            assert_eq!(content, "Hello from hub");
        }
        other => panic!("Expected ChatMessage, got: {:?}", other),
    }

    // Send a stream chunk back
    client
        .send(HubMessage::StreamChunk {
            text: "I received your message".into(),
            agent_name: None,
        })
        .await
        .unwrap();

    // Send a token update
    client
        .send(HubMessage::TokenUpdate {
            input_tokens: 100,
            output_tokens: 50,
            total_input: 100,
            total_output: 50,
            agent_name: None,
        })
        .await
        .unwrap();

    // Send done
    client
        .send(HubMessage::Done {
            agent_id: "agent-1".into(),
            summary: "Task completed".into(),
            exit_code: 0,
        })
        .await
        .unwrap();

    // Disconnect
    client.disconnect().await.unwrap();
    assert!(!client.is_connected());

    // Verify what the hub received
    let received = hub_handle.await.unwrap();
    assert!(received.len() >= 3, "Hub should have received at least 3 messages, got {}", received.len());

    // Verify registration message
    let reg: serde_json::Value = serde_json::from_str(&received[0]).unwrap();
    assert_eq!(reg["type"], "agent_register");
    assert_eq!(reg["agent_id"], "agent-1");
    assert_eq!(reg["agent_name"], "test-agent");

    // Verify stream chunk
    let chunk: serde_json::Value = serde_json::from_str(&received[1]).unwrap();
    assert_eq!(chunk["type"], "stream_chunk");
    assert_eq!(chunk["text"], "I received your message");

    // Verify token update
    let tokens: serde_json::Value = serde_json::from_str(&received[2]).unwrap();
    assert_eq!(tokens["type"], "token_update");
    assert_eq!(tokens["input_tokens"], 100);
    assert_eq!(tokens["output_tokens"], 50);
}

#[tokio::test]
async fn agent_detects_hub_disconnect() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    // Hub that immediately closes
    let hub = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let ws = tokio_tungstenite::accept_async(stream).await.unwrap();
        let (mut sink, _) = ws.split();
        // Close immediately
        sink.send(WsMessage::Close(None)).await.ok();
    });

    let client = HubClientAdapter::new();
    client
        .connect(&format!("http://127.0.0.1:{}", addr.port()), "tok")
        .await
        .unwrap();

    // recv should return an error when hub closes
    let result = client.recv().await;
    assert!(result.is_err());
    assert!(!client.is_connected());

    hub.await.ok();
}

#[tokio::test]
async fn send_fails_when_not_connected() {
    let client = HubClientAdapter::new();
    let result = client
        .send(HubMessage::StreamChunk {
            text: "test".into(),
            agent_name: None,
        })
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn hub_message_serde_roundtrip_all_variants() {
    let messages = vec![
        HubMessage::Register {
            agent_id: "a1".into(),
            agent_name: "coder".into(),
            project_dir: "/proj".into(),
        },
        HubMessage::StreamChunk { text: "hello".into(), agent_name: None },
        HubMessage::ToolCall {
            tool_name: "read_file".into(),
            tool_input: serde_json::json!({"path": "/foo.rs"}),
            agent_name: None,
        },
        HubMessage::ToolResultMsg {
            tool_name: "read_file".into(),
            content: "fn main() {}".into(),
            is_error: false,
            agent_name: None,
        },
        HubMessage::TokenUpdate {
            input_tokens: 100,
            output_tokens: 50,
            total_input: 1000,
            total_output: 500,
            agent_name: None,
        },
        HubMessage::AgentStatus {
            status: "thinking".into(),
            detail: "processing tools".into(),
            agent_name: None,
        },
        HubMessage::ChatMessage {
            content: "What is Rust?".into(),
        },
        HubMessage::Done {
            agent_id: "a1".into(),
            summary: "All tasks complete".into(),
            exit_code: 0,
        },
    ];

    for msg in messages {
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: HubMessage = serde_json::from_str(&json).unwrap();
        // Verify roundtrip by re-serializing
        let json2 = serde_json::to_string(&parsed).unwrap();
        assert_eq!(json, json2, "Serde roundtrip failed for: {}", json);
    }
}
