# ADR-2603241500: TUI Async Channel Architecture

**Status:** Accepted
**Date:** 2026-03-24
- **Supersedes**: ADR-2603241430 (TUI Non-Blocking Phase Execution — two-tick workaround)
- **Relates to**: ADR-2603232005 (Self-Sufficient hex-agent with TUI)

## Context

The hex dev TUI uses ratatui but blocks the render loop during inference calls (5-40s). ADR-2603241430 added a two-tick workaround that shows the phase name before blocking, but the UI is still frozen during execution — no progress updates, no key handling, no animations.

The root cause: pipeline phases run on the same thread as the render loop via `tokio::task::block_in_place()`. The ratatui event loop cannot draw while a phase is executing.

### What's wrong with the current TUI

1. **Frozen during inference** — 30-40s with no visual feedback
2. **No progress within phases** — can't show "step 3/6" or "compiling..."
3. **Keys unresponsive** — can't quit, toggle debug, or interact while a phase runs
4. **Monolithic tick()** — 500+ lines of phase logic mixed with UI state management
5. **Gate dialogs block** — approve/reject stops everything

## Decision

Refactor to the **ratatui async channel pattern**: phases run on separate tokio tasks and communicate with the UI thread via `mpsc` channels. The render loop never blocks.

### Architecture

```
┌─────────────────────────────────────────────────────┐
│  UI Thread (main)                                    │
│                                                      │
│  loop {                                              │
│    terminal.draw(|f| render(f, &state))              │
│    poll events (keys, resize)                        │
│    recv messages from channel (non-blocking)         │
│    update state from messages                        │
│  }                                                   │
│                                                      │
│  Never blocks. Always responsive.                    │
└──────────────────────┬──────────────────────────────┘
                       │ mpsc::channel
                       │
┌──────────────────────▼──────────────────────────────┐
│  Phase Workers (tokio::spawn)                        │
│                                                      │
│  async fn run_adr_phase(tx, session_data) {          │
│    tx.send(Msg::PhaseStarted(Adr)).await;            │
│    let result = inference_call().await;               │
│    tx.send(Msg::Progress("generating...")).await;     │
│    tx.send(Msg::PhaseDone(Adr, result)).await;       │
│  }                                                   │
└─────────────────────────────────────────────────────┘
```

### Message types

```rust
enum UiMessage {
    // Phase lifecycle
    PhaseStarted { phase: PipelinePhase },
    PhaseProgress { phase: PipelinePhase, detail: String },
    PhaseDone { phase: PipelinePhase, result: PhaseResult },
    PhaseError { phase: PipelinePhase, error: String },

    // Gates (need user input)
    GateRequested { phase: PipelinePhase, title: String, body: String },

    // Task tracking
    TaskUpdate { task_id: String, status: String, detail: String },

    // Metrics
    CostUpdate { cost_usd: f64, tokens: u64 },

    // Agent reports
    AgentReport { role: String, status: String, duration_ms: u64, detail: String },
}
```

### User actions (sent back to workers)

```rust
enum UserAction {
    Approve,
    Retry,
    Skip,
    Quit,
    Shell,
}
```

When a gate is requested, the UI shows the gate dialog. User presses a key → `UserAction` sent via a separate channel back to the waiting phase task.

### Render state

```rust
struct UiState {
    session: DevSessionView,     // read-only snapshot
    phases: Vec<PhaseStatus>,    // Pending/Running/Done/Failed per phase
    current_progress: Option<String>,  // "step 3/6 — secondary adapter..."
    tasks: Vec<TaskView>,        // live task list
    gate: Option<GateView>,      // current gate dialog
    agent_reports: Vec<AgentReportView>,
    cost_usd: f64,
    tokens: u64,
    elapsed: Duration,
}

enum PhaseStatus {
    Pending,
    Running { started_at: Instant, detail: String },
    Done { duration: Duration },
    Failed { error: String },
}
```

### Layout

```
┌ hex dev ─────────────────────────────────────────────┐
│ Feature: Create a calculator app                      │
│ [✓ ADR 1.2s] [✓ Plan 2.1s] [✓ Swarm] [◐ Code 38s]  │
├───────────────────────────────────────────────────────┤
│ ◐ Code Generation — step 3/6                          │
│   hex-coder    step-1 ✓  step-2 ✓  step-3 ◐         │
│   hex-reviewer step-1 ✓  step-2 ✓                    │
│   hex-tester   step-1 ✓  step-2 ✓                    │
│                                                       │
│                                                       │
├───────────────────────────────────────────────────────┤
│ $0.12 | 14.2K tokens | 42s elapsed                   │
│ [q]uit [d]ebug [l]og [p]ause                         │
└───────────────────────────────────────────────────────┘
```

When a gate appears:

```
├───────────────────────────────────────────────────────┤
│ ┌ Gate: Code Review ────────────────────────────────┐ │
│ │ step-3: Implement secondary adapter               │ │
│ │                                                   │ │
│ │ Generated: src/adapters/secondary/calculator.ts   │ │
│ │ Lines: 42 | Tests: 3 passing                      │ │
│ │                                                   │ │
│ │ [a]pprove  [r]etry  [s]kip  [e]dit               │ │
│ └───────────────────────────────────────────────────┘ │
├───────────────────────────────────────────────────────┤
```

## Implementation

| Step | Tier | Description |
|------|------|-------------|
| 1 | T0 | Define UiMessage, UserAction enums and UiState struct |
| 2 | T0 | Create mpsc channels (ui_tx/ui_rx for messages, action_tx/action_rx for user input) |
| 3 | T1 | Extract phase logic from tick() into standalone async fns that take tx + data (not &mut self) |
| 4 | T1 | Rewrite render loop: recv from channel, update UiState, draw |
| 5 | T2 | Phase workers send Progress messages during execution (step N/M, compiling...) |
| 6 | T2 | Gate handling via channel: worker sends GateRequested, waits on action_rx |
| 7 | T3 | Agent report streaming — workers send AgentReport messages as agents complete |
| 8 | T3 | Pipeline bar with animated progress (spinner, elapsed time per phase) |
| 9 | T4 | Integration test: verify UI stays responsive during 30s inference call |

## Consequences

### Positive

- **Always responsive** — render loop never blocks, keys always work
- **Live progress** — "step 3/6", "compiling...", agent completion in real-time
- **Clean separation** — UI state is a simple struct, phases are independent tasks
- **Testable** — phase workers can be tested without a terminal
- **Interruptible** — Ctrl+C/quit works instantly, no waiting for phase to finish

### Negative

- **Significant refactor** — tick() is 500+ lines that need to be decomposed
- **Phase methods can't use &mut self** — need to extract all data upfront
- **Two-way channel complexity** — gates need request/response pattern
- **Session persistence** — phase workers need to send session updates back to UI thread

### Mitigations

- Incremental migration: one phase at a time, starting with ADR (simplest)
- UiState is a read-only view — phase workers send deltas, UI thread applies them
- Gate request/response uses a oneshot channel per gate
