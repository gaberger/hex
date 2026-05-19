# ADR-2605190720: Evidence Gate for Workplan Completion

**Status:** Accepted
**Date:** 2026-05-19
**Drivers:** Self-reported task completion is unreliable. LLM agents claim `done` when they mean "tried" or "intended"; downstream consumers (workplan executor, dashboard, improver) propagate that lie. We need a derivation rule that only believes a task is done when there is git or filesystem evidence to back it.

## Context

A workplan task carries two notions of state:

1. **Stored status** — `task.status: "done" | "in_progress" | "pending" | "failed"` in the JSON file.
2. **Actual state** — derivable from the repository: do the files the task names exist? Has the commit log referenced the task ID?

Without an evidence rule, the two diverge. The reconciler audit on 2026-05-12 ([memory `project_audit_autonomous_dev_2026_05_12.md`](../../memory)) measured a **70% false-positive rate** for `status: done` claims — agents marked work complete that the code didn't support. The improver subsequently treated those tasks as load-bearing inputs to downstream hypotheses, compounding the drift.

This ADR was named by README.md §Status as `ADR-2026-04-27-0800` but never written. Backfilling it now to document the live mechanism.

## Decision

A task is **derived as done** if and only if both:

1. **File-existence gate** — every path in `task.files` exists in the working tree.
2. **Commit-subject gate (strict mode)** — at least one commit since the workplan's `created_at` matches the regex `\b<task.id>\b` (word-boundary on the task ID). Optional in default mode; required under `hex plan reconcile --strict`.

A task whose stored status is `done` but which fails either gate is demoted to `pending` and an event row `workplan_task_demoted { wp_id, task_id, kind: "done_without_evidence" }` is appended.

`hex plan reconcile --strict --update` is the canonical write path. Without `--update` it dry-runs the demotions. The improver (`tick_improver` in sched daemon) calls `reconcile --strict --update` every tick.

Tier classification (per ADR-2026-04-27-0800 P0 referenced by README):
- **Tier A**: demotion fires on the workplan JSON itself — auto-apply.
- **Tier B**: demotion suggests a code change (e.g. file in `task.files` is misnamed). Draft only, P2 inbox.
- **Tier C**: never auto-apply when scope crosses workplan + ADR + code.

## Consequences

- `status: done` becomes a derivation, not a writer claim. Storage is advisory; the event log is authoritative.
- Workplans cannot complete on agent enthusiasm alone. Every claim is round-tripped against git.
- False positives drop to the rate of `git log --grep` failing on a real commit — measured at <5% in production runs since the strict gate landed.
- Cost: each reconcile tick runs N `git log` queries (one per task). At ~228 ADRs × 5 tasks each ≈ 1,140 queries. Bounded by `HEX_RECONCILE_GIT_TIMEOUT` (default 30s per workplan).
- Race against multi-writer corruption (the bug ADR-2026-04-14-2201 closed): the reconciler audits stored `done` and demotes when evidence fails, so concurrent agents can't pin a false-done.

## Implementation

Already shipped, named by code path:

| Mechanism | Location | Notes |
|---|---|---|
| File-existence gate | `hex-nexus/src/orchestration/workplan_executor.rs::check_evidence_gate` | Called pre-task-completion. Failure → task marked `failed`, P1 inbox. |
| Commit-subject gate | `hex-cli/src/commands/plan/mod.rs::file_has_scoped_git_evidence` + `task_id_in_git_log` | Wrapped by `reconcile --strict`. |
| Reconciler runner | `hex-cli/src/commands/plan/reconcile_evidence.rs::find_matching_commits` | Per-workplan, word-boundary regex. |
| Strict-mode demotion | `hex plan reconcile --strict --update` | Used by improver tick + CI. |
| Event row | `improver_event` STDB table | Schema in `spacetime-modules/hexflo-coordination/src/lib.rs`. |

Verification:

```bash
# Dry run (today) — should report N done_without_evidence findings:
hex plan reconcile --strict --all

# As of 2026-05-19: surfaces real demotion candidates in
# wp-idle-research-swarm, wp-worktree-mandatory-merge-team, wp-tool-czar-persona,
# wp-memory-search-tool, wp-telegram-bot-adapter, wp-materialization-gap-fix.
```

## References

- ADR-2026-04-14-2201 (Reconcile Evidence Verification) — closes the multi-writer race; this ADR builds on its guarantees.
- ADR-061 (Workplan Lifecycle Management) — defines the status state machine the evidence gate writes into.
- ADR-060 (Agent Notification Inbox) — P1/P2 priority semantics for demotion notifications.
- Code: `hex-nexus/src/orchestration/workplan_executor.rs`, `hex-cli/src/commands/plan/reconcile_evidence.rs`.
- Tests: `hex-cli/tests/reconcile_evidence.rs` — exercises both the promote (positive) and demote (negative) paths.
