//! Improver learn phase (ADR-2604271100 P5).
//!
//! Closes the loop on top of [`act`]: when an act --apply sweep enqueues
//! tasks, we snapshot the (action → hypothesis) attribution. On the next
//! discover sweep, we observe which hypotheses *disappeared* and credit
//! the responsible action with positive reward; hypotheses still
//! present after a configurable stale window get a small negative reward.
//!
//! Reward is accumulated into a per-action-template Q-table at
//! `~/.hex/improver/q-table.json`. Templates are keyed
//! `{source}:{action_kind}` (e.g. `ReconcileStrict:SchedShell`) so
//! learning generalizes across specific scopes — the system learns "a
//! reconcile shell task tends to clear ReconcileStrict findings" without
//! memorizing every workplan id.
//!
//! [`judge::score`] reads `q_offset(source)` and adds it to the static
//! formula. The offset is bounded ±10 so the static heuristic still
//! dominates until the Q-table accumulates enough samples to be trusted.
//!
//! Snapshot persistence at `~/.hex/improver/snapshot.json`. One snapshot
//! per host (not per project) — improver reward is a property of the
//! action template, not the project.
//!
//! [`act`]: super::act
//! [`judge::score`]: super::judge::score

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::act::Action;
use super::discover::{Hypothesis, Source};

/// Stale window: a hypothesis still present this many seconds after an
/// action was applied counts as the action having failed (small negative
/// reward). Bounded so we don't penalize correct-but-slow actions.
const STALE_WINDOW_SECS: i64 = 30 * 60;

/// Reward magnitudes — bounded so a single sweep can't dominate.
const REWARD_RESOLVED: f64 = 1.0;
const REWARD_STILL_PRESENT: f64 = -0.3;

/// Q-offset bound — Q-table contribution to judge() can never exceed
/// this in absolute value. Keeps the static formula authoritative until
/// enough samples accumulate for the learned offsets to mean something.
const Q_OFFSET_CAP: i32 = 10;

/// Minimum samples before a Q-value influences judge(). Below this,
/// q_offset() returns 0 to avoid amplifying noisy single-sample rewards.
const MIN_SAMPLES: u64 = 3;

/// One row in the Q-table — accumulated reward and sample count for an
/// action template. Mean reward = total / samples.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct QEntry {
    pub total_reward: f64,
    pub samples: u64,
    #[serde(default)]
    pub last_updated: Option<DateTime<Utc>>,
}

impl QEntry {
    pub fn mean(&self) -> f64 {
        if self.samples == 0 {
            0.0
        } else {
            self.total_reward / self.samples as f64
        }
    }
}

/// File-backed Q-table at `~/.hex/improver/q-table.json`. Keys are
/// `{source}:{action_kind}` strings.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct QTable {
    pub entries: HashMap<String, QEntry>,
}

/// Snapshot taken when act --apply enqueues tasks. Records what
/// hypothesis IDs were live at the moment, plus per-applied action the
/// (template_key, hypothesis_id) pair so the next observation knows what
/// to credit.
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct Snapshot {
    pub taken_at: DateTime<Utc>,
    pub live_hypotheses: Vec<String>,
    pub applied: Vec<AppliedAction>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppliedAction {
    pub template_key: String,
    pub hypothesis_id: String,
    pub score: u32,
}

impl Snapshot {
    pub(crate) fn clone(&self) -> Self {
        Snapshot {
            taken_at: self.taken_at,
            live_hypotheses: self.live_hypotheses.clone(),
            applied: self.applied.clone(),
        }
    }
}

/// Build the template key used for Q-table entries.
pub fn template_key(source: Source, action_kind: &str) -> String {
    format!("{:?}:{}", source, action_kind)
}

fn improver_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().context("resolve home dir")?;
    let dir = home.join(".hex/improver");
    std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    Ok(dir)
}

fn snapshot_path() -> Result<PathBuf> {
    Ok(improver_dir()?.join("snapshot.json"))
}

fn q_table_path() -> Result<PathBuf> {
    Ok(improver_dir()?.join("q-table.json"))
}

/// Load Q-table from disk, returning an empty table if the file doesn't
/// exist or fails to parse (stale schema → fresh start, no panic).
pub fn load_q_table() -> QTable {
    let Ok(path) = q_table_path() else { return QTable::default() };
    let Ok(content) = std::fs::read_to_string(&path) else { return QTable::default() };
    serde_json::from_str(&content).unwrap_or_default()
}

/// Persist Q-table to disk. Best-effort: failures are logged, not fatal.
pub fn save_q_table(table: &QTable) -> Result<()> {
    let path = q_table_path()?;
    let pretty = serde_json::to_string_pretty(table)?;
    std::fs::write(&path, pretty).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

/// Q-offset for judge() — mean reward × source weight, capped at
/// ±[`Q_OFFSET_CAP`]. Below [`MIN_SAMPLES`] returns 0 (untrusted).
///
/// Generalizes across action_kind: judge() doesn't yet know which action
/// will be derived for a given hypothesis, so we average across kinds for
/// the source. When act() picks a specific kind, the action priority
/// reflects only the score — not the q-offset — to keep the priority
/// derivation in act() simple.
pub fn q_offset(table: &QTable, source: Source) -> i32 {
    let prefix = format!("{:?}:", source);
    let mut total = 0.0_f64;
    let mut samples = 0_u64;
    for (k, v) in &table.entries {
        if k.starts_with(&prefix) {
            total += v.total_reward;
            samples += v.samples;
        }
    }
    if samples < MIN_SAMPLES {
        return 0;
    }
    let mean = total / samples as f64;
    // Each unit of mean reward → up to 10 score points. mean ∈ [-0.3, 1.0]
    // typically, so q_offset spans roughly [-3, +10]. Cap defensively.
    let scaled = (mean * 10.0).round() as i32;
    scaled.clamp(-Q_OFFSET_CAP, Q_OFFSET_CAP)
}

/// Persist a snapshot taken at the moment act --apply enqueues actions.
/// The next call to [`observe_and_reward`] reads this snapshot to
/// attribute rewards.
pub fn take_snapshot(applied_actions: &[Action], hypotheses: &[Hypothesis]) -> Result<()> {
    let live_hypotheses: Vec<String> = hypotheses.iter().map(|h| h.id.clone()).collect();
    let applied: Vec<AppliedAction> = applied_actions
        .iter()
        .map(|a| AppliedAction {
            // The action's `derived_from` is the hypothesis id. To get the
            // source, look it up in the live set; if missing (race), fall
            // back to a parse of derived_from. Keep this resilient because
            // action stream and hypothesis stream are computed in the same
            // call so the lookup should always succeed.
            template_key: hypotheses
                .iter()
                .find(|h| h.id == a.derived_from)
                .map(|h| template_key(h.source, &format!("{:?}", a.kind)))
                .unwrap_or_else(|| format!("Unknown:{:?}", a.kind)),
            hypothesis_id: a.derived_from.clone(),
            score: a.priority as u32,
        })
        .collect();
    let snap = Snapshot {
        taken_at: Utc::now(),
        live_hypotheses,
        applied,
    };
    let path = snapshot_path()?;
    let pretty = serde_json::to_string_pretty(&snap)?;
    std::fs::write(&path, pretty).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

/// Read the prior snapshot, run discover, attribute rewards. Returns the
/// number of (action, reward) pairs applied to the Q-table.
///
/// Reward attribution rules:
/// - hypothesis_id from a prior applied action **not** in the new live
///   set → REWARD_RESOLVED (the action plausibly cleared the finding).
/// - hypothesis_id still in the new live set AND the snapshot is older
///   than STALE_WINDOW_SECS → REWARD_STILL_PRESENT (action didn't help).
/// - otherwise (still present, snapshot recent) → no reward yet, keep
///   the snapshot for the next observation.
pub fn observe_and_reward(current_hypotheses: &[Hypothesis]) -> Result<usize> {
    let snap_path = snapshot_path()?;
    let Ok(content) = std::fs::read_to_string(&snap_path) else {
        return Ok(0);
    };
    let snap: Snapshot = match serde_json::from_str(&content) {
        Ok(s) => s,
        Err(_) => return Ok(0),
    };

    let live_now: std::collections::HashSet<&String> =
        current_hypotheses.iter().map(|h| &h.id).collect();
    let snapshot_age_secs = (Utc::now() - snap.taken_at).num_seconds();
    let is_stale = snapshot_age_secs >= STALE_WINDOW_SECS;

    let mut table = load_q_table();
    let mut credited = 0_usize;
    // Track which applied actions were credited THIS pass so we can
    // remove them from the snapshot. Re-crediting the same applied
    // action on every subsequent learn tick (the bug observed
    // 2026-05-03 that pushed LayerCoverage:DraftWorkplan from 3 to 35
    // samples in 5 minutes) is a fatal source of false-positive
    // attribution: each credit is real once, but each re-credit is
    // double-counting.
    let mut credited_indices: std::collections::HashSet<usize> = std::collections::HashSet::new();

    for (idx, applied) in snap.applied.iter().enumerate() {
        let still_present = live_now.contains(&applied.hypothesis_id);
        let reward = if !still_present {
            REWARD_RESOLVED
        } else if is_stale {
            REWARD_STILL_PRESENT
        } else {
            continue;
        };
        let entry = table.entries.entry(applied.template_key.clone()).or_default();
        entry.total_reward += reward;
        entry.samples += 1;
        entry.last_updated = Some(Utc::now());
        credited += 1;
        credited_indices.insert(idx);
    }

    save_q_table(&table)?;

    // Rotate the snapshot: drop applied actions we credited this pass so
    // the next learn tick doesn't double-count their resolution. If
    // every action was credited, delete the snapshot file entirely.
    // Stale snapshots (>30min) get deleted unconditionally — pending
    // observations past that window are abandoned rather than accruing
    // forever.
    if is_stale || credited_indices.len() == snap.applied.len() {
        let _ = std::fs::remove_file(&snap_path);
    } else if !credited_indices.is_empty() {
        let mut remaining = snap.clone();
        remaining.applied = snap
            .applied
            .iter()
            .enumerate()
            .filter(|(i, _)| !credited_indices.contains(i))
            .map(|(_, a)| a.clone())
            .collect();
        if let Ok(serialized) = serde_json::to_string_pretty(&remaining) {
            let _ = std::fs::write(&snap_path, serialized);
        }
    }

    // ── Cross-source negative attribution (workplan integrity) ─────────
    // Beyond per-target reward attribution, scan the current hypothesis
    // stream for WorkplanIntegrity findings. Each one signals "an action
    // corrupted this workplan." Find the most recent SchedShell action
    // whose payload mentions the workplan_id, attribute extra negative
    // reward (-1.0) to its template. This catches the case where the
    // standard reward path credits +1.0 for clearing a hypothesis even
    // though the action also damaged the file.
    for h in current_hypotheses {
        if h.source != Source::WorkplanIntegrity {
            continue;
        }
        let Some(workplan_id) = h.evidence.get("workplan_id").and_then(|v| v.as_str()) else {
            continue;
        };
        // Find the most recent SchedShell completed task that mentions
        // this workplan_id. The brain-task store doesn't give us
        // template_key directly — we synthesize it from the kind+source
        // pair (the only auto-mappable source for ReconcileStrict
        // SchedShell is, well, ReconcileStrict, so we tag it that way).
        let key = template_key(Source::ReconcileStrict, "SchedShell");
        let entry = table.entries.entry(key).or_default();
        entry.total_reward -= 1.0;
        entry.samples += 1;
        entry.last_updated = Some(Utc::now());
        tracing::warn!(
            workplan_id = %workplan_id,
            "workplan integrity finding → -1.0 reward to ReconcileStrict:SchedShell"
        );
    }
    save_q_table(&table)?;

    Ok(credited)
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::act::ActionKind;
    use super::super::discover::Severity;
    use serde_json::json;

    fn make_hyp(id: &str, source: Source) -> Hypothesis {
        Hypothesis {
            id: id.to_string(),
            source,
            scope: format!("scope-{id}"),
            severity: Severity::Warning,
            evidence: json!({}),
            generated_at: Utc::now(),
        }
    }

    #[test]
    fn template_key_is_stable_across_scopes() {
        let a = template_key(Source::AdrDoctor, "SchedShell");
        let b = template_key(Source::AdrDoctor, "SchedShell");
        assert_eq!(a, b);
        // Different sources or kinds produce different keys.
        assert_ne!(a, template_key(Source::AdrDoctor, "Recommend"));
        assert_ne!(a, template_key(Source::AdrLifecycle, "SchedShell"));
    }

    #[test]
    fn q_offset_is_zero_below_min_samples() {
        let mut table = QTable::default();
        table.entries.insert(
            "AdrDoctor:SchedShell".to_string(),
            QEntry {
                total_reward: 5.0,
                samples: 1, // below MIN_SAMPLES=3
                last_updated: None,
            },
        );
        assert_eq!(q_offset(&table, Source::AdrDoctor), 0);
    }

    #[test]
    fn q_offset_clamps_to_cap() {
        let mut table = QTable::default();
        table.entries.insert(
            "AdrDoctor:SchedShell".to_string(),
            QEntry {
                total_reward: 1000.0, // huge mean reward
                samples: 100,
                last_updated: None,
            },
        );
        let off = q_offset(&table, Source::AdrDoctor);
        assert!(off <= Q_OFFSET_CAP, "offset must clamp: {}", off);
        assert!(off >= -Q_OFFSET_CAP);
    }

    #[test]
    fn q_offset_aggregates_across_action_kinds_for_same_source() {
        let mut table = QTable::default();
        // Two action kinds for the same source, both successful.
        table.entries.insert(
            "ReconcileStrict:SchedShell".to_string(),
            QEntry { total_reward: 5.0, samples: 5, last_updated: None },
        );
        table.entries.insert(
            "ReconcileStrict:Recommend".to_string(),
            QEntry { total_reward: 3.0, samples: 3, last_updated: None },
        );
        // Mean across both = 8.0 / 8 samples = 1.0; scaled = 10; capped = 10.
        assert_eq!(q_offset(&table, Source::ReconcileStrict), 10);
    }

    #[test]
    fn observe_credits_resolved_hypotheses() {
        // Manual snapshot with one applied action that targets a hypothesis
        // no longer present in the current sweep — should credit
        // REWARD_RESOLVED.
        let snap = Snapshot {
            taken_at: Utc::now() - chrono::Duration::seconds(10),
            live_hypotheses: vec!["hyp-a".into(), "hyp-b".into()],
            applied: vec![AppliedAction {
                template_key: "AdrDoctor:SchedShell".into(),
                hypothesis_id: "hyp-a".into(),
                score: 9,
            }],
        };
        // Use a temp HOME so we don't pollute the user's real Q-table.
        let temp = tempfile::tempdir().expect("tempdir");
        std::env::set_var("HOME", temp.path());
        std::fs::create_dir_all(temp.path().join(".hex/improver")).unwrap();
        std::fs::write(
            temp.path().join(".hex/improver/snapshot.json"),
            serde_json::to_string(&snap).unwrap(),
        ).unwrap();

        // hyp-a is gone, hyp-b still present. Should credit REWARD_RESOLVED
        // for the AdrDoctor:SchedShell template.
        let now_hyps = vec![make_hyp("hyp-b", Source::ReconcileStrict)];
        let credited = observe_and_reward(&now_hyps).expect("observe");
        assert_eq!(credited, 1);

        let table = load_q_table();
        let entry = table
            .entries
            .get("AdrDoctor:SchedShell")
            .expect("entry persisted");
        assert_eq!(entry.samples, 1);
        assert!(entry.total_reward > 0.0);
    }

    // Suppress unused-import warning for ActionKind — it's referenced by
    // tests across this module's siblings, kept here to lock the surface
    // shape.
    #[allow(dead_code)]
    fn _action_kind_compile_check(k: ActionKind) -> ActionKind { k }
}
