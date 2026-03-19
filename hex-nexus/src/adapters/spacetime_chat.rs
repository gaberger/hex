//! SpacetimeDB chat-relay adapter.
//!
//! Talks to the `chat-relay` SpacetimeDB module via HTTP to persist
//! conversations and messages for audit and history.

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// HTTP client for the `chat-relay` SpacetimeDB module.
pub struct SpacetimeChatClient {
    http: reqwest::Client,
    host: String,
    database: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub id: String,
    pub conversation_id: String,
    pub role: String,
    pub sender_name: String,
    pub content: String,
    pub timestamp: String,
}

impl SpacetimeChatClient {
    pub fn new(host: String, database: String) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .pool_max_idle_per_host(4)
            .build()
            .expect("failed to build HTTP client");

        Self {
            http,
            host,
            database,
        }
    }

    /// Create a new conversation in SpacetimeDB.
    pub async fn create_conversation(
        &self,
        id: &str,
        agent_id: &str,
        agent_name: &str,
    ) -> Result<(), String> {
        self.call_reducer(
            "create_conversation",
            serde_json::json!([id, agent_id, agent_name]),
        )
        .await
    }

    /// Send a message to an existing conversation in SpacetimeDB.
    pub async fn send_message(
        &self,
        conversation_id: &str,
        role: &str,
        sender_name: &str,
        content: &str,
    ) -> Result<(), String> {
        self.call_reducer(
            "send_message",
            serde_json::json!([conversation_id, role, sender_name, content]),
        )
        .await
    }

    /// Retrieve all messages for a conversation via SQL query.
    pub async fn get_messages(
        &self,
        conversation_id: &str,
    ) -> Result<Vec<ChatMessage>, String> {
        let query = format!(
            "SELECT * FROM message WHERE conversation_id = '{}'",
            conversation_id.replace('\'', "''")
        );
        let rows = self.sql_query(&query).await?;

        let mut messages = Vec::new();
        for row in rows {
            if let Some(cols) = row.as_array() {
                // Column order: id, conversation_id, role, sender_name, content, timestamp
                if cols.len() >= 6 {
                    messages.push(ChatMessage {
                        id: str_col(cols, 0),
                        conversation_id: str_col(cols, 1),
                        role: str_col(cols, 2),
                        sender_name: str_col(cols, 3),
                        content: str_col(cols, 4),
                        timestamp: str_col(cols, 5),
                    });
                }
            }
        }
        Ok(messages)
    }

    // ── Internals ───────────────────────────────────────────────────────

    async fn call_reducer(
        &self,
        reducer_name: &str,
        args: serde_json::Value,
    ) -> Result<(), String> {
        let url = format!(
            "{}/v1/database/{}/call/{}",
            self.host, self.database, reducer_name
        );

        let response = self
            .http
            .post(&url)
            .json(&args)
            .send()
            .await
            .map_err(|e| format!("SpacetimeDB call_reducer({}) failed: {}", reducer_name, e))?;

        if response.status().is_success() {
            Ok(())
        } else {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            Err(format!(
                "Reducer '{}' returned {}: {}",
                reducer_name, status, body
            ))
        }
    }

    async fn sql_query(
        &self,
        query: &str,
    ) -> Result<Vec<serde_json::Value>, String> {
        let url = format!("{}/v1/database/{}/sql", self.host, self.database);

        let response = self
            .http
            .post(&url)
            .body(query.to_string())
            .header("Content-Type", "text/plain")
            .send()
            .await
            .map_err(|e| format!("SpacetimeDB SQL query failed: {}", e))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(format!("SQL query failed ({}): {}", status, body));
        }

        let body = response.text().await.unwrap_or_default();
        let parsed: serde_json::Value = serde_json::from_str(&body)
            .map_err(|e| format!("Failed to parse SQL response: {}", e))?;

        let rows = parsed
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|table| table.get("rows"))
            .and_then(|r| r.as_array())
            .cloned()
            .unwrap_or_default();

        Ok(rows)
    }
}

fn str_col(cols: &[serde_json::Value], idx: usize) -> String {
    cols.get(idx)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}
