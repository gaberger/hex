# ADR-2604101600 — SpacetimeDB Workplan Coordination

**Status:** Accepted
**Date:** 2026-04-10

## Context
The current workplan executor runs in-memory in hex-nexus, causing:
- **Single point of failure**: When hex-nexus process stalls, workplan stalls
- **No distributed coordination**: Can't handle multi-host execution
- **No preemption**: Dead agents/hosts don't trigger task reassignment
- **Silent failures**: No observability when executor gets stuck

## Decision
Move workplan execution coordination to SpacetimeDB with:

### Tables
```sql
-- workplan_execution: global execution state
CREATE TABLE workplan_execution (
    id UUID PRIMARY KEY,
    workplan_id STRING,
    status STRING,           -- running, paused, completed, failed
    current_phase STRING,
    completed_phases U32,
    completed_tasks U32,
    failed_tasks U32,
    started_at STRING,
    updated_at STRING,
);

-- workplan_phase: per-phase state  
CREATE TABLE workplan_phase (
    execution_id UUID,
    phase_name STRING,
    status STRING,        -- pending, running, completed, failed
    gate_result STRING,
    started_at STRING,
    completed_at STRING,
    PRIMARY KEY (execution_id, phase_name),
);

-- workplan_task: per-task state
CREATE TABLE workplan_task (
    execution_id UUID,
    task_id STRING,
    phase_name STRING,
    status STRING,        -- pending, running, completed, failed
    agent_id STRING,
    result STRING,
    started_at STRING,
    completed_at STRING,
    PRIMARY KEY (execution_id, task_id),
);
```

### Reducers
```rust
// phase_start: transition phase to running, log heartbeat
#[reducer]
fn phase_start(db, execution_id, phase_name) {
    // Update phase status to running
    // Log "Phase START" to hexflo_memory for observability
    // Broadcast subscription update
}

// task_spawn: create task, assign agent
#[reducer]
fn task_spawn(db, execution_id, task_id, phase_name, agent_id) {
    // Update task status to running
    // Log "Task START" to hexflo_memory
    // Broadcast subscription update
}

// task_complete: mark task done, check phase gate
#[reducer]
fn task_complete(db, execution_id, task_id, result, status) {
    // Update task status
    // If all tasks complete, trigger phase gate check
    // Log "Task COMPLETE"
}
```

### Subscription Pattern
```rust
// Client subscribes to execution updates
db.subscribe("workplan_execution WHERE id = ?", execution_id);
// Receives real-time updates on phase/task transitions
```

## Consequences
| Aspect | Impact |
|--------|--------|
| Preemption | ✅ Can reassign tasks when agents die |
| Multi-host | ✅ Coordination via database |
| Observability | ✅ Subscription-based heartbeats |
| Recovery | ✅ Restart from database state |
| Complexity | ⚠️ Requires SpacetimeDB module |

## Implementation Notes
1. Add to `spacetime-modules/hexflo-coordination/` or create new module
2. Use `hexflo_memory` for heartbeat logs (already exists)
3. Replicate current `workplan_executor.rs` logic in reducers
4. Keep hex-nexus as executor client (spawns agents, collects output)

## Related
- ADR-2604010000: HexFlo Memory Ledger
- ADR-2603221959: Hex Agent Lifecycle