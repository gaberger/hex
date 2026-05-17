//! `hex insight` — recursive insight routing (ADR-2604142345).
//!
//! The agent emits `★ Insight` blocks during non-trivial turns. These blocks
//! carry architectural observations, actionable gaps, meta-patterns, and
//! failure-mode notes — the highest-signal self-observation the system
//! produces. This module provides the *extractor* — the first half of the
//! spinal cord that turns those blocks into durable, routable artifacts.
//!
//! Phase I1 (this module) covers:
//!
//! 1. The [`Insight`] struct + its supporting enums.
//! 2. The [`extract_insights`] function that parses `★ Insight` blocks from
//!    assistant text, tolerating both structured YAML bodies and legacy prose.
//! 3. PostToolUse hook wiring lives in `hex-cli/src/commands/hook.rs` and
//!    calls into [`extract_insights`] directly.
//!
//! Later phases (I2–I5) add a classifier, router, and closure reconciler.
//! See `docs/workplans/wp-insight-routing.json` for the full plan.

pub mod extractor;

use chrono::{DateTime, Utc};
use clap::Subcommand;
use serde::{Deserialize, Serialize};

/// `hex insight` subcommands. The improver's `punch_list` detector calls
/// `hex insight punch-list --json` to harvest gap/todo items the agent
/// has acknowledged in its own output. Empty findings array when no
/// transcripts exist or no gaps were found — that's a healthy signal,
/// not a failure.
#[derive(Subcommand)]
pub enum InsightAction {
    /// Scan recent assistant transcripts for self-acknowledged punch-list
    /// items (numbered/bullet enumerations of gaps, todos, follow-ups).
    /// Each item without a routing reference (`(out-of-scope)`, task-id,
    /// draft path) becomes a finding for the improver.
    PunchList {
        /// Emit findings as JSON for the improver detector pipeline
        /// (`{findings: [{line_no, raw, classification, severity}]}`).
        #[arg(long)]
        json: bool,
    },
}

pub async fn run(action: InsightAction) -> anyhow::Result<()> {
    match action {
        InsightAction::PunchList { json } => punch_list_run(json).await,
    }
}

/// Scan the most recent Claude Code transcript JSONL files for assistant
/// turns and extract punch-list items via the existing `extract_punch_list`
/// pure function in `commands::hook::punch_list`. Read-only.
///
/// Transcripts live at `~/.claude/projects/<project>/*.jsonl` per Claude
/// Code conventions. We scan the most recent file (or files modified in
/// the last hour) so the detector reflects current-session gaps, not
/// historical ones the user has already routed elsewhere.
async fn punch_list_run(json: bool) -> anyhow::Result<()> {
    use crate::commands::hook::punch_list::{extract_punch_list, Reference};
    use std::path::PathBuf;

    let mut findings: Vec<serde_json::Value> = Vec::new();

    let transcripts_root: Option<PathBuf> = dirs::home_dir().map(|h| h.join(".claude/projects"));
    if let Some(root) = transcripts_root.filter(|r| r.is_dir()) {
        // Find recently-modified JSONL files across all project dirs.
        let cutoff = std::time::SystemTime::now()
            .checked_sub(std::time::Duration::from_secs(60 * 60))
            .unwrap_or(std::time::UNIX_EPOCH);
        let mut recent_files: Vec<PathBuf> = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&root) {
            for project in entries.flatten() {
                let project_path = project.path();
                if !project_path.is_dir() {
                    continue;
                }
                let Ok(jsonls) = std::fs::read_dir(&project_path) else { continue };
                for j in jsonls.flatten() {
                    let p = j.path();
                    if p.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                        continue;
                    }
                    let Ok(meta) = j.metadata() else { continue };
                    if let Ok(modified) = meta.modified() {
                        if modified >= cutoff {
                            recent_files.push(p);
                        }
                    }
                }
            }
        }

        for path in recent_files {
            let Ok(content) = std::fs::read_to_string(&path) else { continue };
            for line in content.lines() {
                let Ok(turn): Result<serde_json::Value, _> = serde_json::from_str(line) else { continue };
                // Claude Code transcript format: assistant turns have
                // `role == "assistant"` and `content` either a string or
                // an array of content blocks with `text` fields.
                if turn.get("role").and_then(|v| v.as_str()) != Some("assistant") {
                    continue;
                }
                let text = if let Some(s) = turn.get("content").and_then(|v| v.as_str()) {
                    s.to_string()
                } else if let Some(blocks) = turn.get("content").and_then(|v| v.as_array()) {
                    blocks
                        .iter()
                        .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                        .collect::<Vec<_>>()
                        .join("\n")
                } else {
                    continue;
                };
                for item in extract_punch_list(&text) {
                    let unrouted = item
                        .references
                        .iter()
                        .all(|r| matches!(r, Reference::None));
                    if !unrouted {
                        continue;
                    }
                    findings.push(serde_json::json!({
                        "line_no": item.line_no,
                        "raw": item.raw,
                        "classification": format!("{:?}", item.classification),
                        "severity": "warning",
                        "remediation": "add a task id, draft path, or (out-of-scope) tag",
                    }));
                }
            }
        }
    }

    if json {
        println!("{}", serde_json::json!({"findings": findings}));
    } else {
        println!("Punch list: {} unrouted item(s)", findings.len());
        for f in &findings {
            println!(
                "  L{}: {}",
                f.get("line_no").and_then(|v| v.as_u64()).unwrap_or(0),
                f.get("raw").and_then(|v| v.as_str()).unwrap_or("").lines().next().unwrap_or(""),
            );
        }
    }
    Ok(())
}

/// A single extracted insight — either structured (parsed from YAML) or a
/// best-effort fallback around legacy prose.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Insight {
    /// Stable identifier. For structured insights this is author-supplied;
    /// for fallback extractions it is synthesized as
    /// `insight-<session>-<turn:03>`.
    pub id: String,
    pub kind: InsightKind,
    pub content: String,
    pub route_to: RouteTarget,
    pub estimated_tier: Tier,
    #[serde(default)]
    pub depends_on: Vec<String>,
    pub source_session: String,
    pub source_turn: usize,
    pub created_at: DateTime<Utc>,
}

/// Classification of what an insight *is*. Informs routing but is not the
/// same thing — an `ActionableGap` can still route to `Memory` if it's a
/// duplicate, etc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InsightKind {
    ArchitecturalObservation,
    ActionableGap,
    MetaPattern,
    FailureMode,
    Duplicate,
}

/// Where a classified insight should be materialized. Decided by the
/// classifier in I2 — the extractor only records what the author said.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RouteTarget {
    Adr,
    Workplan,
    Memory,
    DuplicateOf(String),
    Skip,
}

/// Estimated execution tier per ADR-2604120202 tiered inference routing.
/// The author estimates; later phases can override.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Tier {
    T1,
    T2,
    T3,
}

pub use extractor::extract_insights;
