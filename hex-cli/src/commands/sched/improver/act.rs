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

use anyhow::Result;
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

    // Detector_health hypotheses propose the operator fix the broken
    // detector — there's no safe auto-action because we don't know which
    // CLI flag is missing.
    if h.evidence.get("detector_health").is_some() {
        let cmd = h
            .evidence
            .get("cmd")
            .and_then(|v| v.as_str())
            .unwrap_or("(unknown)");
        return Some(Action {
            kind: ActionKind::Recommend,
            priority,
            payload: format!(
                "fix detector surface: `{}` doesn't produce parseable JSON; \
                 add the missing CLI flag in hex-cli or update detectors.toml",
                cmd
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
        // reward — the action runs but doesn't clear the hypothesis.
        // Recommend operator review of act::derive for that mapping.
        // Never auto-act because the broken mapping is, by definition,
        // part of the act surface itself.
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
                kind: ActionKind::Recommend,
                priority,
                payload: format!(
                    "improver action template `{}` mean reward {:+.2} after ≥3 samples — review act::derive for that (source, kind) mapping, the action runs but doesn't clear the target hypothesis",
                    template, mean
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
                ActionKind::Recommend => {
                    // Recommendations don't enqueue — they print only.
                }
            }
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
    fn detector_health_maps_to_recommendation_with_cmd_in_payload() {
        let s = scored(
            Source::GitDrift,
            Severity::Error,
            json!({"detector_health": "spawn_or_exit_error", "cmd": "hex worktree list --stale"}),
        );
        let action = derive(&s).expect("derive action");
        assert!(matches!(action.kind, ActionKind::Recommend));
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
