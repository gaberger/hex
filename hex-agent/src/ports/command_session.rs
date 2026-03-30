use std::path::Path;
use async_trait::async_trait;

/// Port for sandboxed batch command execution with indexed output.
///
/// Runs shell commands, indexes their output in-memory, and allows
/// semantic search over the results — keeping raw output out of
/// agent context windows.
#[async_trait]
pub trait IBatchExecutionPort: Send + Sync {
    /// Run commands sequentially in working_dir, index all output.
    /// Returns a session handle — NOT the raw output.
    async fn batch_execute(
        &self,
        commands: Vec<String>,
        working_dir: &Path,
    ) -> Result<BatchSession, CommandSessionError>;

    /// Search indexed output in a session for lines matching queries.
    /// Returns up to max_results results sorted by score descending.
    async fn search(
        &self,
        session_id: &str,
        queries: Vec<String>,
        max_results: usize,
    ) -> Result<Vec<SearchResult>, CommandSessionError>;

    /// Discard a session's indexed output and free memory.
    async fn drop_session(&self, session_id: &str);
}

/// Handle returned by batch_execute — contains stats, not raw output.
#[derive(Debug, Clone)]
pub struct BatchSession {
    pub session_id: String,
    pub commands_run: usize,
    pub total_lines: usize,
    /// Exit code per command (-1 if killed by timeout).
    pub exit_codes: Vec<i32>,
}

/// A single line matched by a search query.
#[derive(Debug, Clone)]
pub struct SearchResult {
    /// The command that produced this line.
    pub command: String,
    pub line_number: usize,
    pub line: String,
    /// 1.0 = exact match, lower = fuzzy.
    pub score: f32,
}

#[derive(Debug, thiserror::Error)]
pub enum CommandSessionError {
    #[error("Session '{session_id}' not found or has expired")]
    SessionExpired { session_id: String },
    #[error("Command execution failed: {0}")]
    ExecutionFailed(String),
    #[error("Working directory not found: {path}")]
    WorkingDirNotFound { path: String },
    #[error("Indexed output capacity exceeded (limit: {limit_mb}MB)")]
    CapacityExceeded { limit_mb: usize },
}
