# ADR-2604132330: Brain Inbox — Task Queue for the Supervisor Loop

**Status:** Proposed
**Date:** 2026-04-13
**Drivers:** Brain daemon ticks every 60s running fixed validation checks. But the real use case is "tell brain to do X, it picks it up on next tick." Brain needs an inbox — a task queue that any process can add work to, and the supervisor executes autonomously.

## Context

Today brain daemon runs a hardcoded set of checks (CLI wiring, binary freshness, workplan status, etc.). If a developer wants brain to do something else — calibrate inference, execute a specific workplan, fix an ADR violation — they have to run it manually.

The AIOS promise is ambient autonomy. The supervisor should be directable:

```bash
hex brain enqueue "calibrate all inference providers"
hex brain enqueue "execute docs/workplans/wp-inference-setup-fix.json"
hex brain enqueue "reconcile all workplans"
```

On the next tick, brain pulls from the inbox, executes the task, and moves on. This is the "cron meets queue" pattern.

## Decision

### 1. `hex brain enqueue <task>` — add work to the brain inbox

Tasks are stored in SpacetimeDB as `brain_task` rows:

```rust
struct BrainTask {
    id: String,
    kind: String,           // "shell", "hex-command", "workplan"
    payload: String,        // command or workplan path
    status: String,         // pending, running, completed, failed
    created_at: String,
    completed_at: String,
    result: String,
}
```

### 2. Each tick drains up to N tasks

```rust
async fn tick() {
    validate();              // existing checks
    let tasks = fetch_pending_brain_tasks(limit: 5).await;
    for task in tasks {
        execute_brain_task(task).await;
    }
}
```

### 3. Task types

| Kind | Payload | Execution |
|------|---------|-----------|
| `hex-command` | `inference bench bazzite-qwen3-4b` | Shell out to hex CLI |
| `shell` | `cargo test --workspace` | Shell exec |
| `workplan` | `docs/workplans/wp-foo.json` | `hex plan execute <path>` |
| `validate` | (none) | Run validate() eagerly, not wait for tick |

### 4. Safety

- Whitelist: only `hex` commands and pre-approved shells
- Per-task timeout (default 5min)
- Max concurrent tasks per tick (default 1)
- Failed tasks stay in inbox with `status=failed` until manually cleared

### 5. Integration with existing inbox (ADR-060)

The existing agent notification inbox is for alerts TO agents. Brain inbox is for work TO brain. Same pattern, different channel:
- `inbox_notify` → priority-2 alerts to agents
- `brain_enqueue` → tasks for brain to execute

### 6. CLI

```bash
hex brain enqueue hex-command "inference bench bazzite-qwen3-4b"
hex brain enqueue workplan "docs/workplans/wp-foo.json"
hex brain enqueue shell "cargo test --workspace"

hex brain queue list          # Show pending tasks
hex brain queue clear          # Remove completed/failed tasks
hex brain queue drain          # Force-execute queue now (don't wait for tick)
```

## Consequences

**Positive:**
- Brain becomes truly directable — you tell it what to do, it does it on next tick
- Agents can enqueue tasks for brain (e.g. "after I'm done, calibrate these providers")
- Async execution — user doesn't wait
- All work centralized through brain = single audit log

**Negative:**
- Need whitelist to prevent arbitrary command execution
- Queue could back up if tasks are slow
- Race conditions if multiple brain instances run

**Mitigations:**
- Whitelist-only execution (no arbitrary shell)
- PID file already prevents multiple brain daemons (ADR-2604132300)
- Per-task timeout prevents runaway

## Implementation

| Phase | Description | Status |
|-------|------------|--------|
| P1 | `brain_task` SpacetimeDB table + reducer | Pending |
| P2 | `hex brain enqueue` CLI + REST endpoint | Pending |
| P3 | Daemon drains queue each tick | Pending |
| P4 | Task type executors (hex-command, workplan, shell) | Pending |
| P5 | `hex brain queue list/clear/drain` | Pending |

## References

- ADR-060: Agent Notification Inbox (parallel pattern)
- ADR-2604132300: Brain Daemon Loop
- ADR-2604131945: Brain Self-Consistency Daemon
