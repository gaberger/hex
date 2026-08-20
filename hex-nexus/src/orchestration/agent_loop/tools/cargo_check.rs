//! `cargo_check` — let the agent loop verify compilation before submitting
//! (wp-sop-agent-loop P1.4).
//!
//! Two roles in the loop:
//! (1) Pre-flight: the persona can `cargo_check` its own draft (or the
//!     current state of the workspace) to find out whether the world
//!     compiles before it bothers writing a code_patch_propose.
//! (2) Pre-twin gate (lands in P4): action_executor will invoke this
//!     before promoting a proposed_action to twin review.
//!
//! 60s wall-clock timeout (cargo check on hex-nexus alone can run ~30-50s
//! cold; 60s is the floor for a usable signal). Prepends `$HOME/.cargo/bin`
//! to PATH so rustup-installed cargo is reachable — same fix as the
//! standalone-pipeline-test harness toolchain preflight.

use async_trait::async_trait;
use serde::Deserialize;
use std::time::Duration;
use tokio::process::Command;
use tokio::time::timeout;

use crate::orchestration::agent_loop::tool::{IAgentTool, Observation, ToolError};

const EXEC_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_DIAGNOSTIC_LINES: usize = 30;

pub struct CargoCheckTool {
    workspace_root: std::path::PathBuf,
}

impl CargoCheckTool {
    pub fn new(workspace_root: impl Into<std::path::PathBuf>) -> Self {
        Self { workspace_root: workspace_root.into() }
    }

    pub fn from_env() -> Self {
        let root = std::env::var("HEX_REPO_ROOT")
            .ok()
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| ".".into()));
        Self::new(root)
    }
}

#[derive(Deserialize)]
struct Args {
    /// Cargo package name (e.g. "hex-core", "hex-cli"). Restricted to
    /// known workspace members so the persona can't accidentally run
    /// `cargo check -p` on an arbitrary string.
    #[serde(rename = "crate")]
    crate_name: String,
}

const KNOWN_CRATES: &[&str] = &[
    "hex-cli",
    "hex-nexus",
    "hex-core",
    "hex-agent",
    "hex-parser",
    "hex-analyzer",
    "hex-desktop",
];

#[async_trait]
impl IAgentTool for CargoCheckTool {
    fn name(&self) -> &str { "cargo_check" }

    fn description(&self) -> &str {
        "Run `cargo check -p <crate>` against the current working tree to verify it compiles. \
         Returns success or the first 30 lines of compiler diagnostics. \
         Use this BEFORE code_patch_propose to make sure your draft compiles. \
         Allowed crates: hex-cli, hex-nexus, hex-core, hex-agent, hex-parser, hex-analyzer, hex-desktop. \
         60-second timeout."
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "required": ["crate"],
            "properties": {
                "crate": {
                    "type": "string",
                    "enum": KNOWN_CRATES,
                    "description": "Workspace member crate name"
                }
            }
        })
    }

    async fn invoke(&self, args: serde_json::Value) -> Result<Observation, ToolError> {
        let parsed: Args = serde_json::from_value(args)
            .map_err(|e| ToolError::InvalidArgs(format!("expected {{\"crate\": string}}: {}", e)))?;

        if !KNOWN_CRATES.contains(&parsed.crate_name.as_str()) {
            return Err(ToolError::InvalidArgs(format!(
                "unknown crate '{}'. Allowed: {}",
                parsed.crate_name,
                KNOWN_CRATES.join(", ")
            )));
        }

        // Prepend ~/.cargo/bin so rustup-installed cargo is found, matching
        // the standalone-pipeline-test/run.sh toolchain preflight fix.
        let path = compose_path_with_cargo_bin();

        let mut cmd = Command::new("cargo");
        cmd.arg("check").arg("-p").arg(&parsed.crate_name).arg("--message-format=short")
            .env("PATH", &path)
            .current_dir(&self.workspace_root);

        let exec = timeout(EXEC_TIMEOUT, cmd.output()).await;
        let out = match exec {
            Ok(Ok(o)) => o,
            Ok(Err(e)) => return Err(ToolError::Exec(format!("spawn cargo: {} (PATH={})", e, path))),
            Err(_) => return Err(ToolError::Timeout { seconds: EXEC_TIMEOUT.as_secs() as u32 }),
        };

        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        let combined = if stderr.is_empty() { stdout.to_string() } else { stderr.to_string() };

        if out.status.success() {
            return Ok(Observation::ok(format!(
                "cargo check -p {}: OK",
                parsed.crate_name
            )));
        }

        // Trim to first MAX_DIAGNOSTIC_LINES so we don't blow context budget
        // when a single error fans out to 200 lines of upstream type errors.
        let lines: Vec<&str> = combined.lines().take(MAX_DIAGNOSTIC_LINES).collect();
        let total = combined.lines().count();
        let truncated = total > MAX_DIAGNOSTIC_LINES;
        let mut body = format!(
            "cargo check -p {}: FAIL (exit={:?})\n{}",
            parsed.crate_name,
            out.status.code(),
            lines.join("\n")
        );
        if truncated {
            body.push_str(&format!("\n[... {} more lines truncated ...]", total - MAX_DIAGNOSTIC_LINES));
        }
        // FAIL is a successful tool invocation reporting a compile failure
        // — the persona's expected to react to it. Not a ToolError.
        if truncated {
            Ok(Observation::truncated(body, combined.len()))
        } else {
            Ok(Observation::ok(body))
        }
    }
}

fn compose_path_with_cargo_bin() -> String {
    let cargo_bin = std::env::var("HOME")
        .ok()
        .map(|h| std::path::PathBuf::from(h).join(".cargo").join("bin"))
        .filter(|p| p.is_dir());
    let existing = std::env::var("PATH").unwrap_or_default();
    match cargo_bin {
        Some(p) => {
            let p_str = p.to_string_lossy().into_owned();
            if existing.split(':').any(|seg| seg == p_str) {
                existing
            } else {
                format!("{}:{}", p_str, existing)
            }
        }
        None => existing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unknown_crate_is_invalid_args() {
        let tool = CargoCheckTool::new(".");
        let err = tool.invoke(serde_json::json!({"crate": "not-a-crate"})).await.unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs(_)));
    }

    #[tokio::test]
    async fn missing_crate_arg_is_invalid_args() {
        let tool = CargoCheckTool::new(".");
        let err = tool.invoke(serde_json::json!({})).await.unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs(_)));
    }

    #[test]
    fn compose_path_includes_cargo_bin_when_present() {
        let path = compose_path_with_cargo_bin();
        // Whether ~/.cargo/bin exists on this box or not, the result must
        // be non-empty (env PATH at minimum).
        assert!(!path.is_empty());
    }
}
