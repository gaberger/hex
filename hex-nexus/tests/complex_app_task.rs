//! End-to-end test: send a complex programming task to the remote agent.
//! The agent should create a Python todo-list CLI application on bazzite.
//!
//! Run: cargo test --test complex_app_task -- --ignored --nocapture

use futures::{SinkExt, StreamExt};
use tokio_tungstenite::{connect_async, tungstenite::Message};

#[tokio::test]
#[ignore]
async fn test_create_todo_app() {
    let url = "ws://127.0.0.1:5555/ws";

    let (mut ws, _) = connect_async(url)
        .await
        .expect("Failed to connect to nexus WebSocket");

    // Read welcome
    if let Some(Ok(msg)) = ws.next().await {
        println!("Connected: {}", &msg.to_text().unwrap_or("?")[..80.min(msg.len())]);
    }

    // Subscribe to all agent output
    for topic in &["agent:*", "agent:all:output", "chat:*"] {
        let sub = serde_json::json!({ "type": "subscribe", "topic": topic });
        ws.send(Message::Text(sub.to_string().into())).await.unwrap();
    }
    tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;

    // Send a complex task: build a Python todo CLI app
    let task = serde_json::json!({
        "type": "publish",
        "topic": "agent:broadcast:input",
        "event": "chat_message",
        "data": {
            "content": r#"Create a Python todo-list CLI application in /tmp/agent-workspace/todo-app/. The app should have:

1. `todo.py` - main CLI using argparse with subcommands: add, list, done, remove
2. `storage.py` - JSON file storage backend (saves to ~/.todo.json)
3. `models.py` - TodoItem dataclass with id, title, done, created_at fields

Requirements:
- `python3 todo.py add "Buy groceries"` adds a new item
- `python3 todo.py list` shows all items with [x] or [ ] status
- `python3 todo.py done 1` marks item 1 as done
- `python3 todo.py remove 1` deletes item 1
- Items have auto-incrementing integer IDs
- Created timestamps in ISO format
- Pretty-print with colors using ANSI codes

Create all 3 files. After creating them, run `python3 /tmp/agent-workspace/todo-app/todo.py add "Test task"` and then `python3 /tmp/agent-workspace/todo-app/todo.py list` to verify it works."#,
            "senderName": "test"
        }
    });

    ws.send(Message::Text(task.to_string().into()))
        .await
        .expect("Failed to send task");

    println!("\nComplex task sent: Create Python todo CLI app");
    println!("Waiting for agent to write code on bazzite via Ollama qwen3.5:27b...\n");
    println!("═══════════════════════════════════════════════════");

    let mut got_response = false;
    let mut all_text = String::new();
    let mut tool_calls = 0u32;
    let timeout = tokio::time::Duration::from_secs(600); // 10 min for complex task
    let start = tokio::time::Instant::now();

    while start.elapsed() < timeout {
        let msg = tokio::time::timeout(
            tokio::time::Duration::from_secs(300),
            ws.next(),
        ).await;

        match msg {
            Ok(Some(Ok(Message::Text(text)))) => {
                let text_str = text.to_string();
                if let Ok(data) = serde_json::from_str::<serde_json::Value>(&text_str) {
                    let event = data.get("event")
                        .or_else(|| data.get("type"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let inner = data.get("data").unwrap_or(&data);

                    match event {
                        "stream_chunk" => {
                            let chunk = inner.get("text").and_then(|v| v.as_str()).unwrap_or("");
                            print!("{}", chunk);
                            all_text.push_str(chunk);
                            got_response = true;
                        }
                        "tool_call" => {
                            let tool = inner.get("tool_name").and_then(|v| v.as_str()).unwrap_or("?");
                            let input = inner.get("tool_input").and_then(|v| v.as_str()).unwrap_or("");
                            println!("\n┌─ TOOL: {} ─────────────────", tool);
                            if !input.is_empty() {
                                println!("│ {}", &input[..input.len().min(200)]);
                            }
                            tool_calls += 1;
                            got_response = true;
                        }
                        "tool_result" => {
                            let content = inner.get("content").and_then(|v| v.as_str()).unwrap_or("");
                            let is_err = inner.get("is_error").and_then(|v| v.as_bool()).unwrap_or(false);
                            let prefix = if is_err { "ERROR" } else { "OK" };
                            println!("└─ [{}]: {}", prefix, &content[..content.len().min(300)]);
                        }
                        "agent_status" => {
                            let status = inner.get("status").and_then(|v| v.as_str()).unwrap_or("");
                            if status == "idle" && got_response {
                                println!("\n═══════════════════════════════════════════════════");
                                println!("Agent completed! {} tool calls, {} chars output", tool_calls, all_text.len());
                                break;
                            }
                        }
                        "connected" | "heartbeat" => {}
                        _ => {}
                    }
                }
            }
            Ok(Some(Ok(Message::Close(_)))) => { println!("\nWS closed"); break; }
            Err(_) => { println!("\nTimeout (5 min)"); break; }
            _ => {}
        }
    }

    println!("\nElapsed: {:.1}s", start.elapsed().as_secs_f64());
    assert!(got_response, "Expected response from agent");
    assert!(tool_calls > 0, "Expected tool calls (file writes, shell exec)");
    ws.close(None).await.ok();
}
