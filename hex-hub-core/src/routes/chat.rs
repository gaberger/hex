use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    response::IntoResponse,
};
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::state::{SharedState, WsEnvelope};

#[derive(Debug, Deserialize)]
pub struct ChatWsParams {
    pub token: Option<String>,
    pub agent_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ChatInbound {
    ChatMessage {
        content: String,
        agent_id: Option<String>,
    },
    ConnectAgent {
        agent_id: String,
    },
    SpawnAgent {
        project_dir: String,
        model: Option<String>,
        agent_name: Option<String>,
    },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ChatOutbound {
    #[serde(rename = "type")]
    msg_type: String,
    data: serde_json::Value,
}

/// GET /ws/chat — WebSocket upgrade handler for chat
pub async fn chat_ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<SharedState>,
    Query(params): Query<ChatWsParams>,
) -> impl IntoResponse {
    let authenticated = match &state.auth_token {
        None => true,
        Some(token) => params.token.as_deref() == Some(token.as_str()),
    };

    let agent_id = params.agent_id.clone();

    ws.on_upgrade(move |socket| handle_chat_ws(socket, state, authenticated, agent_id))
}

async fn handle_chat_ws(
    socket: WebSocket,
    state: SharedState,
    authenticated: bool,
    initial_agent_id: Option<String>,
) {
    let (mut sender, mut receiver) = socket.split();
    let session_id = Uuid::new_v4().to_string();

    // Send welcome
    let has_llm = state.anthropic_api_key.is_some();
    let welcome = serde_json::to_string(&ChatOutbound {
        msg_type: "connected".to_string(),
        data: json!({
            "sessionId": session_id,
            "authenticated": authenticated,
            "agentId": initial_agent_id,
            "llmBridge": has_llm,
        }),
    })
    .unwrap();
    let _ = sender.send(Message::Text(welcome.into())).await;

    // Subscribe to agent output broadcasts
    let mut ws_rx = state.ws_tx.subscribe();
    let session_id_for_send = session_id.clone();
    let agent_id_for_send = initial_agent_id.clone();

    // Task: forward agent output and LLM bridge responses to the chat client
    let mut send_task = tokio::spawn(async move {
        loop {
            match ws_rx.recv().await {
                Ok(envelope) => {
                    // Forward agent-related events to chat
                    let dominated = envelope.topic.starts_with("agent:")
                        || envelope.topic == format!("chat:{}:llm", session_id_for_send)
                        || envelope.event == "token_update"
                        || envelope.event == "tool_call"
                        || envelope.event == "tool_result"
                        || envelope.event == "agent_output"
                        || envelope.event == "chat_response"
                        || envelope.event == "stream_chunk"
                        || envelope.event == "chat_message";

                    if !dominated {
                        continue;
                    }

                    // If we have a specific agent_id, filter for it (but always allow session-specific LLM events)
                    if let Some(ref aid) = agent_id_for_send {
                        let is_session_event = envelope.topic == format!("chat:{}:llm", session_id_for_send);
                        if !is_session_event
                            && envelope.topic != format!("agent:{}", aid)
                            && !envelope.topic.starts_with(&format!("agent:{}:", aid))
                        {
                            continue;
                        }
                    }

                    let msg = serde_json::to_string(&ChatOutbound {
                        msg_type: envelope.event.clone(),
                        data: envelope.data,
                    })
                    .unwrap_or_default();

                    if sender.send(Message::Text(msg.into())).await.is_err() {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            }
        }
    });

    // Conversation history for LLM bridge (session-scoped)
    let conversation: std::sync::Arc<tokio::sync::Mutex<Vec<serde_json::Value>>> =
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new()));

    // Task: handle inbound chat messages from the user
    let state2 = state.clone();
    let mut recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            let text = match msg {
                Message::Text(t) => t.to_string(),
                Message::Ping(_) | Message::Pong(_) => continue,
                Message::Close(_) => break,
                _ => continue,
            };

            let parsed: ChatInbound = match serde_json::from_str(&text) {
                Ok(m) => m,
                Err(_) => continue,
            };

            match parsed {
                ChatInbound::ChatMessage { content, agent_id } => {
                    // If no agent is connected and we have an API key, use the LLM bridge
                    let use_bridge = agent_id.is_none()
                        && initial_agent_id.is_none()
                        && state2.anthropic_api_key.is_some();

                    if use_bridge {
                        let api_key = state2.anthropic_api_key.clone().unwrap();
                        let ws_tx = state2.ws_tx.clone();
                        let sid = session_id.clone();
                        let conv = conversation.clone();

                        tokio::spawn(async move {
                            llm_bridge(api_key, content, sid, ws_tx, conv).await;
                        });
                    } else {
                        // Forward as a broadcast so the agent (or agent bridge) can pick it up
                        let target = agent_id.unwrap_or_else(|| "broadcast".to_string());
                        let _ = state2.ws_tx.send(WsEnvelope {
                            topic: format!("agent:{}:input", target),
                            event: "chat_message".to_string(),
                            data: json!({
                                "sessionId": session_id,
                                "content": content,
                            }),
                        });
                    }
                }
                ChatInbound::ConnectAgent { agent_id } => {
                    let _ = state2.ws_tx.send(WsEnvelope {
                        topic: format!("chat:{}:control", session_id),
                        event: "agent_connected".to_string(),
                        data: json!({ "agentId": agent_id }),
                    });
                }
                ChatInbound::SpawnAgent {
                    project_dir,
                    model,
                    agent_name,
                } => {
                    if state2.auth_token.is_some() && !authenticated {
                        continue;
                    }
                    let config = crate::orchestration::agent_manager::SpawnConfig {
                        project_dir,
                        model,
                        agent_name,
                        hub_url: None,
                        hub_token: None,
                    };
                    let spawn_result = if let Some(ref mgr) = state2.agent_manager {
                        mgr.spawn_agent(config).await
                    } else {
                        Err("AgentManager not initialized".to_string())
                    };
                    match spawn_result {
                        Ok(agent) => {
                            let _ = state2.ws_tx.send(WsEnvelope {
                                topic: format!("chat:{}:control", session_id),
                                event: "agent_spawned".to_string(),
                                data: json!({ "agent": agent }),
                            });
                        }
                        Err(e) => {
                            let _ = state2.ws_tx.send(WsEnvelope {
                                topic: format!("chat:{}:control", session_id),
                                event: "spawn_error".to_string(),
                                data: json!({ "error": e }),
                            });
                        }
                    }
                }
            }
        }
    });

    // Wait for either task to finish, then abort the other
    tokio::select! {
        _ = &mut send_task => recv_task.abort(),
        _ = &mut recv_task => send_task.abort(),
    }
}

// ── LLM Bridge ─────────────────────────────────────────────────────
// Lightweight Anthropic API proxy for direct chat (no hex-agent needed).
// Maintains conversation history per WebSocket session.

async fn llm_bridge(
    api_key: String,
    user_message: String,
    session_id: String,
    ws_tx: tokio::sync::broadcast::Sender<WsEnvelope>,
    conversation: std::sync::Arc<tokio::sync::Mutex<Vec<serde_json::Value>>>,
) {
    let topic = format!("chat:{}:llm", session_id);

    // Signal that we're processing
    let _ = ws_tx.send(WsEnvelope {
        topic: topic.clone(),
        event: "agent_status".to_string(),
        data: json!({ "status": "thinking" }),
    });

    // Add user message to conversation history
    {
        let mut conv = conversation.lock().await;
        conv.push(json!({ "role": "user", "content": user_message }));
    }

    // Build request
    let messages = conversation.lock().await.clone();
    let body = json!({
        "model": "claude-sonnet-4-20250514",
        "max_tokens": 4096,
        "system": "You are a helpful AI assistant integrated into the hex architecture dashboard. Be concise and helpful.",
        "messages": messages,
    });

    let client = reqwest::Client::new();
    let result = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", &api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await;

    match result {
        Ok(resp) => {
            let status = resp.status().as_u16();
            if status >= 400 {
                let err_text = resp.text().await.unwrap_or_else(|_| "unknown error".into());
                tracing::error!(status, error = %err_text, "Anthropic API error");
                let _ = ws_tx.send(WsEnvelope {
                    topic: topic.clone(),
                    event: "chat_message".to_string(),
                    data: json!({
                        "content": format!("**API Error** ({}): {}", status, truncate_str(&err_text, 200)),
                    }),
                });
            } else {
                match resp.json::<serde_json::Value>().await {
                    Ok(data) => {
                        let content = data["content"]
                            .as_array()
                            .and_then(|arr| arr.first())
                            .and_then(|block| block["text"].as_str())
                            .unwrap_or("(empty response)");

                        let model = data["model"].as_str().unwrap_or("unknown");
                        let input_tokens = data["usage"]["input_tokens"].as_u64().unwrap_or(0);
                        let output_tokens = data["usage"]["output_tokens"].as_u64().unwrap_or(0);

                        // Add assistant response to conversation history
                        {
                            let mut conv = conversation.lock().await;
                            conv.push(json!({ "role": "assistant", "content": content }));
                        }

                        // Send the response
                        let _ = ws_tx.send(WsEnvelope {
                            topic: topic.clone(),
                            event: "chat_message".to_string(),
                            data: json!({ "content": content }),
                        });

                        // Send token update
                        let _ = ws_tx.send(WsEnvelope {
                            topic: topic.clone(),
                            event: "token_update".to_string(),
                            data: json!({
                                "input_tokens": input_tokens,
                                "output_tokens": output_tokens,
                                "total_input": input_tokens,
                                "total_output": output_tokens,
                                "model": model,
                            }),
                        });

                        tracing::info!(
                            model,
                            input_tokens,
                            output_tokens,
                            "LLM bridge response delivered"
                        );
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "Failed to parse Anthropic response");
                        let _ = ws_tx.send(WsEnvelope {
                            topic: topic.clone(),
                            event: "chat_message".to_string(),
                            data: json!({ "content": format!("**Parse error**: {}", e) }),
                        });
                    }
                }
            }
        }
        Err(e) => {
            tracing::error!(error = %e, "Failed to reach Anthropic API");
            let _ = ws_tx.send(WsEnvelope {
                topic: topic.clone(),
                event: "chat_message".to_string(),
                data: json!({ "content": format!("**Connection error**: {}", e) }),
            });
        }
    }

    // Signal idle
    let _ = ws_tx.send(WsEnvelope {
        topic,
        event: "agent_status".to_string(),
        data: json!({ "status": "idle" }),
    });
}

fn truncate_str(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        &s[..max]
    }
}
