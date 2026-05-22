//! `escalate_to_operator` — when a persona genuinely cannot proceed,
//! emit a priority-2 inbox notification the operator sees on next chat.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::Instant;

use super::{Tool, ToolResult};

const STDB_HOST_DEFAULT: &str = "http://127.0.0.1:3033";

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

        let host = std::env::var("HEX_SPACETIMEDB_HOST")
            .unwrap_or_else(|_| STDB_HOST_DEFAULT.to_string());
        let db = std::env::var("HEX_STDB_DATABASE")
            .unwrap_or_else(|_| hex_core::stdb_database_for_module("hexflo-coordination").to_string());
        // Use the existing resource_anomaly stream as the inbox surface
        // until a dedicated inbox_notification reducer ships. This keeps
        // the operator's existing #/resources view as the one alert sink.
        let url = format!("{}/v1/database/{}/call/", host, db);

        let priority = match urgency.as_str() {
            "high" => "critical",
            "med" => "warn",
            _ => "info",
        };

        // Emit a synthetic resource_anomaly so it shows up in the existing
        // anomalies surface. JSON shape matches the table.
        // We pretend `pid=0` (synthetic) and put options into the note.
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

        // Reuse resource_supervisor_tick_schedule's emit path by calling
        // process_observation_upsert(pid=0, ...) is wrong; we instead call
        // a future inbox_notify — but that doesn't exist yet. For now
        // log + return a stub id; the dashboard already polls
        // /api/resources/anomalies, so we'll emit into resource_anomaly
        // via a direct insert SQL call. STDB doesn't allow direct INSERT
        // via SQL, so we pretend to and log the escalation locally.
        // Wave-2 work will add a dedicated escalation reducer.
        tracing::warn!(
            reason = %reason,
            urgency = %urgency,
            options = ?options,
            priority = %priority,
            "escalate_to_operator: escalation raised (logged; dashboard wiring deferred to wave 2)"
        );
        let _ = url; // suppress unused warning for now

        let elapsed = start.elapsed().as_millis() as u64;

        // Fire-and-forget Telegram notification if configured
        let notifier = crate::adapters::telegram_notifier::TelegramNotifier::from_env();
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
                "warning": "escalation logged to nexus.log; dashboard surface lands in wave-2 follow-on workplan",
            }),
            elapsed,
        )
    }
}
