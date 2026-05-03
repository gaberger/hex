//! Improver act phase (ADR-2604271100 P4).
//!
//! Takes a ranked stream from [`judge::rank`] and turns each top-N
//! hypothesis into a concrete action — a sched task or a recommendation
//! the operator can apply. The mapping from (source, evidence) to action
//! is deliberately a closed switch rather than free-form inference, so
//! the action surface is reviewable without running the loop.
//!
//! Two modes:
//!
//! * `act --dry-run` (default) — print the proposed actions without
//!   enqueueing. Lets operators preview what `--apply` would do.
//! * `act --apply`  — actually enqueue the proposed sched tasks. Each
//!   task carries a priority bump derived from the hypothesis score so
//!   it jumps the queue per the priority drain order.
//!
//! Every proposed action carries `derived_from: <hyp_id>` so the same
//! hypothesis re-firing on a later tick is deduped via the existing
//! sched-task dedup (kind + payload + project_id).
//!
//! [`judge::rank`]: super::judge::rank

use anyhow::{Context, Result};
use serde::Serialize;

use super::discover::Source;
use super::judge::ScoredHypothesis;

/// One proposed action — either an enqueueable sched task or an
/// operator-only recommendation when no auto-mapping is safe.
#[derive(Debug, Clone, Serialize)]
pub struct Action {
    pub kind: ActionKind,
    pub priority: u8,
    pub payload: String,
    pub derived_from: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    /// Enqueue as a sched shell task. Payload is the shell command line.
    SchedShell,
    /// Enqueue as a sched workplan task. Payload is the workplan path.
    SchedWorkplan,
    /// Auto-draft a workplan stub capturing fix-the-loop work for a
    /// degradation that act() can't safely auto-resolve (broken detector
    /// surface, starved action template). Payload is the prompt string.
    /// Closes the homeostatic loop: the system surfaces drift in its own
    /// machinery as actionable workplan drafts the operator can promote
    /// via /hex-feature-dev. Dedup-tracked at
    /// ~/.hex/improver/drafted-hypotheses.json so a recurring hypothesis
    /// only produces one draft per scope.
    DraftWorkplan,
    /// No safe auto-action — surface to operator. Payload is a human-
    /// readable recommendation.
    Recommend,
}

/// Derive an action for one ranked hypothesis. Returns `None` when the
/// hypothesis carries insufficient evidence for a safe mapping (rare —
/// usually means the evidence shape changed without updating this switch).
pub fn derive(scored: &ScoredHypothesis) -> Option<Action> {
    let h = &scored.hypothesis;
    // Score → priority: top of stack (>=80) gets priority 9, mid 60–79
    // gets 5, anything below stays at 0 (FIFO with normal traffic).
    let priority: u8 = if scored.score >= 80 {
        9
    } else if scored.score >= 60 {
        5
    } else {
        0
    };

    // Detector_health hypotheses auto-draft a workplan capturing the
    // fix-the-detector-surface work. The improver can't add CLI flags
    // by itself, but it can ensure the work is queued so it doesn't sit
    // invisible. Dedup is handled by act() so the same broken detector
    // doesn't generate a draft every tick.
    if h.evidence.get("detector_health").is_some() {
        let cmd = h
            .evidence
            .get("cmd")
            .and_then(|v| v.as_str())
            .unwrap_or("(unknown)");
        let detail = h
            .evidence
            .get("detail")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        return Some(Action {
            kind: ActionKind::DraftWorkplan,
            priority,
            payload: format!(
                "Fix improver detector surface for source {:?}: command `{}` doesn't produce parseable JSON. Detail: {}. Either add the missing CLI flag in hex-cli, point detectors.toml at a working command, or relax the detector contract. The detector currently emits a synthetic detector_health hypothesis on every tick; resolving this clears that drift signal.",
                h.source, cmd, detail
            ),
            derived_from: h.id.clone(),
            reason: scored.reason.clone(),
        });
    }

    match h.source {
        // ADR registry findings → run the doctor in fix mode against the
        // specific ADR. Tier-A safe fixes get applied; Tier-B/C land notes.
        Source::AdrDoctor => Some(Action {
            kind: ActionKind::SchedShell,
            priority,
            payload: format!("hex adr doctor --fix --strict"),
            derived_from: h.id.clone(),
            reason: scored.reason.clone(),
        }),

        // Lifecycle: read-only diagnostic actions — never mutate ADR text.
        // - Proposed → enqueue `hex adr review <scope>` so the reviewer
        //   agent surfaces what's missing for promotion (cheap + safe).
        // - unparseable_status → enqueue `hex adr doctor --fix --strict`
        //   which the existing AdrDoctor act path also produces; safe
        //   because doctor's fix tier-A is non-destructive.
        // - Abandoned/Superseded → operator decision (link replacement /
        //   verify backlink); recommend only.
        Source::AdrLifecycle => {
            let kind_str = h.evidence.get("kind").and_then(|v| v.as_str()).unwrap_or("");
            match kind_str {
                "proposed" => Some(Action {
                    kind: ActionKind::SchedShell,
                    priority,
                    payload: format!("hex adr review {}", h.scope),
                    derived_from: h.id.clone(),
                    reason: scored.reason.clone(),
                }),
                "unparseable_status" => Some(Action {
                    kind: ActionKind::SchedShell,
                    priority,
                    payload: format!("hex adr doctor --fix --strict"),
                    derived_from: h.id.clone(),
                    reason: scored.reason.clone(),
                }),
                "abandoned" => Some(Action {
                    kind: ActionKind::Recommend,
                    priority,
                    payload: format!(
                        "Abandoned ADR {}: link a replacement or document why no replacement exists",
                        h.scope
                    ),
                    derived_from: h.id.clone(),
                    reason: scored.reason.clone(),
                }),
                "superseded" => Some(Action {
                    kind: ActionKind::Recommend,
                    priority,
                    payload: format!(
                        "Superseded ADR {}: ensure the successor is linked back",
                        h.scope
                    ),
                    derived_from: h.id.clone(),
                    reason: scored.reason.clone(),
                }),
                _ => Some(Action {
                    kind: ActionKind::Recommend,
                    priority,
                    payload: format!("ADR {} lifecycle issue ({})", h.scope, kind_str),
                    derived_from: h.id.clone(),
                    reason: scored.reason.clone(),
                }),
            }
        }

        // Workplan evidence drift → enqueue an audit reconcile so the
        // workplan JSON gets demoted to match git evidence.
        Source::ReconcileStrict => {
            let workplan_id = h
                .evidence
                .get("workplan_id")
                .and_then(|v| v.as_str())
                .unwrap_or(&h.scope);
            Some(Action {
                kind: ActionKind::SchedShell,
                priority,
                payload: format!(
                    "hex plan reconcile {}.json --audit --update --strict",
                    workplan_id
                ),
                derived_from: h.id.clone(),
                reason: scored.reason.clone(),
            })
        }

        // Stale worktrees → recommend cleanup; never auto-delete because
        // worktree cleanup can destroy uncommitted work (ADR-2604150130).
        Source::GitDrift => {
            let branch = h
                .evidence
                .get("branch")
                .and_then(|v| v.as_str())
                .unwrap_or(&h.scope);
            Some(Action {
                kind: ActionKind::Recommend,
                priority,
                payload: format!(
                    "stale worktree branch `{}`: review and run `hex worktree merge` or cleanup",
                    branch
                ),
                derived_from: h.id.clone(),
                reason: scored.reason.clone(),
            })
        }

        // Stale inbox notifications → recommend operator triage; ack is a
        // human decision (P2 critical especially).
        Source::InboxStale => Some(Action {
            kind: ActionKind::Recommend,
            priority,
            payload: format!(
                "inbox notification overdue: ack with `hex inbox ack <id>` or escalate"
            ),
            derived_from: h.id.clone(),
            reason: scored.reason.clone(),
        }),

        // Escalation: tier:model with high escalation rate → recommend
        // tier_models override in .hex/project.json. Don't auto-edit
        // config.
        Source::EscalationReport => {
            let tier = h.evidence.get("tier").and_then(|v| v.as_str()).unwrap_or("?");
            let model = h.evidence.get("model").and_then(|v| v.as_str()).unwrap_or("?");
            Some(Action {
                kind: ActionKind::Recommend,
                priority,
                payload: format!(
                    "{}:{} escalating frequently — pin a stronger model in `.hex/project.json` → inference.tier_models",
                    tier, model
                ),
                derived_from: h.id.clone(),
                reason: scored.reason.clone(),
            })
        }

        // Q-starvation: a template ran ≥3 times with non-positive mean
        // reward. Auto-draft a workplan to fix the (source, kind)
        // mapping in act::derive — the action runs but doesn't clear
        // the target hypothesis, so the mapping itself is the bug.
        // We can't auto-fix Rust source from here, but we can capture
        // the work as a draft so it doesn't sit invisible in the Q-
        // table waiting for operator attention.
        Source::QStarvation => {
            let template = h
                .evidence
                .get("template")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let mean = h
                .evidence
                .get("mean_reward")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            return Some(Action {
                kind: ActionKind::DraftWorkplan,
                priority,
                payload: format!(
                    "Fix improver action mapping `{}`: mean reward {:+.2} after ≥3 samples — the action runs but doesn't clear the target hypothesis. Review act::derive for this (source, kind) and either pick a different action, refine the action's payload, or change ActionKind from auto-enqueueable to Recommend. Add a regression test that asserts the hypothesis count drops after the action runs.",
                    template, mean
                ),
                derived_from: h.id.clone(),
                reason: scored.reason.clone(),
            });
        }

        // WorkplanIntegrity: a workplan file is missing required fields,
        // most likely from a destructive reconcile run. Auto-draft a
        // workplan capturing the corruption so an operator can restore
        // from git history. Critically, this finding's existence ALSO
        // signals (via the cross-source attribution in learn::observe_
        // and_reward) that the most recent SchedShell action targeting
        // this workplan was destructive — that template's Q-mean drops
        // accordingly, eventually triggering q_starvation if it persists.
        Source::WorkplanIntegrity => {
            let workplan_id = h
                .evidence
                .get("workplan_id")
                .and_then(|v| v.as_str())
                .unwrap_or(&h.scope);
            let kind_str = h
                .evidence
                .get("kind")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            return Some(Action {
                kind: ActionKind::DraftWorkplan,
                priority,
                payload: format!(
                    "Restore workplan integrity for `{}`: detected `{}`. Most recent SchedShell action against this workplan stripped required fields. Either restore the file from git (`git checkout HEAD~1 -- docs/workplans/{}.json`) and rerun the source ReconcileStrict action with a less aggressive flag, OR fix the reconcile mutator in hex-cli/src/commands/plan/reconcile.rs so --audit --update --strict only modifies status, never deletes title/strategy_hint/files.",
                    workplan_id, kind_str, workplan_id
                ),
                derived_from: h.id.clone(),
                reason: scored.reason.clone(),
            });
        }

        // LayerCoverage: a canonical architecture layer is missing.
        // Auto-draft a workplan to scaffold the missing layer. Drafts
        // live at docs/workplans/drafts/ ready for /hex-feature-dev
        // promotion to a real workplan that hex-coder agents execute.
        // This is the "hex develops the app" path: detection → draft →
        // (operator promotes) → workplan execution → layer exists.
        Source::LayerCoverage => {
            let layer = h
                .evidence
                .get("layer")
                .and_then(|v| v.as_str())
                .unwrap_or(&h.scope);
            let remediation = h
                .evidence
                .get("remediation")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            return Some(Action {
                kind: ActionKind::DraftWorkplan,
                priority,
                payload: format!(
                    "Add missing hexagonal-architecture layer `{}` to this project. {}. Each task should follow the existing layer conventions (file naming, imports, hexagonal boundary rules). Tier T2 is appropriate; the work is mostly scaffolding + adapter wiring with established patterns.",
                    layer, remediation
                ),
                derived_from: h.id.clone(),
                reason: scored.reason.clone(),
            });
        }

        // Punch-list items: each unrouted gap recommends the operator
        // route it (task id, draft path, or out-of-scope tag).
        Source::PunchList => {
            let line_no = h
                .evidence
                .get("line_no")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            Some(Action {
                kind: ActionKind::Recommend,
                priority,
                payload: format!(
                    "unrouted punch-list item at line {}: add a task id, draft path, or `(out-of-scope)` tag",
                    line_no
                ),
                derived_from: h.id.clone(),
                reason: scored.reason.clone(),
            })
        }
    }
}

/// Drafted-hypothesis dedup file: maps hypothesis_id → draft path so the
/// same broken detector or starved template doesn't generate a fresh
/// draft every tick. The improver-loop's contract: one degradation =
/// one open draft until the operator processes it.
fn drafted_hypotheses_path() -> Result<std::path::PathBuf> {
    let home = dirs::home_dir().context("resolve home dir")?;
    let dir = home.join(".hex/improver");
    std::fs::create_dir_all(&dir).ok();
    Ok(dir.join("drafted-hypotheses.json"))
}

fn load_drafted_hypotheses() -> std::collections::HashMap<String, String> {
    let Ok(path) = drafted_hypotheses_path() else { return Default::default() };
    let Ok(content) = std::fs::read_to_string(&path) else { return Default::default() };
    serde_json::from_str(&content).unwrap_or_default()
}

fn save_drafted_hypotheses(map: &std::collections::HashMap<String, String>) -> Result<()> {
    let path = drafted_hypotheses_path()?;
    let pretty = serde_json::to_string_pretty(map)?;
    std::fs::write(&path, pretty).context("write drafted-hypotheses")?;
    Ok(())
}

/// Top-level act() entry: derive actions for the top-N hypotheses (or all
/// of them if `n == 0`), optionally enqueueing the SchedShell/SchedWorkplan
/// actions when `apply == true`. Returns the action stream so callers can
/// render or persist it.
pub async fn act(
    ranked: &[ScoredHypothesis],
    n: usize,
    apply: bool,
) -> Result<Vec<Action>> {
    let take_n = if n == 0 { ranked.len() } else { n.min(ranked.len()) };
    let mut actions: Vec<Action> = ranked
        .iter()
        .take(take_n)
        .filter_map(derive)
        .collect();

    if apply {
        let mut drafted = load_drafted_hypotheses();
        for action in &mut actions {
            match action.kind {
                ActionKind::SchedShell => {
                    if let Err(e) = crate::commands::sched::enqueue_brain_task_pub(
                        "shell",
                        &action.payload,
                    )
                    .await
                    {
                        eprintln!("  ✗ enqueue failed for {}: {}", action.derived_from, e);
                    }
                }
                ActionKind::SchedWorkplan => {
                    if let Err(e) = crate::commands::sched::enqueue_brain_task_pub(
                        "workplan",
                        &action.payload,
                    )
                    .await
                    {
                        eprintln!("  ✗ enqueue failed for {}: {}", action.derived_from, e);
                    }
                }
                ActionKind::DraftWorkplan => {
                    // Skip if a draft already exists for this hypothesis
                    // — the same degradation shouldn't produce N drafts.
                    if drafted.contains_key(&action.derived_from) {
                        continue;
                    }
                    match crate::commands::plan::draft_plan_silent(&action.payload).await {
                        Ok(path) => {
                            let path_str = path.display().to_string();
                            drafted.insert(action.derived_from.clone(), path_str.clone());
                            println!("  ✓ drafted: {}", path_str);
                        }
                        Err(e) => {
                            eprintln!("  ✗ draft failed for {}: {}", action.derived_from, e);
                        }
                    }
                }
                ActionKind::Recommend => {
                    // Recommendations don't enqueue — they print only.
                }
            }
        }
        if let Err(e) = save_drafted_hypotheses(&drafted) {
            eprintln!("  ! save drafted-hypotheses: {}", e);
        }
        // Snapshot for the learn phase. The learn pass on the next sweep
        // reads this and credits resolved hypotheses → action templates.
        // We snapshot the full live hypothesis set so an action that
        // resolves a *different* hypothesis (rare but possible — fixing
        // one ADR can clear several lifecycle findings) is observable.
        let snap_hypotheses: Vec<super::discover::Hypothesis> =
            ranked.iter().map(|s| s.hypothesis.clone()).collect();
        if let Err(e) = super::learn::take_snapshot(&actions, &snap_hypotheses) {
            eprintln!("  ! snapshot for learn phase failed: {}", e);
        }
    }

    Ok(actions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::discover::{Hypothesis, Severity};
    use super::super::judge::score;
    use chrono::Utc;
    use serde_json::json;

    fn scored(source: Source, severity: Severity, evidence: serde_json::Value) -> ScoredHypothesis {
        let h = Hypothesis {
            id: format!("hyp-{:?}", source),
            source,
            scope: format!("scope-{:?}", source),
            severity,
            evidence,
            generated_at: Utc::now(),
        };
        let (s, r) = score(&h);
        ScoredHypothesis { hypothesis: h, score: s, reason: r }
    }

    #[test]
    fn detector_health_maps_to_draft_workplan_with_cmd_in_payload() {
        // Updated contract (homeostatic loop closure): detector_health
        // findings auto-draft a workplan capturing the fix-the-detector
        // work, rather than emitting a Recommend that sits invisible.
        // Dedup is handled at act() time so the same broken detector
        // produces one draft, not one per tick.
        let s = scored(
            Source::GitDrift,
            Severity::Error,
            json!({"detector_health": "spawn_or_exit_error", "cmd": "hex worktree list --stale"}),
        );
        let action = derive(&s).expect("derive action");
        assert!(matches!(action.kind, ActionKind::DraftWorkplan));
        assert!(action.payload.contains("hex worktree list --stale"));
    }

    #[test]
    fn adr_doctor_maps_to_sched_shell_doctor_invocation() {
        let s = scored(
            Source::AdrDoctor,
            Severity::Error,
            json!({"adr_id": "ADR-X"}),
        );
        let action = derive(&s).expect("derive action");
        assert!(matches!(action.kind, ActionKind::SchedShell));
        assert!(action.payload.contains("hex adr doctor"));
    }

    #[test]
    fn high_score_hypothesis_gets_priority_9() {
        // Detector health = score 95 → priority 9.
        let s = scored(
            Source::InboxStale,
            Severity::Error,
            json!({"detector_health": "non_json_stdout", "cmd": "x"}),
        );
        let action = derive(&s).expect("derive action");
        assert_eq!(action.priority, 9);
    }
}
