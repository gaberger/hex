//! Self-improvement loop (ADR-2604271100).
//!
//! Pipeline: [`discover`] → variant generation (P2) → judge (P3) → act (P4),
//! tied together by a sched tick (P5). This module hosts the discovery
//! surface; later phases live in `hex-nexus/src/orchestration/`.
//!
//! P1.2 adds the operator-facing CLI surface — `hex sched improver discover`
//! lets a human preview what the autonomous loop would propose, with either
//! a single sweep (`--once`) or polling at the daemon's cadence.

pub mod act;
pub mod discover;
pub mod judge;
pub mod learn;

use std::time::Duration;

use anyhow::{Context, Result};
use clap::Subcommand;
use serde::Serialize;

use crate::fmt::{pretty_table, truncate};

pub use discover::{discover, discover_with, load_detectors, Detector, Hypothesis, Severity, Source};

/// Default polling cadence — matches the improver tick at P5
/// (~every 8 ticks at 30s = 240s). Operator can override with `--interval`.
const DEFAULT_INTERVAL_SECS: u64 = 240;

#[derive(Subcommand)]
pub enum ImproverAction {
    /// Run discover() and print hypotheses (table or JSON).
    /// Default polls at the daemon cadence; `--once` exits after one sweep.
    Discover {
        /// Run a single sweep and exit instead of polling.
        #[arg(long)]
        once: bool,
        /// Emit a JSON array of hypotheses instead of a table.
        #[arg(long)]
        json: bool,
        /// Polling interval in seconds (only meaningful without `--once`).
        #[arg(long, default_value_t = DEFAULT_INTERVAL_SECS)]
        interval: u64,
    },
    /// Run discover() then rank hypotheses by impact (ADR-2604271100 P3).
    /// Outputs the ranked list with score + reason. Read-only.
    Judge {
        /// Emit ranked output as JSON instead of a table.
        #[arg(long)]
        json: bool,
    },
    /// Run the full discover → judge → act pipeline. Maps top-N ranked
    /// hypotheses to concrete actions (sched tasks or operator
    /// recommendations). Defaults to dry-run; pass `--apply` to actually
    /// enqueue the auto-mappable actions (priority-tagged per score).
    Act {
        /// Number of top-ranked hypotheses to act on. 0 = all.
        #[arg(long, default_value_t = 5)]
        top: usize,
        /// Actually enqueue sched tasks. Default is dry-run preview.
        #[arg(long)]
        apply: bool,
        /// Emit actions as JSON instead of a table.
        #[arg(long)]
        json: bool,
    },
    /// Observe outcomes from the prior `act --apply` snapshot and update
    /// the improver Q-table. Run after enough time has passed for the
    /// queued actions to have completed (~one daemon tick is usually
    /// enough). Read-only against the workplan corpus; mutates only
    /// `~/.hex/improver/q-table.json`.
    Learn {
        /// Emit the credited (template, reward) pairs as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Show the improver Q-table (per-action-template mean reward and
    /// sample count). Distinct from `hex sched scores`, which shows the
    /// nexus sched_service's RL engine for model selection.
    Scores {
        /// Emit the table as JSON.
        #[arg(long)]
        json: bool,
        /// Emit findings for action templates with non-positive mean
        /// reward (action ran but didn't resolve its target). Used by
        /// the improver `q_starvation` meta-detector to surface action
        /// mappings that need review. Implies --json.
        #[arg(long)]
        starvation: bool,
    },
    /// Single-pane homeostasis dashboard for the self-improvement loop.
    /// Aggregates discover counts (with delta), Q-table state, detector
    /// health, dead-letter accumulation, and a synthesized homeostasis
    /// score so an operator can answer "is the loop healthy" in one
    /// glance without running five subcommands.
    Status {
        /// Emit the status as a structured JSON object suitable for
        /// dashboards or CI.
        #[arg(long)]
        json: bool,
        /// Re-render every N seconds until interrupted. Useful when
        /// watching an auto-act sweep settle.
        #[arg(long)]
        watch: Option<u64>,
    },
}

pub async fn run(action: ImproverAction) -> Result<()> {
    match action {
        ImproverAction::Discover { once, json, interval } => {
            run_discover(once, json, interval).await
        }
        ImproverAction::Judge { json } => run_judge(json).await,
        ImproverAction::Act { top, apply, json } => run_act(top, apply, json).await,
        ImproverAction::Learn { json } => run_learn(json).await,
        ImproverAction::Scores { json, starvation } => run_scores(json, starvation).await,
        ImproverAction::Status { json, watch } => run_status(json, watch).await,
    }
}

async fn run_status(json: bool, watch: Option<u64>) -> Result<()> {
    loop {
        render_status_once(json).await?;
        match watch {
            Some(secs) if secs > 0 => {
                tokio::time::sleep(Duration::from_secs(secs)).await;
                if !json {
                    // Clear screen between renders for terminal viewers; in
                    // JSON mode emit a newline-separated stream so a tail
                    // consumer doesn't lose history.
                    print!("\x1b[2J\x1b[H");
                }
            }
            _ => return Ok(()),
        }
    }
}

async fn render_status_once(json: bool) -> Result<()> {
    use crate::commands::sched::list_brain_tasks;

    let repo = std::env::current_dir().context("resolve repo root for status")?;
    let hypotheses = discover::discover(&repo).unwrap_or_default();
    let ranked = judge::rank(&hypotheses);
    let table = learn::load_q_table();

    // Per-source counts (drives the by-source breakdown line).
    let mut by_source: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for h in &hypotheses {
        *by_source.entry(format!("{:?}", h.source)).or_insert(0) += 1;
    }

    // Detector_health and q_starvation findings are the homeostatic guards.
    // Their presence means the loop is reporting drift in its own surface.
    let detector_health: Vec<&Hypothesis> = hypotheses
        .iter()
        .filter(|h| h.evidence.get("detector_health").is_some())
        .collect();
    let q_starvation: Vec<&Hypothesis> = hypotheses
        .iter()
        .filter(|h| h.source == Source::QStarvation)
        .collect();

    // Auto-actionable share — the convergence motor. SchedShell + Workplan
    // are auto-mappable; Recommend requires operator decision.
    let mut auto_mappable = 0usize;
    for s in &ranked {
        if let Some(action) = act::derive(s) {
            if !matches!(action.kind, act::ActionKind::Recommend) {
                auto_mappable += 1;
            }
        }
    }
    let auto_share = if hypotheses.is_empty() {
        100.0
    } else {
        auto_mappable as f64 / hypotheses.len() as f64 * 100.0
    };

    // Dead-letter accumulation.
    let dead_letter_count = list_brain_tasks(Some("dead_letter"))
        .await
        .map(|t| t.len())
        .unwrap_or(0);
    let pending_count = list_brain_tasks(Some("pending"))
        .await
        .map(|t| t.len())
        .unwrap_or(0);

    // Mean reward across all Q-table entries — overall learning signal.
    let (q_total_samples, q_mean_reward) = if table.entries.is_empty() {
        (0_u64, 0.0_f64)
    } else {
        let total_samples: u64 = table.entries.values().map(|e| e.samples).sum();
        let total_reward: f64 = table.entries.values().map(|e| e.total_reward).sum();
        let mean = if total_samples == 0 { 0.0 } else { total_reward / total_samples as f64 };
        (total_samples, mean)
    };

    // Homeostasis score 0–100. Lower is better up to a point, then health
    // signals dominate. Start at 100, deduct for unfixable accumulation
    // and surface drift, with caps so any single signal can't dominate.
    //
    // - Each detector_health finding: -8 (capped at -40)
    // - Each q_starvation finding:    -8 (capped at -40)
    // - Dead-letter > 5:              -1 per task above 5 (capped at -20)
    // - Auto-share < 80%:             -(80 - auto_share) (capped at -30)
    // - Negative q_mean_reward:       -10
    let mut score: i32 = 100;
    score -= (detector_health.len() as i32 * 8).min(40);
    score -= (q_starvation.len() as i32 * 8).min(40);
    if dead_letter_count > 5 {
        score -= ((dead_letter_count as i32 - 5).min(20)).max(0);
    }
    if auto_share < 80.0 {
        score -= ((80.0 - auto_share) as i32).min(30);
    }
    if q_total_samples > 0 && q_mean_reward < 0.0 {
        score -= 10;
    }
    let homeostasis = score.max(0).min(100);

    if json {
        let output = serde_json::json!({
            "homeostasis_score": homeostasis,
            "hypotheses": {
                "total": hypotheses.len(),
                "by_source": by_source,
                "auto_actionable": auto_mappable,
                "auto_share_pct": auto_share,
            },
            "guards": {
                "detector_health": detector_health.len(),
                "q_starvation": q_starvation.len(),
            },
            "queue": {
                "pending": pending_count,
                "dead_letter": dead_letter_count,
            },
            "q_table": {
                "templates": table.entries.len(),
                "total_samples": q_total_samples,
                "mean_reward": q_mean_reward,
            },
            "top_hypothesis": ranked.first().map(|s| serde_json::json!({
                "source": format!("{:?}", s.hypothesis.source),
                "scope": s.hypothesis.scope,
                "score": s.score,
                "reason": s.reason,
            })),
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    // Human-readable rendering. Color codes: green = healthy, yellow =
    // attention, red = drift the operator should look at.
    use colored::Colorize;
    let score_str = format!("{}/100", homeostasis);
    let score_colored = if homeostasis >= 80 {
        score_str.green().bold().to_string()
    } else if homeostasis >= 60 {
        score_str.yellow().bold().to_string()
    } else {
        score_str.red().bold().to_string()
    };

    println!("{}", "── Improver Homeostasis ────────────────────".cyan().bold());
    println!("  Score:           {}", score_colored);
    println!();
    println!("  Hypotheses:      {} total ({:.0}% auto-actionable)", hypotheses.len(), auto_share);
    for (source, count) in &by_source {
        println!("    {} {}: {}", "·".dimmed(), source, count);
    }
    println!();
    let guard_line = if detector_health.is_empty() && q_starvation.is_empty() {
        "  Self-monitor:    surface clean (no detector_health or q_starvation findings)"
            .green()
            .to_string()
    } else {
        format!(
            "  Self-monitor:    {} detector_health, {} q_starvation",
            detector_health.len(),
            q_starvation.len(),
        )
        .yellow()
        .to_string()
    };
    println!("{}", guard_line);
    println!();
    println!(
        "  Queue:           {} pending, {} dead-lettered",
        pending_count,
        if dead_letter_count > 5 {
            dead_letter_count.to_string().yellow().to_string()
        } else {
            dead_letter_count.to_string()
        }
    );
    println!();
    if q_total_samples == 0 {
        println!("  Q-table:         {} (no samples yet — run improver act --apply)", "untrained".dimmed());
    } else {
        let reward_colored = if q_mean_reward > 0.0 {
            format!("{:+.3}", q_mean_reward).green().to_string()
        } else if q_mean_reward < 0.0 {
            format!("{:+.3}", q_mean_reward).red().to_string()
        } else {
            format!("{:+.3}", q_mean_reward).yellow().to_string()
        };
        println!(
            "  Q-table:         {} templates, {} samples, mean reward {}",
            table.entries.len(),
            q_total_samples,
            reward_colored
        );
    }
    println!();
    if let Some(top) = ranked.first() {
        println!(
            "  Top hypothesis:  {:?} {} (score {})",
            top.hypothesis.source,
            top.hypothesis.scope,
            top.score
        );
    }
    println!();
    Ok(())
}

async fn run_learn(json: bool) -> Result<()> {
    let repo = std::env::current_dir().context("resolve repo root for learn")?;
    let hypotheses = discover::discover(&repo)?;
    let credited = learn::observe_and_reward(&hypotheses)?;
    if json {
        let table = learn::load_q_table();
        println!("{}", serde_json::to_string_pretty(&serde_json::json!({
            "credited": credited,
            "table": table,
        }))?);
    } else {
        println!(
            "Improver learn: credited {} action(s); Q-table now has {} entries",
            credited,
            learn::load_q_table().entries.len(),
        );
    }
    Ok(())
}

async fn run_scores(json: bool, starvation: bool) -> Result<()> {
    let table = learn::load_q_table();
    if starvation {
        // Templates with ≥3 samples and non-positive mean — the action
        // ran enough times to be statistically meaningful, but didn't
        // resolve its target. Surface as findings for the q_starvation
        // meta-detector so the improver flags its own broken mappings.
        let findings: Vec<_> = table
            .entries
            .iter()
            .filter(|(_, e)| e.samples >= 3 && e.mean() <= 0.0)
            .map(|(k, e)| {
                serde_json::json!({
                    "template": k,
                    "samples": e.samples,
                    "mean_reward": e.mean(),
                    "kind": "action_not_resolving",
                    "severity": "warning",
                    "remediation": "review the (source, action_kind) mapping in act::derive — the action runs but doesn't clear the hypothesis",
                })
            })
            .collect();
        println!("{}", serde_json::json!({"findings": findings}));
        return Ok(());
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&table)?);
        return Ok(());
    }
    if table.entries.is_empty() {
        println!("Improver Q-table empty — run `hex sched improver act --apply` followed by `hex sched improver learn` to start collecting samples.");
        return Ok(());
    }
    let mut keys: Vec<_> = table.entries.keys().cloned().collect();
    keys.sort();
    let rows: Vec<Vec<String>> = keys
        .iter()
        .map(|k| {
            let e = &table.entries[k];
            vec![
                k.clone(),
                format!("{:+.3}", e.mean()),
                e.samples.to_string(),
                e.last_updated
                    .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
                    .unwrap_or_else(|| "—".to_string()),
            ]
        })
        .collect();
    println!(
        "{}",
        pretty_table(&["TEMPLATE", "MEAN REWARD", "SAMPLES", "LAST UPDATE"], &rows)
    );
    Ok(())
}

async fn run_judge(json: bool) -> Result<()> {
    let repo = std::env::current_dir().context("resolve repo root for judge")?;
    let hypotheses = discover::discover(&repo)?;
    let ranked = judge::rank(&hypotheses);
    if json {
        println!("{}", serde_json::to_string_pretty(&ranked)?);
    } else {
        let rows: Vec<Vec<String>> = ranked
            .iter()
            .map(|s| {
                vec![
                    s.score.to_string(),
                    format!("{:?}", s.hypothesis.source),
                    format!("{:?}", s.hypothesis.severity).to_lowercase(),
                    truncate(&s.hypothesis.scope, 36),
                    truncate(&s.reason, 40),
                ]
            })
            .collect();
        println!(
            "{}",
            pretty_table(&["SCORE", "SOURCE", "SEVERITY", "SCOPE", "REASON"], &rows)
        );
    }
    Ok(())
}

async fn run_act(top: usize, apply: bool, json: bool) -> Result<()> {
    let repo = std::env::current_dir().context("resolve repo root for act")?;
    let hypotheses = discover::discover(&repo)?;
    let ranked = judge::rank(&hypotheses);
    let actions = act::act(&ranked, top, apply).await?;
    if json {
        println!("{}", serde_json::to_string_pretty(&actions)?);
    } else {
        let mode = if apply { "apply" } else { "dry-run" };
        println!("Improver actions ({}, top {}):", mode, if top == 0 { ranked.len() } else { top });
        let rows: Vec<Vec<String>> = actions
            .iter()
            .map(|a| {
                vec![
                    a.priority.to_string(),
                    format!("{:?}", a.kind).to_lowercase(),
                    truncate(&a.payload, 60),
                    truncate(&a.derived_from, 22),
                ]
            })
            .collect();
        println!(
            "{}",
            pretty_table(&["PRI", "KIND", "PAYLOAD", "FROM"], &rows)
        );
    }
    Ok(())
}

async fn run_discover(once: bool, json: bool, interval_secs: u64) -> Result<()> {
    let repo = std::env::current_dir().context("resolve repo root for discover")?;
    loop {
        let hypotheses = discover::discover(&repo)?;
        emit(&hypotheses, json)?;
        if once {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_secs(interval_secs)).await;
    }
}

fn emit(hypotheses: &[Hypothesis], json: bool) -> Result<()> {
    if json {
        // Wrap in an envelope so consumers can extend without breaking parsers
        // (the gate command parses this output, and a bare-array → object
        // shape change later would be a silent break).
        let envelope = Envelope {
            count: hypotheses.len(),
            hypotheses,
        };
        println!("{}", serde_json::to_string_pretty(&envelope)?);
    } else {
        println!("{}", render_table(hypotheses));
    }
    Ok(())
}

#[derive(Serialize)]
struct Envelope<'a> {
    count: usize,
    hypotheses: &'a [Hypothesis],
}

fn render_table(hypotheses: &[Hypothesis]) -> String {
    let rows: Vec<Vec<String>> = hypotheses
        .iter()
        .map(|h| {
            vec![
                format!("{:?}", h.source),
                format!("{:?}", h.severity).to_lowercase(),
                truncate(&h.scope, 36),
                truncate(&h.id, 22),
                h.generated_at.format("%H:%M:%SZ").to_string(),
            ]
        })
        .collect();
    pretty_table(&["SOURCE", "SEVERITY", "SCOPE", "ID", "AT"], &rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serde_json::json;

    fn fixture(source: Source, scope: &str, severity: Severity) -> Hypothesis {
        Hypothesis {
            id: format!("hyp-{}-{}", scope, severity_tag(severity)),
            source,
            scope: scope.to_string(),
            severity,
            evidence: json!({"k": "v"}),
            generated_at: Utc::now(),
        }
    }

    fn severity_tag(s: Severity) -> &'static str {
        match s {
            Severity::Info => "info",
            Severity::Warning => "warn",
            Severity::Error => "err",
        }
    }

    #[test]
    fn empty_table_renders_no_results_marker() {
        let out = render_table(&[]);
        assert!(out.contains("no results"), "got: {out}");
    }

    #[test]
    fn table_includes_source_and_scope() {
        let h = fixture(Source::AdrDoctor, "ADR-X", Severity::Error);
        let out = render_table(&[h]);
        assert!(out.contains("AdrDoctor"), "missing source: {out}");
        assert!(out.contains("ADR-X"), "missing scope: {out}");
        assert!(out.contains("error"), "missing severity: {out}");
    }

    #[test]
    fn json_envelope_is_valid_and_round_trips() {
        let h = fixture(Source::ReconcileStrict, "wp-foo", Severity::Warning);
        let envelope = Envelope { count: 1, hypotheses: std::slice::from_ref(&h) };
        let s = serde_json::to_string(&envelope).expect("serialize envelope");
        let v: serde_json::Value = serde_json::from_str(&s).expect("parse envelope");
        assert_eq!(v.get("count").and_then(|c| c.as_u64()), Some(1));
        assert!(v.get("hypotheses").and_then(|x| x.as_array()).is_some());
    }
}
