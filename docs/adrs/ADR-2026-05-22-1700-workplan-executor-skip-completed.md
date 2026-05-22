# ADR-2026-05-22-1700 — workplan-executor-skip-completed

Status: **Accepted**
Date: 2026-05-22

## Context

On 2026-05-22, six consecutive dispatches of `wp-sop-pipeline-redesign-phase-1` died at Phase 1 even after the predecessor tasks had genuinely shipped code:

- Run 1: P1.2 inference timed out at 240s (T2).
- Run 2: P1.1 + P1.2 shipped real commits (`3dd23e16`, `4fbca2fb`, `cf128912`); phase gate then failed.
- Runs 3–5: re-dispatched, executor **re-ran P1.1 and P1.2 every time**, regenerated the same files, then tripped the post-phase gate when the prior fix-up commit (`cf128912 — register classifier_parser in mod.rs`) wasn't replayed.

`hex plan reconcile --update` correctly marked the completed tasks as done in the JSON file. But the workplan executor ignored that state and dispatched the tasks again.

### Root cause

Two cooperating bugs:

1. **String-naming mismatch between reconciler and domain types.** `hex-cli/src/commands/plan/reconcile.rs` writes the literal string `"done"` (lines 389, 401, 416, 518, 545). The canonical task-status enum in `hex-core/src/domain/workplan.rs::TaskStatus` serializes its `Completed` variant as `"completed"` (snake_case derive). When the executor deserialized a reconciled workplan, `"done"` matched no variant → fallback to `#[default] = Pending` → re-dispatch.

2. **Executor has its own local `WorkplanTask` struct.** `hex-nexus/src/orchestration/workplan_executor.rs` defines a private `WorkplanTask` with a Deserialize-only schema, separate from `hex_core::domain::workplan::WorkplanTask`. The local struct had NO `status` field, so even if (1) had been right, the executor wouldn't have seen it.

Combined effect: every re-dispatch was equivalent to a cold start.

## Decision

Two coordinated changes:

1. **`hex-core/src/domain/workplan.rs`** — add `#[serde(alias = "done")]` to `TaskStatus::Completed` and `#[serde(alias = "done")]` to `WorkplanStatus::Complete`. Accepts both the canonical strings (`completed`, `complete`) and the legacy reconciler string (`done`) without requiring a re-reconcile of existing on-disk workplans.

2. **`hex-nexus/src/orchestration/workplan_executor.rs`** — add a `status: String` field to the local `WorkplanTask` (with `#[serde(default)]` for backward compat), and add a skip-if-done check in the dispatch loop at line ~1023:

   ```rust
   if matches!(task.status.as_str(), "completed" | "done") {
       tracing::info!(
           task_id = %task_id,
           task_name = %task_name,
           status = %task.status,
           "Task SKIP — already done per workplan status (reconcile evidence)"
       );
       continue;
   }
   ```

## Why a string field, not the enum

The local executor `WorkplanTask` deliberately does not depend on `hex_core::domain::workplan::TaskStatus` because:

- the local struct is `Deserialize`-only, used as a wire-format adapter
- the `tier` field of the local struct already uses a custom enum (`TaskTier`)
- importing the domain enum here would create a cross-crate `match` on every dispatch; the string comparison is simpler and less ceremonial

Reconciler-side normalization (rewriting every `"done"` to `"completed"`) was rejected as a larger blast radius — every existing reconciled workplan on disk would need a re-reconcile pass to be skip-aware. The alias is one line and forward/backward compatible.

## Consequences

- **Re-runnable workplans.** `hex plan execute <file>` is now idempotent — already-done tasks log a single `Task SKIP` line and the dispatch advances to the next pending task. This is a load-bearing fix for the autonomous loop's "make progress every dispatch" guarantee.
- **Cost reduction.** A re-dispatched task no longer re-spawns a Claude Code subprocess (Path B) or burns Ollama inference (Path C). Measured on 2026-05-22: Phase 1 of `wp-sop-pipeline-redesign-phase-1` re-dispatched in <200ms with both P1.1 and P1.2 skipped, vs ~60s when each was re-run.
- **Tests added** (`hex-nexus/tests/tier_routing.rs`, `hex-cli/src/pipeline/agent_def.rs::parse_hex_coder_yaml`) cover the new field shape.
- **No reconciler change required** — the alias means existing on-disk workplans with `"done"` strings skip correctly without a migration pass.

## Verification

End-to-end (`wp-verify-loop-2026-05-22.json`, exec `02e979f6`):

- Workplan completed in 76s, single-task P1.1 (`Create build_banner module`).
- File `hex-nexus/src/build_banner.rs` landed, commit `7a55efb2` attributed.
- A re-fire of the same workplan logs `Task SKIP — already done` and exits cleanly without re-running inference.

## References

- Commit `67c42d4e` (initial implementation, alongside 3 other structural blockers)
- Commit `e2d6bb00` (test fixture updates)
- Reconciler write sites: `hex-cli/src/commands/plan/reconcile.rs:389, 401, 416, 518, 545`
- Domain type: `hex-core/src/domain/workplan.rs::TaskStatus`
