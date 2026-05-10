//! `dep_audit` tool — cargo audit JSON parser for vulnerability findings.
//!
//! Wraps `cargo audit --json` and filters vulnerabilities by severity and
//! crate name. Returns advisory details (id, title, severity, package, url).
//! 60s timeout, caps at 100 matches and 16KB output.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use tokio::process::Command;
use tokio::time::timeout;

use super::{Tool, ToolResult};

pub struct DepAudit;

#[async_trait]
impl Tool for DepAudit {
    fn name(&self) -> &'static str {
        "dep_audit"
    }
    fn description(&self) -> &'static str {
        "Run `cargo audit --json` and parse vulnerability findings. \
         Filters by minimum severity (low/medium/high/critical) and \
         optional crate substring. Returns advisory id, title, severity, \
         package name/version, and URL. Cap 100 matches, 16KB output, 60s timeout."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "severity_min": {
                    "type": "string",
                    "description": "Minimum severity: low, medium, high, or critical. Default: medium.",
                    "enum": ["low", "medium", "high", "critical"]
                },
                "crate_filter": {
                    "type": "string",
                    "description": "Optional substring filter for package.name. Case-insensitive.",
                }
            },
            "required": []
        })
    }
    async fn execute(&self, input: Value) -> ToolResult {
        let start = Instant::now();
        let severity_min = input
            .get("severity_min")
            .and_then(|v| v.as_str())
            .unwrap_or("medium");
        let crate_filter = input
            .get("crate_filter")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let repo_root = std::env::var("HEX_REPO_ROOT")
            .unwrap_or_else(|_| "/home/operator/hex-intf".to_string());

        let mut cmd = Command::new("cargo");
        cmd.arg("audit").arg("--json");
        cmd.current_dir(&repo_root);
        cmd.env(
            "PATH",
            format!(
                "{}/.cargo/bin:{}",
                std::env::var("HOME").unwrap_or_default(),
                std::env::var("PATH").unwrap_or_default()
            ),
        );

        let fut = cmd.output();
        let out = match timeout(Duration::from_secs(60), fut).await {
            Ok(Ok(out)) => out,
            Ok(Err(e)) => {
                return ToolResult::err(
                    format!("cargo audit spawn failed: {}", e),
                    start.elapsed().as_millis() as u64,
                );
            }
            Err(_) => {
                return ToolResult::err(
                    "cargo audit timed out after 60s",
                    start.elapsed().as_millis() as u64,
                );
            }
        };

        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);

        // Parse JSON output
        let parsed: Value = match serde_json::from_str(&stdout) {
            Ok(v) => v,
            Err(e) => {
                return ToolResult::err(
                    format!("cargo audit JSON parse failed: {} | stderr: {}", e, stderr),
                    start.elapsed().as_millis() as u64,
                );
            }
        };

        // Extract vulnerabilities from database.advisories or vulnerabilities.list
        let advisories = parsed
            .pointer("/database/advisories")
            .or_else(|| parsed.pointer("/vulnerabilities/list"))
            .and_then(|v| v.as_object())
            .map(|obj| obj.values().collect::<Vec<_>>())
            .unwrap_or_default();

        let severity_rank = |s: &str| match s {
            "critical" => 4,
            "high" => 3,
            "medium" => 2,
            "low" => 1,
            _ => 0,
        };
        let min_rank = severity_rank(severity_min);

        let mut findings: Vec<Value> = Vec::new();
        let mut severity_counts = json!({
            "critical": 0,
            "high": 0,
            "medium": 0,
            "low": 0,
        });

        for adv in advisories {
            let severity = adv
                .pointer("/severity")
                .or_else(|| adv.pointer("/cvss/severity"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_lowercase();

            if severity_rank(&severity) < min_rank {
                continue;
            }

            let package_name = adv
                .pointer("/package")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if !crate_filter.is_empty()
                && !package_name.to_lowercase().contains(&crate_filter.to_lowercase())
            {
                continue;
            }

            let advisory_id = adv.pointer("/id").and_then(|v| v.as_str()).unwrap_or("");
            let title = adv
                .pointer("/title")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let version = adv
                .pointer("/versions/patched")
                .or_else(|| adv.pointer("/affected"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let url = adv.pointer("/url").and_then(|v| v.as_str()).unwrap_or("");

            findings.push(json!({
                "advisory_id": advisory_id,
                "title": title.chars().take(200).collect::<String>(),
                "severity": severity,
                "package": {
                    "name": package_name,
                    "version": version,
                },
                "url": url,
            }));

            // Update severity counts
            match severity.as_str() {
                "critical" => {
                    if let Some(c) = severity_counts.get_mut("critical") {
                        *c = json!(c.as_u64().unwrap_or(0) + 1);
                    }
                }
                "high" => {
                    if let Some(c) = severity_counts.get_mut("high") {
                        *c = json!(c.as_u64().unwrap_or(0) + 1);
                    }
                }
                "medium" => {
                    if let Some(c) = severity_counts.get_mut("medium") {
                        *c = json!(c.as_u64().unwrap_or(0) + 1);
                    }
                }
                "low" => {
                    if let Some(c) = severity_counts.get_mut("low") {
                        *c = json!(c.as_u64().unwrap_or(0) + 1);
                    }
                }
                _ => {}
            }

            if findings.len() >= 100 {
                break;
            }
        }

        let total = findings.len();
        let truncated = total >= 100;
        let elapsed = start.elapsed().as_millis() as u64;

        let result = json!({
            "total": total,
            "findings": findings,
            "severity_counts": severity_counts,
            "filters": {
                "severity_min": severity_min,
                "crate_filter": crate_filter,
            },
        });

        let result_str = serde_json::to_string(&result).unwrap_or_default();
        if result_str.len() > 16384 || truncated {
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
        let s = DepAudit.input_schema();
        assert_eq!(s.get("type").and_then(|v| v.as_str()), Some("object"));
        assert!(s.get("properties").is_some());
    }
}
