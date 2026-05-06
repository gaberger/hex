//! Agent Communications Subscriber
//!
//! WebSocket subscription to SpacetimeDB agent-comms for real-time DMs and channel messages.

use anyhow::{Context, Result};
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio_tungstenite::{connect_async, tungstenite::Message};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMessage {
    pub id: u64,
    pub from_agent: String,
    pub to_agent: Option<String>,
    pub channel: Option<String>,
    pub message: String,
    pub thread_id: Option<String>,
    pub timestamp: String,
    pub read_by: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct AgentCommsConfig {
    pub stdb_host: String,
    pub database: String,
    pub agent_role: String,
}

/// Subscribe to agent-comms and process incoming messages
pub async fn subscribe_and_listen(
    config: AgentCommsConfig,
    shutdown: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Result<()> {
    let ws_url = format!(
        "{}/database/subscribe/{}",
        config.stdb_host.replace("http://", "ws://").replace("https://", "wss://"),
        config.database
    );

    tracing::info!(
        role = %config.agent_role,
        url = %ws_url,
        "Connecting to agent-comms WebSocket"
    );

    let (ws_stream, _) = connect_async(&ws_url)
        .await
        .context("Failed to connect to SpacetimeDB WebSocket")?;

    let (mut write, mut read) = ws_stream.split();

    // Subscribe to messages where to_agent = our role
    let subscribe_query = serde_json::json!({
        "subscribe": {
            "query_strings": [
                format!("SELECT * FROM agent_messages WHERE to_agent = '{}'", config.agent_role)
            ]
        }
    });

    write
        .send(Message::Text(subscribe_query.to_string()))
        .await
        .context("Failed to send subscription")?;

    tracing::info!(
        role = %config.agent_role,
        "Subscribed to agent_messages for role"
    );

    // Listen for messages
    while !shutdown.load(std::sync::atomic::Ordering::SeqCst) {
        tokio::select! {
            msg = read.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        if let Err(e) = handle_message(&text, &config).await {
                            tracing::error!(error = %e, "Failed to handle message");
                        }
                    }
                    Some(Ok(Message::Close(_))) => {
                        tracing::warn!("WebSocket closed by server");
                        break;
                    }
                    Some(Err(e)) => {
                        tracing::error!(error = %e, "WebSocket error");
                        break;
                    }
                    None => break,
                    _ => {}
                }
            }
            _ = tokio::time::sleep(tokio::time::Duration::from_secs(1)) => {}
        }
    }

    tracing::info!("Agent-comms subscriber shutting down");
    Ok(())
}

async fn handle_message(text: &str, config: &AgentCommsConfig) -> Result<()> {
    // Parse SpacetimeDB subscription update
    let update: serde_json::Value = serde_json::from_str(text)
        .context("Failed to parse WebSocket message")?;

    // Check if this is a table update with new rows
    if let Some(table_updates) = update.get("table_updates") {
        for table_update in table_updates.as_array().unwrap_or(&vec![]) {
            if let Some(inserts) = table_update.get("inserts") {
                for insert in inserts.as_array().unwrap_or(&vec![]) {
                    // Parse as AgentMessage
                    let msg: AgentMessage = serde_json::from_value(insert.clone())
                        .context("Failed to parse agent message")?;

                    tracing::info!(
                        from = %msg.from_agent,
                        to = ?msg.to_agent,
                        message = %msg.message,
                        "Received agent message"
                    );

                    // Process and respond
                    if let Err(e) = process_and_respond(&msg, config).await {
                        tracing::error!(error = %e, "Failed to process message");
                    }
                }
            }
        }
    }

    Ok(())
}

async fn process_and_respond(msg: &AgentMessage, config: &AgentCommsConfig) -> Result<()> {
    // Generate response based on agent role and message content
    let response = generate_response(msg, &config.agent_role)?;

    // Send response back via HTTP reducer call
    let client = reqwest::Client::new();
    let reducer_url = format!(
        "{}/database/call/{}/send_dm",
        config.stdb_host, config.database
    );

    let payload = serde_json::json!({
        "from": config.agent_role,
        "to": msg.from_agent,
        "message": response,
        "thread_id": msg.thread_id,
    });

    client
        .post(&reducer_url)
        .json(&payload)
        .send()
        .await
        .context("Failed to send response")?;

    tracing::info!(
        from = %config.agent_role,
        to = %msg.from_agent,
        "Sent response via agent-comms"
    );

    Ok(())
}

fn generate_response(msg: &AgentMessage, role: &str) -> Result<String> {
    // Use blocking runtime since we're in sync context
    let rt = tokio::runtime::Handle::try_current()
        .ok()
        .or_else(|| {
            tokio::runtime::Runtime::new()
                .ok()
                .map(|rt| rt.handle().clone())
        });

    if let Some(handle) = rt {
        handle.block_on(async {
            generate_llm_response(msg, role).await
        })
    } else {
        // Fallback if no runtime
        Ok(format!("Acknowledged. I received: \"{}\" - Runtime unavailable", msg.message))
    }
}

async fn generate_llm_response(msg: &AgentMessage, role: &str) -> Result<String> {
    let client = reqwest::Client::new();

    let system_prompt = format!(
        "You are the {} in an AI organization. Respond professionally and concisely to messages from colleagues. Keep responses under 3 sentences.",
        role.replace('-', " ")
    );

    let user_prompt = format!(
        "Message from {}: {}",
        msg.from_agent,
        msg.message
    );

    let payload = serde_json::json!({
        "model": "nemotron-mini",
        "prompt": format!("System: {}\n\nUser: {}\n\nAssistant:", system_prompt, user_prompt),
        "stream": false,
        "options": {
            "temperature": 0.7,
            "num_predict": 150
        }
    });

    let ollama_host = std::env::var("OLLAMA_HOST")
        .unwrap_or_else(|_| "http://127.0.0.1:11434".to_string());

    let response = client
        .post(format!("{}/api/generate", ollama_host))
        .json(&payload)
        .send()
        .await
        .context("Failed to call Ollama")?;

    if !response.status().is_success() {
        let status = response.status();
        return Ok(format!("Acknowledged - LLM error ({})", status));
    }

    let response_json: serde_json::Value = response
        .json()
        .await
        .context("Failed to parse Ollama response")?;

    let text = response_json["response"]
        .as_str()
        .unwrap_or("Acknowledged.")
        .trim()
        .to_string();

    Ok(text)
}
