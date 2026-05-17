//! `adr_draft` — emit a proposed_action(file_write) for a new ADR.
//!
//! The persona's primary "produce an artifact" tool. Validates the body
//! contains the required ADR sections; the resulting proposed_action
//! row flows through the existing digital-twin executor which writes the
//! file under `docs/adrs/<id>-<slug>.md`.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

use super::{Tool, ToolResult};

const STDB_HOST_DEFAULT: &str = "http://127.0.0.1:3033";
const MIN_BODY: usize = 200;
// CTO ADR-2026-05-08-2600 — halved from 50_000 to 24_000 (BSATN crash mitigation).
const MAX_BODY: usize = 24_000;

pub struct AdrDraft;

#[async_trait]
impl Tool for AdrDraft {
    fn name(&self) -> &'static str {
        "adr_draft"
    }
    fn description(&self) -> &'static str {
        "Draft a new Architecture Decision Record. Writes a \
         proposed_action(kind=file_write) row to STDB; the digital-twin \
         executor materialises the file at docs/adrs/<id>-<slug>.md after \
         the twin's approval. Body MUST include ## Context, ## Decision, \
         and ## Consequences sections."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "ADR id, timestamp form (e.g. '2605082600'). Must be unique.",
                },
                "title": {
                    "type": "string",
                    "description": "ADR title, kebab-case-friendly, max 80 chars.",
                },
                "status": {
                    "type": "string",
                    "enum": ["proposed", "accepted", "superseded"],
                    "description": "ADR status. Use 'proposed' for new drafts pending operator review.",
                },
                "body": {
                    "type": "string",
                    "description": "Full ADR markdown body. Must include `## Context`, `## Decision`, and `## Consequences` sections. 200-50000 bytes.",
                }
            },
            "required": ["id", "title", "status", "body"]
        })
    }
    async fn execute(&self, input: Value) -> ToolResult {
        let start = Instant::now();
        let id = match input.get("id").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => return ToolResult::err("missing/empty id", start.elapsed().as_millis() as u64),
        };
        let title = match input.get("title").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() && s.len() <= 80 => s.to_string(),
            _ => return ToolResult::err("missing/invalid title (1-80 chars)", start.elapsed().as_millis() as u64),
        };
        let status = match input.get("status").and_then(|v| v.as_str()) {
            Some(s @ "proposed" | s @ "accepted" | s @ "superseded") => s.to_string(),
            _ => return ToolResult::err("status must be proposed|accepted|superseded", start.elapsed().as_millis() as u64),
        };
        let body = match input.get("body").and_then(|v| v.as_str()) {
            Some(s) if (MIN_BODY..=MAX_BODY).contains(&s.len()) => s.to_string(),
            Some(s) => return ToolResult::err(
                format!("body length {} outside [{}, {}]", s.len(), MIN_BODY, MAX_BODY),
                start.elapsed().as_millis() as u64,
            ),
            None => return ToolResult::err("missing body", start.elapsed().as_millis() as u64),
        };
        // Schema check: required sections.
        for section in ["## Context", "## Decision", "## Consequences"] {
            if !body.contains(section) {
                return ToolResult::err(
                    format!("body missing required section `{}`", section),
                    start.elapsed().as_millis() as u64,
                );
            }
        }
        // ID format
        if !id.chars().all(|c| c.is_ascii_digit()) || id.len() < 10 {
            return ToolResult::err(
                "id must be a 10+ digit timestamp (e.g. 2605082600)",
                start.elapsed().as_millis() as u64,
            );
        }

        let slug = slugify(&title);
        let target_path = format!("docs/adrs/ADR-{}-{}.md", id, slug);
        let full_body = format!(
            "# ADR-{} — {}\n\nStatus: **{}**\nDate: {}\n\n{}",
            id,
            title,
            initial_caps(&status),
            chrono::Utc::now().format("%Y-%m-%d"),
            body.trim_start_matches("# ").trim_start(),
        );

        let payload = serde_json::json!({
            "path": target_path,
            "content": full_body,
        });

        let host = std::env::var("HEX_SPACETIMEDB_HOST")
            .unwrap_or_else(|_| STDB_HOST_DEFAULT.to_string());
        let db = std::env::var("HEX_STDB_DATABASE")
            .unwrap_or_else(|_| hex_core::stdb_database_for_module("hexflo-coordination").to_string());
        let url = format!("{}/v1/database/{}/call/proposed_action_open", host, db);

        let http = match reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
        {
            Ok(c) => c,
            Err(e) => return ToolResult::err(format!("http build: {}", e), start.elapsed().as_millis() as u64),
        };

        // related_commitment_id = 0 (tool-driven, no commitment row)
        let body_call = serde_json::json!([
            "file_write",
            payload.to_string(),
            "tool:adr_draft",
            0u64,
        ]);
        let resp = match http.post(&url).json(&body_call).send().await {
            Ok(r) => r,
            Err(e) => return ToolResult::err(format!("stdb call: {}", e), start.elapsed().as_millis() as u64),
        };
        if !resp.status().is_success() {
            let status_code = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return ToolResult::err(
                format!("proposed_action_open HTTP {}: {}", status_code, body),
                start.elapsed().as_millis() as u64,
            );
        }

        let elapsed = start.elapsed().as_millis() as u64;
        ToolResult::ok(
            json!({
                "ok": true,
                "target_path": target_path,
                "body_bytes": full_body.len(),
                "note": "proposed_action queued; digital-twin executor will write the file after approval",
            }),
            elapsed,
        )
    }
}

fn slugify(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

fn initial_caps(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().chain(chars).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn slugifies() {
        assert_eq!(slugify("Typed Tool Library + SOP Execution"), "typed-tool-library-sop-execution");
    }
    #[test]
    fn schema_requires_all() {
        let s = AdrDraft.input_schema();
        let req: Vec<String> = s
            .get("required").unwrap().as_array().unwrap()
            .iter().map(|v| v.as_str().unwrap().to_string()).collect();
        for r in ["id", "title", "status", "body"] {
            assert!(req.contains(&r.to_string()), "missing required: {}", r);
        }
    }
}
