# ADR-2603221939: Mandatory Swarm Tracking for Background Agents

**Status:** Accepted
**Date:** 2026-03-22
**Drivers:** Background agents spawned without `HEXFLO_TASK:` prefix bypass HexFlo tracking entirely. The `Agent` tool is the only tool without a `PreToolUse` hook gate. Discovered when ADR-056 style conversion agents ran invisibly — no swarm status, no dashboard visibility, no session continuity.

## Context

hex-agent enforces an architecture pipeline (ADR-050) where all code development flows through:

```
ADR → Workplan → HexFlo Swarm → Tracked Agents → Completed Tasks
```

The enforcement relies on hooks that fire on Claude Code events. Current hook coverage:

| Event | Hook | Enforced? |
|-------|------|-----------|
| SessionStart | `hex hook session-start` | Yes — registers agent |
| UserPromptSubmit | `hex hook route` | Yes — heartbeat, inbox, workplan check |
| PreToolUse (Write/Edit) | `hex hook pre-edit` | Yes — boundary validation |
| PreToolUse (Bash) | `hex hook pre-bash` | Yes — destructive command warning |
| **PreToolUse (Agent)** | **none** | **NO — critical gap** |
| SubagentStart | `hex hook subagent-start` | Partial — only tracks if `HEXFLO_TASK:` present |
| SubagentStop | `hex hook subagent-stop` | Partial — only completes if SubagentStart set task |

The `Agent` tool is the most impactful tool — it spawns autonomous processes that edit files, run commands, and make architectural decisions. Yet it has zero pre-flight validation.

### Bypass vectors

1. **No PreToolUse on Agent** — agents spawn without any hook validation
2. **HEXFLO_TASK is optional** — SubagentStart silently no-ops without it
3. **No swarm existence check** — task IDs aren't validated against active swarms
4. **No workplan context propagation** — subagents don't inherit parent workplan
5. **Lazy registration gaps** — if agent unregistered, SubagentStart can't assign tasks
6. **Stderr warnings invisible** — pre_bash warnings go to stderr, not stdout
7. **No task ownership validation** — SubagentStart doesn't check if task is already assigned

### Impact

- Swarm status shows incomplete progress (agents ran but tasks stay "pending")
- Dashboard has no visibility into background work
- Session continuity breaks — future sessions can't reconcile untracked agents via `git log`
- Workplan gates can't auto-close because task completion was never recorded
- Agent fleet shows stale data — background agents never registered

## Decision

### P1: Add PreToolUse hook on Agent tool

Add an `Agent` matcher to the hooks configuration:

```json
{
  "matcher": "Agent",
  "hooks": [{
    "type": "command",
    "command": "hex hook pre-agent",
    "timeout": 3000
  }]
}
```

The `pre-agent` hook reads the tool input from stdin (JSON with `prompt`, `subagent_type`, `run_in_background`, etc.) and enforces:

**For background agents (`run_in_background: true`):**
- MUST contain `HEXFLO_TASK:{uuid}` in the prompt → exit(2) if missing
- The task UUID MUST exist in an active swarm → exit(2) if not found
- The parent session MUST have an active workplan OR the task must be in an existing swarm

**For foreground agents:**
- Advisory warning if no `HEXFLO_TASK:` present (exit(0), not blocking)
- Log the agent spawn to session state for audit trail

**Exempt agent types** (never blocked):
- `Explore` — read-only research
- `Plan` — planning, no code changes
- `claude-code-guide` — documentation queries

### P2: Harden SubagentStart

Enhance `hex hook subagent-start` to:
1. Validate task ownership — reject if task already assigned to a different agent
2. Sync agent registration if session agent_id is empty (lazy connect)
3. Send heartbeat on subagent spawn (not just on UserPromptSubmit)

### P3: Fix stderr visibility

Change `pre_bash` warnings from `eprintln!` to `println!` so Claude sees them. Stderr is invisible to the AI — warnings about destructive commands have zero enforcement value when only the human terminal sees them.

### P4: Workplan context propagation

When `pre-agent` fires, inject the parent session's workplan context into the agent prompt:
- `HEXFLO_WORKPLAN:{workplan_id}` — so subagents know which workplan they serve
- SubagentStart records this in session state alongside `current_task_id`
- SubagentStop includes workplan_id in the task completion payload

### P5: Audit trail for untracked agents

Add a `hex agent audit` command that:
1. Reads `git log` for recent commits
2. Cross-references against HexFlo task completions
3. Flags commits that don't map to any tracked task
4. Reports "dark agents" — work done outside swarm tracking

## Consequences

**Positive:**
- All background agent work becomes visible in HexFlo and dashboard
- Swarm status accurately reflects real progress
- Session continuity works — future sessions can reconcile state
- Workplan gates can auto-close based on task evidence
- Agent fleet shows real-time activity for all agents
- Audit trail catches enforcement bypasses retroactively

**Negative:**
- Slightly more friction when spawning agents (must create swarm/tasks first)
- PreToolUse hook adds ~50ms latency per agent spawn
- Exempt list requires maintenance as new agent types are added

**Mitigations:**
- Exempt list covers read-only agents (Explore, Plan) — no friction for research
- Hook timeout is 3s — fails open if nexus is unreachable (advisory mode)
- `hex swarm init` + `hex task create` are fast (<100ms each)

## Implementation

| Phase | Description | Status |
|-------|------------|--------|
| P1 | Add PreToolUse hook on Agent tool with HEXFLO_TASK enforcement | Done |
| P2 | Harden SubagentStart — ownership validation, lazy connect, heartbeat | Done |
| P3 | Fix stderr → stdout for pre_bash warnings | Done |
| P4 | Workplan context propagation to subagents | Done |
| P5 | `hex agent audit` command for retroactive tracking | Done |

## References

- ADR-048: Claude Code Session Agent Registration
- ADR-050: Hook-Enforced Agent Lifecycle Pipeline
- ADR-027: HexFlo Swarm Coordination
- ADR-065: Registration Lifecycle Gaps
- Incident: ADR-056 style agents ran without swarm tracking (2026-03-22)
