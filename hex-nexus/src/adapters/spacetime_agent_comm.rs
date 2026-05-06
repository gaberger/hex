//! SpacetimeDB agent-comms adapter.
//!
//! Implements IAgentCommPort by calling reducers in the `agent-comms` SpacetimeDB module.

use async_trait::async_trait;
use hex_core::ports::agent_comm::*;
use serde_json::Value;
use std::time::Duration;

/// HTTP client for the `agent-comms` SpacetimeDB module.
pub struct SpacetimeAgentCommAdapter {
    http: reqwest::Client,
    host: String,
    database: String,
}

impl SpacetimeAgentCommAdapter {
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

    async fn call_reducer(&self, reducer: &str, args: Value) -> Result<(), AgentCommError> {
        let url = format!("{}/database/call/{}/{}", self.host, self.database, reducer);

        let res = self
            .http
            .post(&url)
            .json(&args)
            .send()
            .await
            .map_err(|e| AgentCommError::Transport(e.to_string()))?;

        if !res.status().is_success() {
            let body = res
                .text()
                .await
                .unwrap_or_else(|_| "<no body>".to_string());
            return Err(AgentCommError::Transport(format!(
                "Reducer {} failed: {}",
                reducer, body
            )));
        }

        Ok(())
    }

    async fn sql_query(&self, query: &str) -> Result<Vec<Value>, AgentCommError> {
        let url = format!("{}/database/sql/{}", self.host, self.database);

        let res = self
            .http
            .post(&url)
            .json(&serde_json::json!({ "sql_text": query }))
            .send()
            .await
            .map_err(|e| AgentCommError::Transport(e.to_string()))?;

        if !res.status().is_success() {
            let body = res
                .text()
                .await
                .unwrap_or_else(|_| "<no body>".to_string());
            return Err(AgentCommError::Transport(format!(
                "SQL query failed: {}",
                body
            )));
        }

        let body: Value = res
            .json()
            .await
            .map_err(|e| AgentCommError::Transport(e.to_string()))?;

        Ok(body
            .as_array()
            .map(|a| a.to_vec())
            .unwrap_or_default())
    }
}

#[async_trait]
impl IAgentCommPort for SpacetimeAgentCommAdapter {
    async fn send_dm(
        &self,
        from: String,
        to: String,
        message: String,
        thread_id: Option<String>,
    ) -> Result<u64, AgentCommError> {
        self.call_reducer(
            "send_dm",
            serde_json::json!([from, to, message, thread_id]),
        )
        .await?;

        // SpacetimeDB auto-increments the ID, but we can't return it from reducers.
        // Query the latest message ID for this sender.
        let query = format!(
            "SELECT id FROM agent_messages WHERE from_agent = '{}' ORDER BY id DESC LIMIT 1",
            from.replace('\'', "''")
        );
        let rows = self.sql_query(&query).await?;

        if let Some(row) = rows.first() {
            if let Some(cols) = row.as_array() {
                if let Some(id_val) = cols.first() {
                    if let Some(id) = id_val.as_u64() {
                        return Ok(id);
                    }
                }
            }
        }

        Err(AgentCommError::Transport(
            "Failed to retrieve message ID".to_string(),
        ))
    }

    async fn send_to_channel(
        &self,
        from: String,
        channel: String,
        message: String,
        thread_id: Option<String>,
    ) -> Result<u64, AgentCommError> {
        self.call_reducer(
            "send_to_channel",
            serde_json::json!([from, channel, message, thread_id]),
        )
        .await?;

        let query = format!(
            "SELECT id FROM agent_messages WHERE from_agent = '{}' ORDER BY id DESC LIMIT 1",
            from.replace('\'', "''")
        );
        let rows = self.sql_query(&query).await?;

        if let Some(row) = rows.first() {
            if let Some(cols) = row.as_array() {
                if let Some(id_val) = cols.first() {
                    if let Some(id) = id_val.as_u64() {
                        return Ok(id);
                    }
                }
            }
        }

        Err(AgentCommError::Transport(
            "Failed to retrieve message ID".to_string(),
        ))
    }

    async fn mark_read(&self, agent: String, message_id: u64) -> Result<(), AgentCommError> {
        self.call_reducer("mark_read", serde_json::json!([agent, message_id]))
            .await
    }

    async fn create_channel(
        &self,
        name: String,
        members: Vec<String>,
    ) -> Result<(), AgentCommError> {
        self.call_reducer("create_channel", serde_json::json!([name, members]))
            .await
    }

    async fn set_typing(&self, agent: String, channel_or_dm: String) -> Result<(), AgentCommError> {
        self.call_reducer("set_typing", serde_json::json!([agent, channel_or_dm]))
            .await
    }

    async fn clear_typing(&self, agent: String) -> Result<(), AgentCommError> {
        self.call_reducer("clear_typing", serde_json::json!([agent]))
            .await
    }

    async fn query_messages(
        &self,
        agent: String,
        limit: Option<u32>,
    ) -> Result<Vec<AgentMessage>, AgentCommError> {
        let limit_clause = limit.map(|l| format!("LIMIT {}", l)).unwrap_or_default();

        // Query DMs to this agent + channels they're a member of
        let query = format!(
            "SELECT id, from_agent, to_agent, channel, message, thread_id, timestamp, read_by \
             FROM agent_messages \
             WHERE to_agent = '{}' OR channel IN \
             (SELECT name FROM agent_channels WHERE '{}' = ANY(members) OR '*' = ANY(members)) \
             ORDER BY id DESC {}",
            agent.replace('\'', "''"),
            agent.replace('\'', "''"),
            limit_clause
        );

        self.parse_messages(self.sql_query(&query).await?)
    }

    async fn query_channel_messages(
        &self,
        channel: String,
        limit: Option<u32>,
    ) -> Result<Vec<AgentMessage>, AgentCommError> {
        let limit_clause = limit.map(|l| format!("LIMIT {}", l)).unwrap_or_default();

        let query = format!(
            "SELECT id, from_agent, to_agent, channel, message, thread_id, timestamp, read_by \
             FROM agent_messages \
             WHERE channel = '{}' \
             ORDER BY id DESC {}",
            channel.replace('\'', "''"),
            limit_clause
        );

        self.parse_messages(self.sql_query(&query).await?)
    }

    async fn query_thread_messages(
        &self,
        thread_id: String,
        limit: Option<u32>,
    ) -> Result<Vec<AgentMessage>, AgentCommError> {
        let limit_clause = limit.map(|l| format!("LIMIT {}", l)).unwrap_or_default();

        let query = format!(
            "SELECT id, from_agent, to_agent, channel, message, thread_id, timestamp, read_by \
             FROM agent_messages \
             WHERE thread_id = '{}' \
             ORDER BY id {}",
            thread_id.replace('\'', "''"),
            limit_clause
        );

        self.parse_messages(self.sql_query(&query).await?)
    }

    async fn list_channels(&self, agent: String) -> Result<Vec<AgentChannel>, AgentCommError> {
        let query = format!(
            "SELECT name, members, created_at FROM agent_channels \
             WHERE '{}' = ANY(members) OR '*' = ANY(members)",
            agent.replace('\'', "''")
        );

        let rows = self.sql_query(&query).await?;
        let mut channels = Vec::new();

        for row in rows {
            if let Some(cols) = row.as_array() {
                if cols.len() >= 3 {
                    channels.push(AgentChannel {
                        name: str_col(cols, 0),
                        members: vec_col(cols, 1),
                        created_at: str_col(cols, 2),
                    });
                }
            }
        }

        Ok(channels)
    }

    async fn get_typing_indicators(
        &self,
        channel_or_dm: String,
    ) -> Result<Vec<TypingIndicator>, AgentCommError> {
        let query = format!(
            "SELECT agent, channel_or_dm, timestamp FROM agent_typing \
             WHERE channel_or_dm = '{}'",
            channel_or_dm.replace('\'', "''")
        );

        let rows = self.sql_query(&query).await?;
        let mut indicators = Vec::new();

        for row in rows {
            if let Some(cols) = row.as_array() {
                if cols.len() >= 3 {
                    indicators.push(TypingIndicator {
                        agent: str_col(cols, 0),
                        channel_or_dm: str_col(cols, 1),
                        timestamp: str_col(cols, 2),
                    });
                }
            }
        }

        Ok(indicators)
    }
}

impl SpacetimeAgentCommAdapter {
    fn parse_messages(&self, rows: Vec<Value>) -> Result<Vec<AgentMessage>, AgentCommError> {
        let mut messages = Vec::new();

        for row in rows {
            if let Some(cols) = row.as_array() {
                if cols.len() >= 8 {
                    messages.push(AgentMessage {
                        id: Some(u64_col(cols, 0)),
                        from_agent: str_col(cols, 1),
                        to_agent: opt_str_col(cols, 2),
                        channel: opt_str_col(cols, 3),
                        message: str_col(cols, 4),
                        thread_id: opt_str_col(cols, 5),
                        timestamp: str_col(cols, 6),
                        read_by: vec_col(cols, 7),
                    });
                }
            }
        }

        Ok(messages)
    }
}

// ── Column Helpers ──────────────────────────────────────────────────────────

fn str_col(cols: &[Value], idx: usize) -> String {
    cols.get(idx)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

fn opt_str_col(cols: &[Value], idx: usize) -> Option<String> {
    cols.get(idx)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn u64_col(cols: &[Value], idx: usize) -> u64 {
    cols.get(idx).and_then(|v| v.as_u64()).unwrap_or(0)
}

fn vec_col(cols: &[Value], idx: usize) -> Vec<String> {
    cols.get(idx)
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| item.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}
