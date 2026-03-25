//! CliRunner — subprocess wrapper for hex CLI commands (ADR-2603241126).
//!
//! The TUI pipeline uses this to execute hex commands instead of making
//! direct REST calls to nexus, ensuring CLI = MCP = TUI (one code path).

use std::process::Command;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde_json::Value;
use tracing::debug;

/// Subprocess wrapper that invokes the `hex` binary and optionally parses JSON output.
pub struct CliRunner {
    hex_bin: String,
    #[allow(dead_code)] // reserved for future subprocess timeout support
    timeout: Duration,
}

impl CliRunner {
    /// Create a new `CliRunner`. Locates the hex binary via:
    /// 1. `std::env::current_exe()` (same binary we are running as)
    /// 2. `HEX_BIN` env var
    /// 3. `"hex"` on PATH
    pub fn new() -> Self {
        let hex_bin = std::env::current_exe()
            .ok()
            .and_then(|p| p.to_str().map(String::from))
            .or_else(|| std::env::var("HEX_BIN").ok())
            .unwrap_or_else(|| "hex".to_string());

        Self {
            hex_bin,
            timeout: Duration::from_secs(60),
        }
    }

    /// Execute a hex command with `--json` flag and parse the JSON output.
    /// Returns the parsed JSON value on success.
    /// On failure, returns an error with stderr content.
    pub fn run(&self, args: &[&str]) -> Result<Value> {
        let mut full_args: Vec<&str> = args.to_vec();
        full_args.push("--json");

        debug!(cmd = %format!("hex {}", full_args.join(" ")), "cli_runner");

        let output = Command::new(&self.hex_bin)
            .args(&full_args)
            .output()
            .with_context(|| format!("failed to execute: {} {}", self.hex_bin, full_args.join(" ")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!(
                "hex {} exited with {}: {}",
                args.join(" "),
                output.status,
                stderr.trim()
            );
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        serde_json::from_str(stdout.trim())
            .with_context(|| format!("failed to parse JSON from `hex {}`", args.join(" ")))
    }

    /// Execute without parsing (for commands that don't return JSON).
    /// Returns `(stdout, stderr)` on success.
    pub fn run_raw(&self, args: &[&str]) -> Result<(String, String)> {
        debug!(cmd = %format!("hex {}", args.join(" ")), "cli_runner");

        let output = Command::new(&self.hex_bin)
            .args(args)
            .output()
            .with_context(|| format!("failed to execute: {} {}", self.hex_bin, args.join(" ")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!(
                "hex {} exited with {}: {}",
                args.join(" "),
                output.status,
                stderr.trim()
            );
        }

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        Ok((stdout, stderr))
    }

    // ── Convenience methods ──────────────────────────────────────────

    /// `hex task create <swarm_id> <title> [--agent <id>] --json`
    pub fn task_create(&self, swarm_id: &str, title: &str, agent_id: Option<&str>) -> Result<Value> {
        match agent_id {
            Some(aid) => self.run(&["task", "create", swarm_id, title, "--agent", aid]),
            None => self.run(&["task", "create", swarm_id, title]),
        }
    }

    /// `hex task complete <id> [result] --json`
    pub fn task_complete(&self, task_id: &str, result: Option<&str>) -> Result<Value> {
        match result {
            Some(r) => self.run(&["task", "complete", task_id, r]),
            None => self.run(&["task", "complete", task_id]),
        }
    }

    /// `hex analyze <path> --json`
    pub fn analyze(&self, path: &str) -> Result<Value> {
        self.run(&["analyze", path])
    }

    /// `hex swarm init <name> --topology <topology> --json`
    pub fn swarm_init(&self, name: &str, topology: &str) -> Result<Value> {
        self.run(&["swarm", "init", name, "--topology", topology])
    }

    /// `hex swarm cleanup --apply --stale-hours 0` — close all active swarms before creating a new one.
    /// stale-hours=0 treats every active swarm as stale regardless of age.
    pub fn swarm_cleanup(&self) -> Result<Value> {
        self.run(&["swarm", "cleanup", "--apply", "--stale-hours", "0"])
    }

    /// `hex swarm complete <id>` — mark a swarm as completed.
    pub fn swarm_complete(&self, swarm_id: &str) -> Result<Value> {
        self.run(&["swarm", "complete", swarm_id])
    }

    /// `hex swarm list` — return all swarms as JSON array.
    pub fn swarm_list(&self) -> Result<Value> {
        self.run(&["swarm", "list"])
    }

    /// `hex task assign <task_id> <agent_id>`
    /// Uses run_raw because `hex task assign` has no --json flag.
    pub fn task_assign(&self, task_id: &str, agent_id: &str) -> Result<()> {
        self.run_raw(&["task", "assign", task_id, agent_id]).map(|_| ())
    }
}

impl Default for CliRunner {
    fn default() -> Self {
        Self::new()
    }
}
