use async_trait::async_trait;
use serde::{Deserialize, Serialize};

// ── Domain Value Objects ────────────────────────────────

/// Branded session identifier (UUID v4 string).
pub type SessionId = String;

/// Branded message identifier (UUID v4 string).
pub type MessageId = String;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Active,
    Archived,
    Compacted,
}

impl std::fmt::Display for SessionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Active => write!(f, "active"),
            Self::Archived => write!(f, "archived"),
            Self::Compacted => write!(f, "compacted"),
        }
    }
}

impl std::str::FromStr for SessionStatus {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "active" => Ok(Self::Active),
            "archived" => Ok(Self::Archived),
            "compacted" => Ok(Self::Compacted),
            other => Err(format!("unknown session status: {other}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
    System,
    Tool,
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::User => write!(f, "user"),
            Self::Assistant => write!(f, "assistant"),
            Self::System => write!(f, "system"),
            Self::Tool => write!(f, "tool"),
        }
    }
}

impl std::str::FromStr for Role {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "user" => Ok(Self::User),
            "assistant" => Ok(Self::Assistant),
            "system" => Ok(Self::System),
            "tool" => Ok(Self::Tool),
            other => Err(format!("unknown role: {other}")),
        }
    }
}

// ── Message Parts (structured content) ──────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MessagePart {
    Text {
        content: String,
    },
    ToolCall {
        tool_name: String,
        arguments: serde_json::Value,
        call_id: String,
    },
    ToolResult {
        call_id: String,
        content: String,
        is_error: bool,
    },
    File {
        path: String,
        language: Option<String>,
        snippet: Option<String>,
    },
}

// ── Token Usage ─────────────────────────────────────────

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

// ── Aggregate: Session ──────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub id: SessionId,
    pub parent_id: Option<SessionId>,
    pub project_id: String,
    pub title: String,
    pub model: String,
    pub status: SessionStatus,
    pub created_at: String,
    pub updated_at: String,
}

/// Lightweight projection for list views (no messages loaded).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub id: SessionId,
    pub parent_id: Option<SessionId>,
    pub project_id: String,
    pub title: String,
    pub model: String,
    pub status: SessionStatus,
    pub message_count: u32,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub created_at: String,
    pub updated_at: String,
}

// ── Entity: Message ─────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub id: MessageId,
    pub session_id: SessionId,
    pub role: Role,
    pub parts: Vec<MessagePart>,
    pub model: Option<String>,
    pub token_usage: Option<TokenUsage>,
    pub sequence: u32,
    pub created_at: String,
}

/// Input type for appending a new message (id + sequence assigned by port).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewMessage {
    pub role: Role,
    pub parts: Vec<MessagePart>,
    pub model: Option<String>,
    pub token_usage: Option<TokenUsage>,
}

// ── The Port ────────────────────────────────────────────

#[async_trait]
pub trait ISessionPort: Send + Sync {
    // ── Session lifecycle ────────────────────────
    async fn session_create(
        &self,
        project_id: &str,
        model: &str,
        title: Option<&str>,
    ) -> Result<Session, SessionError>;

    async fn session_get(&self, id: &SessionId) -> Result<Option<Session>, SessionError>;

    async fn session_list(
        &self,
        project_id: &str,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<SessionSummary>, SessionError>;

    async fn session_update_title(
        &self,
        id: &SessionId,
        title: &str,
    ) -> Result<(), SessionError>;

    async fn session_archive(&self, id: &SessionId) -> Result<(), SessionError>;

    async fn session_delete(&self, id: &SessionId) -> Result<(), SessionError>;

    // ── Messages ─────────────────────────────────
    async fn message_append(
        &self,
        session_id: &SessionId,
        msg: NewMessage,
    ) -> Result<Message, SessionError>;

    async fn message_list(
        &self,
        session_id: &SessionId,
        limit: u32,
        before_sequence: Option<u32>,
    ) -> Result<Vec<Message>, SessionError>;

    // ── Session operations ───────────────────────

    /// Fork a session: copy all messages up to `at_sequence` (or all) into a new session.
    async fn session_fork(
        &self,
        id: &SessionId,
        at_sequence: Option<u32>,
    ) -> Result<Session, SessionError>;

    /// Revert: delete all messages after `to_sequence`.
    async fn session_revert(
        &self,
        id: &SessionId,
        to_sequence: u32,
    ) -> Result<(), SessionError>;

    /// Compact: replace messages before a threshold with a summary system message.
    /// Original messages are archived for auditability.
    async fn session_compact(
        &self,
        id: &SessionId,
        summary: &str,
    ) -> Result<(), SessionError>;

    // ── Search ───────────────────────────────────
    async fn session_search(
        &self,
        project_id: &str,
        query: &str,
        limit: u32,
    ) -> Result<Vec<SessionSummary>, SessionError>;
}

// ── Errors ──────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("Storage error: {0}")]
    Storage(String),
    #[error("Session not found: {0}")]
    NotFound(String),
    #[error("Invalid operation: {0}")]
    InvalidOperation(String),
    #[error("Serialization error: {0}")]
    Serialization(String),
}
