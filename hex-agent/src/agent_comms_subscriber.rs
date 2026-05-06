//! Agent Communications Subscriber
//!
//! Polls agent-comms REST API for messages directed at this agent.

use anyhow::{Context, Result};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

#[derive(Debug, Clone)]
pub struct AgentCommsConfig {
    pub stdb_host: String,
    pub database: String,
    pub agent_role: String,
}

/// Poll agent-comms for messages and respond
pub async fn subscribe_and_listen(
    config: AgentCommsConfig,
    shutdown: Arc<AtomicBool>,
) -> Result<()> {
    tracing::info!(
        role = %config.agent_role,
        database = %config.database,
        "Starting agent-comms REST poller"
    );

    let client = reqwest::Client::new();
    let mut last_message_id: u64 = 0;

    while !shutdown.load(std::sync::atomic::Ordering::SeqCst) {
        // Poll for new messages
        let query_url = format!(
            "{}/database/sql/{}",
            config.stdb_host, config.database
        );

        let sql = format!(
            "SELECT * FROM agent_messages WHERE to_agent = '{}' AND id > {} ORDER BY id ASC",
            config.agent_role, last_message_id
        );

        match client
            .post(&query_url)
            .body(sql)
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => {
                if let Ok(rows) = response.json::<Vec<serde_json::Value>>().await {
                    for row in rows {
                        if let Some(id) = row.get("id").and_then(|v| v.as_u64()) {
                            last_message_id = last_message_id.max(id);

                            let from = row.get("from_agent")
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown");
                            let message = row.get("message")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            let thread_id = row.get("thread_id")
                                .and_then(|v| v.as_str());

                            tracing::info!(
                                from = %from,
                                message = %message,
                                "Received agent message"
                            );

                            // Process and respond
                            let role = config.agent_role.clone();
                            let from = from.to_string();
                            let message = message.to_string();
                            let thread_id = thread_id.map(|s| s.to_string());
                            let db = config.database.clone();
                            let host = config.stdb_host.clone();

                            tokio::spawn(async move {
                                if let Err(e) = process_and_respond(&role, &from, &message, thread_id.as_deref(), &db, &host).await {
                                    tracing::error!(error = ?e, "Failed to process message");
                                }
                            });
                        }
                    }
                }
            }
            Err(e) => {
                tracing::warn!(error = ?e, "Failed to poll agent-comms");
            }
            _ => {}
        }

        // Poll every 2 seconds
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    }

    tracing::info!("Agent-comms poller shutting down");
    Ok(())
}

async fn process_and_respond(
    role: &str,
    from_agent: &str,
    message: &str,
    thread_id: Option<&str>,
    database: &str,
    stdb_host: &str,
) -> Result<()> {
    // Generate response based on agent role and message content
    let response = generate_llm_response(message, role).await?;

    // Send response back via send_dm reducer
    let client = reqwest::Client::new();
    let reducer_url = format!(
        "{}/database/call/{}/send_dm",
        stdb_host, database
    );

    let payload = serde_json::json!({
        "from": role,
        "to": from_agent,
        "message": response,
        "thread_id": thread_id,
    });

    client
        .post(&reducer_url)
        .json(&payload)
        .send()
        .await
        .context("Failed to send response")?;

    tracing::info!(
        from = %role,
        to = %from_agent,
        "Sent response via agent-comms"
    );

    Ok(())
}

async fn generate_llm_response(msg: &str, role: &str) -> Result<String> {
    let client = reqwest::Client::new();

    let system_prompt = format!(
        "You are the {} in an AI organization. Respond professionally and concisely to messages from colleagues. Keep responses under 3 sentences.",
        role.replace('-', " ")
    );

    let payload = serde_json::json!({
        "model": "nemotron-mini",
        "prompt": format!("System: {}\n\nUser: {}\n\nAssistant:", system_prompt, msg),
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
