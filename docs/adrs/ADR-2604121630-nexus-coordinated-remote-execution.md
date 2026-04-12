# ADR-2604121630: Nexus-Coordinated Remote Agent Execution

**Status:** Proposed
**Date:** 2026-04-12
**Drivers:** Session proving hex plan execute works on bazzite (remote GPU box) with local Ollama, but execution state stays local — coordinator nexus has no visibility into remote task results, gate outcomes, or RL rewards. Remote agents must report back to SpacetimeDB.

## Context

hex can now execute workplans on remote machines via `hex plan execute` with the ADR-005 gate pipeline (compile → test → retry → escalate). We proved this on bazzite: 109 lines of domain code + 5 passing tests, zero intervention, $0 cloud cost.

However, the current local executor is **disconnected from nexus**. Execution state lives only in stdout. The coordinator can't:
- See which tasks are in progress on remote agents
- Collect gate results (compile pass/fail, test outcomes)
- Aggregate RL rewards from remote dispatches
- Generate reports via `hex plan report`
- Reassign failed tasks to other agents

This breaks hex's core promise: **nexus is the single coordinator for all agent activity**.

### What exists today

| Component | Status |
|-----------|--------|
| `hex agent connect` | Working — bazzite registers as remote agent |
| `hex agent list` | Working — shows local + remote agents |
| `hex agent worker` | CLI exists but doesn't poll for workplan tasks |
| `hex plan execute` (local) | Working — ADR-005 gates, but no SpacetimeDB reporting |
| SSH tunnel | Working — bazzite:5556 → mac:5555 |
| SpacetimeDB workplan tables | Exist — `workplan_execution`, `workplan_task_update` reducers |
| HexFlo task tracking | Working — swarm tasks with assignment + completion |

### What's missing

| Gap | Impact |
|-----|--------|
| Worker doesn't poll nexus for tasks | Remote agents can't receive work from coordinator |
| Local executor doesn't report to SpacetimeDB | `hex plan report` is blind to remote executions |
| RL rewards stay local | Coordinator's Q-table doesn't learn from remote dispatches |
| No task reassignment on failure | Failed remote tasks can't cascade to other agents |

## Decision

hex SHALL implement **nexus-coordinated remote execution** where the coordinator nexus drives workplan execution and remote agents act as workers that poll, execute, and report back.

### Architecture

```
Coordinator (Mac + nexus + SpacetimeDB)
  │
  ├── hex plan execute workplan.json
  │     Registers execution in SpacetimeDB
  │     Creates HexFlo tasks per workplan task
  │     Assigns tasks to available agents by capability
  │
  ├── SpacetimeDB (single source of truth)
  │     workplan_execution: id, status, phases, timestamps
  │     workplan_task: id, agent_id, status, gate_results, code_output
  │     rl_experience: rewards from all agents
  │
  └── hex plan report <id>
        Reads from SpacetimeDB — works for local AND remote

Remote Agent (bazzite + hex agent worker)
  │
  ├── hex agent worker --role hex-coder
  │     Connects to coordinator nexus via SSH tunnel
  │     Polls /api/hexflo/tasks/poll?role=hex-coder
  │     Receives task assignment with prompt + tier + model
  │
  ├── Execute locally
  │     Ollama inference (local GPU)
  │     ADR-005 gate pipeline (compile + test + retry)
  │     GBNF grammar constraints
  │
  └── Report back
        PATCH /api/hexflo/tasks/{id} with status + gate_results + code
        POST /api/rl/record_reward with dispatch outcome
        Agent heartbeat continues throughout
```

### Phase 1: Worker poll loop (hex agent worker)

1. `hex agent worker --role hex-coder --nexus http://localhost:5556` connects to coordinator
2. Registers agent in SpacetimeDB via `hex agent connect`
3. Polls `GET /api/hexflo/tasks/poll?agent_id={id}&role=hex-coder` every 2s
4. When a task is assigned:
   a. Downloads task prompt, tier, model, target files
   b. Runs local Ollama inference with tier-appropriate model
   c. Runs ADR-005 gate pipeline (compile → test → retry)
   d. Reports result: `PATCH /api/hexflo/tasks/{id}` with status, gate results, generated code
   e. Records RL reward: `POST /api/rl/record_reward`
5. Polls for next task

### Phase 2: Coordinator dispatch (hex plan execute → nexus)

1. `hex plan execute workplan.json` registers execution in SpacetimeDB
2. For each phase, creates HexFlo tasks with tier + model + prompt
3. Assigns tasks to available agents by matching tier → agent capability
4. Monitors task completion via SpacetimeDB subscription
5. Runs phase gate after all tasks in phase complete
6. Advances to next phase or reports failure

### Phase 3: Reporting and RL aggregation

1. `hex plan report <execution_id>` reads from SpacetimeDB
2. Shows: phases, tasks, agents, gate results, duration, token counts
3. RL rewards from all agents aggregate in the coordinator's Q-table
4. `hex inference escalation-report` shows cross-agent escalation rates

## Impact Analysis

### Consumer Dependency Map

| Artifact | Consumers | Impact |
|----------|-----------|--------|
| `hex agent worker` (new) | hex-cli/commands/agent.rs | LOW — new subcommand |
| `/api/hexflo/tasks/poll` (new) | hex-nexus/routes/swarms.rs | LOW — new endpoint |
| `workplan_execution` SpacetimeDB table | hexflo-coordination module | LOW — uses existing tables |
| `hex plan execute` (modified) | hex-cli/commands/plan.rs | MEDIUM — adds nexus dispatch path |

### Build Verification Gates

| Gate | Command | After Phase |
|------|---------|-------------|
| Workspace compile | `cargo check --workspace` | Every phase |
| CLI tests | `cargo test -p hex-cli` | Phase 1 |
| Nexus tests | `cargo test -p hex-nexus` | Phase 2 |
| E2E: worker on bazzite | `ssh bazzite "hex agent worker --once"` | Phase 1 |
| E2E: coordinated execution | `hex plan execute` with bazzite worker | Phase 2 |

## Consequences

**Positive:**
- `hex plan report` works for all executions (local + remote)
- RL Q-table learns from all agents across the fleet
- Failed tasks can be reassigned to other agents
- Full observability: dashboard shows remote agent activity in real-time

**Negative:**
- Requires SSH tunnel or network connectivity between agent and coordinator
- SpacetimeDB becomes a hard dependency for remote execution (no offline fallback)
- Worker poll loop adds 2s latency per task (vs immediate local dispatch)

**Mitigations:**
- Local fallback (`hex plan execute --local`) still works without nexus for single-machine use
- Poll interval is configurable
- SSH tunnel setup is already proven and documented

## References

- ADR-005: Compile-Lint-Test Feedback Loop with Quality Gates
- ADR-040: Remote Agent Transport — WebSocket over SSH
- ADR-2603241231: Persistent Agent Coordination via SpacetimeDB
- ADR-2604112000: Hex Standalone Dispatch
- ADR-2604120202: Tiered Inference Routing with Local Model Scaffolding
