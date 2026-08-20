//! `repo_read` — let the agent loop read a repo-relative file before drafting
//! (wp-sop-agent-loop P1.2).
//!
//! This is the foundational tool: today's blind-draft failure mode (the
//! drafter writing `use hex_cli::HEX_NEXUS_URL;` when no such item exists)
//! is exactly what `repo_read` cures. With it, the persona can look at
//! the surrounding files BEFORE producing code.
//!
//! Truncates at 16k chars and reports the original byte count so the
//! persona can follow up with a more specific read (or `repo_grep` once
//! P1.3 lands) if the file is too big.

use async_trait::async_trait;
use serde::Deserialize;

use crate::orchestration::agent_loop::policy;
use crate::orchestration::agent_loop::tool::{IAgentTool, Observation, ToolError};

const MAX_BYTES: usize = 16 * 1024;

pub struct RepoReadTool {
    repo_root: std::path::PathBuf,
}

impl RepoReadTool {
    pub fn new(repo_root: impl Into<std::path::PathBuf>) -> Self {
        Self { repo_root: repo_root.into() }
    }

    /// Resolve `repo_root` from `HEX_REPO_ROOT` env, falling back to the
    /// process's current directory. Matches the resolution `action_executor`
    /// uses, so a single hex-nexus process serves both consistently.
    pub fn from_env() -> Self {
        let repo_root = std::env::var("HEX_REPO_ROOT")
            .ok()
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| ".".into()));
        Self::new(repo_root)
    }
}

#[derive(Deserialize)]
struct Args {
    path: String,
}

#[async_trait]
impl IAgentTool for RepoReadTool {
    fn name(&self) -> &str { "repo_read" }

    fn description(&self) -> &str {
        "Read a repo-relative file. Returns the first 16k chars (truncated:true if larger). \
         Use this BEFORE drafting code to ground your output in the actual surrounding files. \
         Allowed paths: hex-*/src/, hex-*/tests/, docs/, examples/, scripts/, spacetime-modules/, \
         */Cargo.toml. Absolute paths and `..` traversal are rejected."
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "required": ["path"],
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Repo-relative path, e.g. 'hex-cli/tests/worker_local_ollama_e2e.rs'"
                }
            }
        })
    }

    async fn invoke(&self, args: serde_json::Value) -> Result<Observation, ToolError> {
        let parsed: Args = serde_json::from_value(args)
            .map_err(|e| ToolError::InvalidArgs(format!("expected {{\"path\": string}}: {}", e)))?;

        policy::allowed_repo_path(&parsed.path).map_err(ToolError::PolicyDenied)?;

        let full = self.repo_root.join(&parsed.path);
        // Guard against symlink escape: canonicalize the resolved path
        // and verify it still lives under repo_root. Catches the
        // `hex-cli/tests/symlink-to-etc-passwd` case the prefix check misses.
        if let (Ok(canonical), Ok(root)) = (
            tokio::fs::canonicalize(&full).await,
            tokio::fs::canonicalize(&self.repo_root).await,
        ) {
            if !canonical.starts_with(&root) {
                return Err(ToolError::PolicyDenied(format!(
                    "symlink escapes repo root: {}",
                    parsed.path
                )));
            }
        }

        let bytes = match tokio::fs::read(&full).await {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(ToolError::NotFound(parsed.path));
            }
            Err(e) => {
                return Err(ToolError::Exec(format!("read {}: {}", parsed.path, e)));
            }
        };

        let original_bytes = bytes.len();
        // Lossy UTF-8 is intentional: binary files (rare for repo_read but
        // possible) yield mojibake rather than panicking. The persona can
        // notice and skip.
        let content_string = String::from_utf8_lossy(&bytes).into_owned();

        if original_bytes > MAX_BYTES {
            let mut truncated = String::with_capacity(MAX_BYTES + 64);
            // char_indices to stay UTF-8 safe when slicing a lossy string.
            for (i, ch) in content_string.char_indices() {
                if i >= MAX_BYTES { break; }
                truncated.push(ch);
            }
            truncated.push_str("\n[... truncated; use repo_grep or a narrower read for more ...]");
            return Ok(Observation::truncated(truncated, original_bytes));
        }

        Ok(Observation {
            output: content_string,
            bytes: original_bytes,
            truncated: false,
            terminal: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_repo() -> (tempfile::TempDir, RepoReadTool) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        // Lay down enough fake repo skeleton that the allowlist will let
        // us read the test fixtures.
        std::fs::create_dir_all(root.join("hex-cli/tests")).unwrap();
        std::fs::create_dir_all(root.join("docs")).unwrap();
        let tool = RepoReadTool::new(root);
        (dir, tool)
    }

    #[tokio::test]
    async fn reads_a_small_file() {
        let (dir, tool) = tmp_repo();
        std::fs::write(dir.path().join("docs/note.md"), "hello agent loop\n").unwrap();

        let obs = tool.invoke(serde_json::json!({"path": "docs/note.md"})).await.unwrap();
        assert_eq!(obs.output, "hello agent loop\n");
        assert_eq!(obs.bytes, 17);
        assert!(!obs.truncated);
        assert!(obs.terminal.is_none());
    }

    #[tokio::test]
    async fn truncates_large_files_and_reports_original_bytes() {
        let (dir, tool) = tmp_repo();
        let big = "a".repeat(20 * 1024);
        std::fs::write(dir.path().join("hex-cli/tests/big.rs"), &big).unwrap();

        let obs = tool.invoke(serde_json::json!({"path": "hex-cli/tests/big.rs"})).await.unwrap();
        assert!(obs.truncated);
        assert_eq!(obs.bytes, 20 * 1024);
        assert!(obs.output.contains("[... truncated"));
        assert!(obs.output.len() < big.len());
    }

    #[tokio::test]
    async fn rejects_absolute_path_via_policy() {
        let (_dir, tool) = tmp_repo();
        let err = tool.invoke(serde_json::json!({"path": "/etc/passwd"})).await.unwrap_err();
        match err {
            ToolError::PolicyDenied(msg) => assert!(msg.contains("absolute")),
            other => panic!("expected PolicyDenied, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn rejects_traversal_via_policy() {
        let (_dir, tool) = tmp_repo();
        let err = tool.invoke(serde_json::json!({"path": "hex-cli/tests/../../../etc/passwd"}))
            .await.unwrap_err();
        assert!(matches!(err, ToolError::PolicyDenied(_)));
    }

    #[tokio::test]
    async fn rejects_unknown_prefix_via_policy() {
        let (_dir, tool) = tmp_repo();
        let err = tool.invoke(serde_json::json!({"path": "foo/bar.rs"})).await.unwrap_err();
        assert!(matches!(err, ToolError::PolicyDenied(_)));
    }

    #[tokio::test]
    async fn missing_file_reports_not_found() {
        let (_dir, tool) = tmp_repo();
        let err = tool.invoke(serde_json::json!({"path": "docs/does-not-exist.md"})).await.unwrap_err();
        match err {
            ToolError::NotFound(p) => assert_eq!(p, "docs/does-not-exist.md"),
            other => panic!("expected NotFound, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn malformed_args_reports_invalid() {
        let (_dir, tool) = tmp_repo();
        let err = tool.invoke(serde_json::json!({})).await.unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs(_)));
    }
}
