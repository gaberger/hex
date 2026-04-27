//! Self-improvement loop (ADR-2604271100).
//!
//! Pipeline: [`discover`] → variant generation (P2) → judge (P3) → act (P4),
//! tied together by a sched tick (P5). This module hosts the discovery
//! surface; later phases live in `hex-nexus/src/orchestration/`.
//!
//! P1.2 adds the operator-facing CLI surface — `hex sched improver discover`
//! lets a human preview what the autonomous loop would propose, with either
//! a single sweep (`--once`) or polling at the daemon's cadence.

pub mod discover;

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
}

pub async fn run(action: ImproverAction) -> Result<()> {
    match action {
        ImproverAction::Discover { once, json, interval } => {
            run_discover(once, json, interval).await
        }
    }
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
