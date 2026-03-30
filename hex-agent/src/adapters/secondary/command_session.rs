use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::io::AsyncBufReadExt;
use tokio::process::Command;
use tokio::sync::RwLock;
use tokio::time::timeout;
use uuid::Uuid;

use crate::ports::command_session::{
    BatchSession, CommandSessionError, IBatchExecutionPort, SearchResult,
};

// ---------------------------------------------------------------------------
// Internal data structures
// ---------------------------------------------------------------------------

struct IndexedLine {
    command: String,
    line_number: usize,
    text: String,
}

#[allow(dead_code)]
struct SessionData {
    lines: Vec<IndexedLine>,
    created_at: Instant,
    total_bytes: usize,
}

// ---------------------------------------------------------------------------
// Adapter
// ---------------------------------------------------------------------------

/// Secondary adapter that executes shell commands via `sh -c`, captures their
/// output line-by-line, indexes everything in memory, and supports full-text
/// search over the indexed lines.
pub struct CommandSessionAdapter {
    sessions: Arc<RwLock<HashMap<String, SessionData>>>,
    max_bytes: usize,
    timeout_secs: u64,
}

impl Default for CommandSessionAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandSessionAdapter {
    pub fn new() -> Self {
        let max_mb: usize = std::env::var("HEX_CMD_SESSION_MAX_MB")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(500);

        let timeout_secs: u64 = std::env::var("HEX_CMD_TIMEOUT_SECONDS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(60);

        let adapter = Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            max_bytes: max_mb * 1024 * 1024,
            timeout_secs,
        };

        // Background sweeper: remove sessions older than 10 minutes.
        let sessions_ref = Arc::clone(&adapter.sessions);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            loop {
                interval.tick().await;
                let ttl = Duration::from_secs(600); // 10 min
                let mut map = sessions_ref.write().await;
                map.retain(|_, s| s.created_at.elapsed() < ttl);
            }
        });

        adapter
    }
}

#[async_trait]
impl IBatchExecutionPort for CommandSessionAdapter {
    async fn batch_execute(
        &self,
        commands: Vec<String>,
    ) -> Result<BatchSession, CommandSessionError> {
        let mut all_lines: Vec<IndexedLine> = Vec::new();
        let mut exit_codes: Vec<i32> = Vec::new();
        let mut total_bytes: usize = 0;

        for cmd in &commands {
            // Capacity check before running each command.
            if total_bytes >= self.max_bytes {
                return Err(CommandSessionError::CapacityExceeded(self.max_bytes));
            }

            let child_result = Command::new("sh")
                .arg("-c")
                .arg(cmd)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn();

            let mut child = match child_result {
                Ok(c) => c,
                Err(e) => {
                    return Err(CommandSessionError::SpawnError(e.to_string()));
                }
            };

            let stdout = child.stdout.take().map(tokio::io::BufReader::new);
            let stderr = child.stderr.take().map(tokio::io::BufReader::new);

            let cmd_clone = cmd.clone();

            // Collect stdout and stderr concurrently, then wait for exit.
            let timeout_duration = Duration::from_secs(self.timeout_secs);
            let run_result = timeout(timeout_duration, async {
                let mut collected: Vec<String> = Vec::new();

                if let Some(mut reader) = stdout {
                    let mut line = String::new();
                    loop {
                        line.clear();
                        match reader.read_line(&mut line).await {
                            Ok(0) => break,
                            Ok(_) => collected.push(line.trim_end_matches('\n').to_string()),
                            Err(e) => return Err(CommandSessionError::Io(e.to_string())),
                        }
                    }
                }

                if let Some(mut reader) = stderr {
                    let mut line = String::new();
                    loop {
                        line.clear();
                        match reader.read_line(&mut line).await {
                            Ok(0) => break,
                            Ok(_) => collected.push(line.trim_end_matches('\n').to_string()),
                            Err(e) => return Err(CommandSessionError::Io(e.to_string())),
                        }
                    }
                }

                Ok(collected)
            })
            .await;

            let (exit_code, lines) = match run_result {
                Err(_elapsed) => {
                    // Timeout — kill the process.
                    let _ = child.kill().await;
                    (-1, Vec::new())
                }
                Ok(Err(e)) => return Err(e),
                Ok(Ok(lines)) => {
                    let status = child
                        .wait()
                        .await
                        .map(|s| s.code().unwrap_or(-1))
                        .unwrap_or(-1);
                    (status, lines)
                }
            };

            exit_codes.push(exit_code);

            for (idx, text) in lines.into_iter().enumerate() {
                total_bytes += text.len() + 1; // +1 for newline
                if total_bytes > self.max_bytes {
                    return Err(CommandSessionError::CapacityExceeded(self.max_bytes));
                }
                all_lines.push(IndexedLine {
                    command: cmd_clone.clone(),
                    line_number: idx + 1,
                    text,
                });
            }
        }

        let total_lines = all_lines.len();
        let session_id = Uuid::new_v4().to_string();

        {
            let mut map = self.sessions.write().await;
            map.insert(
                session_id.clone(),
                SessionData {
                    lines: all_lines,
                    created_at: Instant::now(),
                    total_bytes,
                },
            );
        }

        Ok(BatchSession {
            session_id,
            total_lines,
            exit_codes,
        })
    }

    async fn search(
        &self,
        session_id: &str,
        queries: Vec<String>,
        max_results: usize,
    ) -> Result<Vec<SearchResult>, CommandSessionError> {
        let map = self.sessions.read().await;
        let session = map
            .get(session_id)
            .ok_or_else(|| CommandSessionError::SessionExpired(session_id.to_string()))?;

        // key = (command, line_number) → highest score seen
        let mut best: HashMap<(String, usize), (f32, String)> = HashMap::new();

        for line in &session.lines {
            let mut top_score: f32 = 0.0;

            for query in &queries {
                let score = if line.text.contains(query.as_str()) {
                    1.0
                } else if line
                    .text
                    .to_lowercase()
                    .contains(&query.to_lowercase())
                {
                    0.9
                } else {
                    // Token match: any query token appears in the line
                    let hits = query
                        .split_whitespace()
                        .any(|token| line.text.to_lowercase().contains(&token.to_lowercase()));
                    if hits { 0.5 } else { 0.0 }
                };

                if score > top_score {
                    top_score = score;
                }
            }

            if top_score > 0.0 {
                let key = (line.command.clone(), line.line_number);
                let entry = best.entry(key).or_insert((0.0, line.text.clone()));
                if top_score > entry.0 {
                    *entry = (top_score, line.text.clone());
                }
            }
        }

        // Convert to SearchResult, sort by score descending, truncate.
        let mut results: Vec<SearchResult> = best
            .into_iter()
            .map(|((command, line_number), (score, text))| SearchResult {
                command,
                line_number,
                text,
                score,
            })
            .collect();

        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(max_results);

        Ok(results)
    }

    async fn drop_session(&self, session_id: &str) -> Result<(), CommandSessionError> {
        let mut map = self.sessions.write().await;
        map.remove(session_id)
            .ok_or_else(|| CommandSessionError::SessionExpired(session_id.to_string()))?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_adapter() -> CommandSessionAdapter {
        CommandSessionAdapter {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            max_bytes: 500 * 1024 * 1024,
            timeout_secs: 10,
        }
    }

    #[tokio::test]
    async fn test_batch_execute_returns_session_with_lines_and_exit_codes() {
        let adapter = make_adapter();
        let session = adapter
            .batch_execute(vec![
                "echo hello".to_string(),
                "echo world && exit 2".to_string(),
            ])
            .await
            .expect("batch_execute should succeed");

        assert!(!session.session_id.is_empty(), "session_id must be non-empty");
        assert!(session.total_lines > 0, "should have captured some lines");
        assert_eq!(session.exit_codes.len(), 2);
        assert_eq!(session.exit_codes[0], 0);
        assert_eq!(session.exit_codes[1], 2);
    }

    #[tokio::test]
    async fn test_search_finds_exact_match_with_score_1() {
        let adapter = make_adapter();
        let session = adapter
            .batch_execute(vec!["printf 'alpha\\nbeta\\ngamma\\n'".to_string()])
            .await
            .expect("batch_execute should succeed");

        let results = adapter
            .search(&session.session_id, vec!["beta".to_string()], 10)
            .await
            .expect("search should succeed");

        assert!(!results.is_empty(), "should find at least one result");
        let top = &results[0];
        assert_eq!(top.score, 1.0, "exact substring match must score 1.0");
        assert!(top.text.contains("beta"));
    }

    #[tokio::test]
    async fn test_search_on_expired_session_returns_session_expired() {
        // Build an adapter with extremely short TTL sweep — we manually drop the session
        // from the map to simulate expiry rather than sleeping.
        let adapter = make_adapter();

        let session = adapter
            .batch_execute(vec!["echo test".to_string()])
            .await
            .expect("batch_execute should succeed");

        // Manually remove the session to simulate expiry.
        {
            let mut map = adapter.sessions.write().await;
            map.remove(&session.session_id);
        }

        let err = adapter
            .search(&session.session_id, vec!["test".to_string()], 10)
            .await
            .expect_err("should return error for expired session");

        assert!(
            matches!(err, CommandSessionError::SessionExpired(_)),
            "expected SessionExpired, got: {:?}",
            err
        );
    }

    #[tokio::test]
    async fn test_drop_session_removes_it() {
        let adapter = make_adapter();
        let session = adapter
            .batch_execute(vec!["echo drop_me".to_string()])
            .await
            .expect("batch_execute should succeed");

        adapter
            .drop_session(&session.session_id)
            .await
            .expect("drop_session should succeed");

        let err = adapter
            .search(&session.session_id, vec!["drop_me".to_string()], 10)
            .await
            .expect_err("session should be gone after drop");

        assert!(matches!(err, CommandSessionError::SessionExpired(_)));
    }
}
