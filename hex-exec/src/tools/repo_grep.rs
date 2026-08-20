//! `repo_grep` — wraps ripgrep for the LLM grounding phase.
//!
//! Used in Phase GROUND to surface concrete repo facts before the LLM
//! reasons. Returns file:line:content matches, capped, with a `truncated`
//! flag so the model knows to narrow its query if needed.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use tokio::process::Command;
use tokio::time::timeout;

use super::{Tool, ToolResult};

const MAX_MATCHES_DEFAULT: usize = 50;
const MAX_MATCHES_HARD_CAP: usize = 200;

pub struct RepoGrep;

#[async_trait]
impl Tool for RepoGrep {
    fn name(&self) -> &'static str {
        "repo_grep"
    }
    fn description(&self) -> &'static str {
        "Search the hex repository for a regex pattern. Returns matching \
         file:line snippets. Use this in the GROUND phase to find concrete \
         repo facts before reasoning. Prefer narrow patterns + a glob to \
         keep results focused. Returns at most 50 matches by default."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Regex pattern (ripgrep syntax). Required.",
                },
                "glob": {
                    "type": "string",
                    "description": "Optional glob to restrict files, e.g. '*.rs', 'docs/adrs/*.md', 'hex-nexus/src/orchestration/*'.",
                },
                "max_matches": {
                    "type": "integer",
                    "description": "Max number of matches to return. Default 50, cap 200.",
                }
            },
            "required": ["pattern"]
        })
    }
    async fn execute(&self, input: Value) -> ToolResult {
        let start = Instant::now();
        let pattern = match input.get("pattern").and_then(|v| v.as_str()) {
            Some(p) if !p.is_empty() => p.to_string(),
            _ => return ToolResult::err("missing or empty `pattern`", start.elapsed().as_millis() as u64),
        };
        let glob = input.get("glob").and_then(|v| v.as_str()).map(|s| s.to_string());
        let max_matches = input
            .get("max_matches")
            .and_then(|v| v.as_u64())
            .unwrap_or(MAX_MATCHES_DEFAULT as u64) as usize;
        let max_matches = max_matches.min(MAX_MATCHES_HARD_CAP);

        let repo_root = std::env::var("HEX_REPO_ROOT")
            .unwrap_or_else(|_| "/home/gary/hex-intf".to_string());

        let mut cmd = Command::new("rg");
        cmd.arg("--max-count").arg(format!("{}", max_matches))
            .arg("--line-number")
            .arg("--no-heading")
            .arg("--color=never")
            .arg("--max-filesize").arg("512K")
            .arg("--threads").arg("4");
        if let Some(g) = &glob {
            cmd.arg("--glob").arg(g);
        }
        cmd.arg(&pattern).arg(".").current_dir(&repo_root);

        let fut = cmd.output();
        let out = match timeout(Duration::from_secs(5), fut).await {
            Ok(Ok(out)) => out,
            Ok(Err(e)) => {
                return ToolResult::err(
                    format!("rg spawn failed: {}", e),
                    start.elapsed().as_millis() as u64,
                );
            }
            Err(_) => {
                return ToolResult::err(
                    "repo_grep timed out after 5s — narrow `pattern` or add `glob`",
                    start.elapsed().as_millis() as u64,
                );
            }
        };

        // ripgrep exit codes: 0 = found, 1 = no match, 2 = error.
        let status = out.status.code().unwrap_or(-1);
        if status == 2 {
            return ToolResult::err(
                format!("rg error: {}", String::from_utf8_lossy(&out.stderr).chars().take(200).collect::<String>()),
                start.elapsed().as_millis() as u64,
            );
        }

        let stdout = String::from_utf8_lossy(&out.stdout);
        let mut matches: Vec<Value> = Vec::new();
        let mut truncated = false;
        for line in stdout.lines() {
            if matches.len() >= max_matches {
                truncated = true;
                break;
            }
            // Format: <path>:<line>:<content>
            let mut parts = line.splitn(3, ':');
            let (Some(path), Some(linenum), Some(content)) =
                (parts.next(), parts.next(), parts.next())
            else {
                continue;
            };
            let line_num: u64 = linenum.parse().unwrap_or(0);
            matches.push(json!({
                "path": path,
                "line": line_num,
                "content": content.chars().take(200).collect::<String>(),
            }));
        }

        let elapsed = start.elapsed().as_millis() as u64;
        let result = json!({
            "matches": matches,
            "total_matches": matches.len(),
            "truncated": truncated,
            "pattern": pattern,
            "glob": glob.unwrap_or_default(),
        });
        if truncated {
            ToolResult::ok_truncated(result, elapsed)
        } else {
            ToolResult::ok(result, elapsed)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn schema_requires_pattern() {
        let s = RepoGrep.input_schema();
        let req = s.get("required").and_then(|v| v.as_array()).unwrap();
        assert!(req.iter().any(|v| v.as_str() == Some("pattern")));
    }
}
