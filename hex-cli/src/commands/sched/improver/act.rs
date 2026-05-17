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

use super::discover::{Severity, Source};
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
    /// Drive `hex plan execute <workplan>` through the workplan task
    /// dispatcher (which routes through tiered inference + spawns
    /// hex-coder agents). Closes the gap between improver observation
    /// and the dev-pipeline coder/tester/judge swarms. Payload is the
    /// workplan path.
    ExecuteWorkplan,
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

        // TestCoverage: a source file has no tests (or empty tests).
        // Auto-draft a test-generation workplan that a tester swarm
        // consumes. Prompt captures the source path + the expected
        // test conventions (file naming, framework). The tester agent
        // generates behavioral tests covering happy path + edge cases.
        Source::TestCoverage => {
            let source = h
                .evidence
                .get("source")
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
                    "Generate tests for `{}` (currently {}). Use the project's existing test framework (vitest/jest for TS, cargo test for Rust). Cover: happy-path execution, input validation, error cases, and any state-transition rules visible in the source. Match the file-naming convention (sibling `<name>.test.<ext>`). Each test should assert observable behavior, not implementation details. Tier T2 (codegen) is appropriate.",
                    source, kind_str
                ),
                derived_from: h.id.clone(),
                reason: scored.reason.clone(),
            });
        }

        // BuildReadiness: typecheck or tests are failing. Auto-draft a
        // workplan capturing the errors so a downstream coder swarm can
        // fix them. Critical companion to LayerCoverage — without this,
        // the layer detector credits "structural existence" as success
        // even when the code in those layers doesn't compile.
        Source::BuildReadiness => {
            let gate = h
                .evidence
                .get("gate")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let preview = h
                .evidence
                .get("errors_preview")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let language = h
                .evidence
                .get("language")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            return Some(Action {
                kind: ActionKind::DraftWorkplan,
                priority,
                payload: format!(
                    "Fix {} `{}` failures in this {} project. Compiler/test output:\n\n{}\n\nWork should resolve each error in turn, re-run the gate after each fix, and target zero violations before completion. Tier T2 (codegen) for straightforward type errors; T2.5 (complex reasoning) for cross-adapter or interface-redesign issues.",
                    language, gate, language, preview
                ),
                derived_from: h.id.clone(),
                reason: scored.reason.clone(),
            });
        }

        // BS-5: thought-pattern findings — cross-persona signals from
        // agent_thought. Always Recommend (operator-facing) because the
        // pattern is a *signal* (this ADR keeps coming up; persona X is
        // frustrated), not a remediation. Routing this through
        // DraftWorkplan would be premature: the operator (or another
        // persona via SOP) decides whether the signal warrants a code
        // change, an ADR update, or a process change.
        Source::ThoughtPattern => {
            let pattern = h
                .evidence
                .get("pattern")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let count = h
                .evidence
                .get("count")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let roles_summary = h
                .evidence
                .get("mentioning_roles")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            let payload = match pattern {
                "adr_repetition" => format!(
                    "Cross-persona signal: {} is referenced in {} recent thoughts by [{}]. Likely an unresolved architectural concern — review the ADR's status and decide whether to (a) ship the implementation, (b) split it into actionable sub-ADRs, or (c) update its Status to Abandoned with a rationale.",
                    h.scope, count, roles_summary
                ),
                "frustration_spike" => format!(
                    "Frustration spike: {} kind=frustration thoughts in the recent window. Inspect the source persona(s) for a methodology block (missing tool, recurrent error, resource exhaustion) before more code is generated against the same gap.",
                    count
                ),
                other => format!(
                    "ThoughtPattern '{}' on scope {} (count={}). Inspect recent agent_thought rows for context.",
                    other, h.scope, count
                ),
            };
            return Some(Action {
                kind: ActionKind::Recommend,
                priority,
                payload,
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
                ActionKind::ExecuteWorkplan => {
                    // Same plumbing as SchedWorkplan — both go through the
                    // brain-task `workplan` kind dispatcher in nexus, which
                    // routes through tiered inference + spawns hex-coder
                    // agents. Distinct ActionKind preserved so judge() and
                    // learn() can see "this was a code-generation action"
                    // separately from a metadata reconcile.
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
                    // Drafting itself dedups (same degradation → one draft),
                    // but the inbox notify must fire even on a dedup hit:
                    // the build is still red, the operator still needs to
                    // know, and one inbox entry per failure-recurrence is
                    // the explicit "fail through" signal we want.
                    let existing_path = drafted.get(&action.derived_from).cloned();
                    if let Some(path_str) = existing_path {
                        notify_inbox_for_action(ranked, action, &path_str).await;
                        continue;
                    }
                    match crate::commands::plan::draft_plan_silent(&action.payload).await {
                        Ok(path) => {
                            let path_str = path.display().to_string();
                            drafted.insert(action.derived_from.clone(), path_str.clone());
                            println!("  ✓ drafted: {}", path_str);
                            notify_inbox_for_action(ranked, action, &path_str).await;
                        }
                        Err(e) => {
                            eprintln!("  ✗ draft failed for {}: {}", action.derived_from, e);
                        }
                    }
                }
                ActionKind::Recommend => {
                    // Recommendations don't enqueue any work — but high-signal
                    // patterns push a P2 inbox notification so the operator
                    // sees them on next interaction instead of having to run
                    // `hex sched improver act` manually. BS-5 ThoughtPattern
                    // findings at severity=error are the case worth this:
                    // ≥5 personas circling the same ADR is a real-time
                    // attention signal, not a passive backlog item.
                    notify_inbox_for_recommend(ranked, action).await;
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

/// Best-effort inbox notification for high-signal drafts.
///
/// We notify on:
/// - `detector_health` hypotheses — broken detectors are the loop's
///   silent-failure mode; without a notification they sit invisible
///   in `docs/workplans/drafts/` until the operator runs `hex plan
///   drafts list`.
/// - `BuildReadiness` hypotheses — the build is broken (typecheck or
///   tests). Same rationale: a green-CI illusion is worse than a noisy
///   inbox.
///
/// Failures here are logged but never propagated — a flaky nexus
/// shouldn't break the act phase.
async fn notify_inbox_for_action(
    ranked: &[ScoredHypothesis],
    action: &Action,
    draft_path: &str,
) {
    let Some(scored) = ranked
        .iter()
        .find(|s| s.hypothesis.id == action.derived_from)
    else {
        return;
    };
    let h = &scored.hypothesis;
    let is_detector_health = h.evidence.get("detector_health").is_some();
    let is_build_readiness = matches!(h.source, Source::BuildReadiness);
    if !is_detector_health && !is_build_readiness {
        return;
    }

    let (kind, summary) = if is_detector_health {
        let cmd = h
            .evidence
            .get("cmd")
            .and_then(|v| v.as_str())
            .unwrap_or("(unknown)");
        (
            "detector_health",
            format!(
                "improver detector `{:?}` is silent-failing (cmd: {}). draft: {}",
                h.source, cmd, draft_path
            ),
        )
    } else {
        let gate = h
            .evidence
            .get("gate")
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        (
            "build_readiness",
            format!(
                "build is failing (gate={}, score={}). draft: {}",
                gate, scored.score, draft_path
            ),
        )
    };

    let nexus = crate::nexus_client::NexusClient::from_env();
    if nexus.ensure_running().await.is_err() {
        return;
    }
    let project_id = std::env::current_dir()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "default".to_string());

    // Try the inbox API first (operator inbox if an agent is registered).
    let inbox_body = serde_json::json!({
        "priority": 2,
        "kind": kind,
        "payload": summary,
        "project_id": project_id,
    });
    let inbox_ok = nexus
        .post("/api/hexflo/inbox/notify", &inbox_body)
        .await
        .is_ok();

    // Always also emit a `loop_notification` event so the failure is
    // visible in `hex sched watch`-style streams even when the daemon
    // has no registered agent (HexFlo inbox is auth-gated, /api/events
    // is not). This is the load-bearing "fail through to the user"
    // signal — silent inbox 401s used to swallow it entirely.
    let port = std::env::var("HEX_NEXUS_PORT")
        .unwrap_or_else(|_| "5555".to_string())
        .parse::<u16>()
        .unwrap_or(5555);
    let event_url = format!("http://127.0.0.1:{}/api/events", port);
    let session_id = std::env::var("CLAUDE_SESSION_ID")
        .unwrap_or_else(|_| format!("sched-daemon-{}", std::process::id()));
    let event_body = serde_json::json!({
        "session_id": session_id,
        "event_type": "loop_notification",
        "input_json": serde_json::to_string(&serde_json::json!({
            "priority": 2,
            "kind": kind,
            "summary": summary,
            "draft": draft_path,
            "inbox_ok": inbox_ok,
            "source": format!("{:?}", h.source),
            "score": scored.score,
        })).unwrap_or_default(),
    });
    let _ = reqwest::Client::new()
        .post(&event_url)
        .json(&event_body)
        .timeout(std::time::Duration::from_secs(2))
        .send()
        .await;

    println!(
        "  ⬡ loop notify [{}] inbox={} draft={}",
        kind, inbox_ok, draft_path
    );
}

/// File-backed dedup so the same Recommend hypothesis doesn't re-spam
/// the inbox on every daemon tick. Maps hypothesis_id → ISO-8601 ts of
/// last notification. Re-notify after 24h so persistent patterns
/// (e.g. an ADR that stays open for days) still surface.
fn notified_recommends_path() -> Result<std::path::PathBuf> {
    let home = dirs::home_dir().context("resolve home dir")?;
    let dir = home.join(".hex/improver");
    std::fs::create_dir_all(&dir).ok();
    Ok(dir.join("notified-recommends.json"))
}

fn load_notified_recommends() -> std::collections::HashMap<String, String> {
    let Ok(path) = notified_recommends_path() else { return Default::default() };
    let Ok(content) = std::fs::read_to_string(&path) else { return Default::default() };
    serde_json::from_str(&content).unwrap_or_default()
}

fn save_notified_recommends(map: &std::collections::HashMap<String, String>) {
    let Ok(path) = notified_recommends_path() else { return };
    if let Ok(pretty) = serde_json::to_string_pretty(map) {
        let _ = std::fs::write(&path, pretty);
    }
}

/// 24-hour re-notify window. Long enough that a daemon ticking every 30s
/// doesn't spam (≤1 notification per hypothesis per day), short enough
/// that a still-open ADR after a day re-surfaces in case the operator
/// missed the first ping.
const NOTIFY_RECOMMEND_COOLDOWN: chrono::Duration = chrono::Duration::hours(24);

/// Autonomous notify pass for the daemon — runs every tick and pushes
/// any severity=error Recommend Actions to inbox + events, with dedup.
/// Public surface so `hex sched daemon` can call it independently of the
/// score≥80 auto-act gate (which is reserved for high-confidence
/// remediations; Recommends are signals, not remediations).
pub async fn notify_high_severity_recommends(ranked: &[ScoredHypothesis]) {
    let mut map = load_notified_recommends();
    let now = chrono::Utc::now();
    let mut dirty = false;
    for scored in ranked {
        if !matches!(scored.hypothesis.severity, Severity::Error) {
            continue;
        }
        let Some(action) = derive(scored) else { continue };
        if !matches!(action.kind, ActionKind::Recommend) {
            continue;
        }
        // Cooldown check.
        if let Some(prev) = map.get(&action.derived_from) {
            if let Ok(prev_ts) = chrono::DateTime::parse_from_rfc3339(prev) {
                if now.signed_duration_since(prev_ts.with_timezone(&chrono::Utc))
                    < NOTIFY_RECOMMEND_COOLDOWN
                {
                    continue;
                }
            }
        }
        notify_inbox_for_recommend(ranked, &action).await;
        map.insert(action.derived_from.clone(), now.to_rfc3339());
        dirty = true;
    }
    if dirty {
        save_notified_recommends(&map);
    }
}

/// Inbox/event push for Recommend actions that are urgent enough to
/// interrupt the operator. Currently fires only on `ThoughtPattern`
/// findings at severity=error (the BS-5 escalation point).
///
/// Distinct from `notify_inbox_for_action`:
///   - no `draft_path` (Recommends don't produce drafts)
///   - filter is severity-based, not source-based
///   - kind tag is the underlying pattern (`adr_repetition`,
///     `frustration_spike`, …) so an inbox reader can filter
///
/// Failures swallowed; the action's payload is also printed to stdout
/// by the caller, so the operator can still see the recommendation
/// via `hex sched improver act` even if the inbox/event push fails.
async fn notify_inbox_for_recommend(ranked: &[ScoredHypothesis], action: &Action) {
    let Some(scored) = ranked.iter().find(|s| s.hypothesis.id == action.derived_from) else {
        return;
    };
    let h = &scored.hypothesis;
    if !matches!(h.source, Source::ThoughtPattern) {
        return;
    }
    if !matches!(h.severity, Severity::Error) {
        return;
    }

    let pattern = h
        .evidence
        .get("pattern")
        .and_then(|v| v.as_str())
        .unwrap_or("thought_pattern");
    let count = h
        .evidence
        .get("count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let summary = format!(
        "ThoughtPattern[{}] severity=error on {} (count={}) — {}",
        pattern,
        h.scope,
        count,
        action.payload
    );

    let nexus = crate::nexus_client::NexusClient::from_env();
    if nexus.ensure_running().await.is_err() {
        return;
    }
    let project_id = std::env::current_dir()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "default".to_string());

    let inbox_body = serde_json::json!({
        "priority": 2,
        "kind": format!("thought_pattern:{}", pattern),
        "payload": summary,
        "project_id": project_id,
    });
    let inbox_ok = nexus
        .post("/api/hexflo/inbox/notify", &inbox_body)
        .await
        .is_ok();

    // Same dual-path as notify_inbox_for_action: also publish to
    // /api/events so streams without a registered agent still see it.
    let port = std::env::var("HEX_NEXUS_PORT")
        .unwrap_or_else(|_| "5555".to_string())
        .parse::<u16>()
        .unwrap_or(5555);
    let event_url = format!("http://127.0.0.1:{}/api/events", port);
    let session_id = std::env::var("CLAUDE_SESSION_ID")
        .unwrap_or_else(|_| format!("sched-daemon-{}", std::process::id()));
    let event_body = serde_json::json!({
        "session_id": session_id,
        "event_type": "loop_notification",
        "input_json": serde_json::to_string(&serde_json::json!({
            "priority": 2,
            "kind": format!("thought_pattern:{}", pattern),
            "summary": summary,
            "scope": h.scope,
            "count": count,
            "inbox_ok": inbox_ok,
            "source": format!("{:?}", h.source),
            "score": scored.score,
        }))
        .unwrap_or_default(),
    });
    let _ = reqwest::Client::new()
        .post(&event_url)
        .json(&event_body)
        .timeout(std::time::Duration::from_secs(2))
        .send()
        .await;

    println!(
        "  ⬡ recommend notify [{}] inbox={} scope={}",
        pattern, inbox_ok, h.scope
    );
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
