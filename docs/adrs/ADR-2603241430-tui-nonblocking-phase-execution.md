# ADR-2603241430: TUI Non-Blocking Phase Execution

**Status:** Superseded by ADR-2603241500
**Date:** 2026-03-24
- **Relates to**: ADR-2603232005 (Self-Sufficient hex-agent with TUI)

## Context

The TUI event loop calls `tick()` every 100ms which renders the UI. But `tick()` runs pipeline phases via `tokio::task::block_in_place()` which blocks the entire thread for 5-40 seconds per inference call. The TUI appears frozen during each phase — no progress indicators, no key handling, no visual feedback.

## Decision

Move phase execution to background tasks. The `tick()` function spawns work and polls for completion, never blocking the render loop.

### Architecture

```
render loop (100ms tick):
  terminal.draw(frame)     ← always runs, shows progress
  event::poll(timeout)     ← key handling always responsive
  tick()                   ← checks phase_task, never blocks

tick():
  if phase_task is Some:
    poll JoinHandle → Ready? process result : return
  else if needs_phase_run:
    spawn phase on tokio → store JoinHandle in phase_task
```

### State machine

```rust
enum PhaseTask {
    /// No phase running — idle or waiting for gate approval
    Idle,
    /// Phase running in background
    Running {
        phase: PipelinePhase,
        handle: tokio::task::JoinHandle<Result<PhaseOutput>>,
        started_at: Instant,
    },
}
```

### Phase output

Each phase returns a `PhaseOutput` enum that `tick()` processes:

```rust
enum PhaseOutput {
    Adr { path: String },
    Workplan { path: String, steps: usize },
    Swarm { swarm_id: String, task_count: usize },
    Code { results: Vec<CodeStepResult> },
    Validate { quality: QualityReport },
    Commit,
}
```

### Progress during execution

While a phase is running, `render()` shows:
- Spinner animation on the current phase
- Elapsed time
- "Running ADR phase..." / "Generating code (step 3/6)..."

## Implementation

| Step | Description |
|------|-------------|
| 1 | Add `phase_task: PhaseTask` field to TuiApp |
| 2 | Extract phase logic into standalone async fns that don't take `&mut self` |
| 3 | `tick()` spawns phase, stores JoinHandle, polls on next tick |
| 4 | Render spinner + elapsed time while phase_task is Running |
| 5 | Process PhaseOutput when task completes |

## Consequences

### Positive
- TUI stays responsive during inference (30-40s calls)
- Progress visible: spinner, elapsed time, phase name
- Keys work: user can quit (q), toggle debug (d), toggle log (l) during phases

### Negative
- Phase methods can't take `&mut self` — need to extract data upfront and pass it in
- More complex state management (phase_task enum)
- Error handling via JoinHandle instead of direct `?`
