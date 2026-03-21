//! Dispatch a UI component task to the remote agent on bazzite.
//! The agent should create/update the ProjectDetail component with
//! the hierarchical Project → Agents → Worktrees → Commits view.
//!
//! Run: cargo test --test dispatch_ui_task -- --ignored --nocapture

use futures::{SinkExt, StreamExt};
use tokio_tungstenite::{connect_async, tungstenite::Message};

#[tokio::test]
#[ignore]
async fn test_dispatch_project_hierarchy_component() {
    let url = "ws://127.0.0.1:5555/ws";
    let (mut ws, _) = connect_async(url).await.expect("Failed to connect");

    // Welcome
    ws.next().await;

    // Subscribe
    for topic in &["agent:*", "agent:all:output", "chat:*"] {
        let sub = serde_json::json!({ "type": "subscribe", "topic": topic });
        ws.send(Message::Text(sub.to_string().into())).await.unwrap();
    }
    tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;

    // Dispatch the UI task
    let task = serde_json::json!({
        "type": "publish",
        "topic": "agent:broadcast:input",
        "event": "chat_message",
        "data": {
            "content": r#"Create a new SolidJS component at src/components/views/ProjectHierarchy.tsx that displays the data model: Project → Agents → Worktrees → Commits.

Read the existing file src/components/views/ProjectDetail.tsx first to understand the current patterns and imports.

The component should:

1. Accept props: { projectId: string, projectPath: string, agents: any[], worktrees: WorktreeInfo[], commits: CommitInfo[] }

2. Render a tree view:
   - Project name at top
   - For each agent: show name, host, status badge (green/yellow/red), model
     - Under each agent: show worktrees assigned to that agent (match by agent.worktree_path or branch)
       - Under each worktree: show recent commits on that branch (filter commits by branch name)

3. Use Tailwind CSS classes matching the existing dark theme (bg-[#111827], text-gray-300, etc.)

4. Status badges: online=green, busy=yellow, stale=orange, dead=red

5. Commits show: short SHA (monospace, text-blue-400), message (truncated to 60 chars), relative time

6. Export the component as default.

Write ONLY the ProjectHierarchy.tsx file. Use the same imports pattern as ProjectDetail.tsx."#,
            "senderName": "test"
        }
    });

    ws.send(Message::Text(task.to_string().into())).await.expect("Failed to send");
    println!("UI task dispatched to bazzite agent (qwen3.5:9b)");
    println!("═══════════════════════════════════════════════════");

    let mut got_response = false;
    let mut tool_calls = 0u32;
    let timeout = tokio::time::Duration::from_secs(300);
    let start = tokio::time::Instant::now();

    while start.elapsed() < timeout {
        let msg = tokio::time::timeout(tokio::time::Duration::from_secs(120), ws.next()).await;
        match msg {
            Ok(Some(Ok(Message::Text(text)))) => {
                if let Ok(data) = serde_json::from_str::<serde_json::Value>(&text.to_string()) {
                    let event = data.get("event").and_then(|v| v.as_str()).unwrap_or("");
                    let inner = data.get("data").unwrap_or(&data);
                    match event {
                        "stream_chunk" => {
                            print!("{}", inner.get("text").and_then(|v| v.as_str()).unwrap_or(""));
                            got_response = true;
                        }
                        "tool_call" => {
                            let tool = inner.get("tool_name").and_then(|v| v.as_str()).unwrap_or("?");
                            println!("\n┌─ TOOL: {} ─────", tool);
                            tool_calls += 1;
                            got_response = true;
                        }
                        "tool_result" => {
                            let ok = !inner.get("is_error").and_then(|v| v.as_bool()).unwrap_or(false);
                            let content = inner.get("content").and_then(|v| v.as_str()).unwrap_or("");
                            println!("└─ [{}]: {}", if ok {"OK"} else {"ERR"}, &content[..content.len().min(200)]);
                        }
                        "agent_status" => {
                            let s = inner.get("status").and_then(|v| v.as_str()).unwrap_or("");
                            if s == "idle" && got_response {
                                println!("\n═══════════════════════════════════════════════════");
                                println!("Done! {} tool calls, {:.1}s", tool_calls, start.elapsed().as_secs_f64());
                                break;
                            }
                        }
                        _ => {}
                    }
                }
            }
            Err(_) => { println!("\nTimeout"); break; }
            _ => {}
        }
    }

    assert!(got_response, "Expected response from agent");
    ws.close(None).await.ok();
}
