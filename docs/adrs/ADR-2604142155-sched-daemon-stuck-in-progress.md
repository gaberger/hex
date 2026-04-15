# ADR-2604142155 — sched daemon leaves tasks stuck `in_progress`

**Status:** accepted
**Date:** 2026-04-14
**Relates to:** ADR-2604111800 (executor dispatch-evidence guard), ADR-2604141400 (brain queue swarm-lease)

## Context

Prod-testing `scripts/smoke-path-b-wait.sh` against a fresh release build
(hex-nexus + hex sched daemon) with `wp-sched-evidence-guard.json` as the
workplan-under-test reliably reproduces the following failure mode:

1. `hex sched enqueue workplan <file>` returns a task UUID (OK).
2. The daemon picks the task up within its interval (OK).
3. The daemon transitions the task to `in_progress`.
4. The daemon never records a terminal status — task sits at `in_progress`
   indefinitely (verified via `hex sched queue history`).
5. The poller (smoke script) times out after 120 s.

Previous dispatch attempts show a different failure signature in history:
`"Execution dispatched: Object {"` — a vacuous ack that was supposed to
be rejected by `validate_dispatch_evidence()` (ADR-2604111800).

`validate_dispatch_evidence` exists in
`hex-nexus/src/orchestration/workplan_executor.rs`, but the daemon's
**task-lifecycle** path (enqueue → drain → dispatch → terminate) is
distinct from the executor's **workplan-dispatch** path. The guard
protects the executor output, not the daemon's task-termination signal.

**Formal root cause.** The daemon's task lifecycle was never modeled in
TLA+ (unlike the 7-phase feature pipeline in `docs/algebra/lifecycle.tla`
or swarm coordination in `hexflo.tla`). A new module
`docs/algebra/sched_daemon.tla` (ADR-dep P0) captures the four states
pending → claimed → in_progress → {completed, failed} and proves that:

- **HandleInvariant** (`in_progress ⇒ handle held`) — violated by the
  `DispatchVacuous` action the current daemon allows.
- **TerminalReachable** (`∀t : ◇(status[t] ∈ Terminal)`) — a liveness
  counterexample exists with no `TimeoutSweep` under weak fairness, which
  is precisely the `6db24a29` stuck-in-progress observation.

The fix below is chosen to make `SafetyFixed` and `BoundedTermination`
provable in TLC.

## Decision

The sched daemon's drain loop MUST NOT mark a task `in_progress` until it
has a live handle to the nexus executor's completion future, AND MUST
transition that task to a terminal state (`completed` / `failed`) within
a bounded timeout derived from the workplan's `timeout_s` field (default
600 s). Any task that has been `in_progress` for longer than
`timeout_s + grace (30 s)` must be auto-failed by the daemon with reason
`"daemon timeout — no terminal signal from executor"`.

In addition, the daemon's dispatch call MUST re-use
`validate_dispatch_evidence()` on the executor response before accepting
it as a valid dispatch — otherwise a `"dispatched: Object {}"`
vacuous-ack triggers `in_progress` with no prospect of termination.

## Consequences

- Stuck tasks become self-healing: daemon auto-fails after timeout
  instead of leaving them forever `in_progress`.
- Vacuous executor responses fail at daemon layer, not just at the
  executor-output layer — closes the remaining gap in ADR-2604111800.
- Introduces a dependency: daemon must know workplan's `timeout_s`
  (currently only the executor reads it). Plumb via the enqueue payload.

## Alternatives considered

- **Trust executor to always terminate:** rejected — history shows it
  silently fails to in three+ repro runs.
- **Fixed 5-min daemon timeout (no per-workplan config):** rejected —
  long-running workplans (codegen, benchmarks) would be killed.
