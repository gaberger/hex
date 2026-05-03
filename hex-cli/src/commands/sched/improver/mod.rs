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
    }
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
