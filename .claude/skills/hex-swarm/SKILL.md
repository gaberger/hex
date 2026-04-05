---
name: hex-swarm
description: Manage HexFlo swarm coordination for multi-agent development. Use when the user asks to "start swarm", "swarm status", "coordinate agents", "multi-agent", "parallel agents", "hexflo", or "swarm cleanup".
---

# Hex Swarm — HexFlo Multi-Agent Coordination

HexFlo is hex's native Rust swarm coordination layer (ADR-027). It manages task tracking, agent lifecycle, memory persistence, and heartbeat monitoring for multi-agent feature development. State lives in SpacetimeDB with SQLite fallback.

## When to Use Swarm Mode

| Scenario | Mode | Why |
|----------|------|-----|
| Feature spans 2+ adapters | **Swarm** | Parallel worktrees, multiple hex-coder agents |
| Single adapter change | Single-agent | Overhead not justified |
| Critical/learning feature | Interactive | Human review at each phase |

## Parameters

Ask the user for:
- **action** (required): One of: init, status, monitor, cleanup, recover
- **name** (required for init): Swarm name (usually the feature name)
- **topology** (optional, default: hierarchical): hierarchical, mesh, or pipeline

## Action: init

Initialize a new HexFlo swarm for a feature.

### Steps

1. Initialize the swarm:
```tool
mcp__hex__hex_hexflo_swarm_init({
  topology: "hierarchical",
  maxAgents: 8,
  strategy: "specialized"
})
```

2. Reconcile any prior state from previous sessions:
```tool
mcp__hex__hex_hexflo_task_list()
```

3. Cross-reference against git history:
```bash
git log --oneline -10
```

4. Mark any completed tasks:
```tool
mcp__hex__hex_hexflo_task_complete({
  task_id: "...",
  result: "feat(adapter): summary — commit abc1234"
})
```

5. Store swarm metadata:
```tool
mcp__hex__hex_hexflo_memory_store({
  key: "swarm/<name>/config",
  value: {
    name: "<name>",
    topology: "hierarchical",
    max_agents: 8,
    started_at: "ISO timestamp",
    feature: "<feature-name>"
  }
})
```

### Topology Options

- **hierarchical** (default): Coordinator → planner → hex-coders → integrator. Best for features with clear dependency tiers.
- **mesh**: All agents peer-to-peer. Best for independent tasks with minimal coordination.
- **pipeline**: Sequential handoff. Best for linear workflows (spec → plan → code → validate).

## Action: status

Show the current swarm state.

### Steps

1. Get swarm status:
```tool
mcp__hex__hex_hexflo_swarm_status()
```

2. Get all tasks:
```tool
mcp__hex__hex_hexflo_task_list()
```

3. Report:
   - Active swarm name and topology
   - Tasks by status: pending, in_progress, completed, failed
   - Agent assignments and heartbeat status
   - Current tier being executed
   - Estimated completion (tasks remaining / rate)

## Action: monitor

Continuous monitoring of swarm progress. Use during active multi-agent execution.

### Steps

1. Check task list for newly completed agents:
```tool
mcp__hex__hex_hexflo_task_list()
```

2. For each newly completed task:
   - Verify the commit exists: `git log --oneline -1 <hash>`
   - Mark task complete in HexFlo
   - Check if next-tier tasks are unblocked

3. For stale agents (no heartbeat in 45s):
   - Flag as potentially stuck
   - After 120s, reclaim tasks and reassign

4. When all tasks in a tier complete:
   - Update progress in HexFlo memory
   - Spawn agents for the next tier
   - Report tier completion to user

5. Retrieve progress:
```tool
mcp__hex__hex_hexflo_memory_retrieve({
  key: "feature/<feature>/progress"
})
```

## Action: cleanup

Tear down a completed or abandoned swarm.

### Steps

1. Get final task status:
```tool
mcp__hex__hex_hexflo_task_list()
```

2. Verify all tasks are completed or explicitly abandoned

3. Store final report:
```tool
mcp__hex__hex_hexflo_memory_store({
  key: "swarm/<name>/final-report",
  value: {
    status: "completed",
    tasks_total: N,
    tasks_completed: N,
    tasks_failed: N,
    duration_minutes: N,
    completed_at: "ISO timestamp"
  }
})
```

4. Clean up stale agent registrations:
```bash
hex swarm cleanup
```

## Action: recover

Resume a swarm from a previous session.

### Steps

1. Retrieve swarm config:
```tool
mcp__hex__hex_hexflo_memory_retrieve({
  key: "swarm/<name>/config"
})
```

2. Retrieve progress:
```tool
mcp__hex__hex_hexflo_memory_retrieve({
  key: "feature/<feature>/progress"
})
```

3. Get current task list and reconcile against git log:
```tool
mcp__hex__hex_hexflo_task_list()
```

4. For tasks marked in_progress but with commits:
```tool
mcp__hex__hex_hexflo_task_complete({
  task_id: "...",
  result: "Recovered from previous session — commit <hash>"
})
```

5. Resume spawning agents for the next incomplete tier

## Agent Spawning Rules

**CRITICAL**: Background agents that write files MUST use `mode: "bypassPermissions"`.

```
Agent({
  subagent_type: "coder",
  mode: "bypassPermissions",        # REQUIRED for background file writes
  run_in_background: true,
  prompt: "..."
})
```

**WRONG** — silently blocks all file writes:
```
Agent({ mode: "acceptEdits", run_in_background: true })
```

### Parallel Spawning

Spawn ALL independent agents in a SINGLE message with multiple tool calls:
```
# Tier 1: secondary adapters (all independent)
Agent({ prompt: "git-adapter task...", run_in_background: true })
Agent({ prompt: "fs-adapter task...", run_in_background: true })
Agent({ prompt: "llm-adapter task...", run_in_background: true })
```

## Worktree Agent Dispatch Rules (ADR-2604050900 Learnings)

### Parallelize by FILE BOUNDARY, Serialize by FILE OVERLAP

**RIGHT** — agents modify different files (use `isolation: "worktree"`):
```
Agent({ prompt: "Delete modules from spacetime-modules/", isolation: "worktree" })  # touches Cargo.toml, deletes dirs
Agent({ prompt: "Add table to hexflo-coordination/src/lib.rs", isolation: "worktree" })  # touches one .rs file
```

**WRONG** — multiple agents editing the same file in parallel worktrees:
```
# BAD: all 3 agents append to hexflo-coordination/src/lib.rs
Agent({ prompt: "Absorb fleet-state...", isolation: "worktree" })
Agent({ prompt: "Absorb lifecycle...", isolation: "worktree" })
Agent({ prompt: "Absorb cleanup...", isolation: "worktree" })
# Result: 3 independent diffs that can't merge cleanly
```

**FIX** — batch same-file edits into one agent, or run sequentially:
```
# GOOD: one agent does all 3 absorptions
Agent({ prompt: "Absorb fleet-state, lifecycle, and cleanup into hexflo-coordination..." })
```

### Worktree Branch Alignment

Agent prompts MUST include explicit branch checkout:
```
Agent({
  prompt: "FIRST: git fetch origin && git checkout claude/feature-branch\nTHEN: <actual task>",
  isolation: "worktree"
})
```
Without this, worktree agents branch from an older base commit, causing cherry-pick conflicts.

### Cherry-Pick vs Direct Merge

- Worktree agents commit to `worktree-agent-*` branches
- Use `git cherry-pick <hash>` to bring changes to the feature branch
- If cherry-pick conflicts, the agent's worktree was not aligned — do the work directly instead

### When NOT to Use Worktrees

| Task Type | Use Worktree? | Why |
|-----------|---------------|-----|
| Delete files/dirs | Yes | Independent, no conflicts |
| Add new file | Yes | No overlap risk |
| Modify different files | Yes | Clean parallel execution |
| Multiple edits to same file | **No** | Merge conflicts guaranteed |
| <3 small sequential tasks | **No** | Direct execution is faster than coordination overhead |
| Tasks requiring prior task output | **No** | Sequential dependency — run in order |

## Heartbeat Protocol

| Event | Timeout | Action |
|-------|---------|--------|
| Heartbeat sent | Every 15s | Agent alive |
| No heartbeat | 45s | Mark agent stale |
| No heartbeat | 120s | Mark agent dead, reclaim tasks |

## HexFlo Memory Scopes

| Scope | Key Pattern | Use |
|-------|-------------|-----|
| Global | `swarm/<name>/*` | Swarm-level config and reports |
| Feature | `feature/<name>/*` | Feature progress, worktrees, workplan |
| Agent | `agent/<id>/*` | Per-agent state (rare) |

## Quick Reference

| Command | What it does |
|---------|-------------|
| `/hex-swarm init <name>` | Initialize a new HexFlo swarm |
| `/hex-swarm status` | Show swarm state and task progress |
| `/hex-swarm monitor` | Monitor active multi-agent execution |
| `/hex-swarm cleanup` | Tear down completed swarm |
| `/hex-swarm recover` | Resume swarm from previous session |
