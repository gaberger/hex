//! SpacetimeDB-backed implementation of IStatePort.
//!
//! Two compilation modes:
//! 1. Default (no feature): Stub that returns connection errors.
//!    Used when SpacetimeDB is not available.
//! 2. `spacetimedb` feature: Real implementation using spacetimedb-sdk.
//!    Connects via WebSocket, calls reducers for writes, reads from
//!    subscription cache for queries.
//!
//! Enabled via `.hex/state.json`:
//! ```json
//! { "backend": "spacetimedb", "spacetimedb": { "host": "localhost:3033", "database": "hexflo-coordination" } }
//! ```

use async_trait::async_trait;
use tokio::sync::broadcast;

use crate::ports::state::*;

/// Configuration for connecting to SpacetimeDB.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SpacetimeConfig {
    pub host: String,
    pub database: String,
    pub auth_token: Option<String>,
}

impl Default for SpacetimeConfig {
    fn default() -> Self {
        Self {
            host: "http://localhost:3033".to_string(),
            database: hex_core::stdb_database_for_module("hexflo-coordination").to_string(),
            auth_token: None,
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Feature-gated implementation (real SpacetimeDB SDK)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(feature = "spacetimedb")]
mod real {
    use super::*;
    use tokio::sync::RwLock;

    /// SpacetimeDB-backed state adapter using the real SDK.
    ///
    /// Architecture:
    /// - Connects to SpacetimeDB via WebSocket (DbConnection::builder())
    /// - Writes: call reducers (e.g., conn.reducers().register_agent(...))
    /// - Reads: query the local subscription cache (e.g., conn.db().agent().iter())
    /// - Events: table callbacks (on_insert/on_delete/on_update) feed the broadcast channel
    pub struct SpacetimeStateAdapter {
        config: SpacetimeConfig,
        event_tx: broadcast::Sender<StateEvent>,
        _connected: RwLock<bool>,
        /// HTTP client for calling hexflo-coordination reducers via SpacetimeDB HTTP API.
        http: reqwest::Client,
    }

    impl SpacetimeStateAdapter {
        pub fn new(config: SpacetimeConfig) -> Self {
            let (event_tx, _) = broadcast::channel(256);
            Self {
                config,
                event_tx,
                _connected: RwLock::new(false),
                http: reqwest::Client::new(),
            }
        }

        /// Call a SpacetimeDB reducer via the HTTP API.
        /// POST {host}/database/call/{database}/{reducer}
        async fn call_reducer(&self, reducer: &str, args: serde_json::Value) -> Result<serde_json::Value, StateError> {
            let url = format!(
                "{}/v1/database/{}/call/{}",
                self.config.host, self.config.database, reducer
            );
            let mut req = self.http.post(&url).json(&args);
            if let Some(ref token) = self.config.auth_token {
                if !token.is_empty() {
                    req = req.header("Authorization", format!("Bearer {}", token));
                }
            }
            let resp = req.send().await.map_err(|e| StateError::Connection(e.to_string()))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(StateError::Storage(format!("Reducer {} failed ({}): {}", reducer, status, body)));
            }
            // SpacetimeDB v1 returns empty body on success for most reducers.
            // Some versions return whitespace or non-JSON text — treat as null.
            // NOTE: Some SpacetimeDB versions return HTTP 200 even when the reducer
            // returns Err(String). Detect error patterns in the body.
            let text = resp.text().await.unwrap_or_default();
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                tracing::debug!("call_reducer '{}': response body: {}", reducer, &trimmed[..trimmed.len().min(500)]);
                // Detect reducer-level error returned with HTTP 200
                if let Ok(body_json) = serde_json::from_str::<serde_json::Value>(trimmed) {
                    if let Some(err) = body_json.get("error").and_then(|e| e.as_str()) {
                        return Err(StateError::Storage(format!("Reducer {} failed (body error): {}", reducer, err)));
                    }
                    if let Some(msg) = body_json.get("message").and_then(|m| m.as_str()) {
                        if msg.contains("failed") || msg.contains("error") || msg.contains("Error") {
                            return Err(StateError::Storage(format!("Reducer {} failed (body message): {}", reducer, msg)));
                        }
                    }
                }
            }
            if trimmed.is_empty() {
                Ok(serde_json::Value::Null)
            } else {
                Ok(serde_json::from_str(trimmed).unwrap_or(serde_json::Value::Null))
            }
        }

        /// Call a reducer on a specific SpacetimeDB database (for cross-module calls).
        async fn call_reducer_on(&self, database: &str, reducer: &str, args: serde_json::Value) -> Result<serde_json::Value, StateError> {
            let url = format!(
                "{}/v1/database/{}/call/{}",
                self.config.host, database, reducer
            );
            let mut req = self.http.post(&url).json(&args);
            if let Some(ref token) = self.config.auth_token {
                if !token.is_empty() {
                    req = req.header("Authorization", format!("Bearer {}", token));
                }
            }
            let resp = req.send().await.map_err(|e| StateError::Connection(e.to_string()))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(StateError::Storage(format!("Reducer {} failed ({}): {}", reducer, status, body)));
            }
            let text = resp.text().await.unwrap_or_default();
            let trimmed = text.trim();
            if trimmed.is_empty() {
                Ok(serde_json::Value::Null)
            } else {
                Ok(serde_json::from_str(trimmed).unwrap_or(serde_json::Value::Null))
            }
        }

        /// Query a specific SpacetimeDB database via SQL (for cross-module queries).
        async fn query_table_on(&self, database: &str, sql: &str) -> Result<Vec<serde_json::Value>, StateError> {
            let url = format!(
                "{}/v1/database/{}/sql",
                self.config.host, database
            );
            let mut req = self.http.post(&url).body(sql.to_string());
            if let Some(ref token) = self.config.auth_token {
                if !token.is_empty() {
                    req = req.header("Authorization", format!("Bearer {}", token));
                }
            }
            let resp = req.send().await.map_err(|e| StateError::Connection(e.to_string()))?;
            if !resp.status().is_success() {
                let body = resp.text().await.unwrap_or_default();
                return Err(StateError::Storage(format!("SQL query failed: {}", body)));
            }
            let body: serde_json::Value = resp.json().await.map_err(|e| StateError::Storage(e.to_string()))?;
            Ok(Self::parse_stdb_response(body))
        }

        /// Query a SpacetimeDB table via the HTTP API.
        /// POST {host}/database/sql/{database} with SQL query
        async fn query_table(&self, sql: &str) -> Result<Vec<serde_json::Value>, StateError> {
            let url = format!(
                "{}/v1/database/{}/sql",
                self.config.host, self.config.database
            );
            let mut req = self.http.post(&url).body(sql.to_string());
            if let Some(ref token) = self.config.auth_token {
                if !token.is_empty() {
                    req = req.header("Authorization", format!("Bearer {}", token));
                }
            }
            let resp = req.send().await.map_err(|e| StateError::Connection(e.to_string()))?;
            if !resp.status().is_success() {
                let body = resp.text().await.unwrap_or_default();
                return Err(StateError::Storage(format!("SQL query failed: {}", body)));
            }
            let text = resp.text().await.map_err(|e| StateError::Storage(e.to_string()))?;
            let trimmed = text.trim();
            if trimmed.is_empty() {
                tracing::debug!("query_table: empty response for SQL: {}", sql);
                return Ok(Vec::new());
            }
            let body: serde_json::Value = serde_json::from_str(trimmed).map_err(|e| {
                tracing::warn!("query_table: failed to parse response — error: {e}, raw: {trimmed}");
                StateError::Storage(e.to_string())
            })?;
            tracing::debug!("query_table: raw response for '{}': {}", sql, body);
            Ok(Self::parse_stdb_response(body))
        }

        /// Parse a SpacetimeDB SQL HTTP response into a Vec of named JSON objects.
        ///
        /// SpacetimeDB returns: `[{"schema": {"elements": [{"name": {"some": "col"}, ...}]}, "rows": [["v1", "v2"], ...]}]`
        /// We convert each row array into a `{"col1": "v1", "col2": "v2"}` object using the schema.
        pub(crate) fn parse_stdb_response(body: serde_json::Value) -> Vec<serde_json::Value> {
            let tables = match body.as_array() {
                Some(arr) => arr,
                None => return Vec::new(),
            };

            let mut results = Vec::new();
            for table in tables {
                // Extract column names from schema.elements[].name.some
                let col_names: Vec<String> = table
                    .get("schema")
                    .and_then(|s| s.get("elements"))
                    .and_then(|e| e.as_array())
                    .map(|elements| {
                        elements.iter().filter_map(|el| {
                            el.get("name")
                                .and_then(|n| n.get("some"))
                                .and_then(|s| s.as_str())
                                .map(String::from)
                        }).collect()
                    })
                    .unwrap_or_default();

                // Convert each row array into a named JSON object
                if let Some(rows) = table.get("rows").and_then(|r| r.as_array()) {
                    for row in rows {
                        if let Some(vals) = row.as_array() {
                            let mut obj = serde_json::Map::new();
                            for (i, val) in vals.iter().enumerate() {
                                if let Some(name) = col_names.get(i) {
                                    obj.insert(name.clone(), val.clone());
                                }
                            }
                            results.push(serde_json::Value::Object(obj));
                        }
                    }
                }
            }
            results
        }

        /// Connect to SpacetimeDB and subscribe to all tables.
        ///
        /// Once generated bindings are available, this will:
        /// 1. DbConnection::builder()
        ///      .with_uri(&self.config.host)
        ///      .with_database_name(&self.config.database)
        ///      .on_connect(|ctx| {
        ///          ctx.subscription_builder()
        ///              .on_applied(|ctx| { /* cache ready */ })
        ///              .subscribe([
        ///                  "SELECT * FROM agent",
        ///                  "SELECT * FROM rl_q_entry",
        ///                  "SELECT * FROM rl_pattern",
        ///                  "SELECT * FROM workplan_task",
        ///                  "SELECT * FROM message",
        ///                  "SELECT * FROM compute_node",
        ///                  "SELECT * FROM skill",
        ///                  "SELECT * FROM hook",
        ///                  "SELECT * FROM agent_definition",
        ///              ]);
        ///      })
        ///      .build()
        /// 2. Register on_insert/on_update/on_delete callbacks to forward StateEvents
        /// 3. Store the connection handle for reducer calls
        pub async fn connect(&self) -> Result<(), StateError> {
            tracing::info!(
                host = %self.config.host,
                db = %self.config.database,
                "Connecting to SpacetimeDB via HTTP API"
            );

            // Verify connectivity by running a lightweight SQL query.
            // We use the HTTP API approach (call_reducer + query_table) rather than
            // the WebSocket SDK, so no DbConnection::builder() is needed.
            match self.query_table("SELECT COUNT(*) AS cnt FROM swarm").await {
                Ok(_) => {
                    let mut connected = self._connected.write().await;
                    *connected = true;
                    tracing::info!(
                        host = %self.config.host,
                        db = %self.config.database,
                        "SpacetimeDB HTTP API connection verified"
                    );
                    Ok(())
                }
                Err(e) => {
                    tracing::warn!(
                        host = %self.config.host,
                        db = %self.config.database,
                        error = %e,
                        "SpacetimeDB HTTP API connection check failed — \
                         HexFlo methods will still attempt HTTP calls on demand"
                    );
                    // Don't fail hard — the HTTP methods will retry on each call.
                    // Return Ok so the adapter is still usable.
                    Ok(())
                }
            }
        }

        fn not_connected() -> StateError {
            StateError::Connection("SpacetimeDB not connected".into())
        }
    }

    /// Discretize an RlState into a compact string key for Q-table lookup.
    /// Format: "{task_type}:sz{n}:ag{n}:tk{n}"
    fn discretize_state(state: &RlState) -> String {
        let sz = match state.codebase_size {
            0 => 0,
            1..=9_999 => 1,
            10_000..=99_999 => 2,
            _ => 3,
        };
        let ag = std::cmp::min(state.agent_count, 3);
        let tk = match state.token_usage {
            0 => 0,
            1..=49_999 => 1,
            50_000..=149_999 => 2,
            _ => 3,
        };
        format!("{}:sz{}:ag{}:tk{}", state.task_type, sz, ag, tk)
    }

    #[async_trait]
    impl IStatePort for SpacetimeStateAdapter {
        // ── RL ───────────────────────────────────────────
        // Maps to: rl-engine module reducers

        async fn rl_select_action(&self, state: &RlState) -> Result<String, StateError> {
            let state_key = discretize_state(state);
            let resp = self.call_reducer_on("rl-engine", "select_action", serde_json::json!([state_key])).await?;
            // Reducer returns the selected action as a string
            let action = resp.as_str()
                .map(String::from)
                .or_else(|| resp.get("action").and_then(|a| a.as_str()).map(String::from))
                .unwrap_or_else(|| "explore".to_string());
            Ok(action)
        }

        async fn rl_record_reward(
            &self,
            state_key: &str,
            action: &str,
            reward: f64,
            next_state_key: &str,
            rate_limited: bool,
            openrouter_cost_usd: f64,
        ) -> Result<(), StateError> {
            self.call_reducer_on("rl-engine", "record_reward", serde_json::json!([
                state_key, action, reward, next_state_key, rate_limited, openrouter_cost_usd
            ])).await?;
            Ok(())
        }

        async fn rl_get_stats(&self) -> Result<RlStats, StateError> {
            let q_rows = self.query_table(
                "SELECT COUNT(*) AS cnt, COALESCE(AVG(q_value), 0.0) AS avg_q FROM rl_q_entry"
            ).await?;
            let exp_rows = self.query_table(
                "SELECT COUNT(*) AS cnt FROM rl_experience"
            ).await?;

            let (q_table_size, avg_q_value) = q_rows.first().map(|r| {
                let cnt = r.get("cnt").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                let avg = r.get("avg_q").and_then(|v| v.as_f64()).unwrap_or(0.0);
                (cnt, avg)
            }).unwrap_or((0, 0.0));

            let total_experiences = exp_rows.first()
                .and_then(|r| r.get("cnt"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;

            Ok(RlStats {
                q_table_size,
                avg_q_value,
                epsilon: 0.1, // Default exploration rate; SpacetimeDB module manages this internally
                total_experiences,
            })
        }

        // ── Patterns ────────────────────────────────────
        // Maps to: rl-engine module (rl_pattern table + store_pattern/decay_patterns reducers)

        async fn pattern_store(
            &self,
            category: &str,
            content: &str,
            confidence: f64,
        ) -> Result<String, StateError> {
            let resp = self.call_reducer_on("rl-engine", "store_pattern", serde_json::json!([
                category, content, confidence
            ])).await?;
            // Return the pattern ID from the response, or generate one
            let id = resp.as_str()
                .map(String::from)
                .or_else(|| resp.get("id").and_then(|v| v.as_str()).map(String::from))
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            Ok(id)
        }

        async fn pattern_search(
            &self,
            category: &str,
            query: &str,
            limit: u32,
        ) -> Result<Vec<PatternEntry>, StateError> {
            // SpacetimeDB does not support LIKE — fetch by category and filter client-side.
            let safe_category = category.replace('\'', "''");
            let sql = format!(
                "SELECT * FROM rl_pattern WHERE category = '{}'",
                safe_category
            );
            let rows = self.query_table(&sql).await?;
            let q_lower = query.to_lowercase();
            let mut results: Vec<PatternEntry> = rows.into_iter().filter_map(|r| {
                let content = r.get("content")?.as_str()?.to_string();
                if !content.to_lowercase().contains(&q_lower) {
                    return None;
                }
                Some(PatternEntry {
                    id: r.get("id")?.as_str()?.to_string(),
                    category: r.get("category")?.as_str()?.to_string(),
                    content,
                    confidence: r.get("confidence")?.as_f64()?,
                    access_count: r.get("access_count")?.as_u64()? as u32,
                })
            }).collect();
            // Sort by confidence descending, then truncate to limit
            results.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap_or(std::cmp::Ordering::Equal));
            results.truncate(limit as usize);
            Ok(results)
        }

        async fn pattern_reinforce(&self, id: &str, delta: f64) -> Result<(), StateError> {
            // SQL UPDATE since there is no dedicated reinforce reducer yet.
            // Increment confidence by delta and bump access_count.
            let safe_id = id.replace('\'', "''");
            let sql = format!(
                "UPDATE rl_pattern SET confidence = confidence + {}, \
                 access_count = access_count + 1 WHERE id = '{}'",
                delta, safe_id
            );
            self.query_table(&sql).await?;
            Ok(())
        }

        async fn pattern_decay_all(&self) -> Result<u32, StateError> {
            self.call_reducer_on("rl-engine", "decay_patterns", serde_json::json!([])).await?;
            // The reducer doesn't return a count; query how many patterns remain
            let rows = self.query_table("SELECT COUNT(*) AS cnt FROM rl_pattern").await?;
            let count = rows.first()
                .and_then(|r| r.get("cnt"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32;
            Ok(count)
        }

        // ── Agent Registry ──────────────────────────────
        // Maps to: agent-registry module

        async fn agent_register(&self, info: AgentInfo) -> Result<String, StateError> {
            const AGENT_DB: &str = "agent-registry";
            self.call_reducer_on(AGENT_DB, "register_agent", serde_json::json!({
                "id": info.id,
                "name": info.name,
                "project_id": info.project_id,
                "project_dir": info.project_dir,
                "model": info.model,
                "started_at": info.started_at
            })).await?;
            Ok(info.id)
        }

        async fn agent_update_status(
            &self,
            id: &str,
            status: AgentStatus,
            metrics: Option<AgentMetricsData>,
        ) -> Result<(), StateError> {
            const AGENT_DB: &str = "agent-registry";
            let status_str = match status {
                AgentStatus::Spawning => "spawning",
                AgentStatus::Running => "running",
                AgentStatus::Completed => "completed",
                AgentStatus::Failed => "failed",
                AgentStatus::Terminated => "terminated",
            };
            let metrics_json = metrics
                .map(|m| serde_json::to_string(&m).unwrap_or_else(|_| "{}".into()))
                .unwrap_or_else(|| "{}".into());
            self.call_reducer_on(AGENT_DB, "update_status", serde_json::json!({
                "id": id,
                "status": status_str,
                "metrics_json": metrics_json
            })).await?;
            Ok(())
        }

        async fn agent_list(&self) -> Result<Vec<AgentInfo>, StateError> {
            const AGENT_DB: &str = "agent-registry";
            let rows = self.query_table_on(AGENT_DB, "SELECT * FROM agent").await?;
            Ok(rows.iter().filter_map(|row| {
                Some(AgentInfo {
                    id: row.get("id")?.as_str()?.to_string(),
                    name: row.get("name")?.as_str()?.to_string(),
                    project_id: row.get("project_id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    project_dir: row.get("project_dir").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    model: row.get("model").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    status: match row.get("status").and_then(|v| v.as_str()).unwrap_or("") {
                        "spawning" => AgentStatus::Spawning,
                        "running" => AgentStatus::Running,
                        "completed" => AgentStatus::Completed,
                        "failed" => AgentStatus::Failed,
                        "terminated" => AgentStatus::Terminated,
                        _ => AgentStatus::Running,
                    },
                    started_at: row.get("started_at").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                })
            }).collect())
        }

        async fn agent_get(&self, id: &str) -> Result<Option<AgentInfo>, StateError> {
            const AGENT_DB: &str = "agent-registry";
            let safe_id = id.replace('\'', "''");
            let rows = self.query_table_on(AGENT_DB, &format!("SELECT * FROM agent WHERE id = '{}'", safe_id)).await?;
            Ok(rows.first().and_then(|row| {
                Some(AgentInfo {
                    id: row.get("id")?.as_str()?.to_string(),
                    name: row.get("name")?.as_str()?.to_string(),
                    project_id: row.get("project_id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    project_dir: row.get("project_dir").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    model: row.get("model").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    status: match row.get("status").and_then(|v| v.as_str()).unwrap_or("") {
                        "spawning" => AgentStatus::Spawning,
                        "running" => AgentStatus::Running,
                        "completed" => AgentStatus::Completed,
                        "failed" => AgentStatus::Failed,
                        "terminated" => AgentStatus::Terminated,
                        _ => AgentStatus::Running,
                    },
                    started_at: row.get("started_at").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                })
            }))
        }

        async fn agent_remove(&self, id: &str) -> Result<(), StateError> {
            const AGENT_DB: &str = "agent-registry";
            self.call_reducer_on(AGENT_DB, "remove_agent", serde_json::json!({
                "id": id
            })).await?;
            Ok(())
        }

        // ── Workplan ────────────────────────────────────
        // Maps to: workplan-state module

        async fn workplan_update_task(&self, _update: WorkplanTaskUpdate) -> Result<(), StateError> {
            // conn.reducers().update_task(execution_id, task_id, status, agent_id, result, timestamp)
            Err(Self::not_connected())
        }

        async fn workplan_get_tasks(
            &self,
            _workplan_id: &str,
        ) -> Result<Vec<WorkplanTaskUpdate>, StateError> {
            // conn.db().workplan_task().iter().filter(|t| t.execution_id == workplan_id)
            Err(Self::not_connected())
        }

        // ── Chat ────────────────────────────────────────
        // Maps to: chat-relay module

        async fn chat_send(&self, message: ChatMessage) -> Result<(), StateError> {
            const CHAT_DB: &str = "hex-chat-relay";
            // Ensure conversation exists (ignore error if already exists)
            let _ = self.call_reducer_on(CHAT_DB, "create_conversation", serde_json::json!({
                "id": message.conversation_id,
                "agent_id": message.sender_name,
                "agent_name": message.sender_name
            })).await;
            // Send the message
            self.call_reducer_on(CHAT_DB, "send_message", serde_json::json!({
                "conversation_id": message.conversation_id,
                "role": message.role,
                "sender_name": message.sender_name,
                "content": message.content
            })).await?;
            Ok(())
        }

        async fn chat_history(
            &self,
            conversation_id: &str,
            limit: u32,
        ) -> Result<Vec<ChatMessage>, StateError> {
            const CHAT_DB: &str = "hex-chat-relay";
            let escaped = conversation_id.replace('\'', "''");
            let sql = format!(
                "SELECT * FROM message WHERE conversation_id = '{}' ORDER BY timestamp DESC LIMIT {}",
                escaped, limit
            );
            let rows = self.query_table_on(CHAT_DB, &sql).await?;
            let mut messages: Vec<ChatMessage> = rows.iter().filter_map(|row| {
                Some(ChatMessage {
                    id: row.get("id")?.as_str()?.to_string(),
                    conversation_id: row.get("conversation_id")?.as_str()?.to_string(),
                    role: row.get("role")?.as_str()?.to_string(),
                    sender_name: row.get("sender_name")?.as_str()?.to_string(),
                    content: row.get("content")?.as_str()?.to_string(),
                    timestamp: row.get("timestamp")?.as_str().unwrap_or("").to_string(),
                })
            }).collect();
            messages.reverse(); // chronological order
            Ok(messages)
        }

        // ── Fleet ───────────────────────────────────────
        // Maps to: fleet-state module

        async fn fleet_register(&self, _node: FleetNode) -> Result<(), StateError> {
            // conn.reducers().register_node(id, host, port, max_agents, timestamp)
            Err(Self::not_connected())
        }

        async fn fleet_update_status(&self, _id: &str, _status: &str) -> Result<(), StateError> {
            // conn.reducers().update_health(id, status, timestamp)
            Err(Self::not_connected())
        }

        async fn fleet_list(&self) -> Result<Vec<FleetNode>, StateError> {
            // conn.db().compute_node().iter().map(...)
            Err(Self::not_connected())
        }

        async fn fleet_remove(&self, _id: &str) -> Result<(), StateError> {
            // conn.reducers().remove_node(id)
            Err(Self::not_connected())
        }

        // ── Skill Registry ────────────────────────────────
        // Maps to: skill-registry module

        async fn skill_register(&self, _skill: SkillEntry) -> Result<String, StateError> {
            // conn.reducers().register_skill(id, name, description, triggers_json, body, source, timestamp)
            Err(Self::not_connected())
        }

        async fn skill_update(&self, _id: &str, _description: &str, _triggers_json: &str, _body: &str) -> Result<(), StateError> {
            // conn.reducers().update_skill(id, description, triggers_json, body, timestamp)
            Err(Self::not_connected())
        }

        async fn skill_remove(&self, _id: &str) -> Result<(), StateError> {
            // conn.reducers().remove_skill(id)
            Err(Self::not_connected())
        }

        async fn skill_list(&self) -> Result<Vec<SkillEntry>, StateError> {
            let rows = self.query_table("SELECT * FROM skill_registry").await?;
            Ok(rows.iter().filter_map(|r| {
                Some(SkillEntry {
                    id: r.get("skill_id")?.as_str()?.to_string(),
                    name: r.get("name")?.as_str()?.to_string(),
                    description: r.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    triggers_json: r.get("trigger_cmd").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    body: String::new(),
                    source: r.get("source_path").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    created_at: r.get("synced_at").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    updated_at: r.get("synced_at").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                })
            }).collect())
        }

        async fn skill_get(&self, id: &str) -> Result<Option<SkillEntry>, StateError> {
            let rows = self.query_table(&format!(
                "SELECT * FROM skill_registry WHERE skill_id = '{}'", id
            )).await?;
            Ok(rows.first().and_then(|r| {
                Some(SkillEntry {
                    id: r.get("skill_id")?.as_str()?.to_string(),
                    name: r.get("name")?.as_str()?.to_string(),
                    description: r.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    triggers_json: r.get("trigger_cmd").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    body: String::new(),
                    source: r.get("source_path").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    created_at: r.get("synced_at").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    updated_at: r.get("synced_at").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                })
            }))
        }

        async fn skill_search(&self, _trigger_type: &str, query: &str) -> Result<Vec<SkillEntry>, StateError> {
            let all = self.skill_list().await?;
            let q = query.to_lowercase();
            Ok(all.into_iter().filter(|s| {
                s.name.to_lowercase().contains(&q)
                    || s.description.to_lowercase().contains(&q)
                    || s.triggers_json.to_lowercase().contains(&q)
            }).collect())
        }

        // ── Hook Registry ──────────────────────────────────
        // Maps to: hook-registry module

        async fn hook_register(&self, _hook: HookEntry) -> Result<String, StateError> {
            // conn.reducers().register_hook(...)
            Err(Self::not_connected())
        }

        async fn hook_update(&self, _id: &str, _handler_config_json: &str, _timeout_secs: u32, _blocking: bool, _tool_pattern: &str) -> Result<(), StateError> {
            // conn.reducers().update_hook(...)
            Err(Self::not_connected())
        }

        async fn hook_remove(&self, _id: &str) -> Result<(), StateError> {
            Err(Self::not_connected())
        }

        async fn hook_toggle(&self, _id: &str, _enabled: bool) -> Result<(), StateError> {
            // conn.reducers().toggle_hook(id, enabled, timestamp)
            Err(Self::not_connected())
        }

        async fn hook_list(&self) -> Result<Vec<HookEntry>, StateError> {
            Err(Self::not_connected())
        }

        async fn hook_list_by_event(&self, _event_type: &str) -> Result<Vec<HookEntry>, StateError> {
            // conn.db().hook().iter().filter(|h| h.event_type == event_type && h.enabled)
            Err(Self::not_connected())
        }

        async fn hook_log_execution(&self, _entry: HookExecutionEntry) -> Result<(), StateError> {
            // conn.reducers().log_execution(...)
            Err(Self::not_connected())
        }

        // ── Agent Definition Registry ──────────────────────
        // Maps to: agent-definition-registry module

        async fn agent_def_register(&self, _def: AgentDefinitionEntry) -> Result<String, StateError> {
            // conn.reducers().register_definition(...)
            Err(Self::not_connected())
        }

        async fn agent_def_update(
            &self, _id: &str, _description: &str, _role_prompt: &str,
            _allowed_tools_json: &str, _constraints_json: &str, _model: &str,
            _max_turns: u32, _metadata_json: &str,
        ) -> Result<(), StateError> {
            // conn.reducers().update_definition(...)
            Err(Self::not_connected())
        }

        async fn agent_def_remove(&self, _id: &str) -> Result<(), StateError> {
            // conn.reducers().remove_definition(id)
            Err(Self::not_connected())
        }

        async fn agent_def_list(&self) -> Result<Vec<AgentDefinitionEntry>, StateError> {
            // conn.db().agent_definition().iter().map(...)
            Err(Self::not_connected())
        }

        async fn agent_def_get_by_name(&self, _name: &str) -> Result<Option<AgentDefinitionEntry>, StateError> {
            // conn.db().agent_definition().name().find(name).map(...)
            Err(Self::not_connected())
        }

        async fn agent_def_versions(&self, _definition_id: &str) -> Result<Vec<AgentDefinitionVersionEntry>, StateError> {
            // conn.db().agent_definition_version().iter().filter(|v| v.definition_id == definition_id)
            Err(Self::not_connected())
        }

        // ── HexFlo Coordination (via SpacetimeDB HTTP API) ──
        // Calls hexflo-coordination module reducers directly.

        async fn swarm_init(&self, id: &str, name: &str, topology: &str, project_id: &str, created_by: &str) -> Result<(), StateError> {
            let now = chrono::Utc::now().to_rfc3339();
            self.call_reducer("swarm_init", serde_json::json!([id, name, topology, project_id, created_by, now])).await?;
            Ok(())
        }

        async fn swarm_complete(&self, id: &str) -> Result<(), StateError> {
            let now = chrono::Utc::now().to_rfc3339();
            self.call_reducer("swarm_complete", serde_json::json!([id, now])).await?;
            Ok(())
        }

        async fn swarm_fail(&self, id: &str, reason: &str) -> Result<(), StateError> {
            let now = chrono::Utc::now().to_rfc3339();
            self.call_reducer("swarm_fail", serde_json::json!([id, reason, now])).await?;
            Ok(())
        }

        async fn swarm_list_active(&self) -> Result<Vec<SwarmInfo>, StateError> {
            let rows = self.query_table("SELECT * FROM swarm WHERE status = 'active'").await?;
            Ok(rows.into_iter().filter_map(|r| {
                Some(SwarmInfo {
                    id: r.get("id")?.as_str()?.to_string(),
                    project_id: r.get("project_id")?.as_str()?.to_string(),
                    name: r.get("name")?.as_str()?.to_string(),
                    topology: r.get("topology")?.as_str()?.to_string(),
                    status: r.get("status")?.as_str()?.to_string(),
                    owner_agent_id: r.get("owner_agent_id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    created_by: r.get("created_by").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    created_at: r.get("created_at")?.as_str()?.to_string(),
                    updated_at: r.get("updated_at")?.as_str()?.to_string(),
                })
            }).collect())
        }

        async fn swarm_list_failed(&self) -> Result<Vec<SwarmInfo>, StateError> {
            let rows = self.query_table("SELECT * FROM swarm WHERE status = 'failed'").await?;
            Ok(rows.into_iter().filter_map(|r| {
                Some(SwarmInfo {
                    id: r.get("id")?.as_str()?.to_string(),
                    project_id: r.get("project_id")?.as_str()?.to_string(),
                    name: r.get("name")?.as_str()?.to_string(),
                    topology: r.get("topology")?.as_str()?.to_string(),
                    status: r.get("status")?.as_str()?.to_string(),
                    owner_agent_id: r.get("owner_agent_id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    created_by: r.get("created_by").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    created_at: r.get("created_at")?.as_str()?.to_string(),
                    updated_at: r.get("updated_at")?.as_str()?.to_string(),
                })
            }).collect())
        }

        async fn swarm_list_all(&self, limit: usize) -> Result<Vec<SwarmInfo>, StateError> {
            let rows = self.query_table("SELECT * FROM swarm").await?;
            let mut swarms: Vec<SwarmInfo> = rows.into_iter().filter_map(|r| {
                Some(SwarmInfo {
                    id: r.get("id")?.as_str()?.to_string(),
                    project_id: r.get("project_id")?.as_str()?.to_string(),
                    name: r.get("name")?.as_str()?.to_string(),
                    topology: r.get("topology")?.as_str()?.to_string(),
                    status: r.get("status")?.as_str()?.to_string(),
                    owner_agent_id: r.get("owner_agent_id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    created_by: r.get("created_by").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    created_at: r.get("created_at")?.as_str()?.to_string(),
                    updated_at: r.get("updated_at")?.as_str()?.to_string(),
                })
            }).collect();
            // Most recent first
            swarms.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
            swarms.truncate(limit);
            Ok(swarms)
        }

        async fn swarm_list_by_project(&self, project_id: &str) -> Result<Vec<SwarmInfo>, StateError> {
            let sql = format!("SELECT * FROM swarm WHERE project_id = '{}'", project_id);
            let rows = self.query_table(&sql).await?;
            Ok(rows.into_iter().filter_map(|r| {
                Some(SwarmInfo {
                    id: r.get("id")?.as_str()?.to_string(),
                    project_id: r.get("project_id")?.as_str()?.to_string(),
                    name: r.get("name")?.as_str()?.to_string(),
                    topology: r.get("topology")?.as_str()?.to_string(),
                    status: r.get("status")?.as_str()?.to_string(),
                    owner_agent_id: r.get("owner_agent_id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    created_by: r.get("created_by").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    created_at: r.get("created_at")?.as_str()?.to_string(),
                    updated_at: r.get("updated_at")?.as_str()?.to_string(),
                })
            }).collect())
        }

        async fn swarm_get(&self, id: &str) -> Result<Option<SwarmInfo>, StateError> {
            let sql = format!("SELECT * FROM swarm WHERE id = '{}'", id);
            let rows = self.query_table(&sql).await?;
            Ok(rows.into_iter().find_map(|r| {
                Some(SwarmInfo {
                    id: r.get("id")?.as_str()?.to_string(),
                    project_id: r.get("project_id")?.as_str()?.to_string(),
                    name: r.get("name")?.as_str()?.to_string(),
                    topology: r.get("topology")?.as_str()?.to_string(),
                    status: r.get("status")?.as_str()?.to_string(),
                    owner_agent_id: r.get("owner_agent_id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    created_by: r.get("created_by").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    created_at: r.get("created_at")?.as_str()?.to_string(),
                    updated_at: r.get("updated_at")?.as_str()?.to_string(),
                })
            }))
        }

        async fn swarm_owned_by_agent(&self, agent_id: &str) -> Result<Option<SwarmInfo>, StateError> {
            let sql = format!("SELECT * FROM swarm WHERE owner_agent_id = '{}' AND status = 'active'", agent_id);
            let rows = self.query_table(&sql).await?;
            Ok(rows.into_iter().find_map(|r| {
                Some(SwarmInfo {
                    id: r.get("id")?.as_str()?.to_string(),
                    project_id: r.get("project_id")?.as_str()?.to_string(),
                    name: r.get("name")?.as_str()?.to_string(),
                    topology: r.get("topology")?.as_str()?.to_string(),
                    status: r.get("status")?.as_str()?.to_string(),
                    owner_agent_id: r.get("owner_agent_id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    created_by: r.get("created_by").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    created_at: r.get("created_at")?.as_str()?.to_string(),
                    updated_at: r.get("updated_at")?.as_str()?.to_string(),
                })
            }))
        }

        async fn swarm_transfer(&self, swarm_id: &str, new_owner_agent_id: &str) -> Result<(), StateError> {
            let now = chrono::Utc::now().to_rfc3339();
            self.call_reducer("swarm_transfer", serde_json::json!([swarm_id, new_owner_agent_id, now])).await?;
            Ok(())
        }

        async fn swarm_task_create(&self, id: &str, swarm_id: &str, title: &str, depends_on: &str) -> Result<(), StateError> {
            let now = chrono::Utc::now().to_rfc3339();
            self.call_reducer("task_create", serde_json::json!([id, swarm_id, title, depends_on, now])).await?;
            Ok(())
        }

        async fn swarm_task_assign(&self, task_id: &str, agent_id: &str, expected_version: Option<u64>) -> Result<(), StateError> {
            let now = chrono::Utc::now().to_rfc3339();
            // Use u64::MAX as sentinel for "skip version check"
            let ver = expected_version.unwrap_or(u64::MAX);
            let result = self.call_reducer("task_assign", serde_json::json!([task_id, agent_id, ver, now])).await;
            match result {
                Ok(_) => Ok(()),
                Err(StateError::Storage(ref msg)) if msg.contains("version_mismatch") => {
                    Err(StateError::Conflict(msg.clone()))
                }
                Err(StateError::Storage(ref msg)) if msg.contains("already_claimed") => {
                    Err(StateError::Conflict(msg.clone()))
                }
                Err(e) => Err(e),
            }
        }

        async fn swarm_task_complete(&self, task_id: &str, result: &str) -> Result<(), StateError> {
            let now = chrono::Utc::now().to_rfc3339();
            self.call_reducer("task_complete", serde_json::json!([task_id, result, now])).await?;
            Ok(())
        }

        async fn swarm_task_fail(&self, task_id: &str, reason: &str) -> Result<(), StateError> {
            let now = chrono::Utc::now().to_rfc3339();
            self.call_reducer("task_fail", serde_json::json!([task_id, reason, now])).await?;
            Ok(())
        }

        async fn swarm_task_list(&self, swarm_id: Option<&str>) -> Result<Vec<SwarmTaskInfo>, StateError> {
            let sql = match swarm_id {
                Some(sid) => format!("SELECT * FROM swarm_task WHERE swarm_id = '{}'", sid),
                None => "SELECT * FROM swarm_task".to_string(),
            };
            let rows = self.query_table(&sql).await?;
            Ok(rows.into_iter().filter_map(|r| {
                Some(SwarmTaskInfo {
                    id: r.get("id")?.as_str()?.to_string(),
                    swarm_id: r.get("swarm_id")?.as_str()?.to_string(),
                    title: r.get("title")?.as_str()?.to_string(),
                    status: r.get("status")?.as_str()?.to_string(),
                    agent_id: r.get("agent_id")?.as_str()?.to_string(),
                    result: r.get("result")?.as_str()?.to_string(),
                    depends_on: r.get("depends_on").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    version: r.get("version").and_then(|v| v.as_u64()).unwrap_or(0),
                    claimed_by: r.get("claimed_by").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    created_at: r.get("created_at")?.as_str()?.to_string(),
                    completed_at: r.get("completed_at")?.as_str()?.to_string(),
                })
            }).collect())
        }

        async fn inference_task_create(&self, id: &str, workplan_id: &str, task_id: &str, phase: &str, prompt: &str, role: &str, created_at: &str) -> Result<(), StateError> {
            self.call_reducer("inference_task_create",
                serde_json::json!([id, workplan_id, task_id, phase, prompt, role, created_at])
            ).await?;
            Ok(())
        }

        async fn inference_task_claim(&self, id: &str, agent_id: &str, updated_at: &str) -> Result<(), StateError> {
            let result = self.call_reducer("inference_task_claim",
                serde_json::json!([id, agent_id, updated_at])
            ).await;
            match result {
                Ok(_) => Ok(()),
                Err(StateError::Storage(ref msg)) if msg.contains("already_claimed") => {
                    Err(StateError::Conflict(msg.clone()))
                }
                Err(e) => Err(e),
            }
        }

        async fn inference_task_complete(&self, id: &str, result: &str, updated_at: &str) -> Result<(), StateError> {
            self.call_reducer("inference_task_complete",
                serde_json::json!([id, result, updated_at])
            ).await?;
            Ok(())
        }

        async fn inference_task_fail(&self, id: &str, error: &str, updated_at: &str) -> Result<(), StateError> {
            self.call_reducer("inference_task_fail",
                serde_json::json!([id, error, updated_at])
            ).await?;
            Ok(())
        }

        async fn inference_task_get(&self, id: &str) -> Result<Option<InferenceTaskInfo>, StateError> {
            let sql = format!("SELECT * FROM inference_task WHERE id = '{}'", id);
            let rows = self.query_table(&sql).await?;
            Ok(rows.into_iter().next().and_then(|r| {
                Some(InferenceTaskInfo {
                    id: r.get("id")?.as_str()?.to_string(),
                    workplan_id: r.get("workplan_id")?.as_str()?.to_string(),
                    task_id: r.get("task_id")?.as_str()?.to_string(),
                    phase: r.get("phase")?.as_str()?.to_string(),
                    prompt: r.get("prompt")?.as_str()?.to_string(),
                    role: r.get("role")?.as_str()?.to_string(),
                    status: r.get("status")?.as_str()?.to_string(),
                    agent_id: r.get("agent_id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    result: r.get("result").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    error: r.get("error").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    created_at: r.get("created_at")?.as_str()?.to_string(),
                    updated_at: r.get("updated_at").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                })
            }))
        }

        async fn inference_task_list_pending(&self) -> Result<Vec<InferenceTaskInfo>, StateError> {
            let rows = self.query_table("SELECT * FROM inference_task WHERE status = 'Pending'").await?;
            Ok(rows.into_iter().filter_map(|r| {
                Some(InferenceTaskInfo {
                    id: r.get("id")?.as_str()?.to_string(),
                    workplan_id: r.get("workplan_id")?.as_str()?.to_string(),
                    task_id: r.get("task_id")?.as_str()?.to_string(),
                    phase: r.get("phase")?.as_str()?.to_string(),
                    prompt: r.get("prompt")?.as_str()?.to_string(),
                    role: r.get("role")?.as_str()?.to_string(),
                    status: r.get("status")?.as_str()?.to_string(),
                    agent_id: r.get("agent_id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    result: r.get("result").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    error: r.get("error").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    created_at: r.get("created_at")?.as_str()?.to_string(),
                    updated_at: r.get("updated_at").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                })
            }).collect())
        }

        async fn swarm_agent_register(&self, id: &str, swarm_id: &str, name: &str, role: &str, worktree_path: &str) -> Result<(), StateError> {
            let now = chrono::Utc::now().to_rfc3339();
            self.call_reducer("agent_register", serde_json::json!([id, swarm_id, name, role, worktree_path, now])).await?;
            Ok(())
        }

        async fn swarm_agent_heartbeat(&self, id: &str) -> Result<(), StateError> {
            let now = chrono::Utc::now().to_rfc3339();
            self.call_reducer("agent_heartbeat", serde_json::json!([id, now])).await?;
            Ok(())
        }

        async fn swarm_agent_remove(&self, id: &str) -> Result<(), StateError> {
            self.call_reducer("agent_remove", serde_json::json!([id])).await?;
            Ok(())
        }

        async fn swarm_cleanup_stale(&self, _stale_secs: u64, _dead_secs: u64) -> Result<CleanupReport, StateError> {
            let stale_cutoff = (chrono::Utc::now() - chrono::Duration::seconds(_stale_secs as i64)).to_rfc3339();
            let dead_cutoff = (chrono::Utc::now() - chrono::Duration::seconds(_dead_secs as i64)).to_rfc3339();
            self.call_reducer("agent_mark_stale", serde_json::json!([stale_cutoff])).await?;
            self.call_reducer("agent_mark_dead", serde_json::json!([dead_cutoff])).await?;
            // SpacetimeDB doesn't return affected row counts from reducers,
            // so we report zeros and let the caller query if needed.
            Ok(CleanupReport { stale_count: 0, dead_count: 0, reclaimed_tasks: 0 })
        }

        async fn hexflo_memory_store(&self, key: &str, value: &str, scope: &str) -> Result<(), StateError> {
            let now = chrono::Utc::now().to_rfc3339();
            self.call_reducer("memory_store", serde_json::json!([key, value, scope, now])).await?;
            Ok(())
        }

        async fn hexflo_memory_retrieve(&self, key: &str) -> Result<Option<String>, StateError> {
            let rows = self.query_table(&format!("SELECT value FROM hexflo_memory WHERE key = '{}'", key)).await?;
            Ok(rows.first().and_then(|r| r.get("value")?.as_str().map(String::from)))
        }

        async fn hexflo_memory_search(&self, query: &str) -> Result<Vec<(String, String)>, StateError> {
            // SpacetimeDB does not support LIKE — fetch all rows and filter client-side.
            let rows = self.query_table("SELECT key, value FROM hexflo_memory").await?;
            let q_lower = query.to_lowercase();
            Ok(rows.into_iter().filter_map(|r| {
                let k = r.get("key")?.as_str()?.to_string();
                let v = r.get("value")?.as_str()?.to_string();
                if k.to_lowercase().contains(&q_lower) || v.to_lowercase().contains(&q_lower) {
                    Some((k, v))
                } else {
                    None
                }
            }).collect())
        }

        async fn hexflo_memory_delete(&self, key: &str) -> Result<(), StateError> {
            self.call_reducer("memory_delete", serde_json::json!([key])).await?;
            Ok(())
        }

        // ── Quality Gate & Fix Tasks (Swarm Gate Enforcement) ──

        async fn quality_gate_create(
            &self,
            id: &str,
            swarm_id: &str,
            tier: u32,
            gate_type: &str,
            target_dir: &str,
            language: &str,
            iteration: u32,
        ) -> Result<(), StateError> {
            let now = chrono::Utc::now().to_rfc3339();
            self.call_reducer("create_quality_gate", serde_json::json!([
                id, swarm_id, tier, gate_type, target_dir, language, iteration, now
            ])).await?;
            Ok(())
        }

        async fn quality_gate_complete(
            &self,
            id: &str,
            status: &str,
            score: u32,
            grade: &str,
            violations_count: u32,
            error_output: &str,
        ) -> Result<(), StateError> {
            let now = chrono::Utc::now().to_rfc3339();
            self.call_reducer("complete_quality_gate", serde_json::json!([
                id, status, score, grade, violations_count, error_output, now
            ])).await?;
            Ok(())
        }

        async fn quality_gate_list(&self, swarm_id: &str) -> Result<Vec<QualityGateInfo>, StateError> {
            let sql = format!("SELECT * FROM quality_gate_task WHERE swarm_id = '{}'", swarm_id);
            let rows = self.query_table(&sql).await?;
            Ok(rows.into_iter().filter_map(|r| {
                Some(QualityGateInfo {
                    id: r.get("id")?.as_str()?.to_string(),
                    swarm_id: r.get("swarm_id")?.as_str()?.to_string(),
                    tier: r.get("tier")?.as_u64()? as u32,
                    gate_type: r.get("gate_type")?.as_str()?.to_string(),
                    target_dir: r.get("target_dir")?.as_str()?.to_string(),
                    language: r.get("language")?.as_str()?.to_string(),
                    status: r.get("status")?.as_str()?.to_string(),
                    score: r.get("score")?.as_u64().unwrap_or(0) as u32,
                    grade: r.get("grade")?.as_str()?.to_string(),
                    violations_count: r.get("violations_count")?.as_u64().unwrap_or(0) as u32,
                    error_output: r.get("error_output")?.as_str()?.to_string(),
                    iteration: r.get("iteration")?.as_u64().unwrap_or(1) as u32,
                    created_at: r.get("created_at")?.as_str()?.to_string(),
                    completed_at: r.get("completed_at")?.as_str().unwrap_or("").to_string(),
                })
            }).collect())
        }

        async fn quality_gate_get(&self, id: &str) -> Result<Option<QualityGateInfo>, StateError> {
            let sql = format!("SELECT * FROM quality_gate_task WHERE id = '{}'", id);
            let rows = self.query_table(&sql).await?;
            Ok(rows.into_iter().next().and_then(|r| {
                Some(QualityGateInfo {
                    id: r.get("id")?.as_str()?.to_string(),
                    swarm_id: r.get("swarm_id")?.as_str()?.to_string(),
                    tier: r.get("tier")?.as_u64()? as u32,
                    gate_type: r.get("gate_type")?.as_str()?.to_string(),
                    target_dir: r.get("target_dir")?.as_str()?.to_string(),
                    language: r.get("language")?.as_str()?.to_string(),
                    status: r.get("status")?.as_str()?.to_string(),
                    score: r.get("score")?.as_u64().unwrap_or(0) as u32,
                    grade: r.get("grade")?.as_str()?.to_string(),
                    violations_count: r.get("violations_count")?.as_u64().unwrap_or(0) as u32,
                    error_output: r.get("error_output")?.as_str()?.to_string(),
                    iteration: r.get("iteration")?.as_u64().unwrap_or(1) as u32,
                    created_at: r.get("created_at")?.as_str()?.to_string(),
                    completed_at: r.get("completed_at")?.as_str().unwrap_or("").to_string(),
                })
            }))
        }

        async fn fix_task_create(
            &self,
            id: &str,
            gate_task_id: &str,
            swarm_id: &str,
            fix_type: &str,
            target_file: &str,
            error_context: &str,
        ) -> Result<(), StateError> {
            let now = chrono::Utc::now().to_rfc3339();
            self.call_reducer("create_fix_task", serde_json::json!([
                id, gate_task_id, swarm_id, fix_type, target_file, error_context, now
            ])).await?;
            Ok(())
        }

        async fn fix_task_complete(
            &self,
            id: &str,
            status: &str,
            result: &str,
            model_used: &str,
            tokens: u64,
            cost_usd: &str,
        ) -> Result<(), StateError> {
            let now = chrono::Utc::now().to_rfc3339();
            self.call_reducer("complete_fix_task", serde_json::json!([
                id, status, result, model_used, tokens, cost_usd, now
            ])).await?;
            Ok(())
        }

        async fn fix_task_list_by_gate(&self, gate_task_id: &str) -> Result<Vec<FixTaskInfo>, StateError> {
            let sql = format!("SELECT * FROM fix_task WHERE gate_task_id = '{}'", gate_task_id);
            let rows = self.query_table(&sql).await?;
            Ok(rows.into_iter().filter_map(|r| {
                Some(FixTaskInfo {
                    id: r.get("id")?.as_str()?.to_string(),
                    gate_task_id: r.get("gate_task_id")?.as_str()?.to_string(),
                    swarm_id: r.get("swarm_id")?.as_str()?.to_string(),
                    fix_type: r.get("fix_type")?.as_str()?.to_string(),
                    target_file: r.get("target_file")?.as_str()?.to_string(),
                    error_context: r.get("error_context")?.as_str()?.to_string(),
                    model_used: r.get("model_used")?.as_str().unwrap_or("").to_string(),
                    tokens: r.get("tokens")?.as_u64().unwrap_or(0),
                    cost_usd: r.get("cost_usd")?.as_str().unwrap_or("").to_string(),
                    status: r.get("status")?.as_str()?.to_string(),
                    result: r.get("result")?.as_str().unwrap_or("").to_string(),
                    created_at: r.get("created_at")?.as_str()?.to_string(),
                    completed_at: r.get("completed_at")?.as_str().unwrap_or("").to_string(),
                })
            }).collect())
        }

        // ── Project Registry (ADR-042) ──────────────────
        async fn project_register(&self, project: ProjectRegistration) -> Result<(), StateError> {
            let registered_at = chrono::Utc::now().to_rfc3339();
            self.call_reducer("register_project", serde_json::json!([
                project.id, project.name, project.description, project.root_path, registered_at
            ])).await?;
            Ok(())
        }
        async fn project_unregister(&self, id: &str) -> Result<bool, StateError> {
            self.call_reducer("remove_project", serde_json::json!([id])).await?;
            Ok(true)
        }
        async fn project_get(&self, id: &str) -> Result<Option<ProjectRecord>, StateError> {
            // SpacetimeDB SQL WHERE on string PKs is unreliable — scan full table and filter in Rust
            let all = self.project_list().await?;
            Ok(all.into_iter().find(|p| p.id == id))
        }
        async fn project_list(&self) -> Result<Vec<ProjectRecord>, StateError> {
            let rows = self.query_table("SELECT * FROM project").await?;
            tracing::debug!("project_list raw rows: {:?}", rows);
            let result: Vec<ProjectRecord> = rows.into_iter().filter_map(|r| {
                match serde_json::from_value::<ProjectRecord>(r.clone()) {
                    Ok(p) => Some(p),
                    Err(e) => {
                        tracing::warn!("project_list: failed to deserialize row: {e} — raw: {r}");
                        None
                    }
                }
            }).collect();
            Ok(result)
        }
        async fn project_update_state(&self, id: &str, push_type: &str, data: serde_json::Value, file_path: Option<&str>) -> Result<(), StateError> {
            self.call_reducer("project_update_state", serde_json::json!([
                id, push_type, data, file_path
            ])).await?;
            Ok(())
        }
        async fn project_find(&self, query: &str) -> Result<Option<ProjectRecord>, StateError> {
            // Try by ID first, then name, then basename
            if let Some(p) = self.project_get(query).await? {
                return Ok(Some(p));
            }
            let all = self.project_list().await?;
            Ok(all.into_iter().find(|p| p.name == query || p.root_path.rsplit('/').next().unwrap_or("") == query))
        }
        // ── Instance Coordination (ADR-042) ─────────────
        async fn instance_register(&self, info: InstanceRecord) -> Result<String, StateError> {
            let id = info.instance_id.clone();
            self.call_reducer("instance_register", serde_json::json!([
                info.instance_id, info.project_id, info.pid, info.session_label, info.registered_at, info.last_seen
            ])).await?;
            Ok(id)
        }
        async fn instance_heartbeat(&self, id: &str, update: InstanceHeartbeat) -> Result<(), StateError> {
            let now = chrono::Utc::now().to_rfc3339();
            self.call_reducer("instance_heartbeat", serde_json::json!([
                id, now, update.agent_count, update.active_task_count, update.completed_task_count, update.topology
            ])).await?;
            Ok(())
        }
        async fn instance_list(&self, project_id: Option<&str>) -> Result<Vec<InstanceRecord>, StateError> {
            let query = if let Some(pid) = project_id {
                format!("SELECT * FROM instance WHERE project_id = '{}'", pid)
            } else {
                "SELECT * FROM instance".to_string()
            };
            let rows = self.query_table(&query).await?;
            Ok(rows.into_iter().filter_map(|r| serde_json::from_value(r).ok()).collect())
        }
        async fn instance_remove(&self, id: &str) -> Result<(), StateError> {
            self.call_reducer("instance_remove", serde_json::json!([id])).await?;
            Ok(())
        }
        // ── Worktree Locks (ADR-042) ────────────────────
        async fn worktree_lock_acquire(&self, lock: WorktreeLockRecord) -> Result<bool, StateError> {
            let resp = self.call_reducer("worktree_lock_acquire", serde_json::json!([
                lock.key, lock.instance_id, lock.project_id, lock.feature, lock.layer, lock.acquired_at, lock.heartbeat_at, lock.ttl_secs
            ])).await?;
            Ok(resp.as_bool().unwrap_or(true))
        }
        async fn worktree_lock_release(&self, key: &str) -> Result<bool, StateError> {
            self.call_reducer("worktree_lock_release", serde_json::json!([key])).await?;
            Ok(true)
        }
        async fn worktree_lock_list(&self, project_id: Option<&str>) -> Result<Vec<WorktreeLockRecord>, StateError> {
            let query = if let Some(pid) = project_id {
                format!("SELECT * FROM worktree_lock WHERE project_id = '{}'", pid)
            } else {
                "SELECT * FROM worktree_lock".to_string()
            };
            let rows = self.query_table(&query).await?;
            Ok(rows.into_iter().filter_map(|r| serde_json::from_value(r).ok()).collect())
        }
        async fn worktree_lock_refresh(&self, instance_id: &str, heartbeat_at: &str) -> Result<(), StateError> {
            self.call_reducer("worktree_lock_refresh", serde_json::json!([instance_id, heartbeat_at])).await?;
            Ok(())
        }
        async fn worktree_lock_evict_expired(&self) -> Result<u32, StateError> {
            let resp = self.call_reducer("worktree_lock_evict_expired", serde_json::json!([])).await?;
            Ok(resp.as_u64().unwrap_or(0) as u32)
        }
        // ── Task Claims (ADR-042) ───────────────────────
        async fn task_claim_acquire(&self, claim: TaskClaimRecord) -> Result<bool, StateError> {
            let resp = self.call_reducer("task_claim_acquire", serde_json::json!([
                claim.task_id, claim.instance_id, claim.claimed_at, claim.heartbeat_at
            ])).await?;
            Ok(resp.as_bool().unwrap_or(true))
        }
        async fn task_claim_release(&self, task_id: &str) -> Result<bool, StateError> {
            self.call_reducer("task_claim_release", serde_json::json!([task_id])).await?;
            Ok(true)
        }
        async fn task_claim_list(&self, project_id: Option<&str>) -> Result<Vec<TaskClaimRecord>, StateError> {
            let query = if let Some(_pid) = project_id {
                // Join with instances to filter by project_id
                format!("SELECT tc.* FROM task_claim tc JOIN instance i ON tc.instance_id = i.instance_id WHERE i.project_id = '{}'", _pid)
            } else {
                "SELECT * FROM task_claim".to_string()
            };
            let rows = self.query_table(&query).await?;
            Ok(rows.into_iter().filter_map(|r| serde_json::from_value(r).ok()).collect())
        }
        async fn task_claim_refresh(&self, instance_id: &str, heartbeat_at: &str) -> Result<(), StateError> {
            self.call_reducer("task_claim_refresh", serde_json::json!([instance_id, heartbeat_at])).await?;
            Ok(())
        }
        // ── Unstaged Files (ADR-042) ────────────────────
        async fn unstaged_update(&self, instance_id: &str, state: UnstagedRecord) -> Result<(), StateError> {
            self.call_reducer("unstaged_update", serde_json::json!([
                instance_id, state.project_id, state.files, state.captured_at
            ])).await?;
            Ok(())
        }
        async fn unstaged_list(&self, project_id: Option<&str>) -> Result<Vec<UnstagedRecord>, StateError> {
            let query = if let Some(pid) = project_id {
                format!("SELECT * FROM unstaged WHERE project_id = '{}'", pid)
            } else {
                "SELECT * FROM unstaged".to_string()
            };
            let rows = self.query_table(&query).await?;
            Ok(rows.into_iter().filter_map(|r| serde_json::from_value(r).ok()).collect())
        }
        async fn unstaged_remove(&self, instance_id: &str) -> Result<(), StateError> {
            self.call_reducer("unstaged_remove", serde_json::json!([instance_id])).await?;
            Ok(())
        }
        // ── Coordination Cleanup (ADR-042) ──────────────
        async fn coordination_cleanup_stale(&self, stale_threshold_secs: u64) -> Result<CoordinationCleanupReport, StateError> {
            let cutoff = (chrono::Utc::now() - chrono::Duration::seconds(stale_threshold_secs as i64)).to_rfc3339();
            self.call_reducer("coordination_cleanup", serde_json::json!([cutoff])).await?;
            Ok(CoordinationCleanupReport { instances_removed: 0, locks_released: 0, claims_released: 0, unstaged_removed: 0 })
        }

        // ── Unified Agent Registry (ADR-058) ─────────────

        async fn hex_agent_connect(&self, id: &str, name: &str, host: &str, project_id: &str, project_dir: &str, model: &str, session_id: &str, capabilities_json: &str) -> Result<(), StateError> {
            let now = chrono::Utc::now().to_rfc3339();
            self.call_reducer("agent_connect", serde_json::json!([id, name, host, project_id, project_dir, model, session_id, capabilities_json, now])).await?;
            Ok(())
        }

        async fn hex_agent_disconnect(&self, id: &str) -> Result<(), StateError> {
            let now = chrono::Utc::now().to_rfc3339();
            self.call_reducer("agent_disconnect", serde_json::json!([id, now])).await?;
            Ok(())
        }

        async fn hex_agent_heartbeat(&self, id: &str) -> Result<(), StateError> {
            let now = chrono::Utc::now().to_rfc3339();
            self.call_reducer("agent_heartbeat_update", serde_json::json!([id, now])).await?;
            Ok(())
        }

        async fn hex_agent_list(&self) -> Result<Vec<serde_json::Value>, StateError> {
            self.query_table("SELECT * FROM hex_agent").await
        }

        async fn hex_agent_get(&self, id: &str) -> Result<Option<serde_json::Value>, StateError> {
            let rows = self.query_table(&format!("SELECT * FROM hex_agent WHERE id = '{}'", id)).await?;
            Ok(rows.into_iter().next())
        }

        async fn hex_agent_evict_dead(&self) -> Result<(), StateError> {
            self.call_reducer("agent_evict_dead", serde_json::json!([])).await?;
            Ok(())
        }

        // ── Agent Notification Inbox (ADR-060) ─────────────

        async fn inbox_notify(&self, agent_id: &str, priority: u8, kind: &str, payload: &str) -> Result<(), StateError> {
            let now = chrono::Utc::now().to_rfc3339();
            self.call_reducer("notify_agent", serde_json::json!([agent_id, priority, kind, payload, now])).await?;
            Ok(())
        }

        async fn inbox_notify_all(&self, project_id: &str, priority: u8, kind: &str, payload: &str) -> Result<(), StateError> {
            let now = chrono::Utc::now().to_rfc3339();
            self.call_reducer("notify_all_agents", serde_json::json!([project_id, priority, kind, payload, now])).await?;
            Ok(())
        }

        async fn inbox_query(&self, agent_id: &str, min_priority: Option<u8>, unacked_only: bool) -> Result<Vec<InboxNotification>, StateError> {
            let mut sql = format!("SELECT * FROM agent_inbox WHERE agent_id = '{}' AND expired_at = ''", agent_id);
            if unacked_only {
                sql.push_str(" AND acknowledged_at = ''");
            }
            if let Some(min_p) = min_priority {
                sql.push_str(&format!(" AND priority >= {}", min_p));
            }
            let rows = self.query_table(&sql).await?;
            Ok(rows.into_iter().filter_map(|r| {
                Some(InboxNotification {
                    id: r.get("id")?.as_u64()?,
                    agent_id: r.get("agent_id")?.as_str()?.to_string(),
                    priority: r.get("priority")?.as_u64()? as u8,
                    kind: r.get("kind")?.as_str()?.to_string(),
                    payload: r.get("payload")?.as_str()?.to_string(),
                    created_at: r.get("created_at")?.as_str()?.to_string(),
                    acknowledged_at: r.get("acknowledged_at").and_then(|v| v.as_str()).filter(|s| !s.is_empty()).map(String::from),
                    expired_at: r.get("expired_at").and_then(|v| v.as_str()).filter(|s| !s.is_empty()).map(String::from),
                })
            }).collect())
        }

        async fn inbox_acknowledge(&self, notification_id: u64, agent_id: &str) -> Result<(), StateError> {
            let now = chrono::Utc::now().to_rfc3339();
            self.call_reducer("acknowledge_notification", serde_json::json!([notification_id, agent_id, now])).await?;
            Ok(())
        }

        async fn inbox_expire(&self, max_age_secs: u64) -> Result<u32, StateError> {
            let threshold = (chrono::Utc::now() - chrono::Duration::seconds(max_age_secs as i64)).to_rfc3339();
            self.call_reducer("expire_stale_notifications", serde_json::json!([threshold])).await?;
            Ok(0) // SpacetimeDB reducers don't return counts
        }

        async fn hex_agent_mark_inactive(&self) -> Result<(), StateError> {
            let stale = (chrono::Utc::now() - chrono::Duration::minutes(2)).to_rfc3339();
            let dead = (chrono::Utc::now() - chrono::Duration::minutes(10)).to_rfc3339();
            self.call_reducer("agent_mark_inactive", serde_json::json!([stale, dead])).await?;
            Ok(())
        }

        // ── Neural Lab (architecture search) ──────────────

        async fn neural_lab_config_list(&self, status: Option<&str>) -> Result<Vec<serde_json::Value>, StateError> {
            const DB: &str = "neural-lab";
            let sql = match status {
                Some(s) => format!("SELECT * FROM network_config WHERE status = '{}'", s),
                None => "SELECT * FROM network_config".to_string(),
            };
            self.query_table_on(DB, &sql).await
        }

        async fn neural_lab_config_get(&self, id: &str) -> Result<Option<serde_json::Value>, StateError> {
            const DB: &str = "neural-lab";
            let sql = format!("SELECT * FROM network_config WHERE id = '{}'", id);
            let rows = self.query_table_on(DB, &sql).await?;
            Ok(rows.into_iter().next())
        }

        async fn neural_lab_config_create(&self, args: serde_json::Value) -> Result<serde_json::Value, StateError> {
            const DB: &str = "neural-lab";
            self.call_reducer_on(DB, "config_create", args).await
        }

        async fn neural_lab_layer_specs(&self, config_id: &str) -> Result<Vec<serde_json::Value>, StateError> {
            const DB: &str = "neural-lab";
            let sql = format!("SELECT * FROM layer_spec WHERE config_id = '{}' ORDER BY layer_index", config_id);
            self.query_table_on(DB, &sql).await
        }

        async fn neural_lab_experiment_list(&self, lineage: Option<&str>, status: Option<&str>) -> Result<Vec<serde_json::Value>, StateError> {
            const DB: &str = "neural-lab";
            let mut sql = "SELECT * FROM experiment".to_string();
            let mut conditions = Vec::new();
            if let Some(l) = lineage {
                conditions.push(format!("lineage_name = '{}'", l));
            }
            if let Some(s) = status {
                conditions.push(format!("status = '{}'", s));
            }
            if !conditions.is_empty() {
                sql.push_str(" WHERE ");
                sql.push_str(&conditions.join(" AND "));
            }
            self.query_table_on(DB, &sql).await
        }

        async fn neural_lab_experiment_get(&self, id: &str) -> Result<Option<serde_json::Value>, StateError> {
            const DB: &str = "neural-lab";
            let sql = format!("SELECT * FROM experiment WHERE id = '{}'", id);
            let rows = self.query_table_on(DB, &sql).await?;
            Ok(rows.into_iter().next())
        }

        async fn neural_lab_experiment_create(&self, args: serde_json::Value) -> Result<serde_json::Value, StateError> {
            const DB: &str = "neural-lab";
            self.call_reducer_on(DB, "experiment_create", args).await
        }

        async fn neural_lab_experiment_start(&self, id: &str, gpu_node_id: &str) -> Result<(), StateError> {
            const DB: &str = "neural-lab";
            self.call_reducer_on(DB, "experiment_start", serde_json::json!([id, gpu_node_id])).await?;
            Ok(())
        }

        async fn neural_lab_experiment_complete(&self, args: serde_json::Value) -> Result<(), StateError> {
            const DB: &str = "neural-lab";
            self.call_reducer_on(DB, "experiment_complete", args).await?;
            Ok(())
        }

        async fn neural_lab_experiment_fail(&self, id: &str, error_message: &str) -> Result<(), StateError> {
            const DB: &str = "neural-lab";
            self.call_reducer_on(DB, "experiment_fail", serde_json::json!([id, error_message])).await?;
            Ok(())
        }

        async fn neural_lab_frontier_get(&self, lineage: &str) -> Result<Option<serde_json::Value>, StateError> {
            const DB: &str = "neural-lab";
            let sql = format!("SELECT * FROM research_frontier WHERE lineage_name = '{}'", lineage);
            let rows = self.query_table_on(DB, &sql).await?;
            Ok(rows.into_iter().next())
        }

        async fn neural_lab_strategies_list(&self) -> Result<Vec<serde_json::Value>, StateError> {
            const DB: &str = "neural-lab";
            self.query_table_on(DB, "SELECT * FROM mutation_strategy").await
        }

        // ── Subscriptions ───────────────────────────────
        // SpacetimeDB forwards table change callbacks through this channel

        fn subscribe(&self) -> broadcast::Receiver<StateEvent> {
            self.event_tx.subscribe()
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Stub implementation (no SpacetimeDB SDK)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(not(feature = "spacetimedb"))]
mod stub {
    use super::*;

    pub struct SpacetimeStateAdapter {
        config: SpacetimeConfig,
        event_tx: broadcast::Sender<StateEvent>,
    }

    impl SpacetimeStateAdapter {
        pub fn new(config: SpacetimeConfig) -> Self {
            let (event_tx, _) = broadcast::channel(256);
            Self { config, event_tx }
        }

        pub async fn connect(&self) -> Result<(), StateError> {
            tracing::info!(host = %self.config.host, db = %self.config.database, "SpacetimeDB feature not enabled");
            Err(StateError::Connection("SpacetimeDB not compiled — rebuild with --features spacetimedb".into()))
        }

        fn err() -> StateError { StateError::Connection("SpacetimeDB not compiled".into()) }
    }

    #[async_trait]
    impl IStatePort for SpacetimeStateAdapter {
        async fn rl_select_action(&self, _: &RlState) -> Result<String, StateError> { Err(Self::err()) }
        async fn rl_record_reward(&self, _: &str, _: &str, _: f64, _: &str, _: bool, _: f64) -> Result<(), StateError> { Err(Self::err()) }
        async fn rl_get_stats(&self) -> Result<RlStats, StateError> { Err(Self::err()) }
        async fn pattern_store(&self, _: &str, _: &str, _: f64) -> Result<String, StateError> { Err(Self::err()) }
        async fn pattern_search(&self, _: &str, _: &str, _: u32) -> Result<Vec<PatternEntry>, StateError> { Err(Self::err()) }
        async fn pattern_reinforce(&self, _: &str, _: f64) -> Result<(), StateError> { Err(Self::err()) }
        async fn pattern_decay_all(&self) -> Result<u32, StateError> { Err(Self::err()) }
        async fn agent_register(&self, _: AgentInfo) -> Result<String, StateError> { Err(Self::err()) }
        async fn agent_update_status(&self, _: &str, _: AgentStatus, _: Option<AgentMetricsData>) -> Result<(), StateError> { Err(Self::err()) }
        async fn agent_list(&self) -> Result<Vec<AgentInfo>, StateError> { Err(Self::err()) }
        async fn agent_get(&self, _: &str) -> Result<Option<AgentInfo>, StateError> { Err(Self::err()) }
        async fn agent_remove(&self, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        async fn workplan_update_task(&self, _: WorkplanTaskUpdate) -> Result<(), StateError> { Err(Self::err()) }
        async fn workplan_get_tasks(&self, _: &str) -> Result<Vec<WorkplanTaskUpdate>, StateError> { Err(Self::err()) }
        async fn chat_send(&self, _: ChatMessage) -> Result<(), StateError> { Err(Self::err()) }
        async fn chat_history(&self, _: &str, _: u32) -> Result<Vec<ChatMessage>, StateError> { Err(Self::err()) }
        async fn fleet_register(&self, _: FleetNode) -> Result<(), StateError> { Err(Self::err()) }
        async fn fleet_update_status(&self, _: &str, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        async fn fleet_list(&self) -> Result<Vec<FleetNode>, StateError> { Err(Self::err()) }
        async fn fleet_remove(&self, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        async fn skill_register(&self, _: SkillEntry) -> Result<String, StateError> { Err(Self::err()) }
        async fn skill_update(&self, _: &str, _: &str, _: &str, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        async fn skill_remove(&self, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        async fn skill_list(&self) -> Result<Vec<SkillEntry>, StateError> { Err(Self::err()) }
        async fn skill_get(&self, _: &str) -> Result<Option<SkillEntry>, StateError> { Err(Self::err()) }
        async fn skill_search(&self, _: &str, _: &str) -> Result<Vec<SkillEntry>, StateError> { Err(Self::err()) }
        async fn hook_register(&self, _: HookEntry) -> Result<String, StateError> { Err(Self::err()) }
        async fn hook_update(&self, _: &str, _: &str, _: u32, _: bool, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        async fn hook_remove(&self, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        async fn hook_toggle(&self, _: &str, _: bool) -> Result<(), StateError> { Err(Self::err()) }
        async fn hook_list(&self) -> Result<Vec<HookEntry>, StateError> { Err(Self::err()) }
        async fn hook_list_by_event(&self, _: &str) -> Result<Vec<HookEntry>, StateError> { Err(Self::err()) }
        async fn hook_log_execution(&self, _: HookExecutionEntry) -> Result<(), StateError> { Err(Self::err()) }
        async fn agent_def_register(&self, _: AgentDefinitionEntry) -> Result<String, StateError> { Err(Self::err()) }
        async fn agent_def_update(&self, _: &str, _: &str, _: &str, _: &str, _: &str, _: &str, _: u32, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        async fn agent_def_remove(&self, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        async fn agent_def_list(&self) -> Result<Vec<AgentDefinitionEntry>, StateError> { Err(Self::err()) }
        async fn agent_def_get_by_name(&self, _: &str) -> Result<Option<AgentDefinitionEntry>, StateError> { Err(Self::err()) }
        async fn agent_def_versions(&self, _: &str) -> Result<Vec<AgentDefinitionVersionEntry>, StateError> { Err(Self::err()) }
        async fn swarm_init(&self, _: &str, _: &str, _: &str, _: &str, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        async fn swarm_complete(&self, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        async fn swarm_fail(&self, _: &str, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        async fn swarm_list_active(&self) -> Result<Vec<SwarmInfo>, StateError> { Err(Self::err()) }
        async fn swarm_list_failed(&self) -> Result<Vec<SwarmInfo>, StateError> { Err(Self::err()) }
        async fn swarm_list_all(&self, _: usize) -> Result<Vec<SwarmInfo>, StateError> { Err(Self::err()) }
        async fn swarm_list_by_project(&self, _: &str) -> Result<Vec<SwarmInfo>, StateError> { Err(Self::err()) }
        async fn swarm_get(&self, _: &str) -> Result<Option<SwarmInfo>, StateError> { Err(Self::err()) }
        async fn swarm_owned_by_agent(&self, _: &str) -> Result<Option<SwarmInfo>, StateError> { Err(Self::err()) }
        async fn swarm_transfer(&self, _: &str, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        async fn swarm_task_create(&self, _: &str, _: &str, _: &str, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        async fn swarm_task_assign(&self, _: &str, _: &str, _: Option<u64>) -> Result<(), StateError> { Err(Self::err()) }
        async fn swarm_task_complete(&self, _: &str, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        async fn swarm_task_fail(&self, _: &str, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        async fn swarm_task_list(&self, _: Option<&str>) -> Result<Vec<SwarmTaskInfo>, StateError> { Err(Self::err()) }
        async fn inference_task_create(&self, _: &str, _: &str, _: &str, _: &str, _: &str, _: &str, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        async fn inference_task_claim(&self, _: &str, _: &str, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        async fn inference_task_complete(&self, _: &str, _: &str, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        async fn inference_task_fail(&self, _: &str, _: &str, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        async fn inference_task_get(&self, _: &str) -> Result<Option<InferenceTaskInfo>, StateError> { Err(Self::err()) }
        async fn inference_task_list_pending(&self) -> Result<Vec<InferenceTaskInfo>, StateError> { Err(Self::err()) }
        async fn swarm_agent_register(&self, _: &str, _: &str, _: &str, _: &str, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        async fn swarm_agent_heartbeat(&self, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        async fn swarm_agent_remove(&self, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        async fn swarm_cleanup_stale(&self, _: u64, _: u64) -> Result<CleanupReport, StateError> { Err(Self::err()) }
        async fn hexflo_memory_store(&self, _: &str, _: &str, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        async fn hexflo_memory_retrieve(&self, _: &str) -> Result<Option<String>, StateError> { Err(Self::err()) }
        async fn hexflo_memory_search(&self, _: &str) -> Result<Vec<(String, String)>, StateError> { Err(Self::err()) }
        async fn hexflo_memory_delete(&self, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        // ── Quality Gate & Fix Tasks (Swarm Gate Enforcement) ──
        async fn quality_gate_create(&self, _: &str, _: &str, _: u32, _: &str, _: &str, _: &str, _: u32) -> Result<(), StateError> { Err(Self::err()) }
        async fn quality_gate_complete(&self, _: &str, _: &str, _: u32, _: &str, _: u32, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        async fn quality_gate_list(&self, _: &str) -> Result<Vec<QualityGateInfo>, StateError> { Err(Self::err()) }
        async fn quality_gate_get(&self, _: &str) -> Result<Option<QualityGateInfo>, StateError> { Err(Self::err()) }
        async fn fix_task_create(&self, _: &str, _: &str, _: &str, _: &str, _: &str, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        async fn fix_task_complete(&self, _: &str, _: &str, _: &str, _: &str, _: u64, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        async fn fix_task_list_by_gate(&self, _: &str) -> Result<Vec<FixTaskInfo>, StateError> { Err(Self::err()) }
        // ── Project Registry (ADR-042) ──────────────────
        async fn project_register(&self, _: ProjectRegistration) -> Result<(), StateError> { Err(Self::err()) }
        async fn project_unregister(&self, _: &str) -> Result<bool, StateError> { Err(Self::err()) }
        async fn project_get(&self, _: &str) -> Result<Option<ProjectRecord>, StateError> { Err(Self::err()) }
        async fn project_list(&self) -> Result<Vec<ProjectRecord>, StateError> { Err(Self::err()) }
        async fn project_update_state(&self, _: &str, _: &str, _: serde_json::Value, _: Option<&str>) -> Result<(), StateError> { Err(Self::err()) }
        async fn project_find(&self, _: &str) -> Result<Option<ProjectRecord>, StateError> { Err(Self::err()) }
        // ── Instance Coordination (ADR-042) ─────────────
        async fn instance_register(&self, _: InstanceRecord) -> Result<String, StateError> { Err(Self::err()) }
        async fn instance_heartbeat(&self, _: &str, _: InstanceHeartbeat) -> Result<(), StateError> { Err(Self::err()) }
        async fn instance_list(&self, _: Option<&str>) -> Result<Vec<InstanceRecord>, StateError> { Err(Self::err()) }
        async fn instance_remove(&self, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        // ── Worktree Locks (ADR-042) ────────────────────
        async fn worktree_lock_acquire(&self, _: WorktreeLockRecord) -> Result<bool, StateError> { Err(Self::err()) }
        async fn worktree_lock_release(&self, _: &str) -> Result<bool, StateError> { Err(Self::err()) }
        async fn worktree_lock_list(&self, _: Option<&str>) -> Result<Vec<WorktreeLockRecord>, StateError> { Err(Self::err()) }
        async fn worktree_lock_refresh(&self, _: &str, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        async fn worktree_lock_evict_expired(&self) -> Result<u32, StateError> { Err(Self::err()) }
        // ── Task Claims (ADR-042) ───────────────────────
        async fn task_claim_acquire(&self, _: TaskClaimRecord) -> Result<bool, StateError> { Err(Self::err()) }
        async fn task_claim_release(&self, _: &str) -> Result<bool, StateError> { Err(Self::err()) }
        async fn task_claim_list(&self, _: Option<&str>) -> Result<Vec<TaskClaimRecord>, StateError> { Err(Self::err()) }
        async fn task_claim_refresh(&self, _: &str, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        // ── Unstaged Files (ADR-042) ────────────────────
        async fn unstaged_update(&self, _: &str, _: UnstagedRecord) -> Result<(), StateError> { Err(Self::err()) }
        async fn unstaged_list(&self, _: Option<&str>) -> Result<Vec<UnstagedRecord>, StateError> { Err(Self::err()) }
        async fn unstaged_remove(&self, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        // ── Coordination Cleanup (ADR-042) ──────────────
        async fn coordination_cleanup_stale(&self, _: u64) -> Result<CoordinationCleanupReport, StateError> { Err(Self::err()) }
        // ── Unified Agent Registry (ADR-058) ─────────────
        async fn hex_agent_connect(&self, _: &str, _: &str, _: &str, _: &str, _: &str, _: &str, _: &str, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        async fn hex_agent_disconnect(&self, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        async fn hex_agent_heartbeat(&self, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        async fn hex_agent_list(&self) -> Result<Vec<serde_json::Value>, StateError> { Err(Self::err()) }
        async fn hex_agent_get(&self, _: &str) -> Result<Option<serde_json::Value>, StateError> { Err(Self::err()) }
        async fn hex_agent_evict_dead(&self) -> Result<(), StateError> { Err(Self::err()) }
        async fn hex_agent_mark_inactive(&self) -> Result<(), StateError> { Err(Self::err()) }
        // ── Agent Notification Inbox (ADR-060) ──────────────
        async fn inbox_notify(&self, _: &str, _: u8, _: &str, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        async fn inbox_notify_all(&self, _: &str, _: u8, _: &str, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        async fn inbox_query(&self, _: &str, _: Option<u8>, _: bool) -> Result<Vec<InboxNotification>, StateError> { Err(Self::err()) }
        async fn inbox_acknowledge(&self, _: u64, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        async fn inbox_expire(&self, _: u64) -> Result<u32, StateError> { Err(Self::err()) }
        // ── Neural Lab (architecture search) ──────────────
        async fn neural_lab_config_list(&self, _: Option<&str>) -> Result<Vec<serde_json::Value>, StateError> { Err(Self::err()) }
        async fn neural_lab_config_get(&self, _: &str) -> Result<Option<serde_json::Value>, StateError> { Err(Self::err()) }
        async fn neural_lab_config_create(&self, _: serde_json::Value) -> Result<serde_json::Value, StateError> { Err(Self::err()) }
        async fn neural_lab_layer_specs(&self, _: &str) -> Result<Vec<serde_json::Value>, StateError> { Err(Self::err()) }
        async fn neural_lab_experiment_list(&self, _: Option<&str>, _: Option<&str>) -> Result<Vec<serde_json::Value>, StateError> { Err(Self::err()) }
        async fn neural_lab_experiment_get(&self, _: &str) -> Result<Option<serde_json::Value>, StateError> { Err(Self::err()) }
        async fn neural_lab_experiment_create(&self, _: serde_json::Value) -> Result<serde_json::Value, StateError> { Err(Self::err()) }
        async fn neural_lab_experiment_start(&self, _: &str, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        async fn neural_lab_experiment_complete(&self, _: serde_json::Value) -> Result<(), StateError> { Err(Self::err()) }
        async fn neural_lab_experiment_fail(&self, _: &str, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        async fn neural_lab_frontier_get(&self, _: &str) -> Result<Option<serde_json::Value>, StateError> { Err(Self::err()) }
        async fn neural_lab_strategies_list(&self) -> Result<Vec<serde_json::Value>, StateError> { Err(Self::err()) }
        fn subscribe(&self) -> broadcast::Receiver<StateEvent> { self.event_tx.subscribe() }
    }
}

#[cfg(feature = "spacetimedb")]
pub use real::SpacetimeStateAdapter;
#[cfg(not(feature = "spacetimedb"))]
pub use stub::SpacetimeStateAdapter;
