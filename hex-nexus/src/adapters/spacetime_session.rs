//! SpacetimeDB-backed session persistence adapter (ADR-036 / ADR-042 P2.5).
//!
//! Implements `ISessionPort` by calling the `chat-relay` SpacetimeDB module's
//! session reducers and SQL queries over HTTP. SpacetimeDB is the single state
//! authority (ADR-051, ADR-2604020900) — no SQLite fallback.

use async_trait::async_trait;
use serde_json::Value;
use std::time::Duration;
use uuid::Uuid;

use crate::ports::session::{
    ISessionPort, Message, MessagePart, NewMessage, Role, Session, SessionError, SessionId,
    SessionStatus, SessionSummary, TokenUsage,
};

/// SpacetimeDB-backed session adapter using the `chat-relay` module.
pub struct SpacetimeSessionAdapter {
    http: reqwest::Client,
    host: String,
    database: String,
}

impl SpacetimeSessionAdapter {
    pub fn new(host: String, database: String) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .pool_max_idle_per_host(4)
            .build()
            .expect("failed to build HTTP client");

        Self {
            http,
            host,
            database,
        }
    }

    /// Probe connectivity — returns true if the chat-relay module is reachable.
    pub async fn probe(&self) -> bool {
        let url = format!("{}/v1/database/{}/sql", self.host, self.database);
        match self
            .http
            .post(&url)
            .body("SELECT id FROM chat_session LIMIT 1")
            .header("Content-Type", "text/plain")
            .send()
            .await
        {
            Ok(resp) => resp.status().is_success(),
            Err(_) => false,
        }
    }

    fn now() -> String {
        chrono::Utc::now().to_rfc3339()
    }

    async fn call_reducer(&self, name: &str, args: Value) -> Result<(), SessionError> {
        let url = format!(
            "{}/v1/database/{}/call/{}",
            self.host, self.database, name
        );

        let response = self
            .http
            .post(&url)
            .json(&args)
            .send()
            .await
            .map_err(|e| SessionError::Storage(format!("SpacetimeDB {name}: {e}")))?;

        if response.status().is_success() {
            Ok(())
        } else {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            Err(SessionError::Storage(format!(
                "Reducer '{name}' returned {status}: {body}"
            )))
        }
    }

    async fn sql_query(&self, query: &str) -> Result<Vec<Value>, SessionError> {
        let url = format!("{}/v1/database/{}/sql", self.host, self.database);

        let response = self
            .http
            .post(&url)
            .body(query.to_string())
            .header("Content-Type", "text/plain")
            .send()
            .await
            .map_err(|e| SessionError::Storage(format!("SQL query failed: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(SessionError::Storage(format!(
                "SQL query failed ({status}): {body}"
            )));
        }

        let body = response.text().await.unwrap_or_default();
        let parsed: Value = serde_json::from_str(&body)
            .map_err(|e| SessionError::Serialization(format!("parse SQL response: {e}")))?;

        let rows = parsed
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|table| table.get("rows"))
            .and_then(|r| r.as_array())
            .cloned()
            .unwrap_or_default();

        Ok(rows)
    }

    fn str_col(cols: &[Value], idx: usize) -> String {
        cols.get(idx)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    }

    fn u64_col(cols: &[Value], idx: usize) -> u64 {
        cols.get(idx)
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
    }

    fn row_to_session(cols: &[Value]) -> Session {
        // Column order: id, parent_id, project_id, title, model, status, created_at, updated_at
        let parent_id = Self::str_col(cols, 1);
        Session {
            id: Self::str_col(cols, 0),
            parent_id: if parent_id.is_empty() { None } else { Some(parent_id) },
            project_id: Self::str_col(cols, 2),
            title: Self::str_col(cols, 3),
            model: Self::str_col(cols, 4),
            status: Self::str_col(cols, 5).parse().unwrap_or(SessionStatus::Active),
            created_at: Self::str_col(cols, 6),
            updated_at: Self::str_col(cols, 7),
        }
    }

    fn row_to_message(cols: &[Value]) -> Message {
        // Column order: id, session_id, role, parts_json, model, input_tokens, output_tokens, sequence, created_at
        let parts_json = Self::str_col(cols, 3);
        let parts: Vec<MessagePart> = serde_json::from_str(&parts_json).unwrap_or_default();
        let role: Role = Self::str_col(cols, 2).parse().unwrap_or(Role::User);
        let input = Self::u64_col(cols, 5);
        let output = Self::u64_col(cols, 6);
        let model_str = Self::str_col(cols, 4);
        Message {
            id: Self::str_col(cols, 0),
            session_id: Self::str_col(cols, 1),
            role,
            parts,
            model: if model_str.is_empty() { None } else { Some(model_str) },
            token_usage: if input > 0 || output > 0 {
                Some(TokenUsage {
                    input_tokens: input,
                    output_tokens: output,
                })
            } else {
                None
            },
            sequence: Self::u64_col(cols, 7) as u32,
            created_at: Self::str_col(cols, 8),
        }
    }

    fn escape_sql(s: &str) -> String {
        s.replace('\'', "''")
    }
}

#[async_trait]
impl ISessionPort for SpacetimeSessionAdapter {
    async fn session_create(
        &self,
        project_id: &str,
        model: &str,
        title: Option<&str>,
    ) -> Result<Session, SessionError> {
        let id = Uuid::new_v4().to_string();
        let now = Self::now();
        let title = title.unwrap_or("New conversation").to_string();

        self.call_reducer(
            "session_create",
            serde_json::json!([id, project_id, model, title, now]),
        )
        .await?;

        Ok(Session {
            id,
            parent_id: None,
            project_id: project_id.to_string(),
            title,
            model: model.to_string(),
            status: SessionStatus::Active,
            created_at: now.clone(),
            updated_at: now,
        })
    }

    async fn session_get(&self, id: &SessionId) -> Result<Option<Session>, SessionError> {
        let query = format!(
            "SELECT id, parent_id, project_id, title, model, status, created_at, updated_at \
             FROM chat_session WHERE id = '{}'",
            Self::escape_sql(id)
        );
        let rows = self.sql_query(&query).await?;
        if let Some(row) = rows.first() {
            if let Some(cols) = row.as_array() {
                if cols.len() >= 8 {
                    return Ok(Some(Self::row_to_session(cols)));
                }
            }
        }
        Ok(None)
    }

    async fn session_list(
        &self,
        project_id: &str,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<SessionSummary>, SessionError> {
        // SpacetimeDB SQL doesn't support JOINs well, so we do two queries
        let session_query = format!(
            "SELECT id, parent_id, project_id, title, model, status, created_at, updated_at \
             FROM chat_session WHERE project_id = '{}'",
            Self::escape_sql(project_id)
        );
        let session_rows = self.sql_query(&session_query).await?;

        let mut summaries: Vec<SessionSummary> = Vec::new();
        for row in &session_rows {
            if let Some(cols) = row.as_array() {
                if cols.len() >= 8 {
                    let sid = Self::str_col(cols, 0);
                    let parent_id = Self::str_col(cols, 1);

                    // Get message stats for this session
                    let stats_query = format!(
                        "SELECT id, input_tokens, output_tokens FROM chat_session_message WHERE session_id = '{}'",
                        Self::escape_sql(&sid)
                    );
                    let msg_rows = self.sql_query(&stats_query).await.unwrap_or_default();
                    let msg_count = msg_rows.len() as u32;
                    let mut total_in: u64 = 0;
                    let mut total_out: u64 = 0;
                    for mrow in &msg_rows {
                        if let Some(mcols) = mrow.as_array() {
                            total_in += Self::u64_col(mcols, 1);
                            total_out += Self::u64_col(mcols, 2);
                        }
                    }

                    summaries.push(SessionSummary {
                        id: sid,
                        parent_id: if parent_id.is_empty() { None } else { Some(parent_id) },
                        project_id: Self::str_col(cols, 2),
                        title: Self::str_col(cols, 3),
                        model: Self::str_col(cols, 4),
                        status: Self::str_col(cols, 5).parse().unwrap_or(SessionStatus::Active),
                        created_at: Self::str_col(cols, 6),
                        updated_at: Self::str_col(cols, 7),
                        message_count: msg_count,
                        total_input_tokens: total_in,
                        total_output_tokens: total_out,
                    });
                }
            }
        }

        // Sort by updated_at DESC
        summaries.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

        // Apply offset and limit
        let start = offset as usize;
        let end = std::cmp::min(start + limit as usize, summaries.len());
        if start >= summaries.len() {
            return Ok(Vec::new());
        }
        Ok(summaries[start..end].to_vec())
    }

    async fn session_update_title(
        &self,
        id: &SessionId,
        title: &str,
    ) -> Result<(), SessionError> {
        self.call_reducer(
            "session_update_title",
            serde_json::json!([id, title, Self::now()]),
        )
        .await
    }

    async fn session_archive(&self, id: &SessionId) -> Result<(), SessionError> {
        self.call_reducer(
            "session_set_status",
            serde_json::json!([id, "archived", Self::now()]),
        )
        .await
    }

    async fn session_delete(&self, id: &SessionId) -> Result<(), SessionError> {
        self.call_reducer("session_delete", serde_json::json!([id])).await
    }

    async fn message_append(
        &self,
        session_id: &SessionId,
        msg: NewMessage,
    ) -> Result<Message, SessionError> {
        let id = Uuid::new_v4().to_string();
        let now = Self::now();
        let parts_json = serde_json::to_string(&msg.parts)
            .map_err(|e| SessionError::Serialization(e.to_string()))?;
        let input_tokens = msg.token_usage.map(|t| t.input_tokens).unwrap_or(0);
        let output_tokens = msg.token_usage.map(|t| t.output_tokens).unwrap_or(0);
        let model_str = msg.model.clone().unwrap_or_default();

        // Get next sequence
        let seq_query = format!(
            "SELECT sequence FROM chat_session_message WHERE session_id = '{}'",
            Self::escape_sql(session_id)
        );
        let seq_rows = self.sql_query(&seq_query).await?;
        let max_seq: u32 = seq_rows
            .iter()
            .filter_map(|r| r.as_array())
            .filter_map(|cols| cols.first())
            .filter_map(|v| v.as_u64())
            .max()
            .unwrap_or(0) as u32;
        let sequence = max_seq + 1;

        self.call_reducer(
            "session_message_append",
            serde_json::json!([
                id, session_id, msg.role.to_string(), parts_json, model_str,
                input_tokens, output_tokens, sequence, now
            ]),
        )
        .await?;

        Ok(Message {
            id,
            session_id: session_id.clone(),
            role: msg.role,
            parts: msg.parts,
            model: msg.model,
            token_usage: msg.token_usage,
            sequence,
            created_at: now,
        })
    }

    async fn message_list(
        &self,
        session_id: &SessionId,
        limit: u32,
        before_sequence: Option<u32>,
    ) -> Result<Vec<Message>, SessionError> {
        let query = if let Some(before) = before_sequence {
            format!(
                "SELECT id, session_id, role, parts_json, model, input_tokens, output_tokens, sequence, created_at \
                 FROM chat_session_message WHERE session_id = '{}' AND sequence < {}",
                Self::escape_sql(session_id),
                before
            )
        } else {
            format!(
                "SELECT id, session_id, role, parts_json, model, input_tokens, output_tokens, sequence, created_at \
                 FROM chat_session_message WHERE session_id = '{}'",
                Self::escape_sql(session_id)
            )
        };

        let rows = self.sql_query(&query).await?;
        let mut messages: Vec<Message> = Vec::new();
        for row in &rows {
            if let Some(cols) = row.as_array() {
                if cols.len() >= 9 {
                    messages.push(Self::row_to_message(cols));
                }
            }
        }

        // Sort by sequence ASC
        messages.sort_by_key(|m| m.sequence);

        // Apply limit (from end if before_sequence, from start otherwise)
        if before_sequence.is_some() {
            // Take last `limit` messages
            let len = messages.len();
            if len > limit as usize {
                messages = messages[len - limit as usize..].to_vec();
            }
        } else if messages.len() > limit as usize {
            messages.truncate(limit as usize);
        }

        Ok(messages)
    }

    async fn session_fork(
        &self,
        id: &SessionId,
        at_sequence: Option<u32>,
    ) -> Result<Session, SessionError> {
        // Load parent session
        let parent = self
            .session_get(id)
            .await?
            .ok_or_else(|| SessionError::NotFound(id.clone()))?;

        // Check fork depth (max 5)
        let mut depth = 0u32;
        let mut cursor = parent.parent_id.clone();
        while let Some(ref pid) = cursor {
            depth += 1;
            if depth >= 5 {
                return Err(SessionError::InvalidOperation(
                    "fork depth limit (5) reached".to_string(),
                ));
            }
            cursor = self
                .session_get(pid)
                .await?
                .and_then(|s| s.parent_id);
        }

        let new_id = Uuid::new_v4().to_string();
        let now = Self::now();
        let new_title = format!("{} (fork)", parent.title);

        // Create forked session
        self.call_reducer(
            "session_insert_forked",
            serde_json::json!([new_id, id, parent.project_id, new_title, parent.model, now]),
        )
        .await?;

        // Copy messages up to at_sequence
        let all_messages = self.message_list(id, u32::MAX, None).await?;
        for msg in &all_messages {
            if let Some(seq) = at_sequence {
                if msg.sequence > seq {
                    continue;
                }
            }
            let parts_json = serde_json::to_string(&msg.parts)
                .map_err(|e| SessionError::Serialization(e.to_string()))?;
            let input = msg.token_usage.map(|t| t.input_tokens).unwrap_or(0);
            let output = msg.token_usage.map(|t| t.output_tokens).unwrap_or(0);
            let msg_id = Uuid::new_v4().to_string();
            self.call_reducer(
                "session_message_append",
                serde_json::json!([
                    msg_id, new_id, msg.role.to_string(), parts_json,
                    msg.model.clone().unwrap_or_default(),
                    input, output, msg.sequence, msg.created_at
                ]),
            )
            .await?;
        }

        Ok(Session {
            id: new_id,
            parent_id: Some(id.clone()),
            project_id: parent.project_id,
            title: new_title,
            model: parent.model,
            status: SessionStatus::Active,
            created_at: now.clone(),
            updated_at: now,
        })
    }

    async fn session_revert(
        &self,
        id: &SessionId,
        to_sequence: u32,
    ) -> Result<(), SessionError> {
        self.call_reducer(
            "session_revert",
            serde_json::json!([id, to_sequence, Self::now()]),
        )
        .await
    }

    async fn session_compact(
        &self,
        id: &SessionId,
        summary: &str,
    ) -> Result<(), SessionError> {
        let now = Self::now();

        // Get all messages to find max sequence
        let messages = self.message_list(id, u32::MAX, None).await?;
        let max_seq = messages.iter().map(|m| m.sequence).max().unwrap_or(0);

        if max_seq <= 1 {
            return Err(SessionError::InvalidOperation(
                "nothing to compact (0-1 messages)".to_string(),
            ));
        }

        // Keep last 20% (minimum 2)
        let keep_count = std::cmp::max((max_seq as f64 * 0.2).ceil() as u32, 2);
        let archive_threshold = max_seq.saturating_sub(keep_count);

        // Archive old messages via reducer
        self.call_reducer(
            "session_archive_messages",
            serde_json::json!([id, archive_threshold, now]),
        )
        .await?;

        // Insert summary as system message at sequence 0
        let summary_parts = serde_json::to_string(&vec![MessagePart::Text {
            content: format!("[Compacted] {summary}"),
        }])
        .map_err(|e| SessionError::Serialization(e.to_string()))?;

        let summary_id = Uuid::new_v4().to_string();
        self.call_reducer(
            "session_message_append",
            serde_json::json!([
                summary_id, id, "system", summary_parts, "", 0u64, 0u64, 0u32, now
            ]),
        )
        .await?;

        // Update session status
        self.call_reducer(
            "session_set_status",
            serde_json::json!([id, "compacted", now]),
        )
        .await?;

        Ok(())
    }

    async fn session_search(
        &self,
        project_id: &str,
        query: &str,
        limit: u32,
    ) -> Result<Vec<SessionSummary>, SessionError> {
        // SpacetimeDB doesn't support LIKE natively the same way, so we fetch
        // all sessions for the project and filter in Rust
        let all = self.session_list(project_id, u32::MAX, 0).await?;
        let query_lower = query.to_lowercase();

        let mut results: Vec<SessionSummary> = all
            .into_iter()
            .filter(|s| {
                if s.title.to_lowercase().contains(&query_lower) {
                    return true;
                }
                // We can't efficiently search message content via SQL here,
                // so title-only search for SpacetimeDB backend
                false
            })
            .take(limit as usize)
            .collect();

        // If no title matches, try message content search (slower)
        if results.is_empty() {
            let session_query = format!(
                "SELECT id FROM chat_session WHERE project_id = '{}'",
                Self::escape_sql(project_id)
            );
            let session_rows = self.sql_query(&session_query).await?;
            for row in &session_rows {
                if results.len() >= limit as usize {
                    break;
                }
                if let Some(cols) = row.as_array() {
                    let sid = Self::str_col(cols, 0);
                    let msg_query = format!(
                        "SELECT parts_json FROM chat_session_message WHERE session_id = '{}'",
                        Self::escape_sql(&sid)
                    );
                    let msg_rows = self.sql_query(&msg_query).await.unwrap_or_default();
                    let has_match = msg_rows.iter().any(|mr| {
                        if let Some(mc) = mr.as_array() {
                            Self::str_col(mc, 0).to_lowercase().contains(&query_lower)
                        } else {
                            false
                        }
                    });
                    if has_match {
                        if let Ok(summaries) = self.session_list(project_id, 1, 0).await {
                            if let Some(s) = summaries.into_iter().find(|s| s.id == sid) {
                                results.push(s);
                            }
                        }
                    }
                }
            }
        }

        Ok(results)
    }
}
