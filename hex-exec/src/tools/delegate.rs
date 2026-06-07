//! `delegate` — fan-out work to another lean-fleet persona.
//!
//! Closes the subagent gap surfaced 2026-05-28 ebay-mvp scaling test:
//! - The classifier `route` decision exists but its STDB reducer returned
//!   404 for peer-aware route attempts, so personas couldn't actually hand
//!   off work even when their prompt told them to.
//! - `escalate_to_operator` works but routes to the human, not another
//!   persona — wrong granularity for "hex-coder finished its part, now
//!   hex-tester should write the integration test."
//!
//! `delegate` is a fire-and-forget DM from one persona to another using
//! the proven `/api/org/send-message` path. It is one tool call inside
//! a `tool_plan` array, so the parent persona can do `code_patch +
//! delegate(hex-tester) + delegate(integrator)` in a single accept and
//! the work fans out across the fleet.
//!
//! Targets are constrained to the lean fleet (5 personas). Unknown targets
//! are rejected at execute() time so a hallucinated peer name produces an
//! immediate error instead of a silent drop.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

use super::{Tool, ToolResult};

/// Valid delegation targets — the lean fleet.
const LEAN_FLEET: &[&str] = &[
    "hex-coder",
    "hex-tester",
    "hex-reviewer",
    "integrator",
    "engineering-lead",
];

const SEND_FROM: &str = "nexus-delegate";

pub struct Delegate;

#[async_trait]
impl Tool for Delegate {
    fn name(&self) -> &'static str {
        "delegate"
    }
    fn description(&self) -> &'static str {
        "Delegate one piece of work to another lean-fleet persona by DM. \
         Use when the ask involves work outside your domain but a peer \
         can clearly own it (e.g. hex-coder writes the source then \
         delegates the test to hex-tester). The peer receives a board \
         ask and processes it through its own classify→accept→tool_plan \
         loop. Fire-and-forget — does not block on the peer's result. \
         Valid targets: hex-coder, hex-tester, hex-reviewer, integrator, \
         engineering-lead."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "target_persona": {
                    "type": "string",
                    "enum": LEAN_FLEET,
                    "description": "The peer persona to delegate to. Must be in the lean fleet.",
                },
                "subject": {
                    "type": "string",
                    "description": "One-line subject for the DM. Max 120 chars.",
                },
                "brief": {
                    "type": "string",
                    "description": "Plain-language description of what you want the peer to do. Include enough context that the peer can act without a follow-up clarify. 100-4000 chars.",
                },
            },
            "required": ["target_persona", "brief"]
        })
    }
    async fn execute(&self, input: Value) -> ToolResult {
        let start = Instant::now();
        let target = match input.get("target_persona").and_then(|v| v.as_str()) {
            Some(t) if LEAN_FLEET.contains(&t) => t.to_string(),
            Some(other) => {
                return ToolResult::err(
                    format!(
                        "target_persona '{}' is not in the lean fleet (valid: {})",
                        other,
                        LEAN_FLEET.join(", ")
                    ),
                    start.elapsed().as_millis() as u64,
                );
            }
            None => {
                return ToolResult::err(
                    "target_persona required",
                    start.elapsed().as_millis() as u64,
                );
            }
        };
        let brief = match input.get("brief").and_then(|v| v.as_str()) {
            Some(s) if (100..=4000).contains(&s.len()) => s.to_string(),
            Some(s) => {
                return ToolResult::err(
                    format!("brief must be 100-4000 chars (got {})", s.len()),
                    start.elapsed().as_millis() as u64,
                );
            }
            None => {
                return ToolResult::err("brief required", start.elapsed().as_millis() as u64)
            }
        };
        let subject = input
            .get("subject")
            .and_then(|v| v.as_str())
            .map(|s| s.chars().take(120).collect::<String>())
            .unwrap_or_else(|| format!("delegation to {}", target));

        let nexus_port = std::env::var("HEX_NEXUS_PORT").unwrap_or_else(|_| "5555".to_string());
        let nexus_base = format!("http://127.0.0.1:{}", nexus_port);
        let url = format!("{}/api/org/send-message", nexus_base);

        let body = json!({
            "from": SEND_FROM,
            "to": target,
            "subject": subject,
            "content": brief,
        });

        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                return ToolResult::err(
                    format!("http client build failed: {}", e),
                    start.elapsed().as_millis() as u64,
                );
            }
        };

        match client.post(&url).json(&body).send().await {
            Ok(resp) if resp.status().is_success() => {
                let body_text = resp.text().await.unwrap_or_default();
                let parsed: Value = serde_json::from_str(&body_text).unwrap_or_else(|_| json!({}));
                let msg_id = parsed
                    .get("message_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?")
                    .to_string();
                tracing::info!(
                    target_persona = %target,
                    subject = %subject,
                    message_id = %msg_id,
                    "delegate: dispatched"
                );
                ToolResult::ok(
                    json!({
                        "delegated_to": target,
                        "subject": subject,
                        "message_id": msg_id,
                        "brief_bytes": brief.len(),
                    }),
                    start.elapsed().as_millis() as u64,
                )
            }
            Ok(resp) => {
                let status = resp.status();
                let txt = resp.text().await.unwrap_or_default();
                tracing::warn!(
                    target_persona = %target,
                    status = %status,
                    "delegate: send-message non-success"
                );
                ToolResult::err(
                    format!(
                        "send-message HTTP {}: {}",
                        status,
                        txt.chars().take(200).collect::<String>()
                    ),
                    start.elapsed().as_millis() as u64,
                )
            }
            Err(e) => {
                tracing::warn!(target_persona = %target, error = %e, "delegate: transport error");
                ToolResult::err(
                    format!("send-message transport: {}", e),
                    start.elapsed().as_millis() as u64,
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rejects_unknown_target_persona() {
        let t = Delegate;
        let r = t.execute(json!({"target_persona":"ghostly-engineer","brief":"x".repeat(120)})).await;
        assert!(!r.ok);
        let err = r.error.unwrap_or_default();
        assert!(err.contains("not in the lean fleet"), "got: {}", err);
    }

    #[tokio::test]
    async fn rejects_brief_too_short() {
        let t = Delegate;
        let r = t.execute(json!({"target_persona":"hex-tester","brief":"too short"})).await;
        assert!(!r.ok);
        let err = r.error.unwrap_or_default();
        assert!(err.contains("100-4000"), "got: {}", err);
    }

    #[tokio::test]
    async fn rejects_missing_target() {
        let t = Delegate;
        let r = t.execute(json!({"brief":"x".repeat(120)})).await;
        assert!(!r.ok);
        let err = r.error.unwrap_or_default();
        assert!(err.contains("target_persona required"), "got: {}", err);
    }

    #[tokio::test]
    async fn accepts_valid_lean_fleet_target() {
        // Smoke: the validation passes — execute will fail on the network
        // call because nexus isn't running in unit tests, but we just
        // confirm input validation lets us through.
        let t = Delegate;
        let r = t.execute(json!({
            "target_persona": "hex-tester",
            "subject": "test delegation",
            "brief": "Please write an integration test for the auction close path. The endpoint is POST /api/v1/listings/<id>/close and should verify that the highest bidder is recorded as the winner. ".repeat(2)
        })).await;
        // Either the http call succeeded (if nexus is up) or failed at
        // transport — both are acceptable; we just verify input validation
        // didn't reject the call.
        if !r.ok {
            let err = r.error.unwrap_or_default();
            // Must NOT be an input-validation error.
            assert!(!err.contains("must be"), "input rejected: {}", err);
            assert!(!err.contains("required"), "input rejected: {}", err);
        }
    }

    #[test]
    fn schema_advertises_lean_fleet_only() {
        let t = Delegate;
        let s = t.input_schema();
        let enum_vals = s["properties"]["target_persona"]["enum"]
            .as_array()
            .unwrap();
        assert_eq!(enum_vals.len(), 5);
        for f in LEAN_FLEET {
            assert!(enum_vals.iter().any(|v| v.as_str() == Some(f)));
        }
    }
}
