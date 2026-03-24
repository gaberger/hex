# ADR-2603241230: Persistent Agent Coordination via SpacetimeDB

- **Status**: Proposed
- **Date**: 2026-03-24
- **Relates to**: ADR-025 (SpacetimeDB State Backend), ADR-027 (HexFlo Swarm Coordination), ADR-037 (Agent Lifecycle), ADR-048 (Session Agent Registration), ADR-2603240130 (Declarative Swarm from YAML)

## Context

### The fundamental flaw

The hex dev pipeline's supervisor (`supervisor.rs`) executes agents as **inline function calls** within a single Rust process. Despite having:

- SpacetimeDB tables for swarm coordination (`hexflo-coordination` module)
- Agent registry with heartbeats (`agent-registry` module)
- Task state machine (pending → in_progress → completed)
- WebSocket subscriptions for real-time state sync

...none of this coordination infrastructure is actually used during `hex dev`. The supervisor:

1. Calls `ReviewerAgent::execute()` as a local function — no process, no heartbeat
2. Calls `TesterAgent::execute()` next — sequential, not parallel
3. Creates HexFlo tasks via CLI for tracking but never consumes them from a queue
4. Has no mechanism for agents to see each other's output
5. Cannot recover if the process crashes mid-pipeline

### What should happen (per hex architecture)

```
SpacetimeDB (real-time coordination)
  │
  ├─ swarm table: { id, name, topology, status }
  ├─ swarm_task table: { id, swarm_id, title, status, agent_id, result }
  ├─ swarm_agent table: { id, name, status, last_heartbeat }
  └─ hexflo_memory table: { key, value, scope }
        │
        ├── Agent A (hex-coder) ──── WebSocket subscription ────┐
        ├── Agent B (hex-tester) ─── WebSocket subscription ────┤
        ├── Agent C (hex-reviewer) ── WebSocket subscription ───┤
        └── Supervisor ──────────── WebSocket subscription ─────┘
              │
              └─ Sees all task state changes in real-time
              └─ Assigns tasks, monitors progress, handles failures
```

Each agent is a **persistent process** that:
- Connects to SpacetimeDB via WebSocket
- Subscribes to task assignments for its role
- Sends heartbeats every 30 seconds
- Picks up tasks from the queue when assigned
- Writes results back to SpacetimeDB
- Can be on a different machine (ADR-040)

### Why this matters

1. **Conflict prevention**: Two coders editing the same file don't know about each other
2. **Parallelism**: Reviewer and tester can work simultaneously on different files
3. **Resilience**: If an agent crashes, the supervisor detects missing heartbeats and reassigns
4. **Visibility**: Dashboard shows live agent status, not just post-hoc reports
5. **Scalability**: Add more agents to handle more tasks (across machines)

## Decision

### Architecture: Supervisor as Orchestrator, Agents as Workers

```
hex dev start "feature"
      │
      ▼
  Supervisor Process (hex dev TUI)
      │
      ├─ Creates swarm + tasks in SpacetimeDB
      ├─ Spawns agent worker processes (or connects to existing ones)
      ├─ Assigns tasks to agents via SpacetimeDB reducers
      ├─ Monitors progress via WebSocket subscriptions
      ├─ Handles failures (reassign, escalate, retry)
      └─ Collects results and advances pipeline phases
      │
      ├── hex-coder worker ──── persistent process
      │     ├─ Subscribes to tasks where role = "hex-coder"
      │     ├─ Heartbeats every 30s
      │     ├─ Executes code generation
      │     └─ Writes result to SpacetimeDB
      │
      ├── hex-tester worker ─── persistent process
      │     ├─ Subscribes to tasks where role = "hex-tester"
      │     ├─ Runs after hex-coder completes (dependency)
      │     └─ Writes test results to SpacetimeDB
      │
      └── hex-reviewer worker ── persistent process
            ├─ Subscribes to tasks where role = "hex-reviewer"
            ├─ Can run in parallel with hex-tester
            └─ Writes review findings to SpacetimeDB
```

### Worker process model

Each agent worker is a lightweight process that:

```
loop {
    // 1. Check for assigned tasks (SpacetimeDB subscription)
    let task = wait_for_task_assignment(my_agent_id, my_role);

    // 2. Execute the task
    let result = execute_task(task, agent_definition);

    // 3. Write result back
    call_reducer("task_complete", task.id, result);

    // 4. Heartbeat
    call_reducer("agent_heartbeat", my_agent_id);
}
```

Workers can be:
- **Spawned by supervisor** (`std::process::Command` → `hex agent worker --role hex-coder`)
- **Already running** (long-lived daemon, found via agent registry)
- **Remote** (connected via SSH, ADR-040)

### Task dependency graph

Tasks have dependencies defined by the swarm YAML (ADR-2603240130):

```yaml
# A task becomes assignable when all its dependencies are completed
tasks:
  - id: code-step-1
    role: hex-coder
    depends_on: []

  - id: test-step-1
    role: hex-tester
    depends_on: [code-step-1]    # waits for coder to finish

  - id: review-step-1
    role: hex-reviewer
    depends_on: [code-step-1]    # can run parallel with tester
    parallel_with: [test-step-1]
```

The supervisor creates all tasks upfront. SpacetimeDB reducers enforce: a task can only move to `in_progress` when all `depends_on` tasks are `completed`.

### Heartbeat and failure recovery

```
hex-nexus cleanup loop (existing, ADR-027):
  - Agent stale after 45s without heartbeat
  - Agent dead after 120s → reclaim tasks

Supervisor watches for:
  - Task stuck in in_progress > 60s → check agent heartbeat
  - Agent declared dead → reassign task to another worker
  - All workers for a role dead → spawn new worker
  - Max retries exceeded → escalate or fail the tier
```

### Communication between agents

Agents share context via SpacetimeDB `hexflo_memory` table:

```
hex-coder writes:   memory["step-1:generated_files"] = ["src/domain/calc.ts"]
hex-tester reads:   memory["step-1:generated_files"] → knows what to test
hex-reviewer reads: memory["step-1:generated_files"] → knows what to review
```

This replaces the supervisor's in-memory context passing.

## Implementation

| Phase | Description | Priority |
|-------|------------|----------|
| P1 | `hex agent worker --role <role>` — persistent worker process with task loop + heartbeat | Critical |
| P2 | Supervisor spawns workers (or discovers existing ones) at swarm init | Critical |
| P3 | Task dependency graph in SpacetimeDB — reducers enforce ordering | Critical |
| P4 | Supervisor monitors via WebSocket subscription — reassign on failure | High |
| P5 | Agent-to-agent communication via hexflo_memory | High |
| P6 | Worker reads AgentDefinition from YAML (ADR-2603240130) | Medium |
| P7 | Remote workers via SSH (ADR-040 integration) | Low |
| P8 | Dashboard live agent view — heartbeat indicators, task assignment | Low |

## Consequences

### Positive

- **Real coordination** — agents are independent processes that coordinate through SpacetimeDB
- **Conflict-free** — task assignment prevents two agents editing the same files
- **Resilient** — crashed agents detected and tasks reassigned
- **Parallel** — reviewer and tester work simultaneously
- **Observable** — dashboard shows live agent status
- **Scalable** — add workers on any machine

### Negative

- **Complexity** — process management is harder than function calls
- **Latency** — SpacetimeDB round-trip adds ~5-10ms per coordination step
- **Debugging** — distributed state is harder to inspect than local variables

### Mitigations

- Keep the inline "quick mode" for simple single-file features
- SpacetimeDB WebSocket is fast enough for coordination (not data transfer)
- `hex agent list` + dashboard provide visibility into distributed state
