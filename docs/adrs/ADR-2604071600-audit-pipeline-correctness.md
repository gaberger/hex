# ADR-2604071600: Audit Pipeline Correctness — TestsExist Escalation + completed_steps Wiring

**Status:** Accepted
**Date:** 2026-04-07
**Drivers:** Three bugs caused incorrect audit reports: `TestsExist` stalled the pipeline indefinitely, `completed_steps` was never populated by the supervisor (always empty), and the fallback marked all steps done even on pipeline failure.
**Supersedes:** Partial fixes in ADR-2604071300 (P0–P1 completed; these bugs were P2 regressions)

## Context

The hex dev pipeline (`hex dev --auto`) runs a `Supervisor` that drives a goal-oriented
objective loop per workplan tier. After each run, the `DevSession` accumulates:

- `tool_calls` — per-agent inference records (costs, tokens, model, duration)
- `completed_steps` — workplan step IDs that were completed
- `quality_result` — overall grade/score from objective states

Three independent bugs caused the audit trail to be wrong and the pipeline to stall:

### Bug 1: `TestsExist` never escalated to `hex-fixer`

`agent_for_objective(TestsExist, has_prior_result)` used a wildcard match `(TestsExist, _) => "hex-tester"`. After the first tester run failed (e.g., Go test files written to wrong path), every subsequent iteration still dispatched `hex-tester` with no fixer escalation. This differs from `TestsPass` and `ReviewPasses`, which both escalate to `hex-fixer` on `has_prior=true`. The result: `TestsExist` consumed all `max_iterations` dispatching the tester in a loop, then halted the pipeline.

### Bug 2: Supervisor never wrote `completed_steps` to the shared session

`Supervisor::run_tier` calls `session.log_tool_call()` (via `log_agent_performance`) when each agent completes, but never calls anything to push step IDs into `session.completed_steps`. After `supervisor.run()`, the TUI sync-back at `tui/mod.rs:1103` checked `sup_session.completed_steps.is_empty()` — always true — and fell through to the fallback.

### Bug 3: `completed_steps` fallback was unconditional

The fallback at `tui/mod.rs:1108` populated `completed_steps` with ALL workplan step IDs regardless of whether `supervisor.run()` returned `Ok` or `Err`. A halted or errored pipeline showed all steps as "completed" in `hex dev status` and `hex dev report`.

## Impact Analysis

### Consumer Dependency Map

```
Artifact: pipeline/objectives.rs :: agent_for_objective()
├── Direct consumers:
│   └── pipeline/supervisor.rs:1308 — agent_role = agent_for_objective(obj, has_prior)
├── Transitive consumers:
│   └── tui/mod.rs (supervisor dispatched from here)
└── Tests:
    └── None (logic tested implicitly via integration)

Artifact: session.completed_steps (DevSession field)
├── Written by:
│   ├── tui/mod.rs:1104 — sync from sup_session (was always empty)
│   ├── tui/mod.rs:1109 — fallback: all workplan step IDs (was unconditional)
│   └── NOW ALSO: supervisor.rs:1186 — per-tier when AllPassed
├── Read by:
│   ├── tui/mod.rs:1103 — sync-back check
│   ├── tui/mod.rs:1462,1505,1512,1601 — progress display
│   ├── commands/report.rs — Phase 4 display
│   ├── commands/dev.rs — hex dev status display
│   └── session.rs:285 — SessionSummary::completed_steps_count
```

### Blast Radius

| Artifact | Impact | Mitigation |
|----------|--------|------------|
| `objectives.rs:195` — `TestsExist` match arm | HIGH — pipeline stalls on Go/Rust test placement failures | Escalate to `hex-fixer` on `has_prior=true` |
| `session.completed_steps` — never populated by supervisor | HIGH — audit trail always shows 0 or all steps | Supervisor writes step IDs on tier pass |
| `tui/mod.rs:1108` — unconditional fallback | MEDIUM — misleading audit; reports success on failure | Guard with `result.is_ok()` |

### Build Verification Gates

| Gate | Command | Result |
|------|---------|--------|
| Workspace compile | `cargo build -p hex-cli` | ✓ Passed (fd31bad3) |

## Decision

Three targeted fixes, all in the supervisor/TUI pipeline layer:

1. **`objectives.rs`**: Split `(TestsExist, _)` into `(TestsExist, false) => "hex-tester"` and `(TestsExist, true) => "hex-fixer"`. Mirrors the established pattern for `TestsPass` and `ReviewPasses`.

2. **`supervisor.rs`**: After `run_tier` returns `TierResult::AllPassed`, push that tier's workplan step IDs into `self.session.completed_steps` (via the shared Mutex) and call `session.save()`. This gives the TUI sync-back real data instead of an empty vec.

3. **`tui/mod.rs`**: Guard the fallback at line 1108 with `&& result.is_ok()`. On `Err`, `completed_steps` stays empty — accurately reflecting that no steps are confirmed done.

## Consequences

**Positive:**
- `TestsExist` failures now call `hex-fixer` after the first tester attempt, allowing the fixer to correct file placement or test structure without consuming all iterations
- `hex dev status` and `hex dev report` now show accurate step completion, not misleading "all done" on failed pipelines
- Partial success is now correctly represented: tiers 0..N-1 that passed show their steps; a failed tier N shows nothing

**Negative:**
- None. Fixes are local to the supervisor/TUI boundary; no schema changes, no new dependencies

**Mitigations:**
- N/A

## Implementation

| Phase | Description | Status |
|-------|-------------|--------|
| P0 | Fix `agent_for_objective(TestsExist, true)` → `hex-fixer` in `objectives.rs` | Done |
| P1 | Write tier step IDs to `session.completed_steps` in `supervisor.rs::run()` | Done |
| P2 | Guard `completed_steps` fallback with `result.is_ok()` in `tui/mod.rs` | Done |
| P3 | Build gate: `cargo build -p hex-cli` | Done (fd31bad3) |

## References

- ADR-2604071300 — Unified hex dev audit trail via SpacetimeDB (parent ADR)
- ADR-2604070400 — Swarm code quality improvements (fixer loop detection context)
- `hex-cli/src/pipeline/objectives.rs:190` — `agent_for_objective`
- `hex-cli/src/pipeline/supervisor.rs:1186` — tier completion step wiring
- `hex-cli/src/tui/mod.rs:1108` — fallback guard
- Commit `fd31bad3` — implementation
