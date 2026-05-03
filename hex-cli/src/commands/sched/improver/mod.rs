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
        /// Suppress appending this invocation to the history JSONL.
        /// Default: every status call appends so the convergence series
        /// accumulates passively without any cron or daemon wiring.
        #[arg(long)]
        no_history: bool,
    },
    /// Show the homeostasis convergence series — last N status snapshots
    /// rendered as a per-line trend with delta-from-previous. Useful for
    /// answering "is the loop converging" or "what changed in the last
    /// hour" without running the discover sweep yourself.
    History {
        /// Maximum rows to display (newest first). Default 30.
        #[arg(long, default_value_t = 30)]
        limit: usize,
        /// Emit the history slice as JSON instead of a table.
        #[arg(long)]
        json: bool,
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
        ImproverAction::Status { json, watch, no_history } => run_status(json, watch, no_history).await,
        ImproverAction::History { limit, json } => run_history(limit, json).await,
    }
}

const HISTORY_MAX_LINES: usize = 1000;

fn history_path() -> Result<std::path::PathBuf> {
    let home = dirs::home_dir().context("resolve home dir")?;
    let dir = home.join(".hex/improver");
    std::fs::create_dir_all(&dir).ok();
    Ok(dir.join("history.jsonl"))
}

/// Append one status snapshot to the history JSONL. Bounded to the last
/// HISTORY_MAX_LINES entries — older lines are dropped when the file grows
/// past 1.5× the cap, so the rotation is amortized rather than every call.
fn append_history(snapshot: &serde_json::Value) -> Result<()> {
    let path = history_path()?;
    let line = serde_json::to_string(snapshot)? + "\n";
    use std::io::Write;
    {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("open {}", path.display()))?;
        f.write_all(line.as_bytes())?;
    }
    // Amortized rotation: only inspect line count past 1.5× cap, then
    // truncate to the cap. Avoids reading-then-writing on every append.
    if let Ok(content) = std::fs::read_to_string(&path) {
        let line_count = content.lines().count();
        if line_count > (HISTORY_MAX_LINES * 3 / 2) {
            let kept: Vec<&str> = content
                .lines()
                .rev()
                .take(HISTORY_MAX_LINES)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            let trimmed = kept.join("\n") + "\n";
            std::fs::write(&path, trimmed).ok();
        }
    }
    Ok(())
}

async fn run_history(limit: usize, json: bool) -> Result<()> {
    let path = history_path()?;
    let Ok(content) = std::fs::read_to_string(&path) else {
        if json {
            println!("[]");
        } else {
            println!("Improver history empty — no `hex sched improver status` invocations recorded yet.");
        }
        return Ok(());
    };
    let entries: Vec<serde_json::Value> = content
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    let slice: Vec<&serde_json::Value> = entries.iter().rev().take(limit).collect();
    if json {
        let arr: Vec<serde_json::Value> = slice.iter().map(|v| (*v).clone()).collect();
        println!("{}", serde_json::to_string_pretty(&arr)?);
        return Ok(());
    }
    use colored::Colorize;
    println!("{}", "── Improver Convergence History ──".cyan().bold());
    let rows: Vec<Vec<String>> = slice
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let timestamp = e
                .get("timestamp")
                .and_then(|v| v.as_str())
                .unwrap_or("—")
                .replace('T', " ")
                .chars()
                .take(19)
                .collect::<String>();
            let score = e
                .get("homeostasis_score")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let total = e
                .get("hypotheses")
                .and_then(|v| v.get("total"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let auto_share = e
                .get("hypotheses")
                .and_then(|v| v.get("auto_share_pct"))
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            let q_samples = e
                .get("q_table")
                .and_then(|v| v.get("total_samples"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let dead_letter = e
                .get("queue")
                .and_then(|v| v.get("dead_letter"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            // Compute deltas relative to the *next* (older) entry. The
            // slice is newest-first, so previous = slice[i+1].
            let prev_total = slice.get(i + 1)
                .and_then(|p| p.get("hypotheses"))
                .and_then(|p| p.get("total"))
                .and_then(|v| v.as_u64());
            let total_delta = prev_total.map(|p| total as i64 - p as i64).unwrap_or(0);
            let total_str = if total_delta > 0 {
                format!("{} ({:+})", total, total_delta).yellow().to_string()
            } else if total_delta < 0 {
                format!("{} ({:+})", total, total_delta).green().to_string()
            } else {
                total.to_string()
            };
            let score_str = if score >= 80 {
                score.to_string().green().to_string()
            } else if score >= 60 {
                score.to_string().yellow().to_string()
            } else {
                score.to_string().red().to_string()
            };
            vec![
                timestamp,
                score_str,
                total_str,
                format!("{:.0}%", auto_share),
                q_samples.to_string(),
                dead_letter.to_string(),
            ]
        })
        .collect();
    println!(
        "{}",
        crate::fmt::pretty_table(
            &["TIME (UTC)", "SCORE", "HYPS", "AUTO%", "Q-N", "DEAD"],
            &rows
        )
    );
    Ok(())
}

async fn run_status(json: bool, watch: Option<u64>, no_history: bool) -> Result<()> {
    loop {
        render_status_once(json, !no_history).await?;
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

/// Compute a snapshot value and append it to history without rendering
/// any output. Used by the daemon tick so the convergence series
/// accumulates passively. Returns the snapshot for any caller that
/// wants to inspect it without the side-effect of printing.
pub async fn record_history_snapshot() -> Result<serde_json::Value> {
    let snapshot = build_snapshot().await?;
    append_history(&snapshot)?;
    Ok(snapshot)
}

/// Build the homeostasis snapshot value (JSON object). Pure read of
/// hypotheses + Q-table + queue state; no rendering, no side effects.
async fn build_snapshot() -> Result<serde_json::Value> {
    use crate::commands::sched::list_brain_tasks;

    let repo = std::env::current_dir().context("resolve repo root for snapshot")?;
    let hypotheses = discover::discover(&repo).unwrap_or_default();
    let ranked = judge::rank(&hypotheses);
    let table = learn::load_q_table();

    let mut by_source: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for h in &hypotheses {
        *by_source.entry(format!("{:?}", h.source)).or_insert(0) += 1;
    }
    let detector_health: Vec<&Hypothesis> = hypotheses
        .iter()
        .filter(|h| h.evidence.get("detector_health").is_some())
        .collect();
    let q_starvation: Vec<&Hypothesis> = hypotheses
        .iter()
        .filter(|h| h.source == Source::QStarvation)
        .collect();

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

    let dead_letter_count = list_brain_tasks(Some("dead_letter"))
        .await
        .map(|t| t.len())
        .unwrap_or(0);
    let pending_count = list_brain_tasks(Some("pending"))
        .await
        .map(|t| t.len())
        .unwrap_or(0);

    let (q_total_samples, q_mean_reward) = if table.entries.is_empty() {
        (0_u64, 0.0_f64)
    } else {
        let total_samples: u64 = table.entries.values().map(|e| e.samples).sum();
        let total_reward: f64 = table.entries.values().map(|e| e.total_reward).sum();
        let mean = if total_samples == 0 { 0.0 } else { total_reward / total_samples as f64 };
        (total_samples, mean)
    };

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

    Ok(serde_json::json!({
        "timestamp": chrono::Utc::now().to_rfc3339(),
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
    }))
}

/// Build + render one homeostasis snapshot, optionally appending to
/// history. `record_history=false` is used by terminal `--watch` polls
/// (which would otherwise flood the file with redundant entries) and
/// any internal callers (the daemon tick already appends separately).
pub async fn render_status_once(json: bool, record_history: bool) -> Result<()> {
    let snapshot = build_snapshot().await?;
    if record_history {
        if let Err(e) = append_history(&snapshot) {
            eprintln!("  ! history append failed: {}", e);
        }
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&snapshot)?);
        return Ok(());
    }
    render_snapshot_table(&snapshot);
    Ok(())
}

/// Render a snapshot JSON object as a colored terminal table. Pulls the
/// fields it needs out of the snapshot rather than recomputing — keeps
/// the on-screen view in sync with the history JSONL.
fn render_snapshot_table(snapshot: &serde_json::Value) {
    use colored::Colorize;
    let homeostasis = snapshot
        .get("homeostasis_score")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let total = snapshot
        .get("hypotheses")
        .and_then(|v| v.get("total"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let auto_share = snapshot
        .get("hypotheses")
        .and_then(|v| v.get("auto_share_pct"))
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let by_source: std::collections::BTreeMap<String, u64> = snapshot
        .get("hypotheses")
        .and_then(|v| v.get("by_source"))
        .and_then(|v| v.as_object())
        .map(|o| {
            o.iter()
                .filter_map(|(k, v)| v.as_u64().map(|n| (k.clone(), n)))
                .collect()
        })
        .unwrap_or_default();
    let detector_health = snapshot
        .get("guards")
        .and_then(|v| v.get("detector_health"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let q_starvation = snapshot
        .get("guards")
        .and_then(|v| v.get("q_starvation"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let pending_count = snapshot
        .get("queue")
        .and_then(|v| v.get("pending"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let dead_letter_count = snapshot
        .get("queue")
        .and_then(|v| v.get("dead_letter"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let q_templates = snapshot
        .get("q_table")
        .and_then(|v| v.get("templates"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let q_total_samples = snapshot
        .get("q_table")
        .and_then(|v| v.get("total_samples"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let q_mean_reward = snapshot
        .get("q_table")
        .and_then(|v| v.get("mean_reward"))
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);

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
    println!(
        "  Hypotheses:      {} total ({:.0}% auto-actionable)",
        total, auto_share
    );
    for (source, count) in &by_source {
        println!("    {} {}: {}", "·".dimmed(), source, count);
    }
    println!();
    let guard_line = if detector_health == 0 && q_starvation == 0 {
        "  Self-monitor:    surface clean (no detector_health or q_starvation findings)"
            .green()
            .to_string()
    } else {
        format!(
            "  Self-monitor:    {} detector_health, {} q_starvation",
            detector_health, q_starvation
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
            q_templates, q_total_samples, reward_colored
        );
    }
    println!();
    if let Some(top) = snapshot.get("top_hypothesis").and_then(|v| v.as_object()) {
        let source = top.get("source").and_then(|v| v.as_str()).unwrap_or("?");
        let scope = top.get("scope").and_then(|v| v.as_str()).unwrap_or("?");
        let score = top.get("score").and_then(|v| v.as_u64()).unwrap_or(0);
        println!(
            "  Top hypothesis:  {} {} (score {})",
            source, scope, score
        );
    }
    println!();
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
