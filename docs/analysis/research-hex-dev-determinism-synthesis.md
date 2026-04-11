# Synthesis: hex dev Pipeline Determinism — Ranked Improvements
*Research swarm b1c8ac49 — 2026-03-31*
*Sources: research-tui-phase-gaps.md, research-session-statemachine.md, research-adr-pipeline-gaps.md, research-swarm-task-lifecycle.md*

---

## The Core Problem in One Sentence

The `hex dev` TUI pipeline has no enforcement gates between phases — every phase can advance without upstream work existing, and `SessionStatus::Completed` is set by 4+ independent code paths with no shared invariant validation.

---

## Root Cause Chain (The Bug That Explains Everything)

```
session.workplan_path is never written to session JSON in TUI mode
  → run_code_phase() finds workplan_path=None
  → skips entire Code phase, jumps to Validate     ← PRIMARY BUG
  → Validate runs with no code artifacts
  → advance_to_next_phase → Commit → Completed
  → 0 tasks executed, quality_result=None
  → report shows "Status: completed, Quality: PASS"  ← now fixed
```

**Single-line fix for the primary bug** (unblocks everything):
In the workplan phase completion handler, write `session.workplan_path = Some(path)` before advancing.

---

## Ranked Improvements

### P0 — Fix Immediately (1-2 days)

#### 1. Write workplan_path to session when workplan phase completes
**File**: `hex-cli/src/tui/mod.rs` — workplan phase completion handler
**Fix**: After workplan is created/updated on disk, write its path to `session.workplan_path` and call `session.save()`. Same for `adr_path` and `swarm_id`.
**Why**: This single fix unblocks Code phase execution for all TUI sessions.
**ADR**: Amend ADR-2603232005 with a "session field population contract"

#### 2. Harden Cascade Skip — Missing Upstream = Hard Stop, Not Silent Skip
**File**: `hex-cli/src/tui/mod.rs` — `run_code_phase()`, `run_swarm_phase()`
**Fix**: Replace warn+skip with an error gate dialog: "No workplan found — cannot proceed to Code. Return to Workplan phase?"
**Why**: Silent skips are the mechanism by which the entire pipeline collapses silently.
**ADR**: New ADR — "Pipeline Phase Pre-condition Gates"

#### 3. Fix GateResult::Skip at Commit → SessionStatus::Paused
**File**: `hex-cli/src/tui/mod.rs:2131-2135`
**Fix**: `GateResult::Skip | GateResult::Retry => { self.session.status = SessionStatus::Paused; }`
**Why**: Skip means "not done yet", not "completed successfully". Paused sessions can resume.
**ADR**: Amend ADR-2603232005

#### 4. Fix advance_to_next_phase at Commit — Run the Gate
**File**: `hex-cli/src/tui/mod.rs:1941-1945`
**Fix**: Instead of setting Completed directly, call `self.run_commit_phase()` so the gate runs.
**Why**: Keyboard navigation bypasses commit review entirely.

---

### P1 — This Sprint (3-5 days)

#### 5. Centralize Completion — Single finalize_session() Function
**Fix**: Remove all 4 direct assignments of `SessionStatus::Completed`. Replace with:
```rust
fn finalize_session(&mut self, approved: bool) -> Result<()> {
    if !approved {
        self.session.status = SessionStatus::Paused;
        return self.session.save();
    }
    // Invariant checks
    if self.session.swarm_id.is_some() && self.get_swarm_tasks_completed() == 0 {
        return Err(anyhow!("cannot complete: swarm has 0 tasks done"));
    }
    self.session.status = SessionStatus::Completed;
    self.session.save()
}
```
**ADR**: New ADR — "Session Completion Invariants"

#### 6. Implement Validate Loop (ADR-2603232340 P0-P9)
**Fix**: Wire actual compile check, test runner, and `hex analyze` grade gate with auto-fix loop into the Validate phase. This is ADR-2603232340 which has all tasks pending.
**ADR**: Execute ADR-2603232340 workplan

#### 7. Fix Worker Spawn Failure — Hard Error or Explicit Inline Path
**File**: `hex-nexus/src/orchestration/supervisor.rs:1031`
**Fix**: When worker spawn fails, either: (a) surface a blocking error to the user, OR (b) use the SwarmPhase task IDs in the inline execution path (not shadow tasks). Currently the fallback creates shadow tasks leaving the original tasks orphaned.
**ADR**: Amend ADR-2603282000 (Docker Sandbox)

#### 8. Add SessionStatus::Incomplete
**Fix**: New status variant for "pipeline advanced without completing all phases." Sessions in this state appear in `hex dev list` with a warning, can be resumed, and are excluded from success metrics.
**ADR**: Amend ADR-2603232005

---

### P2 — Next Sprint (1-2 weeks)

#### 9. Refactor Supervisor REST → CLI Surrogate (ADR-2603241126)
Write supervisor operations through `hex task complete` / `hex analyze .` CLI calls so tool call tracking is populated in session JSON and the audit report works.

#### 10. Implement TUI Async Channels (ADR-2603241500)
Non-blocking inference in the render loop. Currently the TUI freezes for 30-40s during inference, which likely causes users to force-quit → triggering the advance_to_next_phase → Completed path.

#### 11. Document and Enforce Task State Machine
Write an ADR for the swarm task state machine (pending→in_progress→completed/failed), define clear ownership for each transition, and add invariant checks to the SpacetimeDB reducer.

#### 12. Session Resume for Incomplete/Paused Sessions
Allow `hex dev start` to detect a paused/incomplete session for the same feature description and offer to resume from the last successful phase, with state reconstructed from git + swarm API.

---

## Proposed New ADR: "Pipeline Phase Pre-condition Gates" (ADR-2603311900)

**Decision**: Each pipeline phase MUST validate that upstream artifacts exist before executing. Missing upstream = blocking gate dialog, not silent skip.

**Pre-conditions**:
| Phase | Required upstream |
|---|---|
| Workplan | `session.adr_path` is Some AND file exists |
| Swarm | `session.workplan_path` is Some AND file parses |
| Code | `session.swarm_id` is Some AND swarm has > 0 tasks |
| Validate | `session.completed_steps` is non-empty OR quality_result exists |
| Commit | `session.quality_result` is Some AND grade ≥ C |

**Implementation**: `fn check_phase_preconditions(phase: PipelinePhase) -> Result<(), PreconditionError>`
Called at the start of each phase runner. `PreconditionError` presents a blocking gate: Retry / Skip-to-phase / Abort.

---

## Immediate Recovery Actions

```bash
# Close the orphaned swarm (9 stuck tasks)
hex swarm complete 3903f6e8-5d4c-44d1-90df-8639e46fbb2a

# The session is permanently "completed" — start fresh
hex dev start "hex docs static site generator"
```

The ADR (ADR-2603311711), workplan (feat-hex-docs-static-site-generator.json), and swarm topology are all correct. Once P0 fix #1 (write workplan_path to session) lands, the next `hex dev start` will execute Code phase correctly.
