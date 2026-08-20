//! `spec_draft` — Wave 2 anchor tool for product specs.
//!
//! Mirrors `adr_draft` but writes to `docs/specs/<slug>.md` instead of
//! `docs/adrs/ADR-<id>-<slug>.md`. CPO escalated tonight specifically
//! because there was no tool for non-ADR documentation; this closes
//! that gap.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

use super::{Tool, ToolResult};

const STDB_HOST_DEFAULT: &str = "http://127.0.0.1:3033";
const MIN_BODY: usize = 200;
// CTO ADR-2026-05-08-2600 — halved from 50_000 to 24_000 (BSATN crash mitigation).
const MAX_BODY: usize = 24_000;

pub struct SpecDraft;

#[async_trait]
impl Tool for SpecDraft {
    fn name(&self) -> &'static str {
        "spec_draft"
    }
    fn description(&self) -> &'static str {
        "Draft a product / behavioural / design spec under docs/specs/. \
         Use this for non-ADR documentation: user-facing specs, UX \
         descriptions, behavioural scenarios, design notes. Writes a \
         proposed_action(kind=file_write) row; the digital-twin \
         executor materialises the file at docs/specs/<slug>.md after \
         approval. Use adr_draft instead for architecture decisions."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "slug": {
                    "type": "string",
                    "description": "kebab-case file slug (e.g. 'mission-control-ux'). Becomes docs/specs/<slug>.md.",
                },
                "title": {
                    "type": "string",
                    "description": "Human-readable title shown as the H1.",
                },
                "status": {
                    "type": "string",
                    "enum": ["draft", "proposed", "accepted"],
                    "description": "Spec lifecycle. 'draft' for new content; 'proposed' when ready for operator review.",
                },
                "body": {
                    "type": "string",
                    "description": "Full spec markdown body. Should describe user flow, observable artifacts, success criteria, and (where relevant) which source files implement it. 200-50000 chars.",
                }
            },
            "required": ["slug", "title", "status", "body"]
        })
    }
    async fn execute(&self, input: Value) -> ToolResult {
        let start = Instant::now();
        let slug_raw = match input.get("slug").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => return ToolResult::err("missing/empty slug", start.elapsed().as_millis() as u64),
        };
        let slug = sanitise_slug(&slug_raw);
        if slug.is_empty() || slug.len() > 80 {
            return ToolResult::err(
                "slug after sanitisation must be 1-80 chars; use kebab-case",
                start.elapsed().as_millis() as u64,
            );
        }
        let title = match input.get("title").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() && s.len() <= 120 => s.to_string(),
            _ => return ToolResult::err("missing/invalid title (1-120 chars)", start.elapsed().as_millis() as u64),
        };
        let status = match input.get("status").and_then(|v| v.as_str()) {
            Some(s @ "draft" | s @ "proposed" | s @ "accepted") => s.to_string(),
            _ => return ToolResult::err("status must be draft|proposed|accepted", start.elapsed().as_millis() as u64),
        };
        let body = match input.get("body").and_then(|v| v.as_str()) {
            Some(s) if (MIN_BODY..=MAX_BODY).contains(&s.len()) => s.to_string(),
            Some(s) => return ToolResult::err(
                format!("body length {} outside [{}, {}]", s.len(), MIN_BODY, MAX_BODY),
                start.elapsed().as_millis() as u64,
            ),
            None => return ToolResult::err("missing body", start.elapsed().as_millis() as u64),
        };

        let target_path = format!("docs/specs/{}.md", slug);
        let full_body = format!(
            "# {}\n\n*status*: {}  ·  *date*: {}\n\n{}",
            title,
            status,
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

        let body_call = serde_json::json!([
            "file_write",
            payload.to_string(),
            "tool:spec_draft",
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
                "note": "proposed_action queued; digital-twin executor (auto-approved as tool:* via ADR-2026-05-08-2500 fix) will write the file shortly",
            }),
            elapsed,
        )
    }
}

fn sanitise_slug(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn slugifies() {
        assert_eq!(sanitise_slug("Mission Control UX!"), "mission-control-ux");
        assert_eq!(sanitise_slug("docs/specs/foo bar"), "docs-specs-foo-bar");
    }
    #[test]
    fn schema_requires_all() {
        let s = SpecDraft.input_schema();
        let req: Vec<String> = s
            .get("required").unwrap().as_array().unwrap()
            .iter().map(|v| v.as_str().unwrap().to_string()).collect();
        for r in ["slug", "title", "status", "body"] {
            assert!(req.contains(&r.to_string()), "missing required: {}", r);
        }
    }
}
