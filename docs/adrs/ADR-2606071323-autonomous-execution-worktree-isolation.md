# ADR-2606071323: Autonomous execution must isolate to its own git worktree — never the operator's session branch

**Status:** Completed
**Date:** 2026-06-07
**Epoch:** single-agent
**Drivers:** A live data-loss-class race observed 2026-06-07: with `autonomous.enabled: true` + `rollback_on_failure: true`, the running nexus executor committed and `git reset`-rolled-back directly on the operator's checked-out branch. A failed autonomous attempt's rollback reset swept an operator commit (ADR-2606071243, `e8437655`) out of the branch as collateral. Recovered only because the reset was `--mixed` (disk preserved). Root cause: `hex-nexus/src/direct_exec.rs::commit()` (line ~900) runs `git commit` with `.current_dir(repo_root)`, and `repo_root()` (line ~421) is the single shared main working tree — autonomous execution has no worktree isolation.
**Relates-To:** ADR-2606061359 (single-agent loop), ADR-2606071500 (ReAct tool-use loop / direct_exec executor), ADR-2026-04-13-1930 (`hex worktree merge`, never raw `git checkout <branch> -- <file>`), ADR-2026-05-08-1126 (worktree-mandatory development).

<!-- LIFECYCLE: Proposed → Accepted → Completed. Change Status only via adr_status_set / the adr-steward. -->

## Context

The single-agent factory (`direct_exec.rs`, `direct_react.rs`, driven by `sched_service`)
runs autonomous agents that **commit on evidence-gate success and `git reset` rollback on
failure** (`autonomous.rollback_on_failure`). Today all of this happens against the **main
working tree**: `commit()` and `run_evidence()` use `.current_dir(repo_root())`, where
`repo_root()` resolves to the one shared checkout (`/home/gary/development/hex`).

Two properties turn this into a data-loss-class race when an operator is also working:

1. **Shared branch.** The factory commits to whatever branch the operator has checked out.
   A rollback `git reset` to a prior "good" commit discards *any* commit interleaved after
   it — including operator commits — as collateral.
2. **Shared identity.** Autonomous agents author commits as `hex-coder`, which is also the
   operator's configured git identity. Factory and human commits are indistinguishable, so
   there is no signal (and no guard) separating them.

This is not hypothetical — it happened, and the only reason no work was lost is that the
reset was `--mixed`. A `--hard` rollback, or an operator who didn't notice, would lose work
outright. It also contradicts existing policy: ADR-2026-04-13-1930 already forbids raw
`git checkout <branch> -- <file>` because it "silently drops parallel-worktree code," and
ADR-2026-05-08-1126 mandates worktree isolation for *development* — but the autonomous
executor itself was never brought under that rule.

**Forces:**
- The factory must be free to commit and roll back aggressively (that is how the evidence
  gate works) — *without* that freedom ever touching the operator's tree.
- Results must still flow back to trunk through a reviewed, non-destructive path.
- Isolation must be cheap enough to apply per autonomous run (worktrees are ~ms + disk).

**Alternatives considered:**
- *Separate git identity for the factory only.* Helps attribution/guarding but does not stop
  a shared-branch reset from eating operator commits. Necessary but insufficient.
- *Refuse autonomous commits while an interactive session is active.* Brittle (no reliable
  "session active" signal) and defeats the point of overnight autonomy.
- *Lock the branch.* Serializes operator and factory; kills concurrency, the whole value.

## Decision

**Autonomous execution MUST run in a dedicated git worktree on a dedicated branch, and MUST
NOT commit to, reset, or otherwise mutate the operator's checked-out working tree or branch.**

1. **Per-run worktree.** Before any autonomous `commit`/`run_evidence`/rollback, the executor
   creates (or reuses) a worktree under a hex-owned path (e.g. `.hex/worktrees/auto-<run-id>/`)
   on a branch named `hex/auto/<run-id>`. `direct_exec.rs`/`direct_react.rs` resolve their
   working directory from the **run's worktree**, never `repo_root()` of the main checkout.
2. **Rollback is worktree-scoped.** `rollback_on_failure` performs its `git reset` inside the
   autonomous worktree only. A reset can never reach a commit the operator authored, because
   the operator's commits are not in that worktree's branch.
3. **Merge back through the sanctioned path.** Completed autonomous work returns to trunk via
   `hex worktree merge` (ADR-2026-04-13-1930) — a reviewed, non-destructive merge — never a
   raw reset/checkout on the shared root. Unmerged/abandoned auto-worktrees are GC'd.
4. **Distinct factory identity.** Autonomous commits are authored under a factory identity
   (e.g. `hex-factory <bot@hex>`), distinct from the operator's `hex-coder`, so the two are
   attributable and guardable.
5. **Operator-tree guard.** The commit/reset helpers assert the working directory is an
   `hex/auto/*` worktree when invoked from the autonomous path; committing/resetting the
   operator's branch from an autonomous context is a hard error, not a silent action.

Scope: this governs the *autonomous* execution path. Operator-driven `hex do` in an
interactive session is out of scope (the human owns their own tree).

## Consequences

**Positive:**
- The factory can commit and roll back as aggressively as the evidence gate needs, with zero
  ability to damage the operator's branch — the observed race becomes structurally impossible.
- Brings the autonomous executor under the same worktree-isolation rule that already governs
  development (ADR-2026-05-08-1126, ADR-2026-04-13-1930) instead of exempting itself.
- Factory vs operator commits become attributable; the dashboard/agent feed can show which.

**Negative:**
- Per-run worktree setup cost (~ms + disk) and lifecycle management (creation, GC).
- The executor must thread a per-run working directory instead of the global `repo_root()` —
  touches `direct_exec.rs`, `direct_react.rs`, and the `sched_service` dispatch.
- Merge-back is now an explicit step (it was implicitly "already on the branch").

**Mitigations:**
- Worktrees are cheap and auto-removed when unchanged (mirror the existing feature-dev GC).
- Centralize working-dir resolution behind a single `run_workdir(run_id)` helper so the
  global `repo_root()` autonomous-commit path has exactly one call site to migrate and guard.
- Reuse the existing `hex worktree` lifecycle (status/merge/approve/reject) rather than build
  a parallel mechanism.

## Implementation

| Phase | Description | Status | Verification |
|-------|------------|--------|--------------|
| P1 | This ADR (autonomous worktree-isolation policy) | Pending | code:docs/adrs/ADR-2606071323-autonomous-execution-worktree-isolation.md |
| P2 | `run_workdir(run_id)` helper + per-run worktree creation on the autonomous path | Pending | code:hex-nexus/src/direct_exec.rs |
| P3 | Scope `commit`/`run_evidence`/rollback to the run worktree; assert non-operator tree | Pending | code:hex-nexus/src/direct_exec.rs, test:cargo test -p hex-nexus autonomous_worktree |
| P4 | Merge-back via `hex worktree merge`; auto-worktree GC; factory git identity | Pending | code:hex-nexus/src/direct_react.rs |
| P5 | Operator-tree guard: hard error if autonomous commit/reset targets the session branch | Pending | test:cargo test -p hex-nexus operator_tree_guard |

## References

- ADR-2606061359 — collapse org-sim to the single-agent loop
- ADR-2606071500 — ReAct tool-use loop / the `direct_exec` executor
- ADR-2026-04-13-1930 — `hex worktree merge`, never raw checkout (parallel-worktree safety)
- ADR-2026-05-08-1126 — worktree-mandatory development
- Incident 2026-06-07: autonomous `rollback_on_failure` reset dropped operator commit `e8437655` (recovered via reflog + tag)
- Code: `hex-nexus/src/direct_exec.rs` `repo_root()` (~L421), `commit()` (~L900, `.current_dir(repo_root)`)
