use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    response::IntoResponse,
};
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::state::{SharedState, WsEnvelope};

#[derive(Debug, Deserialize)]
pub struct ChatWsParams {
    pub token: Option<String>,
    pub agent_id: Option<String>,
    /// If provided, resumes an existing session. If absent, a new session is created.
    pub session_id: Option<String>,
    /// Project ID for session scoping. Required for new sessions.
    pub project_id: Option<String>,
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
    /// Agent registration — sent by hex-agent on connect
    AgentRegister {
        agent_id: String,
        agent_name: String,
        project_dir: String,
    },
}

// ChatOutbound removed — we now send WsEnvelope directly on the wire.
// Both browser (checks raw.event && raw.data) and agent (checks envelope["event"])
// know how to unwrap WsEnvelope into flat {type, ...fields} messages.

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
    let session_id = params.session_id.clone();
    let project_id = params.project_id.clone().unwrap_or_default();

    ws.on_upgrade(move |socket| {
        handle_chat_ws(socket, state, authenticated, agent_id, session_id, project_id)
    })
}

async fn handle_chat_ws(
    socket: WebSocket,
    state: SharedState,
    authenticated: bool,
    initial_agent_id: Option<String>,
    requested_session_id: Option<String>,
    project_id: String,
) {
    let (mut sender, mut receiver) = socket.split();

    // Session persistence (ADR-036): resume or create a session
    #[cfg(feature = "sqlite-session")]
    let persistent_session_id = {
        use crate::ports::session::{MessagePart, NewMessage, Role, TokenUsage};
        if let Some(ref port) = state.session_port {
            if let Some(ref sid) = requested_session_id {
                // Verify the session exists
                match port.session_get(sid).await {
                    Ok(Some(_)) => Some(sid.clone()),
                    _ => {
                        tracing::warn!(session_id = %sid, "requested session not found, creating new");
                        match port.session_create(&project_id, "claude-sonnet-4-20250514", None).await {
                            Ok(s) => Some(s.id),
                            Err(e) => { tracing::error!("session create failed: {e}"); None }
                        }
                    }
                }
            } else if !project_id.is_empty() {
                // Auto-create a new session
                match port.session_create(&project_id, "claude-sonnet-4-20250514", None).await {
                    Ok(s) => {
                        tracing::info!(session_id = %s.id, "auto-created chat session");
                        Some(s.id)
                    }
                    Err(e) => { tracing::error!("session create failed: {e}"); None }
                }
            } else {
                None
            }
        } else {
            None
        }
    };
    #[cfg(not(feature = "sqlite-session"))]
    let persistent_session_id: Option<String> = None;

    // Use persistent session ID if available, otherwise generate ephemeral one
    let session_id = persistent_session_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    // Send welcome as WsEnvelope so both browser and agent can unwrap it
    let has_llm = state.anthropic_api_key.is_some();
    let welcome = serde_json::to_string(&WsEnvelope {
        topic: format!("chat:{}:control", session_id),
        event: "connected".to_string(),
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

    // Task: forward agent output and LLM bridge responses to the chat client.
    // We send WsEnvelope directly on the wire so both browser and agent can unwrap:
    //   browser: checks raw.event && raw.data → {type: event, ...data}
    //   agent:   checks envelope["event"] → reconstructs HubMessage
    let mut send_task = tokio::spawn(async move {
        loop {
            match ws_rx.recv().await {
                Ok(envelope) => {
                    // Forward agent-related events and session-specific LLM events
                    let dominated = envelope.topic.starts_with("agent:")
                        || envelope.topic == format!("chat:{}:llm", session_id_for_send)
                        || envelope.topic == format!("chat:{}:control", session_id_for_send)
                        || envelope.event == "token_update"
                        || envelope.event == "tool_call"
                        || envelope.event == "tool_result"
                        || envelope.event == "agent_output"
                        || envelope.event == "agent_status"
                        || envelope.event == "agent_register"
                        || envelope.event == "agent_connected"
                        || envelope.event == "agent_disconnected"
                        || envelope.event == "chat_response"
                        || envelope.event == "stream_chunk"
                        || envelope.event == "chat_message"
                        || envelope.event == "heartbeat";

                    if !dominated {
                        continue;
                    }

                    // If we have a specific agent_id, filter for it
                    // (but always allow session-specific LLM/control events)
                    if let Some(ref aid) = agent_id_for_send {
                        let is_session_event =
                            envelope.topic == format!("chat:{}:llm", session_id_for_send)
                            || envelope.topic == format!("chat:{}:control", session_id_for_send);
                        if !is_session_event
                            && envelope.topic != format!("agent:{}", aid)
                            && !envelope.topic.starts_with(&format!("agent:{}:", aid))
                        {
                            continue;
                        }
                    }

                    // Send WsEnvelope directly — no ChatOutbound wrapping
                    let msg = serde_json::to_string(&envelope).unwrap_or_default();

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

    // Task: handle inbound chat messages from the user (or agent)
    let state2 = state.clone();
    let mut recv_task = tokio::spawn(async move {
        // Track connected agent for disconnect notification
        let mut registered_agent_id: Option<String> = None;
        let mut registered_agent_name: Option<String> = None;

        while let Some(Ok(msg)) = receiver.next().await {
            let text = match msg {
                Message::Text(t) => t.to_string(),
                Message::Ping(_) | Message::Pong(_) => continue,
                Message::Close(_) => break,
                _ => continue,
            };

            let parsed: ChatInbound = match serde_json::from_str(&text) {
                Ok(m) => m,
                Err(_) => {
                    // Not a ChatInbound — might be agent output (stream_chunk, tool_call, heartbeat, etc.)
                    // Forward it to the broadcast channel so browser clients can see it
                    if let Ok(raw) = serde_json::from_str::<serde_json::Value>(&text) {
                        if let Some(msg_type) = raw.get("type").and_then(|t| t.as_str()) {
                            // Handle heartbeat: broadcast as agent_status with last_seen
                            if msg_type == "heartbeat" {
                                let hb_name = raw.get("agent_name").and_then(|v| v.as_str()).unwrap_or("unknown");
                                // Update registered name from heartbeat if we don't have one yet
                                if registered_agent_name.is_none() && hb_name != "unknown" {
                                    registered_agent_name = Some(hb_name.to_string());
                                }
                                let agent_name = registered_agent_name.as_deref().unwrap_or(hb_name);
                                let status = raw.get("status").and_then(|v| v.as_str()).unwrap_or("idle");
                                let uptime = raw.get("uptime_secs").and_then(|v| v.as_u64()).unwrap_or(0);
                                let _ = state2.ws_tx.send(WsEnvelope {
                                    topic: format!("agent:{}:output", session_id),
                                    event: "agent_status".to_string(),
                                    data: json!({
                                        "status": status,
                                        "agent_name": agent_name,
                                        "uptime_secs": uptime,
                                        "last_seen": chrono::Utc::now().to_rfc3339(),
                                    }),
                                });
                            } else {
                                let _ = state2.ws_tx.send(WsEnvelope {
                                    topic: format!("agent:{}:output", session_id),
                                    event: msg_type.to_string(),
                                    data: raw,
                                });
                            }
                        }
                    }
                    continue;
                }
            };

            match parsed {
                ChatInbound::ChatMessage { content, agent_id } => {
                    // If no agent is connected and we have an API key, use the LLM bridge
                    let has_agent = agent_id.is_some()
                        || initial_agent_id.is_some()
                        || registered_agent_id.is_some();
                    let use_bridge = !has_agent && state2.anthropic_api_key.is_some();

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
                        let target = agent_id
                            .or_else(|| registered_agent_id.clone())
                            .unwrap_or_else(|| "broadcast".to_string());
                        let _ = state2.ws_tx.send(WsEnvelope {
                            topic: format!("agent:{}:input", target),
                            event: "chat_message".to_string(),
                            data: json!({
                                "sessionId": session_id,
                                "content": content,
                                "senderName": "user",
                            }),
                        });
                    }
                }
                ChatInbound::ConnectAgent { agent_id } => {
                    registered_agent_id = Some(agent_id.clone());
                    let _ = state2.ws_tx.send(WsEnvelope {
                        topic: format!("chat:{}:control", session_id),
                        event: "agent_connected".to_string(),
                        data: json!({ "agentId": agent_id }),
                    });
                }
                ChatInbound::AgentRegister {
                    agent_id,
                    agent_name,
                    project_dir,
                } => {
                    // Store the agent identity for this session
                    registered_agent_id = Some(agent_id.clone());
                    registered_agent_name = Some(agent_name.clone());
                    tracing::info!(
                        agent_id = %agent_id,
                        agent_name = %agent_name,
                        project_dir = %project_dir,
                        "Agent registered on chat session {}",
                        session_id
                    );
                    // Broadcast agent_connected so browser clients know the agent's name
                    let _ = state2.ws_tx.send(WsEnvelope {
                        topic: format!("agent:{}:output", session_id),
                        event: "agent_connected".to_string(),
                        data: json!({
                            "agentId": agent_id,
                            "agentName": agent_name,
                            "projectDir": project_dir,
                        }),
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
                        secret_keys: vec![],
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

        // When the receive loop exits (WS closed), broadcast agent_disconnected
        // if an agent was registered on this session
        if let Some(agent_id) = registered_agent_id {
            tracing::info!(agent_id = %agent_id, "Agent disconnected from chat session {}", session_id);
            let _ = state2.ws_tx.send(WsEnvelope {
                topic: format!("agent:{}:output", session_id),
                event: "agent_disconnected".to_string(),
                data: json!({ "agentId": agent_id }),
            });
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
