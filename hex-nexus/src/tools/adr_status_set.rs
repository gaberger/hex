//! `adr_status_set` — flip an ADR's status header.
//!
//! Closes the reconcile → ADR-Accepted gap. When `hex plan reconcile`
//! confirms all phases of a workplan-tied-to-ADR are done-with-evidence,
//! the orchestration layer (or a persona) calls this to update the ADR's
//! status line from "Proposed" to "Accepted" (or "Superseded").
//!
//! Reads the ADR file, locates the line `Status: **<old>**`, replaces
//! with `Status: **<new>**`, emits proposed_action(file_write). Twin
//! auto-approves `proposed_by="tool:adr_status_set"`.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::Path;
use std::time::{Duration, Instant};

use super::{Tool, ToolResult};

const STDB_HOST_DEFAULT: &str = "http://127.0.0.1:3033";

pub struct AdrStatusSet;

#[async_trait]
impl Tool for AdrStatusSet {
    fn name(&self) -> &'static str {
        "adr_status_set"
    }
    fn description(&self) -> &'static str {
        "Update an ADR's Status header line. Use Accepted when the \
         ADR's implementation has been verified by `hex plan reconcile` \
         (all workplan phases complete + evidence in code). Use \
         Superseded when a later ADR replaces this one. The ADR file \
         must contain a single line matching `Status: **<word>**`."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "adr_id": {
                    "type": "string",
                    "description": "ADR id (e.g. '2605082600' or 'ADR-2026-05-08-2600'). Resolves docs/adrs/ADR-<id>-*.md"
                },
                "new_status": {
                    "type": "string",
                    "enum": ["Proposed", "Accepted", "Superseded"],
                    "description": "Target status. Capitalised."
                },
                "rationale": {
                    "type": "string",
                    "description": "One-line why — surfaces in the audit trail."
                }
            },
            "required": ["adr_id", "new_status", "rationale"]
        })
    }
    async fn execute(&self, input: Value) -> ToolResult {
        let start = Instant::now();
        let adr_id = match input.get("adr_id").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s.trim_start_matches("ADR-").to_string(),
            _ => return ToolResult::err("missing adr_id", start.elapsed().as_millis() as u64),
        };
        if !adr_id.chars().all(|c| c.is_ascii_digit()) || adr_id.len() < 8 {
            return ToolResult::err("adr_id must be a digits-only timestamp ≥8 chars (e.g. 2605082600)", start.elapsed().as_millis() as u64);
        }
        let new_status = match input.get("new_status").and_then(|v| v.as_str()) {
            Some(s @ "Proposed" | s @ "Accepted" | s @ "Superseded") => s.to_string(),
            _ => return ToolResult::err("new_status must be Proposed|Accepted|Superseded", start.elapsed().as_millis() as u64),
        };
        let rationale = match input.get("rationale").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() && s.len() <= 300 => s.to_string(),
            _ => return ToolResult::err("rationale required, 1-300 chars", start.elapsed().as_millis() as u64),
        };

        // Resolve the ADR file via filesystem glob — accept any slug suffix.
        let repo_root = std::env::var("HEX_REPO_ROOT").unwrap_or_else(|_| "/home/gary/hex-intf".to_string());
        let adrs_dir = Path::new(&repo_root).join("docs/adrs");
        let entries = match std::fs::read_dir(&adrs_dir) {
            Ok(e) => e,
            Err(e) => return ToolResult::err(format!("read_dir docs/adrs: {}", e), start.elapsed().as_millis() as u64),
        };
        let target = entries
            .flatten()
            .find(|entry| {
                entry.file_name().to_str()
                    .map(|n| n.starts_with(&format!("ADR-{}-", adr_id)) && n.ends_with(".md"))
                    .unwrap_or(false)
            });
        let target = match target {
            Some(e) => e.path(),
            None => return ToolResult::err(format!("no ADR file matches ADR-{}-*.md", adr_id), start.elapsed().as_millis() as u64),
        };
        let rel_path = target.strip_prefix(&repo_root)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| target.to_string_lossy().to_string());

        let existing = match std::fs::read_to_string(&target) {
            Ok(s) => s,
            Err(e) => return ToolResult::err(format!("read: {}", e), start.elapsed().as_millis() as u64),
        };

        // Find a line matching `Status: **<word>**` (case-sensitive on Status:).
        let mut new_content = String::with_capacity(existing.len() + 32);
        let mut found = false;
        for line in existing.split_inclusive('\n') {
            let trimmed = line.trim_start();
            if !found && trimmed.starts_with("Status:") {
                let prefix_len = line.len() - trimmed.len();
                let prefix = &line[..prefix_len];
                new_content.push_str(prefix);
                new_content.push_str("Status: **");
                new_content.push_str(&new_status);
                new_content.push_str("**");
                if line.ends_with('\n') {
                    new_content.push('\n');
                }
                found = true;
            } else {
                new_content.push_str(line);
            }
        }
        if !found {
            return ToolResult::err(
                "no `Status:` header line found in ADR — file format unexpected",
                start.elapsed().as_millis() as u64,
            );
        }

        // Emit proposed_action(file_write) — same path adr_draft uses.
        let payload = serde_json::json!({
            "path": rel_path,
            "content": new_content,
        });
        let host = std::env::var("HEX_SPACETIMEDB_HOST").unwrap_or_else(|_| STDB_HOST_DEFAULT.to_string());
        let db = std::env::var("HEX_STDB_DATABASE")
            .unwrap_or_else(|_| hex_core::stdb_database_for_module("hexflo-coordination").to_string());
        let url = format!("{}/v1/database/{}/call/proposed_action_open", host, db);
        let http = match reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
        {
            Ok(c) => c,
            Err(e) => return ToolResult::err(format!("http: {}", e), start.elapsed().as_millis() as u64),
        };
        let body_call = serde_json::json!([
            "file_write",
            payload.to_string(),
            "tool:adr_status_set",
            0u64,
        ]);
        let resp = match http.post(&url).json(&body_call).send().await {
            Ok(r) => r,
            Err(e) => return ToolResult::err(format!("stdb: {}", e), start.elapsed().as_millis() as u64),
        };
        if !resp.status().is_success() {
            let s = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return ToolResult::err(format!("proposed_action_open HTTP {}: {}", s, body), start.elapsed().as_millis() as u64);
        }
        ToolResult::ok(
            json!({
                "ok": true,
                "adr_path": rel_path,
                "new_status": new_status,
                "rationale": rationale,
                "byte_len": new_content.len(),
                "note": "proposed_action queued; twin auto-approves tool:* per ADR-2026-05-08-2500",
            }),
            start.elapsed().as_millis() as u64,
        )
    }
}
