# ADR-2604010000: Unified Execution Path

**Status:** Accepted
**Date:** 2026-04-01
**Supersedes:** ADR-2603312210 (Claude Code Bypass Mode — retired, approach is architecturally incorrect)
**Extends:** ADR-031 (RL model selection), ADR-061 (workplan lifecycle), ADR-2603221939 (mandatory swarm tracking)
**Relates to:** ADR-046 (SpacetimeDB single authority), ADR-027 (HexFlo coordination), ADR-060 (agent inbox)

---

## Context

ADR-2603312210 introduced `claude -p --dangerously-skip-permissions` as a bypass when the Anthropic vault had no credits. This approach is **architecturally incorrect**:

- Bypasses RL model selection (ADR-031) — model is always whatever `claude` defaults to
- Bypasses HexFlo task tracking (ADR-2603221939) — agents run invisibly
- Bypasses inference gateway audit trail — no cost tracking, no token counts
- Breaks SpacetimeDB as single execution authority (ADR-046)
- Creates a divergent code path that only works from a Claude Code session
- Subprocess approach caused silent hangs (nexus daemon PATH ≠ user shell PATH)

Additionally, ADR-061 has two documented implementation gaps that have remained open:
1. No `commit_sha` correlation per workplan task
2. Workplan executor tracks phases in SpacetimeDB but does not create individual HexFlo tasks per workplan task

HexFlo memory is used as an execution store but with no documented namespace conventions — keys are implementation details, not contracts.

---

## Decision

### 1. Two canonical execution paths — no bypass

All agent execution flows through one of two paths. Both are mandatory. Neither bypasses RL or swarm tracking.

#### Path A: Daemon (no active Claude Code session)

```
workplan executor
  → HexFlo task created (matching workplan task ID)
  → inference gateway: POST /api/inference/execute
      → RL model selection (ADR-031) → vault → LLM provider
  → agent output → gate validation
  → HexFlo task completed (result + commit_sha stored)
  → HexFlo memory ledger updated
```

#### Path B: Claude Code session (CLAUDECODE=1 in nexus environment)

```
workplan executor
  → HexFlo task created (matching workplan task ID)
  → inference gateway: POST /api/inference/queue (marks task as awaiting_session)
  → inbox notification sent to active Claude Code agent (ADR-060)
  → outer Claude Code session receives notification
  → spawns Agent tool with prompt + HEXFLO_TASK:{id}
  → SubagentStop hook marks HexFlo task complete (commit_sha from git)
  → HexFlo memory ledger updated
```

Detection: nexus checks `CLAUDECODE` env var (reliable, set by Claude Code for all child processes). If set, use Path B. Otherwise Path A.

Both paths are **identical from the workplan executor's perspective** — it creates a HexFlo task and POSTs to `/api/inference/execute` or `/api/inference/queue`. The gateway routes appropriately.

### 2. HexFlo task per workplan task (closes ADR-061 gap 1)

The workplan executor **must** create a HexFlo swarm task for each workplan task before spawning an agent. Task ID in HexFlo must match the workplan task ID exactly (e.g., `P1.2`). This enables:

- Per-task status visibility in `hex plan active`
- Commit correlation (SubagentStop hook writes `commit_sha` to the HexFlo task result)
- RL reward signal tied to specific task outcome

### 3. Commit SHA correlation per workplan task (closes ADR-061 gap 2)

When a task agent completes, the SubagentStop hook calls `git log -1 --format=%H` in the worktree and POSTs it to `PATCH /api/hexflo/tasks/{id}` as `commit_sha`. This creates a durable link:

```
workplan task P1.2 → HexFlo task P1.2 → commit abc1234 → files changed
```

### 4. HexFlo memory ledger conventions

All execution artifacts are stored in HexFlo memory under the following namespace contract:

| Key pattern | Content | Scope |
|---|---|---|
| `workplan:{workplan_id}:execution:{execution_id}` | Full execution record: phases, gate results, verdict | global |
| `workplan:{workplan_id}:task:{task_id}:outcome` | `{model, tokens, iterations, gate_passed, commit_sha, duration_ms}` | global |
| `workplan:{workplan_id}:model-performance` | Aggregated RL reward signal per task_type from this workplan | global |
| `agent:{role}:task:{task_id}:context` | Prior decisions, errors, and relevant ADRs injected before agent dispatch | per-swarm |

These keys are **documented contracts**, not implementation details. Any code reading or writing execution state must use these keys.

### 5. RL model selection applies to all workplan agents

The workplan executor passes `task_type` (derived from the workplan task `layer` field: `domain` → `CodeGeneration`, `ports` → `InterfaceDesign`, `primary`/`secondary` → `CodeGeneration`, `test` → `Testing`) to the inference gateway. The gateway applies ADR-031 Q-table lookup before selecting a model. Outcome (gate pass/fail, iterations) is written back as a reward signal under `workplan:{id}:model-performance`.

---

## Consequences

**Positive:**
- Single execution path regardless of environment — no conditional logic per execution context
- RL model selection applies to 100% of workplan agents
- Every workplan task has a corresponding HexFlo task — full per-task visibility
- Commit SHAs are durably linked to workplan tasks
- HexFlo memory keys are stable contracts — agents can query prior execution context

**Negative:**
- Path B (Claude Code) requires the outer session to be active and polling inbox — daemon-only deployments must use Path A
- Removing bypass requires vault credits OR a funded inference provider for Path A

**Migration:**
- ADR-2603312210 is retired. Remove `claude -p` subprocess code from `agent_manager.rs`
- Implement Path B queue endpoint in nexus: `POST /api/inference/queue`
- Update workplan executor to create HexFlo tasks before spawning agents
- Update SubagentStop hook to write commit_sha

---

## Implementation

| Phase | Task | Files |
|---|---|---|
| P1 | Remove bypass spawn code from agent_manager; add CLAUDECODE detection → path routing | `hex-nexus/src/orchestration/agent_manager.rs` |
| P2 | Add `POST /api/inference/queue` endpoint; inbox notification on queue | `hex-nexus/src/routes/inference.rs`, `hex-nexus/src/routes/mod.rs` |
| P3 | Workplan executor creates HexFlo task per workplan task before spawn | `hex-nexus/src/orchestration/workplan_executor.rs` |
| P4 | SubagentStop hook writes commit_sha to HexFlo task result | `hex-cli/assets/hooks/hex/subagent-stop.yml` or hook handler |
| P5 | Inference gateway applies RL model selection to workplan agent tasks | `hex-nexus/src/orchestration/agent_manager.rs`, inference routes |
| P6 | Memory ledger: store task outcome under documented key after each task | `hex-nexus/src/orchestration/workplan_executor.rs` |
