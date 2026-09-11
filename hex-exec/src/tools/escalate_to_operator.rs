//! `escalate_to_operator` — when a persona genuinely cannot proceed,
//! emit a priority-2 inbox notification the operator sees on next chat.

use async_trait::async_trait;
use hex_core::ports::local_store::EmissionKind;
use serde_json::{json, Value};
use std::time::Instant;

use super::emit;
use super::{Tool, ToolResult};


pub struct EscalateToOperator;

#[async_trait]
impl Tool for EscalateToOperator {
    fn name(&self) -> &'static str {
        "escalate_to_operator"
    }
    fn description(&self) -> &'static str {
        "Escalate to the human operator when you genuinely cannot \
         proceed: paradigm questions, ambiguous asks, novel domains, or \
         situations where the operator should pick from options. Inserts \
         a priority-2 inbox notification visible on the dashboard. Do NOT \
         use for routine completion — only when human judgment is required."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "reason": {
                    "type": "string",
                    "description": "One-paragraph plain-language explanation of WHY this needs operator attention. Max 500 chars.",
                },
                "urgency": {
                    "type": "string",
                    "enum": ["low", "med", "high"],
                    "description": "How urgent. 'high' = blocks other work; 'med' = shape decision; 'low' = nice-to-decide."
                },
                "options": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional: 1-6 concrete options the operator can pick from. Each is a short paragraph.",
                }
            },
            "required": ["reason", "urgency"]
        })
    }
    async fn execute(&self, input: Value) -> ToolResult {
        let start = Instant::now();
        let reason = match input.get("reason").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() && s.len() <= 500 => s.to_string(),
            _ => return ToolResult::err("reason required, 1-500 chars", start.elapsed().as_millis() as u64),
        };
        let urgency = match input.get("urgency").and_then(|v| v.as_str()) {
            Some(s @ "low" | s @ "med" | s @ "high") => s.to_string(),
            _ => return ToolResult::err("urgency must be low|med|high", start.elapsed().as_millis() as u64),
        };
        let options: Vec<String> = input
            .get("options")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
            .unwrap_or_default();
        if options.len() > 6 {
            return ToolResult::err("max 6 options", start.elapsed().as_millis() as u64);
        }

        let priority = match urgency.as_str() {
            "high" => "critical",
            "med" => "warn",
            _ => "info",
        };

        let note = if options.is_empty() {
            reason.clone()
        } else {
            let opts_joined = options
                .iter()
                .enumerate()
                .map(|(i, o)| format!("({}) {}", i + 1, o))
                .collect::<Vec<_>>()
                .join(" | ");
            format!("{} — Options: {}", reason, opts_joined)
        };

        // Record it (ADR-2608241500 P3.2). The previous code built a
        // SpacetimeDB reducer URL, discarded it with `let _ = url`, and logged
        // — so an escalation reached the operator only if they happened to be
        // reading the daemon's log. It is now an emission record that
        // `hex do runs` can surface, plus the existing notifier.
        emit::record(
            EmissionKind::Escalation,
            &format!("escalation/{}", priority),
            "tool:escalate_to_operator",
            note.len() as u64,
            &note,
        );
        tracing::warn!(
            reason = %reason,
            urgency = %urgency,
            options = ?options,
            priority = %priority,
            "escalate_to_operator: escalation raised"
        );

        let elapsed = start.elapsed().as_millis() as u64;

        // Fire-and-forget Telegram notification if configured
        let notifier = crate::telegram_notifier::TelegramNotifier::from_env();
        let telegram_message = format!(
            "🚨 hex escalation: {} | urgency={} priority={}",
            reason, urgency, priority
        );
        if let Err(e) = notifier.send(&telegram_message).await {
            tracing::warn!(
                error = %e,
                "telegram_notifier send failed; escalation still recorded locally"
            );
        }

        ToolResult::ok(
            json!({
                "ok": true,
                "escalation_id": chrono::Utc::now().timestamp_millis(),
                "priority": priority,
                "note": note,
                "warning": "escalation recorded locally and sent to the configured notifier; no operator inbox exists in solo mode",
            }),
            elapsed,
        )
    }
}
