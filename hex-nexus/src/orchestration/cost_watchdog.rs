//! Cost watchdog - scheduled background task that monitors inference burn rate.
//!
//! ADR-[PHONE]: Nobody is actively monitoring inference burn rate today;
//! cost_meter exists but has 3 log mentions total and no periodic invocation.
//! This watchdog calls cost_meter every N minutes, compares against thresholds,
//! and escalates via escalate_to_operator (which pages Telegram per ADR-[PHONE])
//! on breach.
//!
//! Thresholds defined in docs/specs/cost-and-token-efficiency.md.
//! Escalation flow defined in docs/specs/cost-ops-runbook.md.

use serde_json::json;
use std::time::Duration;

const DEFAULT_INTERVAL_SECS: u64 = 1800; // 30 minutes
const DEFAULT_HOURLY_THRESHOLD_USD: f64 = 1.00;
const COST_WINDOW_SECS: u64 = 3600; // 1 hour lookback

/// Infinite loop: poll cost_meter, compare against threshold, escalate on breach.
///
/// Reads configuration from environment:
/// - `HEX_COST_WATCHDOG_INTERVAL_SECS` (default 1800 = 30 min)
/// - `HEX_COST_HOURLY_USD_THRESHOLD` (default 1.00)
///
/// Logs every tick at `tracing::info` regardless of breach for audit trail.
pub async fn run(_stdb_host: String, _hex_db: String) {
    let interval_secs = std::env::var("HEX_COST_WATCHDOG_INTERVAL_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(DEFAULT_INTERVAL_SECS);
    let threshold_usd = std::env::var("HEX_COST_HOURLY_USD_THRESHOLD")
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(DEFAULT_HOURLY_THRESHOLD_USD);

    tracing::info!(
        interval_secs,
        threshold_usd,
        "cost_watchdog started - will poll every {}s and escalate if hourly burn exceeds ${:.2}",
        interval_secs,
        threshold_usd
    );

    let mut interval = tokio::time::interval(Duration::from_secs(interval_secs));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        interval.tick().await;

        // Query cost_meter tool
        let cost_meter = crate::tools::cost_meter::CostMeter;
        let input = json!({
            "window_secs": COST_WINDOW_SECS,
            "group_by": "model"
        });

        use crate::tools::Tool;
        let result = cost_meter.execute(input).await;

        if result.ok {
            let output = &result.output;
            {
                let total_cost = output
                    .get("totals")
                    .and_then(|t| t.get("cost_usd"))
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);
                let window_secs_value = output
                    .get("window_secs")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(COST_WINDOW_SECS);
                let window_secs = window_secs_value;

                tracing::info!(
                    total_cost_usd = total_cost,
                    window_secs,
                    threshold_usd,
                    "cost_watchdog tick: ${:.4} over {}s window",
                    total_cost,
                    window_secs
                );

                // Check threshold breach
                if total_cost > threshold_usd {
                    tracing::warn!(
                        total_cost_usd = total_cost,
                        threshold_usd,
                        "cost_watchdog BREACH: ${:.4} exceeds ${:.2} threshold - escalating",
                        total_cost,
                        threshold_usd
                    );

                    // Escalate via escalate_to_operator tool
                    let escalator = crate::tools::escalate_to_operator::EscalateToOperator;
                    let escalation_input = json!({
                        "reason": format!(
                            "Inference burn ${:.4}/hour exceeds threshold ${:.2}. \
                             Review cost-and-token-efficiency.md thresholds and \
                             cost-ops-runbook.md mitigation steps.",
                            total_cost,
                            threshold_usd
                        ),
                        "urgency": "med",
                        "options": []
                    });

                    let esc_result = escalator.execute(escalation_input).await;
                    if esc_result.ok {
                        tracing::info!(
                            escalation_id = ?esc_result.output.get("escalation_id"),
                            "cost_watchdog escalation sent"
                        );
                    } else {
                        tracing::warn!(error = ?esc_result.error, "cost_watchdog escalation failed");
                    }
                } else {
                    tracing::debug!(
                        total_cost_usd = total_cost,
                        threshold_usd,
                        utilization_pct = (total_cost / threshold_usd * 100.0),
                        "cost_watchdog: burn within threshold ({:.1}% utilization)",
                        (total_cost / threshold_usd * 100.0)
                    );
                }
            }
        } else {
            tracing::warn!(
                error = ?result.error,
                "cost_watchdog tick: cost_meter query failed"
            );
        }
    }
}
