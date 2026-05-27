//! `repo_grep` — search the repo for a pattern (wp-sop-agent-loop P1.3).
//!
//! Lets the agent loop ground its drafts in what already exists. Without
//! it, the persona has to either guess at sibling patterns or repo_read
//! a wide net of files just to find one occurrence.
//!
//! Prefers `rg --json` (ripgrep) for speed + structured output. Falls
//! back to `grep -RnI` if rg isn't on PATH. Caps results at 50 matches
//! and 5 seconds wallclock so a runaway pattern can't starve the loop.

use async_trait::async_trait;
use serde::Deserialize;
use std::time::Duration;
use tokio::process::Command;
use tokio::time::timeout;

use crate::orchestration::agent_loop::tool::{IAgentTool, Observation, ToolError};

const MAX_MATCHES: usize = 50;
const EXEC_TIMEOUT: Duration = Duration::from_secs(5);

pub struct RepoGrepTool {
    repo_root: std::path::PathBuf,
}

impl RepoGrepTool {
    pub fn new(repo_root: impl Into<std::path::PathBuf>) -> Self {
        Self { repo_root: repo_root.into() }
    }

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
    pattern: String,
    /// Optional glob restricting the search. `rg -g <glob>` style.
    #[serde(default)]
    path_glob: Option<String>,
}

#[async_trait]
impl IAgentTool for RepoGrepTool {
    fn name(&self) -> &str { "repo_grep" }

    fn description(&self) -> &str {
        "Search the repo for a regex pattern. Returns up to 50 'path:line: text' matches. \
         Use this to find sibling tests, similar functions, or call sites before drafting. \
         Args: pattern (required, regex), path_glob (optional, restricts to e.g. 'hex-cli/tests/**.rs')."
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "required": ["pattern"],
            "properties": {
                "pattern":   { "type": "string", "description": "Regex pattern to search for" },
                "path_glob": { "type": "string", "description": "Optional glob, e.g. 'hex-cli/**/*.rs'" }
            }
        })
    }

    async fn invoke(&self, args: serde_json::Value) -> Result<Observation, ToolError> {
        let parsed: Args = serde_json::from_value(args)
            .map_err(|e| ToolError::InvalidArgs(format!("expected {{\"pattern\": string, \"path_glob\"?: string}}: {}", e)))?;
        if parsed.pattern.is_empty() {
            return Err(ToolError::InvalidArgs("pattern is empty".into()));
        }

        // Prefer rg if on PATH; fall back to grep.
        let have_rg = which_in_path("rg");
        let mut cmd = if have_rg {
            let mut c = Command::new("rg");
            c.arg("--no-messages")        // suppress permission denieds etc.
             .arg("--max-count").arg(MAX_MATCHES.to_string())
             .arg("--line-number")
             .arg("--no-heading");
            if let Some(ref g) = parsed.path_glob {
                c.arg("--glob").arg(g);
            }
            c.arg("--").arg(&parsed.pattern);
            c
        } else {
            // POSIX grep fallback. -R recursive, -n line-number, -I skip binary.
            // No native glob restrict; honor path_glob by shelling out only
            // inside that subtree if the glob points at a directory.
            let mut c = Command::new("grep");
            c.arg("-RnI").arg("--").arg(&parsed.pattern);
            if let Some(ref g) = parsed.path_glob {
                if !g.contains('*') && !g.contains('?') {
                    c.arg(g);
                }
            }
            c
        };

        cmd.current_dir(&self.repo_root);

        let exec = timeout(EXEC_TIMEOUT, cmd.output()).await;
        let out = match exec {
            Ok(Ok(o)) => o,
            Ok(Err(e)) => return Err(ToolError::Exec(format!("spawn {}: {}",
                if have_rg { "rg" } else { "grep" }, e))),
            Err(_) => return Err(ToolError::Timeout { seconds: EXEC_TIMEOUT.as_secs() as u32 }),
        };

        // grep + rg both return exit 1 when nothing matches — that's NOT
        // an error to bubble; it's a successful empty result. Only exit
        // codes ≥ 2 indicate a real failure (bad regex, IO error, …).
        if let Some(code) = out.status.code() {
            if code >= 2 {
                let stderr = String::from_utf8_lossy(&out.stderr);
                return Err(ToolError::Exec(format!("exit {}: {}", code,
                    stderr.lines().next().unwrap_or("(no stderr)"))));
            }
        }

        let stdout = String::from_utf8_lossy(&out.stdout);
        let total_lines = stdout.lines().count();
        let mut kept: Vec<&str> = stdout.lines().take(MAX_MATCHES).collect();

        if kept.is_empty() {
            return Ok(Observation::ok(format!(
                "(no matches for pattern '{}'{})",
                parsed.pattern,
                parsed.path_glob.as_ref().map(|g| format!(" in {}", g)).unwrap_or_default()
            )));
        }

        let truncated = total_lines > MAX_MATCHES;
        let body = kept.join("\n");
        if truncated {
            let footer = format!("\n[... {} more matches truncated; narrow the pattern or path_glob ...]",
                total_lines - MAX_MATCHES);
            kept.push(&footer); // keep ownership rules happy via shadowing below
            return Ok(Observation::truncated(format!("{}{}", body, footer), stdout.len()));
        }
        Ok(Observation::ok(body))
    }
}

fn which_in_path(bin: &str) -> bool {
    if let Ok(path) = std::env::var("PATH") {
        for dir in path.split(':') {
            let p = std::path::Path::new(dir).join(bin);
            if p.is_file() { return true; }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_repo_with_content() -> (tempfile::TempDir, RepoGrepTool) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        std::fs::create_dir_all(root.join("hex-core/src")).unwrap();
        std::fs::create_dir_all(root.join("hex-cli/src")).unwrap();
        std::fs::write(root.join("hex-core/src/lib.rs"),
            "pub fn alpha() {}\npub fn beta() {}\n").unwrap();
        std::fs::write(root.join("hex-cli/src/main.rs"),
            "pub fn alpha_runner() {}\nfn main() {}\n").unwrap();
        let tool = RepoGrepTool::new(root);
        (dir, tool)
    }

    #[tokio::test]
    async fn finds_pattern_across_files() {
        let (_dir, tool) = tmp_repo_with_content();
        let obs = tool.invoke(serde_json::json!({"pattern": "alpha"})).await.unwrap();
        // Match count varies by tool (rg may produce different output);
        // just assert the keyword appears.
        assert!(obs.output.contains("alpha"));
        assert!(!obs.truncated);
        assert!(obs.terminal.is_none());
    }

    #[tokio::test]
    async fn no_match_returns_empty_observation_not_error() {
        let (_dir, tool) = tmp_repo_with_content();
        let obs = tool.invoke(serde_json::json!({"pattern": "no_such_symbol_anywhere"})).await.unwrap();
        assert!(obs.output.contains("no matches"));
        assert!(!obs.truncated);
    }

    #[tokio::test]
    async fn empty_pattern_is_invalid_args() {
        let (_dir, tool) = tmp_repo_with_content();
        let err = tool.invoke(serde_json::json!({"pattern": ""})).await.unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs(_)));
    }

    #[tokio::test]
    async fn missing_pattern_arg_is_invalid_args() {
        let (_dir, tool) = tmp_repo_with_content();
        let err = tool.invoke(serde_json::json!({})).await.unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs(_)));
    }
}
