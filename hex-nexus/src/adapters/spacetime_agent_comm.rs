//! SpacetimeDB agent-comms adapter.
//!
//! Implements IAgentCommPort by calling reducers in the `agent-comms` SpacetimeDB module.

use async_trait::async_trait;
use hex_core::ports::agent_comm::*;
use serde_json::Value;
use std::time::Duration;

/// SpacetimeDB sum-type encoding for `Option<String>`. The v1 HTTP API
/// rejects bare strings or JSON null for sum types — it expects
/// `{"some": "..."}` or `{"none": []}`. Bare null happened to work for
/// some legacy reducers but Some(s) does NOT — pass everything through
/// this helper.
fn encode_option_string(o: &Option<String>) -> serde_json::Value {
    match o {
        Some(s) => serde_json::json!({ "some": s }),
        None => serde_json::json!({ "none": [] }),
    }
}

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
        let url = format!("{}/v1/database/{}/call/{}", self.host, self.database, reducer);

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
        // STDB SQL endpoint: POST /v1/database/<db>/sql, raw SQL in body,
        // response is `[{ "rows": [...] }, ...]`. Matches spacetime_chat / spacetime_state.
        let url = format!("{}/v1/database/{}/sql", self.host, self.database);

        let res = self
            .http
            .post(&url)
            .header("Content-Type", "text/plain")
            .body(query.to_string())
            .send()
            .await
            .map_err(|e| AgentCommError::Transport(e.to_string()))?;

        if !res.status().is_success() {
            let status = res.status();
            let body = res
                .text()
                .await
                .unwrap_or_else(|_| "<no body>".to_string());
            return Err(AgentCommError::Transport(format!(
                "SQL query failed ({}): {}",
                status, body
            )));
        }

        let body: Value = res
            .json()
            .await
            .map_err(|e| AgentCommError::Transport(e.to_string()))?;

        Ok(body
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|t| t.get("rows"))
            .and_then(|r| r.as_array())
            .cloned()
            .unwrap_or_default())
    }

    /// Best-effort lookup of the most recent agent_messages id for a sender.
    /// STDB SQL rejects `ORDER BY id` here, so we scan a window and pick the
    /// max ourselves. Returns None on any failure — callers fall back to 0.
    async fn try_lookup_latest_id(&self, from: &str) -> Option<u64> {
        let safe = from.replace('\'', "''");
        let q = format!(
            "SELECT id FROM agent_messages WHERE from_agent = '{}' LIMIT 200",
            safe
        );
        let rows = self.sql_query(&q).await.ok()?;
        rows.into_iter()
            .filter_map(|r| {
                r.as_array()
                    .and_then(|cols| cols.first())
                    .and_then(|id| id.as_u64())
            })
            .max()
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
            serde_json::json!([from, to, message, encode_option_string(&thread_id)]),
        )
        .await?;

        // STDB SQL doesn't support `ORDER BY id` on this column, so the
        // post-insert ID lookup can't reliably return the row we just wrote.
        // The reducer call succeeded — return 0 as a placeholder. Callers use
        // the returned id only for logging.
        Ok(self.try_lookup_latest_id(&from).await.unwrap_or(0))
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
            serde_json::json!([from, channel, message, encode_option_string(&thread_id)]),
        )
        .await?;

        Ok(self.try_lookup_latest_id(&from).await.unwrap_or(0))
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
        // STDB SQL constraints we have to work around:
        //   1. `to_agent = 'foo'` errors because to_agent is Option<String>
        //      (Sum type) — can't push the inbound predicate down.
        //   2. `ORDER BY id DESC LIMIT N` is unsupported on this table —
        //      so we can't ask STDB for the newest N rows directly.
        //   3. Plain `LIMIT N` returns the OLDEST N rows by insertion order,
        //      which means new traffic is INVISIBLE if total > N. This was
        //      the silent bug that made org_responder skip every recent DM.
        //
        // Workaround: scan all rows up to HEX_AGENT_COMM_SCAN_CAP (default
        // 5000), filter both directions in Rust, sort newest-first by id,
        // dedup, truncate to caller's limit. For an agent_messages table at
        // 117 rows this costs ~1 ms.
        //
        // If/when agent_messages grows past ~5K rows the right move is a
        // side-table indexing inbound (to_agent, id) pairs so we can push
        // the filter back down to STDB.
        let scan_cap: u32 = std::env::var("HEX_AGENT_COMM_SCAN_CAP")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(5000);
        let cols = "id, from_agent, to_agent, channel, message, thread_id, timestamp, read_by";
        let scan_q = format!("SELECT {cols} FROM agent_messages LIMIT {scan_cap}");

        let rows = self.sql_query(&scan_q).await?;
        let all = self.parse_messages(rows)?;

        let mut filtered: Vec<AgentMessage> = all
            .into_iter()
            .filter(|m| {
                m.from_agent == agent
                    || m.to_agent.as_deref() == Some(agent.as_str())
            })
            .collect();

        // Newest-first by id, dedup, truncate.
        filtered.sort_by(|a, b| b.id.cmp(&a.id));
        filtered.dedup_by_key(|m| m.id);
        if let Some(l) = limit {
            filtered.truncate(l as usize);
        }

        Ok(filtered)
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
    let v = cols.get(idx)?;
    // Plain string (older row formats / non-Option columns).
    if let Some(s) = v.as_str() {
        return Some(s.to_string());
    }
    // STDB BSATN-JSON encodes Option<T> as a 2-element array:
    //   Some(x) → [0, x]   None → [1, []]
    if let Some(arr) = v.as_array() {
        let tag = arr.first().and_then(|t| t.as_u64())?;
        if tag == 0 {
            return arr.get(1).and_then(|x| x.as_str()).map(|s| s.to_string());
        }
        return None;
    }
    None
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
