# ADR-2603311900: Pipeline Phase Pre-condition Gates

**Status:** Accepted
**Date:** 2026-03-31
**Drivers:** Research swarm b1c8ac49 identified that every `hex dev` TUI session silently skips code generation because `session.workplan_path` is never written to the session JSON — `run_code_phase()` finds `None` and skips to Validate. The broader pattern: 12 code paths where phases advance without upstream artifacts existing, all because the pipeline treats missing upstream state as best-effort rather than a blocking error.

---

## Context

The `hex dev` TUI pipeline follows the phase sequence:

```
Adr → Workplan → Swarm → Code → Validate → Commit
```

Each phase depends on artifacts from the previous phase:
- **Workplan** needs an ADR file
- **Swarm** needs a parsed workplan
- **Code** needs swarm tasks to dispatch agents against
- **Validate** needs code artifacts to check
- **Commit** needs a passing quality result

**Current state**: None of these dependencies are enforced. Every phase has a "warn and skip" path for missing upstream state. The research found:

1. `session.workplan_path`, `session.adr_path`, and `session.swarm_id` are **never written** to the session JSON during TUI execution — so downstream phases always find `None`
2. `run_code_phase()` explicitly skips to Validate when `workplan_path=None`, with only a `warn!()` log
3. The Swarm phase comments "Advancing anyway — swarm is best-effort" even on task dispatch failure
4. A single ADR gate skip cascades: ADR skip → workplan auto-skips → swarm auto-skips → code skips → validate skips → Completed
5. `SessionStatus::Completed` is set by 4 independent code paths with no shared invariant validation
6. `GateResult::Skip` at the Commit gate sets `Completed`, meaning "skip" and "done" are indistinguishable

**Result**: Every TUI session reports `Status: completed` with 0 artifacts produced and 0 swarm tasks executed. This is the direct cause of the recurring "session completed but no code" pattern.

---

## Decision

**We will enforce pre-condition gates at the entry of each pipeline phase.** A missing upstream artifact is a blocking error presented as a gate dialog — not a silent skip.

### 1. Session field population contract

Every phase completion handler **must** write its output path/ID to the session JSON before advancing:

```rust
// workplan phase completion
self.session.workplan_path = Some(workplan_path.to_string());
self.session.save()?;

// swarm phase completion
self.session.swarm_id = Some(swarm_id.clone());
self.session.save()?;

// adr phase completion
self.session.adr_path = Some(adr_path.to_string());
self.session.save()?;
```

### 2. Phase pre-condition checks

Each phase runner calls `check_phase_preconditions()` before executing:

```rust
fn check_phase_preconditions(&self, phase: PipelinePhase) -> Result<(), PreconditionError> {
    match phase {
        PipelinePhase::Workplan => {
            require_some(&self.session.adr_path, "ADR must be created before workplan")?;
        }
        PipelinePhase::Swarm => {
            require_some(&self.session.workplan_path, "Workplan must exist before swarm")?;
            require_file_parseable(self.session.workplan_path.as_ref().unwrap())?;
        }
        PipelinePhase::Code => {
            require_some(&self.session.swarm_id, "Swarm must be initialized before code")?;
            require_swarm_has_tasks(&self.session.swarm_id.as_ref().unwrap())?;
        }
        PipelinePhase::Validate => {
            require_nonempty(&self.session.completed_steps, "Code phase must complete at least one step")?;
        }
        PipelinePhase::Commit => {
            require_some(&self.session.quality_result, "Validate phase must produce a quality result")?;
        }
        _ => {}
    }
    Ok(())
}
```

`PreconditionError` presents a blocking gate dialog:
- **Retry** — return to previous phase and re-run it
- **Skip-to-phase N** — explicitly jump back (user intent is captured)
- **Abort** — set `SessionStatus::Paused`, save state, exit

### 3. Centralize session completion

Remove all 4 direct assignments of `SessionStatus::Completed`. Replace with a single:

```rust
fn finalize_session(&mut self, outcome: CompletionOutcome) -> Result<()> {
    match outcome {
        CompletionOutcome::Approved => {
            // Invariant checks
            if self.session.swarm_id.is_some() {
                let tasks_done = self.get_swarm_tasks_completed();
                if tasks_done == 0 {
                    bail!("cannot finalize: 0/N swarm tasks completed");
                }
            }
            self.session.status = SessionStatus::Completed;
        }
        CompletionOutcome::Skipped | CompletionOutcome::Aborted => {
            self.session.status = SessionStatus::Paused;
        }
    }
    self.session.save()
}
```

### 4. GateResult::Skip → SessionStatus::Paused

Change `handle_commit_gate` and `advance_to_next_phase` at Commit:

```rust
// Before (wrong)
GateResult::Skip | GateResult::Retry => {
    self.session.status = SessionStatus::Completed;
    ...
}

// After (correct)
GateResult::Skip | GateResult::Retry => {
    self.finalize_session(CompletionOutcome::Skipped)?;
    ...
}
```

### 5. Add SessionStatus::Incomplete for detection

Add a new status variant for sessions that completed the pipeline but without all work done (for backward compatibility with existing sessions):

```rust
pub enum SessionStatus {
    InProgress,
    Paused,       // user skipped, can resume
    Completed,    // all phases ran, quality gate passed
    Incomplete,   // pipeline advanced but work was missing (legacy detection)
    Failed,       // hard error
}
```

Sessions with `completed_steps.is_empty() && quality_result.is_none()` are retroactively shown as `Incomplete` in reports and `hex dev list`.

---

## Consequences

**Positive:**
- Code generation actually runs on every `hex dev` session
- `Status: completed` means something — all phases ran with real artifacts
- Cascade skip failure mode eliminated
- Session JSON is the single source of truth for pipeline state (no git fallback needed)
- `hex report` phases show accurate data without fallback heuristics

**Negative:**
- More blocking dialogs — users who intentionally skip phases get more friction
- Existing "quick mode" flow needs explicit "skip-with-intent" UI pattern
- Some legitimate use cases (ADR-only sessions, workplan-only sessions) currently rely on silent skips

**Mitigations:**
- Pre-condition gate dialogs offer explicit "Skip-to-phase" as a named action — intent is captured, not silently assumed
- Quick mode can pre-configure skip-with-intent flags per phase
- `SessionStatus::Paused` allows interrupted sessions to resume from the last successful phase

---

## Implementation

| Phase | Task | Status |
|-------|------|--------|
| P0.1 | Write `adr_path`, `workplan_path`, `swarm_id` to session JSON in TUI phase handlers | Done |
| P0.2 | Remove warn+skip in `run_code_phase()` — replace with `PreconditionError` gate | Done |
| P0.3 | Remove warn+skip in swarm phase "advancing anyway" path | Done |
| P1.1 | Implement `check_phase_preconditions()` function | Done |
| P1.2 | Centralize to `finalize_session(CompletionOutcome)` — remove 4 direct Completed assignments | Done |
| P1.3 | Fix `GateResult::Skip` at Commit → `SessionStatus::Paused` | Done |
| P1.4 | Fix `advance_to_next_phase` at Commit → run gate, not bypass | Done |
| P2.1 | Add `SessionStatus::Incomplete` variant + detection logic | Done |
| P2.2 | `hex dev list` shows Incomplete sessions with warning + resume hint | Done |
| P2.3 | Update `hex report` — remove git fallback heuristics once session fields are reliable | Done |

---

## References

- [Research: TUI Phase Gaps](../analysis/research-tui-phase-gaps.md)
- [Research: Session State Machine](../analysis/research-session-statemachine.md)
- [Research: ADR Pipeline Gaps](../analysis/research-adr-pipeline-gaps.md)
- [Research: Swarm Task Lifecycle](../analysis/research-swarm-task-lifecycle.md)
- [Synthesis](../analysis/research-hex-dev-determinism-synthesis.md)
- ADR-2603232005 — hex-dev-usability (session management)
- ADR-2603232340 — validate loop (pre-condition: code phase must complete)
- ADR-2603241126 — TUI CLI surrogate (traceability)
- ADR-2603311000 — workflow reliability hardening (superseded by this ADR for phase gates)
