//! `cargo_check` tool — Phase 4 verifier of last resort.
//!
//! Wraps `cargo check --workspace --message-format=json` (or a single
//! crate via `-p`). Returns structured errors + warnings the LLM can
//! reason about. 60s timeout — bigger workspaces should narrow `crate`.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use tokio::process::Command;
use tokio::time::timeout;

use super::{Tool, ToolResult};

pub struct CargoCheck;

#[async_trait]
impl Tool for CargoCheck {
    fn name(&self) -> &'static str {
        "cargo_check"
    }
    fn description(&self) -> &'static str {
        "Run `cargo check` on a hex crate (or the whole workspace) and \
         return structured errors and warnings. Use this to verify code \
         changes compile before claiming an artifact is done. The result \
         is the deterministic oracle for whether a Rust change is sound."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "crate": {
                    "type": "string",
                    "description": "Crate name to check, e.g. 'hex-nexus', 'hex-cli'. Omit or pass empty string to check the whole workspace.",
                },
                "release": {
                    "type": "boolean",
                    "description": "If true, check release profile; default false (faster dev profile).",
                }
            },
            "required": []
        })
    }
    async fn execute(&self, input: Value) -> ToolResult {
        let start = Instant::now();
        let crate_name = input.get("crate").and_then(|v| v.as_str()).unwrap_or("");
        let release = input.get("release").and_then(|v| v.as_bool()).unwrap_or(false);

        let repo_root = std::env::var("HEX_REPO_ROOT")
            .unwrap_or_else(|_| "/home/gary/hex-intf".to_string());

        let mut cmd = Command::new("cargo");
        cmd.arg("check").arg("--message-format=json");
        if !crate_name.is_empty() {
            cmd.arg("-p").arg(crate_name);
        }
        if release {
            cmd.arg("--release");
        }
        cmd.current_dir(&repo_root);
        cmd.env("PATH", format!("{}/.cargo/bin:{}", std::env::var("HOME").unwrap_or_default(), std::env::var("PATH").unwrap_or_default()));
        cmd.env("HEX_HUB_BUILD_HASH", "tool-cargo-check");

        let fut = cmd.output();
        let out = match timeout(Duration::from_secs(60), fut).await {
            Ok(Ok(out)) => out,
            Ok(Err(e)) => {
                return ToolResult::err(
                    format!("cargo spawn failed: {}", e),
                    start.elapsed().as_millis() as u64,
                );
            }
            Err(_) => {
                return ToolResult::err(
                    "cargo check timed out after 60s — narrow the `crate` arg",
                    start.elapsed().as_millis() as u64,
                );
            }
        };

        let stdout = String::from_utf8_lossy(&out.stdout);
        let mut errors: Vec<Value> = Vec::new();
        let mut warnings: Vec<Value> = Vec::new();

        for line in stdout.lines() {
            let v: Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if v.get("reason").and_then(|x| x.as_str()) != Some("compiler-message") {
                continue;
            }
            let msg = match v.get("message") {
                Some(m) => m,
                None => continue,
            };
            let level = msg.get("level").and_then(|x| x.as_str()).unwrap_or("");
            let rendered = msg
                .get("rendered")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            let primary_span = msg
                .get("spans")
                .and_then(|s| s.as_array())
                .and_then(|arr| arr.iter().find(|s| s.get("is_primary").and_then(|x| x.as_bool()) == Some(true)))
                .cloned()
                .unwrap_or(Value::Null);
            let entry = json!({
                "level": level,
                "rendered": rendered.chars().take(800).collect::<String>(),
                "file": primary_span.get("file_name").and_then(|x| x.as_str()).unwrap_or(""),
                "line": primary_span.get("line_start").and_then(|x| x.as_u64()).unwrap_or(0),
            });
            match level {
                "error" | "error: internal compiler error" => errors.push(entry),
                "warning" => warnings.push(entry),
                _ => {}
            }
        }

        let truncated = errors.len() > 30 || warnings.len() > 30;
        if errors.len() > 30 {
            errors.truncate(30);
        }
        if warnings.len() > 30 {
            warnings.truncate(30);
        }

        let elapsed = start.elapsed().as_millis() as u64;
        let ok = out.status.success();
        let result = json!({
            "ok": ok,
            "exit_code": out.status.code(),
            "errors": errors,
            "warnings": warnings,
            "stderr_tail": String::from_utf8_lossy(&out.stderr).chars().rev().take(800).collect::<String>().chars().rev().collect::<String>(),
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
    fn schema_is_object() {
        let s = CargoCheck.input_schema();
        assert_eq!(s.get("type").and_then(|v| v.as_str()), Some("object"));
        assert!(s.get("properties").is_some());
    }
}
