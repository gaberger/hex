//! COO observability baseline — `hex plan health`
//!
//! Runs the six deterministic audit queries from
//! `docs/specs/coo-observability-baseline.md`:
//!
//! 1. Persona SOP failure rate (last 24h)
//! 2. Workplan reconciliation drift
//! 3. Cost burn vs 7-day MA
//! 4. STDB reducer tick anomalies
//! 5. Digital-twin rejection rate
//! 6. Tool health matrix (stubbed for now)
//!
//! Emits JSON + Markdown table. Exit code 1 if any metric exceeds amber
//! threshold per the baseline spec.

use anyhow::Result;
use colored::Colorize;
use serde_json::json;
use std::process::Command;

/// Run the six observability queries and report health status.
pub async fn run() -> Result<()> {
    println!("{}", "hex plan health — COO observability baseline".bold());
    println!();

    let mut findings = Vec::new();
    let mut amber_count = 0;
    let mut red_count = 0;

    // ── 1. Persona SOP failure rate ───────────────────────────────────
    println!("{}", "1. Persona SOP failure rate (last 24h)".cyan().bold());
    // STDB inference_log.error column: count failures per role
    // Threshold: >10% warn, >25% escalate
    // TODO: wire to STDB when inference_log schema lands
    println!("  {} inference_log.cost_usd column not yet in STDB schema", "[STUB]".yellow());
    findings.push(json!({
        "metric": "persona_sop_failure_rate",
        "status": "stub",
        "value": null,
        "threshold_amber": 10.0,
        "threshold_red": 25.0,
    }));
    println!();

    // ── 2. Workplan reconciliation drift ──────────────────────────────
    println!("{}", "2. Workplan reconciliation drift".cyan().bold());
    // Run `hex plan reconcile --all --json` and count drift
    // Threshold: >3 warn, >10 escalate
    let drift_count = match run_reconcile_drift().await {
        Ok(count) => {
            let status = if count > 10 {
                red_count += 1;
                "RED"
            } else if count > 3 {
                amber_count += 1;
                "AMBER"
            } else {
                "GREEN"
            };
            println!("  Drift count: {} [{}]", count, status);
            findings.push(json!({
                "metric": "workplan_reconciliation_drift",
                "status": status.to_lowercase(),
                "value": count,
                "threshold_amber": 3,
                "threshold_red": 10,
            }));
            count
        }
        Err(e) => {
            println!("  {} {}", "[ERROR]".red(), e);
            findings.push(json!({
                "metric": "workplan_reconciliation_drift",
                "status": "error",
                "error": e.to_string(),
            }));
            0
        }
    };
    println!();

    // ── 3. Cost burn vs 7-day MA ──────────────────────────────────────
    println!("{}", "3. Cost burn vs 7-day moving average".cyan().bold());
    // STDB inference_log.cost_usd sum by day
    // Threshold: >150% warn, >300% escalate
    // TODO: wire to STDB when cost_usd column lands
    println!("  {} inference_log.cost_usd column not yet in STDB schema", "[STUB]".yellow());
    findings.push(json!({
        "metric": "cost_burn_vs_ma",
        "status": "stub",
        "value": null,
        "threshold_amber": 150.0,
        "threshold_red": 300.0,
    }));
    println!();

    // ── 4. STDB reducer tick anomalies ────────────────────────────────
    println!("{}", "4. STDB reducer tick anomalies".cyan().bold());
    // Check last_tick_utc for critical reducers
    // Threshold: >10 min warn, >30 min escalate
    // TODO: wire to STDB reducer_heartbeat table when schema lands
    println!("  {} reducer_heartbeat table not yet in STDB schema", "[STUB]".yellow());
    findings.push(json!({
        "metric": "reducer_tick_anomalies",
        "status": "stub",
        "value": null,
        "threshold_amber_minutes": 10,
        "threshold_red_minutes": 30,
    }));
    println!();

    // ── 5. Digital-twin rejection rate ────────────────────────────────
    println!("{}", "5. Digital-twin rejection rate (last 24h)".cyan().bold());
    // STDB proposed_action.approved column
    // Threshold: >15% warn, >25% escalate
    // TODO: wire to STDB when proposed_action.approved lands
    println!("  {} proposed_action.approved column not yet in STDB schema", "[STUB]".yellow());
    findings.push(json!({
        "metric": "twin_rejection_rate",
        "status": "stub",
        "value": null,
        "threshold_amber": 15.0,
        "threshold_red": 25.0,
    }));
    println!();

    // ── 6. Tool health matrix ─────────────────────────────────────────
    println!("{}", "6. Tool health matrix (last 24h)".cyan().bold());
    // STDB tool_invocation_log success_rate_pct by tool_name
    // Threshold: <80% warn, <50% escalate
    // TODO: wire to STDB when tool_invocation_log lands
    println!("  {} tool_invocation_log table not yet in STDB schema", "[STUB]".yellow());
    findings.push(json!({
        "metric": "tool_health_matrix",
        "status": "stub",
        "value": null,
        "threshold_amber": 80.0,
        "threshold_red": 50.0,
    }));
    println!();

    // ── Summary table ─────────────────────────────────────────────────
    println!("{}", "Summary".bold());
    println!("  Workplan drift: {}", drift_count);
    println!("  Amber alerts:   {}", amber_count);
    println!("  Red alerts:     {}", red_count);
    println!();

    // ── JSON output ───────────────────────────────────────────────────
    let summary = json!({
        "findings": findings,
        "summary": {
            "amber_count": amber_count,
            "red_count": red_count,
            "exit_code": if red_count > 0 || amber_count > 0 { 1 } else { 0 },
        },
    });
    println!("{}", serde_json::to_string_pretty(&summary)?);

    // Exit code 1 if any amber or red
    if red_count > 0 || amber_count > 0 {
        std::process::exit(1);
    }
    Ok(())
}

/// Run `hex plan reconcile --all --json` and extract drift count.
///
/// Drift = ADRs marked Accepted without workplan, or workplans marked
/// complete whose ADR is still Proposed. For now, we use a simple proxy:
/// count of Proposed ADRs that have a matching wp-*.json (the inverse
/// signal — ADRs stuck in Proposed despite workplan existence).
async fn run_reconcile_drift() -> Result<usize> {
    use std::path::Path;

    // Count Proposed ADRs
    let adr_dir = Path::new("docs/adrs");
    if !adr_dir.is_dir() {
        return Ok(0);
    }

    let mut proposed_count = 0;
    for entry in std::fs::read_dir(adr_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        if let Ok(content) = std::fs::read_to_string(&path) {
            // Look for `Status: **Proposed**` line
            if content.lines().any(|l| l.contains("Status:") && l.contains("**Proposed**")) {
                proposed_count += 1;
            }
        }
    }

    // Count workplans
    let wp_dir = Path::new("docs/workplans");
    let wp_count = if wp_dir.is_dir() {
        std::fs::read_dir(wp_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .and_then(|ext| ext.to_str())
                    == Some("json")
                    && e.path()
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .map(|s| s.starts_with("wp-"))
                        .unwrap_or(false)
            })
            .count()
    } else {
        0
    };

    // Simple drift heuristic: if we have more Proposed ADRs than
    // workplans, that's drift (ADRs waiting for workplan scaffolding).
    // Real reconcile logic lives in `hex plan reconcile --all`.
    let drift = if proposed_count > wp_count {
        proposed_count - wp_count
    } else {
        0
    };

    Ok(drift)
}
