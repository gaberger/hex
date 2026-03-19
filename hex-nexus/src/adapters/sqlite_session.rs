use async_trait::async_trait;
use rusqlite::Connection;
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::ports::session::{
    ISessionPort, Message, MessagePart, NewMessage, Role, Session, SessionError, SessionId,
    SessionStatus, SessionSummary, TokenUsage,
};

/// SQLite-backed session persistence adapter.
///
/// Uses a dedicated connection to `~/.hex/hub.db` (same DB as IStatePort)
/// with two tables: `sessions` and `session_messages`.
pub struct SqliteSessionAdapter {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteSessionAdapter {
    pub async fn new(conn: Arc<Mutex<Connection>>) -> Result<Self, SessionError> {
        let adapter = Self { conn };
        let conn_guard = adapter.conn.lock().await;
        Self::migrate(&conn_guard)?;
        drop(conn_guard);
        Ok(adapter)
    }

    /// Create from a file path (opens or creates the DB).
    pub async fn from_path(path: &str) -> Result<Self, SessionError> {
        let conn = Connection::open(path)
            .map_err(|e| SessionError::Storage(format!("failed to open DB: {e}")))?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .map_err(|e| SessionError::Storage(format!("pragma error: {e}")))?;
        let shared = Arc::new(Mutex::new(conn));
        let adapter = Self { conn: shared };
        let conn_guard = adapter.conn.lock().await;
        Self::migrate(&conn_guard)?;
        drop(conn_guard);
        Ok(adapter)
    }

    fn migrate(conn: &Connection) -> Result<(), SessionError> {
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                parent_id TEXT REFERENCES sessions(id),
                project_id TEXT NOT NULL,
                title TEXT NOT NULL DEFAULT '',
                model TEXT NOT NULL DEFAULT '',
                status TEXT NOT NULL DEFAULT 'active',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS session_messages (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                role TEXT NOT NULL,
                parts_json TEXT NOT NULL,
                model TEXT,
                input_tokens INTEGER DEFAULT 0,
                output_tokens INTEGER DEFAULT 0,
                sequence INTEGER NOT NULL,
                created_at TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_session_messages_session
                ON session_messages(session_id, sequence);
            CREATE INDEX IF NOT EXISTS idx_sessions_project
                ON sessions(project_id, updated_at DESC);

            CREATE TABLE IF NOT EXISTS session_messages_archive (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                role TEXT NOT NULL,
                parts_json TEXT NOT NULL,
                model TEXT,
                input_tokens INTEGER DEFAULT 0,
                output_tokens INTEGER DEFAULT 0,
                sequence INTEGER NOT NULL,
                created_at TEXT NOT NULL,
                archived_at TEXT NOT NULL
            );
            ",
        )
        .map_err(|e| SessionError::Storage(format!("migration error: {e}")))?;
        Ok(())
    }

    fn now() -> String {
        chrono::Utc::now().to_rfc3339()
    }

    fn row_to_message(row: &rusqlite::Row<'_>) -> rusqlite::Result<Message> {
        let parts_json: String = row.get(3)?;
        let parts: Vec<MessagePart> = serde_json::from_str(&parts_json).unwrap_or_default();
        let role_str: String = row.get(2)?;
        let role: Role = role_str.parse().unwrap_or(Role::User);
        let input: i64 = row.get(5)?;
        let output: i64 = row.get(6)?;
        let token_usage = if input > 0 || output > 0 {
            Some(TokenUsage {
                input_tokens: input as u64,
                output_tokens: output as u64,
            })
        } else {
            None
        };
        Ok(Message {
            id: row.get(0)?,
            session_id: row.get(1)?,
            role,
            parts,
            model: row.get(4)?,
            token_usage,
            sequence: row.get::<_, i64>(7)? as u32,
            created_at: row.get(8)?,
        })
    }
}

#[async_trait]
impl ISessionPort for SqliteSessionAdapter {
    async fn session_create(
        &self,
        project_id: &str,
        model: &str,
        title: Option<&str>,
    ) -> Result<Session, SessionError> {
        let id = Uuid::new_v4().to_string();
        let now = Self::now();
        let title = title.unwrap_or("New conversation").to_string();
        let session = Session {
            id: id.clone(),
            parent_id: None,
            project_id: project_id.to_string(),
            title: title.clone(),
            model: model.to_string(),
            status: SessionStatus::Active,
            created_at: now.clone(),
            updated_at: now.clone(),
        };
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO sessions (id, project_id, title, model, status, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, 'active', ?5, ?6)",
            rusqlite::params![id, project_id, title, model, now, now],
        )
        .map_err(|e| SessionError::Storage(e.to_string()))?;
        Ok(session)
    }

    async fn session_get(&self, id: &SessionId) -> Result<Option<Session>, SessionError> {
        let conn = self.conn.lock().await;
        let mut stmt = conn
            .prepare(
                "SELECT id, parent_id, project_id, title, model, status, created_at, updated_at
                 FROM sessions WHERE id = ?1",
            )
            .map_err(|e| SessionError::Storage(e.to_string()))?;
        let result = stmt
            .query_row(rusqlite::params![id], |row| {
                let status_str: String = row.get(5)?;
                Ok(Session {
                    id: row.get(0)?,
                    parent_id: row.get(1)?,
                    project_id: row.get(2)?,
                    title: row.get(3)?,
                    model: row.get(4)?,
                    status: status_str.parse().unwrap_or(SessionStatus::Active),
                    created_at: row.get(6)?,
                    updated_at: row.get(7)?,
                })
            })
            .optional()
            .map_err(|e| SessionError::Storage(e.to_string()))?;
        Ok(result)
    }

    async fn session_list(
        &self,
        project_id: &str,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<SessionSummary>, SessionError> {
        let conn = self.conn.lock().await;
        let mut stmt = conn
            .prepare(
                "SELECT s.id, s.parent_id, s.project_id, s.title, s.model, s.status,
                        s.created_at, s.updated_at,
                        COUNT(m.id) as msg_count,
                        COALESCE(SUM(m.input_tokens), 0) as total_in,
                        COALESCE(SUM(m.output_tokens), 0) as total_out
                 FROM sessions s
                 LEFT JOIN session_messages m ON m.session_id = s.id
                 WHERE s.project_id = ?1
                 GROUP BY s.id
                 ORDER BY s.updated_at DESC
                 LIMIT ?2 OFFSET ?3",
            )
            .map_err(|e| SessionError::Storage(e.to_string()))?;
        let rows = stmt
            .query_map(rusqlite::params![project_id, limit, offset], |row| {
                let status_str: String = row.get(5)?;
                Ok(SessionSummary {
                    id: row.get(0)?,
                    parent_id: row.get(1)?,
                    project_id: row.get(2)?,
                    title: row.get(3)?,
                    model: row.get(4)?,
                    status: status_str.parse().unwrap_or(SessionStatus::Active),
                    created_at: row.get(6)?,
                    updated_at: row.get(7)?,
                    message_count: row.get::<_, i64>(8)? as u32,
                    total_input_tokens: row.get::<_, i64>(9)? as u64,
                    total_output_tokens: row.get::<_, i64>(10)? as u64,
                })
            })
            .map_err(|e| SessionError::Storage(e.to_string()))?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(|e| SessionError::Storage(e.to_string()))?);
        }
        Ok(results)
    }

    async fn session_update_title(
        &self,
        id: &SessionId,
        title: &str,
    ) -> Result<(), SessionError> {
        let conn = self.conn.lock().await;
        let updated = conn
            .execute(
                "UPDATE sessions SET title = ?1, updated_at = ?2 WHERE id = ?3",
                rusqlite::params![title, Self::now(), id],
            )
            .map_err(|e| SessionError::Storage(e.to_string()))?;
        if updated == 0 {
            return Err(SessionError::NotFound(id.clone()));
        }
        Ok(())
    }

    async fn session_archive(&self, id: &SessionId) -> Result<(), SessionError> {
        let conn = self.conn.lock().await;
        let updated = conn
            .execute(
                "UPDATE sessions SET status = 'archived', updated_at = ?1 WHERE id = ?2",
                rusqlite::params![Self::now(), id],
            )
            .map_err(|e| SessionError::Storage(e.to_string()))?;
        if updated == 0 {
            return Err(SessionError::NotFound(id.clone()));
        }
        Ok(())
    }

    async fn session_delete(&self, id: &SessionId) -> Result<(), SessionError> {
        let conn = self.conn.lock().await;
        // CASCADE will delete session_messages too
        let deleted = conn
            .execute("DELETE FROM sessions WHERE id = ?1", rusqlite::params![id])
            .map_err(|e| SessionError::Storage(e.to_string()))?;
        if deleted == 0 {
            return Err(SessionError::NotFound(id.clone()));
        }
        Ok(())
    }

    async fn message_append(
        &self,
        session_id: &SessionId,
        msg: NewMessage,
    ) -> Result<Message, SessionError> {
        let conn = self.conn.lock().await;
        let id = Uuid::new_v4().to_string();
        let now = Self::now();
        let parts_json = serde_json::to_string(&msg.parts)
            .map_err(|e| SessionError::Serialization(e.to_string()))?;
        let input_tokens = msg.token_usage.map(|t| t.input_tokens as i64).unwrap_or(0);
        let output_tokens = msg.token_usage.map(|t| t.output_tokens as i64).unwrap_or(0);

        // Get next sequence number
        let sequence: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(sequence), 0) + 1 FROM session_messages WHERE session_id = ?1",
                rusqlite::params![session_id],
                |row| row.get(0),
            )
            .map_err(|e| SessionError::Storage(e.to_string()))?;

        conn.execute(
            "INSERT INTO session_messages (id, session_id, role, parts_json, model, input_tokens, output_tokens, sequence, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![id, session_id, msg.role.to_string(), parts_json, msg.model, input_tokens, output_tokens, sequence, now],
        )
        .map_err(|e| SessionError::Storage(e.to_string()))?;

        // Touch session updated_at
        conn.execute(
            "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
            rusqlite::params![now, session_id],
        )
        .map_err(|e| SessionError::Storage(e.to_string()))?;

        Ok(Message {
            id,
            session_id: session_id.clone(),
            role: msg.role,
            parts: msg.parts,
            model: msg.model,
            token_usage: msg.token_usage,
            sequence: sequence as u32,
            created_at: now,
        })
    }

    async fn message_list(
        &self,
        session_id: &SessionId,
        limit: u32,
        before_sequence: Option<u32>,
    ) -> Result<Vec<Message>, SessionError> {
        let conn = self.conn.lock().await;
        let messages = if let Some(before) = before_sequence {
            let mut stmt = conn
                .prepare(
                    "SELECT id, session_id, role, parts_json, model, input_tokens, output_tokens, sequence, created_at
                     FROM session_messages
                     WHERE session_id = ?1 AND sequence < ?2
                     ORDER BY sequence DESC LIMIT ?3",
                )
                .map_err(|e| SessionError::Storage(e.to_string()))?;
            let rows = stmt
                .query_map(rusqlite::params![session_id, before, limit], Self::row_to_message)
                .map_err(|e| SessionError::Storage(e.to_string()))?;
            let mut msgs: Vec<Message> = Vec::new();
            for row in rows {
                msgs.push(row.map_err(|e| SessionError::Storage(e.to_string()))?);
            }
            msgs.reverse(); // Return in ascending order
            msgs
        } else {
            let mut stmt = conn
                .prepare(
                    "SELECT id, session_id, role, parts_json, model, input_tokens, output_tokens, sequence, created_at
                     FROM session_messages
                     WHERE session_id = ?1
                     ORDER BY sequence ASC LIMIT ?2",
                )
                .map_err(|e| SessionError::Storage(e.to_string()))?;
            let rows = stmt
                .query_map(rusqlite::params![session_id, limit], Self::row_to_message)
                .map_err(|e| SessionError::Storage(e.to_string()))?;
            let mut msgs: Vec<Message> = Vec::new();
            for row in rows {
                msgs.push(row.map_err(|e| SessionError::Storage(e.to_string()))?);
            }
            msgs
        };
        Ok(messages)
    }

    async fn session_fork(
        &self,
        id: &SessionId,
        at_sequence: Option<u32>,
    ) -> Result<Session, SessionError> {
        let conn = self.conn.lock().await;

        // Load parent session
        let parent: Session = conn
            .query_row(
                "SELECT id, parent_id, project_id, title, model, status, created_at, updated_at
                 FROM sessions WHERE id = ?1",
                rusqlite::params![id],
                |row| {
                    let status_str: String = row.get(5)?;
                    Ok(Session {
                        id: row.get(0)?,
                        parent_id: row.get(1)?,
                        project_id: row.get(2)?,
                        title: row.get(3)?,
                        model: row.get(4)?,
                        status: status_str.parse().unwrap_or(SessionStatus::Active),
                        created_at: row.get(6)?,
                        updated_at: row.get(7)?,
                    })
                },
            )
            .optional()
            .map_err(|e| SessionError::Storage(e.to_string()))?
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
            cursor = conn
                .query_row(
                    "SELECT parent_id FROM sessions WHERE id = ?1",
                    rusqlite::params![pid],
                    |row| row.get::<_, Option<String>>(0),
                )
                .optional()
                .map_err(|e| SessionError::Storage(e.to_string()))?
                .flatten();
        }

        let new_id = Uuid::new_v4().to_string();
        let now = Self::now();
        let new_title = format!("{} (fork)", parent.title);

        conn.execute(
            "INSERT INTO sessions (id, parent_id, project_id, title, model, status, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 'active', ?6, ?7)",
            rusqlite::params![new_id, id, parent.project_id, new_title, parent.model, now, now],
        )
        .map_err(|e| SessionError::Storage(e.to_string()))?;

        // Copy messages up to at_sequence
        let seq_clause = if let Some(seq) = at_sequence {
            format!("AND sequence <= {seq}")
        } else {
            String::new()
        };
        conn.execute(
            &format!(
                "INSERT INTO session_messages (id, session_id, role, parts_json, model, input_tokens, output_tokens, sequence, created_at)
                 SELECT hex(randomblob(16)), ?1, role, parts_json, model, input_tokens, output_tokens, sequence, created_at
                 FROM session_messages WHERE session_id = ?2 {seq_clause}
                 ORDER BY sequence"
            ),
            rusqlite::params![new_id, id],
        )
        .map_err(|e| SessionError::Storage(e.to_string()))?;

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
        let conn = self.conn.lock().await;
        conn.execute(
            "DELETE FROM session_messages WHERE session_id = ?1 AND sequence > ?2",
            rusqlite::params![id, to_sequence],
        )
        .map_err(|e| SessionError::Storage(e.to_string()))?;
        conn.execute(
            "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
            rusqlite::params![Self::now(), id],
        )
        .map_err(|e| SessionError::Storage(e.to_string()))?;
        Ok(())
    }

    async fn session_compact(
        &self,
        id: &SessionId,
        summary: &str,
    ) -> Result<(), SessionError> {
        let conn = self.conn.lock().await;
        let now = Self::now();

        // Get current max sequence
        let max_seq: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(sequence), 0) FROM session_messages WHERE session_id = ?1",
                rusqlite::params![id],
                |row| row.get(0),
            )
            .map_err(|e| SessionError::Storage(e.to_string()))?;

        if max_seq <= 1 {
            return Err(SessionError::InvalidOperation(
                "nothing to compact (0-1 messages)".to_string(),
            ));
        }

        // Keep the last 20% of messages (minimum 2), archive the rest
        let keep_count = std::cmp::max((max_seq as f64 * 0.2).ceil() as i64, 2);
        let archive_threshold = max_seq - keep_count;

        // Archive old messages
        conn.execute(
            "INSERT INTO session_messages_archive (id, session_id, role, parts_json, model, input_tokens, output_tokens, sequence, created_at, archived_at)
             SELECT id, session_id, role, parts_json, model, input_tokens, output_tokens, sequence, created_at, ?1
             FROM session_messages WHERE session_id = ?2 AND sequence <= ?3",
            rusqlite::params![now, id, archive_threshold],
        )
        .map_err(|e| SessionError::Storage(e.to_string()))?;

        // Delete archived messages from main table
        conn.execute(
            "DELETE FROM session_messages WHERE session_id = ?1 AND sequence <= ?2",
            rusqlite::params![id, archive_threshold],
        )
        .map_err(|e| SessionError::Storage(e.to_string()))?;

        // Insert summary as new sequence 0 system message
        let summary_parts = serde_json::to_string(&vec![MessagePart::Text {
            content: format!("[Compacted] {summary}"),
        }])
        .map_err(|e| SessionError::Serialization(e.to_string()))?;

        conn.execute(
            "INSERT INTO session_messages (id, session_id, role, parts_json, model, input_tokens, output_tokens, sequence, created_at)
             VALUES (?1, ?2, 'system', ?3, NULL, 0, 0, 0, ?4)",
            rusqlite::params![Uuid::new_v4().to_string(), id, summary_parts, now],
        )
        .map_err(|e| SessionError::Storage(e.to_string()))?;

        // Update session status
        conn.execute(
            "UPDATE sessions SET status = 'compacted', updated_at = ?1 WHERE id = ?2",
            rusqlite::params![now, id],
        )
        .map_err(|e| SessionError::Storage(e.to_string()))?;

        Ok(())
    }

    async fn session_search(
        &self,
        project_id: &str,
        query: &str,
        limit: u32,
    ) -> Result<Vec<SessionSummary>, SessionError> {
        let conn = self.conn.lock().await;
        let pattern = format!("%{query}%");
        let mut stmt = conn
            .prepare(
                "SELECT DISTINCT s.id, s.parent_id, s.project_id, s.title, s.model, s.status,
                        s.created_at, s.updated_at,
                        (SELECT COUNT(*) FROM session_messages WHERE session_id = s.id) as msg_count,
                        COALESCE((SELECT SUM(input_tokens) FROM session_messages WHERE session_id = s.id), 0) as total_in,
                        COALESCE((SELECT SUM(output_tokens) FROM session_messages WHERE session_id = s.id), 0) as total_out
                 FROM sessions s
                 LEFT JOIN session_messages m ON m.session_id = s.id
                 WHERE s.project_id = ?1
                   AND (s.title LIKE ?2 OR m.parts_json LIKE ?2)
                 ORDER BY s.updated_at DESC
                 LIMIT ?3",
            )
            .map_err(|e| SessionError::Storage(e.to_string()))?;
        let rows = stmt
            .query_map(rusqlite::params![project_id, pattern, limit], |row| {
                let status_str: String = row.get(5)?;
                Ok(SessionSummary {
                    id: row.get(0)?,
                    parent_id: row.get(1)?,
                    project_id: row.get(2)?,
                    title: row.get(3)?,
                    model: row.get(4)?,
                    status: status_str.parse().unwrap_or(SessionStatus::Active),
                    created_at: row.get(6)?,
                    updated_at: row.get(7)?,
                    message_count: row.get::<_, i64>(8)? as u32,
                    total_input_tokens: row.get::<_, i64>(9)? as u64,
                    total_output_tokens: row.get::<_, i64>(10)? as u64,
                })
            })
            .map_err(|e| SessionError::Storage(e.to_string()))?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(|e| SessionError::Storage(e.to_string()))?);
        }
        Ok(results)
    }
}

// ── Convenience trait (rusqlite optional row) ───────────

trait OptionalExt<T> {
    fn optional(self) -> Result<Option<T>, rusqlite::Error>;
}

impl<T> OptionalExt<T> for Result<T, rusqlite::Error> {
    fn optional(self) -> Result<Option<T>, rusqlite::Error> {
        match self {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::session::{MessagePart, NewMessage, Role, TokenUsage};

    async fn test_adapter() -> SqliteSessionAdapter {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        let shared = Arc::new(Mutex::new(conn));
        SqliteSessionAdapter::new(shared).await.unwrap()
    }

    #[tokio::test]
    async fn create_and_get_session() {
        let adapter = test_adapter().await;
        let session = adapter.session_create("proj-1", "claude-sonnet-4-20250514", Some("Test chat")).await.unwrap();
        assert_eq!(session.title, "Test chat");
        assert_eq!(session.status, SessionStatus::Active);

        let fetched = adapter.session_get(&session.id).await.unwrap().unwrap();
        assert_eq!(fetched.id, session.id);
        assert_eq!(fetched.project_id, "proj-1");
    }

    #[tokio::test]
    async fn append_and_list_messages() {
        let adapter = test_adapter().await;
        let session = adapter.session_create("proj-1", "sonnet", None).await.unwrap();

        let m1 = adapter
            .message_append(
                &session.id,
                NewMessage {
                    role: Role::User,
                    parts: vec![MessagePart::Text {
                        content: "Hello".to_string(),
                    }],
                    model: None,
                    token_usage: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(m1.sequence, 1);

        let m2 = adapter
            .message_append(
                &session.id,
                NewMessage {
                    role: Role::Assistant,
                    parts: vec![MessagePart::Text {
                        content: "Hi there!".to_string(),
                    }],
                    model: Some("sonnet".to_string()),
                    token_usage: Some(TokenUsage {
                        input_tokens: 10,
                        output_tokens: 5,
                    }),
                },
            )
            .await
            .unwrap();
        assert_eq!(m2.sequence, 2);

        let msgs = adapter.message_list(&session.id, 100, None).await.unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, Role::User);
        assert_eq!(msgs[1].role, Role::Assistant);
    }

    #[tokio::test]
    async fn session_list_with_counts() {
        let adapter = test_adapter().await;
        let s = adapter.session_create("proj-1", "sonnet", Some("Chat A")).await.unwrap();
        adapter
            .message_append(
                &s.id,
                NewMessage {
                    role: Role::User,
                    parts: vec![MessagePart::Text { content: "hi".into() }],
                    model: None,
                    token_usage: Some(TokenUsage { input_tokens: 100, output_tokens: 50 }),
                },
            )
            .await
            .unwrap();

        let list = adapter.session_list("proj-1", 10, 0).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].message_count, 1);
        assert_eq!(list[0].total_input_tokens, 100);
    }

    #[tokio::test]
    async fn fork_session() {
        let adapter = test_adapter().await;
        let s = adapter.session_create("proj-1", "sonnet", Some("Original")).await.unwrap();
        for i in 1..=5 {
            adapter
                .message_append(
                    &s.id,
                    NewMessage {
                        role: Role::User,
                        parts: vec![MessagePart::Text { content: format!("msg {i}") }],
                        model: None,
                        token_usage: None,
                    },
                )
                .await
                .unwrap();
        }

        // Fork at sequence 3
        let forked = adapter.session_fork(&s.id, Some(3)).await.unwrap();
        assert_eq!(forked.parent_id, Some(s.id.clone()));
        assert!(forked.title.contains("fork"));

        let msgs = adapter.message_list(&forked.id, 100, None).await.unwrap();
        assert_eq!(msgs.len(), 3);
    }

    #[tokio::test]
    async fn revert_session() {
        let adapter = test_adapter().await;
        let s = adapter.session_create("proj-1", "sonnet", None).await.unwrap();
        for i in 1..=5 {
            adapter
                .message_append(
                    &s.id,
                    NewMessage {
                        role: Role::User,
                        parts: vec![MessagePart::Text { content: format!("msg {i}") }],
                        model: None,
                        token_usage: None,
                    },
                )
                .await
                .unwrap();
        }

        adapter.session_revert(&s.id, 2).await.unwrap();
        let msgs = adapter.message_list(&s.id, 100, None).await.unwrap();
        assert_eq!(msgs.len(), 2);
    }

    #[tokio::test]
    async fn delete_cascades() {
        let adapter = test_adapter().await;
        let s = adapter.session_create("proj-1", "sonnet", None).await.unwrap();
        adapter
            .message_append(
                &s.id,
                NewMessage {
                    role: Role::User,
                    parts: vec![MessagePart::Text { content: "hi".into() }],
                    model: None,
                    token_usage: None,
                },
            )
            .await
            .unwrap();

        adapter.session_delete(&s.id).await.unwrap();
        assert!(adapter.session_get(&s.id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn search_by_title_and_content() {
        let adapter = test_adapter().await;
        let s = adapter.session_create("proj-1", "sonnet", Some("Debugging auth flow")).await.unwrap();
        adapter
            .message_append(
                &s.id,
                NewMessage {
                    role: Role::User,
                    parts: vec![MessagePart::Text { content: "fix the JWT validation".into() }],
                    model: None,
                    token_usage: None,
                },
            )
            .await
            .unwrap();

        let by_title = adapter.session_search("proj-1", "auth", 10).await.unwrap();
        assert_eq!(by_title.len(), 1);

        let by_content = adapter.session_search("proj-1", "JWT", 10).await.unwrap();
        assert_eq!(by_content.len(), 1);

        let no_match = adapter.session_search("proj-1", "zzz_nonexistent", 10).await.unwrap();
        assert!(no_match.is_empty());
    }
}
