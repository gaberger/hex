# ADR-2604101700: Workplan Progress Visibility via SpacetimeDB

**Status:** Accepted

**Author:** hex

**Date:** 2026-04-10

## Summary

Add `hex plan progress` command to query real-time workplan phase/task status from SpacetimeDB, enabling users and external tools to poll progress without parsing logs or running the executor.

## Problem

Workplans execute autonomously via HexFlo, but users cannot read progress in real-time:

1. **No polling endpoint** — `hex plan history` returns only completed executions
2. **Log filtering** — phase START/COMPLETE logs filtered at INFO level, invisible in output
3. **No dedicated progress CLI** — users must guess completion from incomplete data

## Solution

Add SpacetimeDB tables and CLI command for real-time progress visibility:

### SpacetimeDB Tables

```rust
#[table(name = workplan_phase, public)]
pub struct WorkplanPhase {
    pub execution_id: String,
    pub phase_name: String,
    pub status: String,       // "pending" | "running" | "completed" | "failed"
    pub gate_result: String,   // JSON of gate command output
    pub started_at: String,
    pub completed_at: String,
}

#[table(name = workplan_task, public)]
pub struct WorkplanTask {
    pub execution_id: String,
    pub task_id: String,
    pub phase_name: String,
    pub status: String,        // "pending" | "in_progress" | "completed" | "failed"
    pub agent_id: String,
    pub result: String,
    pub error: String,
    pub started_at: String,
    pub completed_at: String,
}
```

### CLI Command

```bash
hex plan progress <execution-id>   # Show current phase + task progress
hex plan watch <execution-id>    # Poll with watch(1)-like output
```

### Workplan Executor Updates

On phase/task transitions, executor calls reducers to update tables:

```rust
reducer fn phase_start(execution_id, phase_name) { ... }
reducer fn phase_complete(execution_id, phase_name, gate_result) { ... }
reducer fn task_start(execution_id, task_id, agent_id) { ... }
reducer fn task_complete(execution_id, task_id, result) { ... }
```

## Integration

- **Depends on:** ADR-2604101600 (SpacetimeDB Workplan Coordination)
- **Executor wiring:** Updates tables on each phase/task transition
- **Dashboard:** Subscribe to workplan_phase/workplan_task tables for live updates

## Alternatives Considered

1. **Parse logs** — fragile, requires log level changes
2. **WebSocket subscription** — ADR-2604101600 already adds this
3. **hex status** — too generic, not workplan-specific

This ADR adds dedicated visibility on top of ADR-2604101600's coordination infrastructure.