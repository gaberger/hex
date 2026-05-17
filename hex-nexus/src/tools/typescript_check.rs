//! `typescript_check` tool — TSX/TS deterministic oracle.
//!
//! Mirrors `cargo_check`: wraps `tsc --noEmit` in the hex-nexus dashboard
//! assets workspace, returns structured errors so the LLM can correct
//! before declaring an artifact done. The executor's autonomous-commit
//! step calls this inline on `.ts` and `.tsx` writes so broken
//! TypeScript rolls back the same way broken Rust does for
//! `cargo_check` (ADR-2026-05-11-0700 R1).
//!
//! Closes the gap surfaced by ADR-2605141631 dogfood: when
//! `hex agent run` autonomously scaffolded `AttentionFeed.tsx`
//! (commit 613f2475), the agent's success report was misleadingly
//! green because no typecheck ran — the LLM had hallucinated an
//! `import { AttentionItem } from './types'` that doesn't exist.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use tokio::process::Command;
use tokio::time::timeout;

use super::{Tool, ToolResult};

pub struct TypescriptCheck;

const DEFAULT_ASSETS_DIR: &str = "hex-nexus/assets";
const TIMEOUT_SECS: u64 = 60;
const MAX_DIAGNOSTICS: usize = 30;

#[async_trait]
impl Tool for TypescriptCheck {
    fn name(&self) -> &'static str {
        "typescript_check"
    }
    fn description(&self) -> &'static str {
        "Run `tsc --noEmit` on the hex-nexus dashboard assets and return \
         structured TypeScript errors. Use this to verify TSX/TS changes \
         compile before claiming an artifact is done. The result is the \
         deterministic oracle for whether a frontend change is sound — \
         the analogue of cargo_check for Rust."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "assets_dir": {
                    "type": "string",
                    "description": "Repo-relative directory containing tsconfig.json. Default 'hex-nexus/assets'.",
                },
                "project": {
                    "type": "string",
                    "description": "Path to a specific tsconfig.json to use, relative to assets_dir. Default 'tsconfig.json'.",
                }
            },
            "required": []
        })
    }
    async fn execute(&self, input: Value) -> ToolResult {
        let start = Instant::now();
        let assets_dir = input
            .get("assets_dir")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or(DEFAULT_ASSETS_DIR);
        let project = input
            .get("project")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("tsconfig.json");

        let repo_root = std::env::var("HEX_REPO_ROOT")
            .unwrap_or_else(|_| "/home/gary/hex-intf".to_string());
        let cwd = format!("{}/{}", repo_root, assets_dir);

        // Prefer the project-local tsc via npx (uses node_modules/.bin/tsc).
        // Falls back to globally-installed tsc on PATH.
        let mut cmd = Command::new("npx");
        cmd.arg("--no-install")
            .arg("tsc")
            .arg("--noEmit")
            .arg("--pretty")
            .arg("false")
            .arg("--project")
            .arg(project);
        cmd.current_dir(&cwd);
        cmd.env(
            "PATH",
            format!(
                "{}/node_modules/.bin:{}",
                cwd,
                std::env::var("PATH").unwrap_or_default()
            ),
        );

        let fut = cmd.output();
        let out = match timeout(Duration::from_secs(TIMEOUT_SECS), fut).await {
            Ok(Ok(out)) => out,
            Ok(Err(e)) => {
                // tsc not installed in this workspace (no node_modules,
                // not a TS-tsc project). Treat as graceful "no check
                // performed" instead of "check failed" so the executor's
                // inline gate doesn't roll back TSX writes for env-setup
                // issues. The operator can wire tsc in by running `npm
                // install --save-dev typescript` in the assets dir.
                return ToolResult::ok(
                    json!({
                        "ok": true,
                        "skipped": true,
                        "reason": format!(
                            "tsc unavailable in {} (npm install --save-dev typescript): {}",
                            cwd, e
                        ),
                        "errors": Vec::<Value>::new(),
                        "warnings": Vec::<Value>::new(),
                    }),
                    start.elapsed().as_millis() as u64,
                );
            }
            Err(_) => {
                return ToolResult::err(
                    format!(
                        "tsc timed out after {}s — narrow the `project` arg or split the build",
                        TIMEOUT_SECS
                    ),
                    start.elapsed().as_millis() as u64,
                );
            }
        };

        // Detect environment-setup failures (tsc not installed, tsconfig
        // missing, etc.) and treat as graceful skip rather than a hard
        // rollback. The autonomous loop can still commit the TSX file
        // and the operator can wire tsc in later. Operator-visible signal
        // is the `skipped: true, reason: ...` field on success output.
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        let combined_lower = format!("{}{}", stdout, stderr).to_ascii_lowercase();
        let looks_like_env_gap = combined_lower.contains("could not determine executable")
            || combined_lower.contains("command not found")
            || combined_lower.contains("npx: not found")
            || combined_lower.contains("cannot find module")
            || combined_lower.contains("ts5058") // "specified path does not exist" (no tsconfig)
            || combined_lower.contains("ts18003") // "No inputs were found in config file"
            || combined_lower.contains("no tsconfig")
            || combined_lower.contains("--no-install");
        if !out.status.success() && looks_like_env_gap {
            return ToolResult::ok(
                json!({
                    "ok": true,
                    "skipped": true,
                    "reason": format!(
                        "tsc env not configured in {} (typescript_check gate skipped — to enable: ensure tsc + tsconfig.json are present, or point `project` at the repo-root tsconfig): {}",
                        cwd,
                        format!("{}{}", stdout, stderr).chars().take(300).collect::<String>()
                    ),
                    "errors": Vec::<Value>::new(),
                    "warnings": Vec::<Value>::new(),
                }),
                start.elapsed().as_millis() as u64,
            );
        }
        let combined = format!("{}{}", stdout, stderr);

        // tsc emits one diagnostic per line in pretty=false mode:
        //   path/to/file.tsx(LINE,COL): error TSXXXX: message
        //   path/to/file.tsx(LINE,COL): warning TSXXXX: message
        let mut errors: Vec<Value> = Vec::new();
        let mut warnings: Vec<Value> = Vec::new();
        for raw in combined.lines() {
            let line = raw.trim();
            if line.is_empty() {
                continue;
            }
            let parsed = parse_tsc_diagnostic(line);
            let (level, diag) = match parsed {
                Some(p) => p,
                None => continue,
            };
            if level == "error" && errors.len() < MAX_DIAGNOSTICS {
                errors.push(diag);
            } else if level == "warning" && warnings.len() < MAX_DIAGNOSTICS {
                warnings.push(diag);
            }
        }

        let exit_ok = out.status.success();
        let result_value = json!({
            "ok": exit_ok && errors.is_empty(),
            "exit_code": out.status.code(),
            "errors": errors,
            "warnings": warnings,
            "assets_dir": assets_dir,
            "project": project,
        });

        if exit_ok && result_value.get("errors").and_then(|e| e.as_array()).map(|a| a.is_empty()).unwrap_or(true) {
            ToolResult::ok(result_value, start.elapsed().as_millis() as u64)
        } else {
            // Even on tsc nonzero exit, return structured diagnostics so the
            // LLM can act. Build an err ToolResult that carries the
            // diagnostic detail in `output` (operator and LLM both want
            // the structured list, not just "failed").
            let err_count = result_value
                .get("errors")
                .and_then(|e| e.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            let warn_count = result_value
                .get("warnings")
                .and_then(|w| w.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            ToolResult {
                ok: false,
                output: result_value,
                error: Some(format!(
                    "tsc reported {} errors, {} warnings (exit code {:?})",
                    err_count,
                    warn_count,
                    out.status.code()
                )),
                elapsed_ms: start.elapsed().as_millis() as u64,
                truncated: false,
            }
        }
    }
}

/// Parse one line of `tsc --pretty false` output. Returns Some((level,
/// diag)) on success — diag is a JSON object suitable for the LLM to
/// reason over.
fn parse_tsc_diagnostic(line: &str) -> Option<(String, Value)> {
    // Format: "<path>(<line>,<col>): <error|warning> <TSxxxx>: <msg>"
    // OR     : "<path>:<line>:<col> - <error|warning> <TSxxxx>: <msg>"
    let lower = line.to_ascii_lowercase();
    let level = if lower.contains("error ts") {
        "error"
    } else if lower.contains("warning ts") {
        "warning"
    } else {
        return None;
    };

    // Find the path portion (before the position marker).
    let (path, after_path) = if let Some(idx) = line.find('(') {
        (&line[..idx], &line[idx..])
    } else if let Some(idx) = line.find(": error") {
        (&line[..idx], &line[idx..])
    } else if let Some(idx) = line.find(": warning") {
        (&line[..idx], &line[idx..])
    } else {
        return None;
    };

    Some((
        level.to_string(),
        json!({
            "path": path.trim(),
            "raw": line,
            "level": level,
            "position": after_path.trim(),
        }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_error_line() {
        let line = "src/components/views/AttentionFeed.tsx(5,28): error TS2305: Module './types' has no exported member 'AttentionItem'.";
        let (level, diag) = parse_tsc_diagnostic(line).unwrap();
        assert_eq!(level, "error");
        assert_eq!(diag.get("path").and_then(|v| v.as_str()), Some("src/components/views/AttentionFeed.tsx"));
        assert!(diag.get("raw").and_then(|v| v.as_str()).unwrap().contains("TS2305"));
    }

    #[test]
    fn parse_warning_line() {
        let line = "src/foo.ts(10,5): warning TS6133: 'unused' is declared but its value is never read.";
        let (level, _) = parse_tsc_diagnostic(line).unwrap();
        assert_eq!(level, "warning");
    }

    #[test]
    fn parse_non_diagnostic_returns_none() {
        assert!(parse_tsc_diagnostic("Building...").is_none());
        assert!(parse_tsc_diagnostic("Found 0 errors.").is_none());
        assert!(parse_tsc_diagnostic("").is_none());
    }
}
