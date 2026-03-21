//! End-to-end test: send a programming task to a remote agent via nexus WebSocket.
//!
//! The agent connects to /ws/chat and registers. This test also connects to
//! /ws/chat. The chat handler broadcasts chat_message events to all connected
//! clients on the session, and the agent's hub_client.recv() unwraps the
//! WsEnvelope to extract the ChatMessage.
//!
//! Prerequisites:
//! - hex-nexus running on localhost:5555
//! - hex-agent running in hub-managed mode (e.g., on bazzite via SSH tunnel)
//!
//! Run: cargo test --test send_task_to_agent -- --ignored --nocapture

use futures::{SinkExt, StreamExt};
use tokio_tungstenite::{connect_async, tungstenite::Message};

#[tokio::test]
#[ignore]
async fn test_send_fizzbuzz_task() {
    // Connect to generic /ws for pub/sub — allows cross-session agent interaction
    let url = "ws://127.0.0.1:5555/ws";

    let (mut ws, _) = connect_async(url)
        .await
        .expect("Failed to connect to nexus WebSocket");

    // Read welcome message and extract session info
    let welcome_text = if let Some(Ok(msg)) = ws.next().await {
        let t = msg.to_text().unwrap_or("{}").to_string();
        println!("Welcome: {}", &t[..t.len().min(200)]);
        t
    } else {
        panic!("No welcome message");
    };

    // Parse welcome to check if agent is connected
    if let Ok(w) = serde_json::from_str::<serde_json::Value>(&welcome_text) {
        let has_llm = w.get("data")
            .and_then(|d| d.get("llmBridge"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        println!("LLM bridge available: {}", has_llm);
    }

    // Subscribe to all agent output events
    let sub = serde_json::json!({ "type": "subscribe", "topic": "agent:*" });
    ws.send(Message::Text(sub.to_string().into())).await.unwrap();

    // Also subscribe to chat events
    let sub2 = serde_json::json!({ "type": "subscribe", "topic": "chat:*" });
    ws.send(Message::Text(sub2.to_string().into())).await.unwrap();

    // Small delay to let subscriptions register
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

    // Publish chat_message to agent:broadcast:input via the /ws pub/sub channel.
    // The agent's chat session forwards chat_message events to the agent's WS.
    let task = serde_json::json!({
        "type": "publish",
        "topic": "agent:broadcast:input",
        "event": "chat_message",
        "data": {
            "content": "Write a Python fizzbuzz(n) function. Return a list: multiples of 3='Fizz', 5='Buzz', both='FizzBuzz', else str(i). Just the function, no tools.",
            "senderName": "test"
        }
    });

    ws.send(Message::Text(task.to_string().into()))
        .await
        .expect("Failed to send task");

    println!("\nTask sent. Waiting for response...\n");
    println!("─────────────────────────────────────────");

    // Collect responses
    let mut got_response = false;
    let mut all_text = String::new();
    let timeout = tokio::time::Duration::from_secs(120);
    let start = tokio::time::Instant::now();

    while start.elapsed() < timeout {
        let msg = tokio::time::timeout(
            tokio::time::Duration::from_secs(60),
            ws.next(),
        )
        .await;

        match msg {
            Ok(Some(Ok(Message::Text(text)))) => {
                let text_str = text.to_string();
                if let Ok(data) = serde_json::from_str::<serde_json::Value>(&text_str) {
                    let event = data.get("event")
                        .or_else(|| data.get("type"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");

                    let inner = data.get("data").unwrap_or(&data);

                    match event {
                        "stream_chunk" => {
                            let chunk = inner.get("text")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            print!("{}", chunk);
                            all_text.push_str(chunk);
                            got_response = true;
                        }
                        "chat_response" => {
                            let content = inner.get("content")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            println!("{}", content);
                            all_text.push_str(content);
                            got_response = true;
                        }
                        "tool_call" => {
                            let tool = inner.get("tool_name")
                                .and_then(|v| v.as_str())
                                .unwrap_or("?");
                            println!("\n[TOOL: {}]", tool);
                            got_response = true;
                        }
                        "tool_result" => {
                            let content = inner.get("content")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            println!("[RESULT: {}]", &content[..content.len().min(200)]);
                        }
                        "agent_status" => {
                            let status = inner.get("status")
                                .and_then(|v| v.as_str())
                                .unwrap_or("?");
                            if got_response || (status != "idle") {
                                println!("\n[STATUS: {}]", status);
                            }
                            if status == "idle" && got_response {
                                println!("─────────────────────────────────────────");
                                println!("\nAgent completed the task!");
                                break;
                            }
                        }
                        "connected" | "heartbeat" | "agent_connected" => {
                            // skip noise
                        }
                        _ => {
                            // Print any unexpected events for debugging
                            println!("[{}]: {}", event, &text_str[..text_str.len().min(150)]);
                        }
                    }
                }
            }
            Ok(Some(Ok(Message::Close(_)))) => {
                println!("\nWebSocket closed");
                break;
            }
            Err(_) => {
                println!("\nTimeout waiting for response (60s)");
                break;
            }
            _ => {}
        }
    }

    if !all_text.is_empty() {
        println!("\n\n=== Full response ({} chars) ===", all_text.len());
    }
    assert!(got_response, "Expected at least one response from the agent");
    ws.close(None).await.ok();
}
