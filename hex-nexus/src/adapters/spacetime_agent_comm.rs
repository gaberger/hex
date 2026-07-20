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

/// Pure binary-search over an `exists_above(x)` probe, independent of any
/// live STDB connection so it can be unit tested in isolation.
async fn binary_search_max_id<F, Fut>(exists_above: F) -> Option<u64>
where
    F: Fn(u64) -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    if !exists_above(0).await {
        return None;
    }

    let mut lo: u64 = 0;
    let mut hi: u64 = 1;
    while exists_above(hi).await {
        lo = hi;
        hi = match hi.checked_mul(2) {
            Some(v) => v,
            None => break,
        };
    }

    while lo + 1 < hi {
        let mid = lo + (hi - lo) / 2;
        if exists_above(mid).await {
            lo = mid;
        } else {
            hi = mid;
        }
    }

    Some(hi)
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

    /// Binary-search STDB for the current max `id` in agent_messages.
    /// STDB SQL rejects `ORDER BY id` on this table, so we can't just ask
    /// for the max directly — instead we probe `WHERE id > x LIMIT 1` and
    /// binary-search the boundary. Returns None if the table is empty.
    async fn find_max_id(&self) -> Option<u64> {
        binary_search_max_id(|x| async move {
            let q = format!("SELECT id FROM agent_messages WHERE id > {} LIMIT 1", x);
            self.sql_query(&q).await.map(|rows| !rows.is_empty()).unwrap_or(false)
        })
        .await
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
        //      so an unbounded window would make new traffic INVISIBLE once
        //      the table grows past N rows.
        //
        // Fix: floor-bound the scan window via find_max_id() so it always
        // covers the newest scan_cap rows regardless of table size, instead
        // of relying on scan_cap alone to outrun table growth (that already
        // failed once — see the 5000 → 20000 bump below). Filter both
        // directions in Rust, sort newest-first by id, dedup, truncate to
        // caller's limit.
        //
        // Default bumped 5000 → 20000 on 2026-05-29 after the post-recap
        // diagnosis: agent_messages grew past 5000 rows during the ebay-mvp
        // scaling test and the newest messages became INVISIBLE to org_responder
        // (oldest-first LIMIT semantics on STDB). The recap doc listed this as
        // a "if/when agent_messages grows past ~5K rows" follow-up; we hit
        // that today when a test ask at id=9421 never got processed.
        let scan_cap: u32 = std::env::var("HEX_AGENT_COMM_SCAN_CAP")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(20000);
        let cols = "id, from_agent, to_agent, channel, message, thread_id, timestamp, read_by";
        let floor = self.find_max_id().await.unwrap_or(0).saturating_sub(scan_cap as u64);
        let scan_q = format!("SELECT {cols} FROM agent_messages WHERE id > {floor} LIMIT {scan_cap}");

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

#[cfg(test)]
mod binary_search_max_id_tests {
    use super::binary_search_max_id;
    use std::sync::atomic::{AtomicUsize, Ordering};

    async fn max_id_in(dataset_max: Option<u64>, probe_calls: &AtomicUsize) -> Option<u64> {
        binary_search_max_id(|x| {
            probe_calls.fetch_add(1, Ordering::Relaxed);
            let exists = dataset_max.map(|m| m > x).unwrap_or(false);
            async move { exists }
        })
        .await
    }

    #[tokio::test]
    async fn empty_table_returns_none() {
        let calls = AtomicUsize::new(0);
        assert_eq!(max_id_in(None, &calls).await, None);
    }

    #[tokio::test]
    async fn single_row_at_id_one() {
        let calls = AtomicUsize::new(0);
        assert_eq!(max_id_in(Some(1), &calls).await, Some(1));
    }

    #[tokio::test]
    async fn finds_max_id_far_beyond_any_fixed_scan_window() {
        // Regression guard: this mirrors the real production scenario that
        // caused the agent_messages scan-window bug -- a max id (32813) far
        // past any fixed LIMIT/cap must still be found exactly, not missed.
        let calls = AtomicUsize::new(0);
        assert_eq!(max_id_in(Some(32813), &calls).await, Some(32813));
    }

    #[tokio::test]
    async fn converges_in_logarithmic_probes_not_linear_scan() {
        // The whole point of this search over a plain LIMIT scan is to stay
        // cheap regardless of table size -- assert it does not degrade into
        // an O(n) probe-per-row scan for a large max id.
        let calls = AtomicUsize::new(0);
        max_id_in(Some(1_000_000), &calls).await;
        assert!(
            calls.load(Ordering::Relaxed) < 60,
            "expected O(log n) probes, got {}",
            calls.load(Ordering::Relaxed)
        );
    }

    #[tokio::test]
    async fn max_id_zero_is_indistinguishable_from_empty() {
        // id=0 can never be found as a max since exists_above(0) is the
        // empty-table check itself -- document this boundary explicitly
        // rather than leaving it as an unstated assumption (STDB ids here
        // are never actually 0 in practice, but the function should not
        // panic or loop forever on this edge case).
        let calls = AtomicUsize::new(0);
        assert_eq!(max_id_in(Some(0), &calls).await, None);
    }
}
