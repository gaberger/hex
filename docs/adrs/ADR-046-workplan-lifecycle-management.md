# ADR-046: Workplan Lifecycle Management — Creation, Tracking, and Reporting

**Status:** Accepted
**Accepted Date:** 2026-03-22
## Date: 2026-03-21

> **Implementation Evidence:** WorkplanExecutor in `hex-nexus/src/orchestration/workplan_executor.rs` executes phases sequentially, spawns agents per task, enforces phase gates (GateResult), blocks advancement on gate failure. CLI: `hex plan execute/status/pause/resume/report`. MCP tools: `hex_plan_execute/status/report`. REST endpoints wired. Remaining gaps: git commit_sha correlation per task, dashboard WorkplanProgress component.

## Context

Workplans are the primary unit of work coordination in hex. They decompose features into tier-ordered phases with tasks assigned to agents in isolated worktrees. The system has significant implementation:

- **Domain model** (`hex-core/src/domain/workplan.rs`): `Workplan`, `WorkplanPhase`, `WorkplanTask`, `PhaseGate`, `TaskStatus`
- **Executor** (`hex-nexus/src/orchestration/workplan_executor.rs`): Runs phases sequentially, spawns agents per task, supports pause/resume
- **SpacetimeDB tables**: `workplan_execution` (status, current phase) and `workplan_task` (per-task progress)
- **REST endpoints**: Execute, status, pause, resume
- **CLI**: `hex plan create`, `hex plan list`, `hex plan status`
- **15 existing workplan JSONs** in `docs/workplans/`

However, **no ADR governs the workplan lifecycle end-to-end**. Key gaps:

1. **Creation**: How are workplans generated? From requirements text? From ADRs? From chat? The CLI has structural inference but no LLM-driven decomposition path.
2. **Tracking**: SpacetimeDB stores execution state but there's no dashboard view. Remote agents can't see workplan progress. The `workplan:{id}` HexFlo memory key is an implementation detail, not a documented contract.
3. **Reporting**: No aggregate view ("3 of 5 phases complete, 2 tasks failed, ETA unknown"). No historical execution data. No correlation between workplan tasks and git commits.
4. **Phase gates**: Parsed in the domain model but never executed. Validation gates are blocking in name only.
5. **Multi-agent coordination**: Tasks within a phase can run in parallel, but agents can't see what other agents are working on within the same workplan.
6. **Git integration**: No link between workplan tasks and the commits/worktrees they produce.

## Decision

Define the complete workplan lifecycle as a first-class architectural concept with SpacetimeDB as the single source of truth.

### Lifecycle Phases

```
CREATE  →  PLAN  →  EXECUTE  →  VALIDATE  →  REPORT
  │          │         │           │            │
  │          │         │           │            └─ Aggregate results, store in SpacetimeDB
  │          │         │           └─ Run phase gates (blocking/non-blocking)
  │          │         └─ Spawn agents per task, track in SpacetimeDB
  │          └─ Decompose into phases/tasks with tier ordering
  └─ Accept requirements (text, ADR ref, or chat message)
```

### 1. Creation

Workplans are created from three sources:

| Source | Command | Flow |
|--------|---------|------|
| Requirements text | `hex plan create "add auth"` | LLM decomposes → phases/tasks |
| ADR reference | `hex plan create --adr ADR-044` | Extract scope from ADR → decompose |
| Feature dev flow | `/hex-feature-dev` | Interactive: specs → plan → workplan JSON |

All creation paths produce a `docs/workplans/feat-{name}.json` file AND register the workplan in SpacetimeDB.

**Workplan JSON format** (canonical — unify the two existing flavors):

```json
{
  "id": "wp-{uuid}",
  "feature": "git-integration",
  "description": "Add git status, log, diff, and worktree endpoints to hex-nexus",
  "adr": "ADR-044",
  "created_at": "2026-03-21T...",
  "created_by": "planner",
  "phases": [
    {
      "id": "P1",
      "name": "Git Query Service",
      "tier": 1,
      "gate": {
        "type": "build",
        "command": "cargo build -p hex-nexus",
        "blocking": true
      },
      "tasks": [
        {
          "id": "P1.1",
          "name": "Implement git status module",
          "layer": "secondary",
          "description": "...",
          "deps": [],
          "files": ["hex-nexus/src/git/status.rs"],
          "agent": "hex-coder",
          "worktree": "feat/git/status"
        }
      ]
    }
  ]
}
```

### 2. Tracking (SpacetimeDB)

All workplan state lives in SpacetimeDB. The REST executor and CLI are stateless compute that read/write to these tables.

**Tables** (extend existing `workplan-state` module):

```rust
#[spacetimedb(table)]
pub struct WorkplanExecution {
    #[primarykey]
    pub id: String,
    pub project_id: String,
    pub feature: String,
    pub workplan_path: String,
    pub status: String,           // pending, running, paused, completed, failed
    pub current_phase: String,
    pub total_phases: u32,
    pub completed_phases: u32,
    pub total_tasks: u32,
    pub completed_tasks: u32,
    pub failed_tasks: u32,
    pub created_by: String,       // agent name or "user"
    pub started_at: String,
    pub updated_at: String,
    pub completed_at: String,
}

#[spacetimedb(table)]
pub struct WorkplanTask {
    #[primarykey]
    pub id: String,
    pub workplan_id: String,
    pub phase_id: String,
    pub name: String,
    pub layer: String,
    pub status: String,           // pending, running, completed, failed, blocked
    pub agent_id: String,
    pub worktree_branch: String,
    pub commit_sha: String,       // Set on completion
    pub result: String,
    pub started_at: String,
    pub completed_at: String,
}

#[spacetimedb(table)]
pub struct WorkplanGateResult {
    #[primarykey]
    pub id: String,
    pub workplan_id: String,
    pub phase_id: String,
    pub gate_type: String,        // build, test, analyze, custom
    pub passed: bool,
    pub output: String,
    pub checked_at: String,
}
```

**Key behaviors:**
- Dashboard subscribes to `WorkplanExecution` and `WorkplanTask` tables for real-time progress
- Remote agents query `WorkplanTask` to see what's assigned/available
- Phase transitions update `current_phase` and `completed_phases` atomically via reducer
- Task completion sets `commit_sha` linking to the git commit

### 3. Execution

The existing `WorkplanExecutor` is correct architecturally (stateless compute that delegates to SpacetimeDB). Extend it with:

| Feature | Current | Proposed |
|---------|---------|----------|
| Phase gates | Parsed, not executed | Execute gate command, store result in `WorkplanGateResult`, block if `blocking: true` |
| Task dependencies | Phase-level only | Intra-phase deps checked via `ready_tasks()` before spawning |
| Agent output | Fire-and-forget | Stream to SpacetimeDB `WorkplanTask.result` |
| Git correlation | `commit_hash` in `task_complete` | Also set `worktree_branch` and link via `WorkplanTask.commit_sha` |
| Failure handling | Entire workplan fails | Configurable: `fail_fast` (default) or `continue_on_error` |

### 4. Reporting

Add reporting endpoints and CLI commands:

| Endpoint | CLI | Description |
|----------|-----|-------------|
| `GET /api/{project_id}/workplan/active` | `hex plan active` | Currently running workplans |
| `GET /api/{project_id}/workplan/history` | `hex plan history` | All workplan executions |
| `GET /api/{project_id}/workplan/{id}` | `hex plan status <id>` | Detailed status with task breakdown |
| `GET /api/{project_id}/workplan/{id}/report` | `hex plan report <id>` | Aggregate report: phases, tasks, commits, gates, duration |

**Report format:**

```json
{
  "workplan": { "id": "...", "feature": "...", "status": "completed" },
  "summary": {
    "duration_minutes": 12,
    "phases_total": 3,
    "phases_completed": 3,
    "tasks_total": 8,
    "tasks_completed": 7,
    "tasks_failed": 1,
    "commits": 5,
    "agents_used": ["hex-coder", "integrator"],
    "adr_violations_introduced": 0
  },
  "phases": [
    {
      "id": "P1",
      "name": "...",
      "status": "completed",
      "gate": { "passed": true },
      "tasks": [
        { "id": "P1.1", "status": "completed", "commit": "abc1234", "agent": "hex-coder" }
      ]
    }
  ]
}
```

### 5. Integration Points

| System | Integration |
|--------|-------------|
| **Git (ADR-044)** | Task completion records commit SHA. Timeline endpoint merges workplan events with commits. |
| **ADR compliance (ADR-045)** | Phase gates can run `hex analyze --adr-compliance` as a blocking gate. |
| **HexFlo (ADR-027)** | Workplan tasks map 1:1 to HexFlo swarm tasks. Swarm topology matches workplan tier structure. |
| **Dashboard** | SpacetimeDB subscription on `WorkplanExecution` + `WorkplanTask` tables drives real-time progress view. |
| **Remote agents** | Agents read assigned tasks from SpacetimeDB. No polling — subscription pushes updates. |

### 6. MCP Tools

Expose workplan operations as MCP tools for Claude Code integration:

```
mcp__hex__hex_plan_create    → hex plan create <requirements>
mcp__hex__hex_plan_list      → hex plan list
mcp__hex__hex_plan_status    → hex plan status <id>
mcp__hex__hex_plan_execute   → hex plan execute <file>
mcp__hex__hex_plan_pause     → hex plan pause
mcp__hex__hex_plan_resume    → hex plan resume
```

## Consequences

### Positive

- Single source of truth for workplan state (SpacetimeDB)
- Remote agents see real-time progress without polling
- Workplan → git commit traceability (who wrote what, when, in which task)
- Phase gates actually run, blocking broken code from advancing
- Historical execution data enables learning (which task decompositions work?)
- Unified JSON format eliminates the two-flavor inconsistency

### Negative

- More SpacetimeDB tables to maintain
- Phase gates add latency between phases (build/test gates can take minutes)
- Workplan JSON format change requires migrating 15 existing workplans
- Agent output streaming to SpacetimeDB may generate high write volume

### Risks

- **Gate timeouts**: Build gates on large projects could block indefinitely. Mitigate with configurable timeout per gate.
- **Stale workplans**: Abandoned workplans leave worktrees on disk. Mitigate with cleanup protocol (ADR-004) and stale detection.
- **Conflicting edits**: Two agents assigned the same task via race condition. Mitigate with SpacetimeDB reducer atomicity — `assign_task` reducer checks `status == 'pending'` before assigning.

## References

- ADR-004: Git Worktrees for Parallel Agent Isolation
- ADR-022: Coordination Wiring
- ADR-027: HexFlo Swarm Coordination
- ADR-032: SpacetimeDB Migration
- ADR-039: Nexus Agent Control Plane
- ADR-044: Git Integration
- ADR-045: ADR Compliance Enforcement
