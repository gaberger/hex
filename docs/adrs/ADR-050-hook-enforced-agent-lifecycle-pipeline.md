# ADR-050: Hook-Enforced Agent Lifecycle Pipeline

**Status:** Accepted
**Date:** 2026-03-22
**Supersedes:** None
**Extends:** ADR-048 (Session Registration), ADR-046 (Workplan Lifecycle), ADR-027 (HexFlo)

## Context

ADR-048 established session agent registration via SessionStart/SessionEnd hooks. ADR-046 defined workplan lifecycle management. ADR-027 built the HexFlo coordination layer with swarm/task/memory APIs. However, these three systems operate independently — an agent can edit files without an active workplan, create swarms without ADR justification, or bypass HexFlo memory entirely.

The result: stale agents accumulate (no heartbeats), work happens outside tracked swarms, and the dashboard shows an incomplete picture of development activity.

## Decision

Enforce a mandatory lifecycle pipeline through Claude Code hooks:

```
ADR → WorkPlan → HexFlo Memory → HexFlo Swarm → Agent Work → Completion
```

Every hook event validates that the current session participates in this pipeline. The enforcement is **advisory** (warnings, not blocks) in Phase 1, graduating to **mandatory** (blocks) in Phase 2 for production projects.

### Hook Responsibilities

| Hook Event | Lifecycle Enforcement |
|------------|----------------------|
| `SessionStart` | Register agent (ADR-048) + load active workplan from HexFlo memory + resume swarm context |
| `UserPromptSubmit` | Send heartbeat + warn if no active workplan/swarm for the current project |
| `PreToolUse` (Write/Edit) | Validate file falls within workplan's adapter boundary (if workplan active) |
| `PostToolUse` (Write/Edit) | Update HexFlo memory with edit event + increment task progress |
| `PreToolUse` (Bash) | Warn on destructive ops (`git push --force`, `rm -rf`) outside workplan SHIP phase |
| `SessionEnd` | Flush progress to HexFlo memory + disconnect agent + update swarm task status |

### Session State File (Extended)

The session state file (`~/.hex/sessions/agent-{sessionId}.json`) is extended to track lifecycle context:

```json
{
  "agentId": "uuid",
  "name": "claude-abc12345",
  "project": "hex-intf",
  "registered_at": "2026-03-22T...",
  "claude_pid": 16601,
  "workplan_id": "feat-lifecycle-hooks",
  "swarm_id": "swarm-uuid-or-null",
  "current_task_id": "task-uuid-or-null",
  "last_heartbeat": "2026-03-22T...",
  "edits": 0,
  "phase": "CODE"
}
```

### Heartbeat Protocol

The `UserPromptSubmit` hook sends a lightweight heartbeat to hex-nexus on every user interaction. This solves the stale agent problem identified during ADR-048 implementation:

- `POST /api/agents/{agentId}/heartbeat` with `{ "timestamp": "...", "phase": "CODE" }`
- Timeout: 2s, fire-and-forget (never blocks the user)
- Nexus marks agents stale after 45s, dead after 120s (existing cleanup.rs logic)

### Workplan Loading

On SessionStart, after agent registration:
1. Query HexFlo memory for `workplan:active:{project_id}` key
2. If found, load workplan ID, current phase, and assigned task
3. Persist to session state file for use by subsequent hooks
4. Print workplan context in the session banner

### Advisory vs Mandatory Mode

Controlled by `.hex/project.json` field `"lifecycle_enforcement"`:

| Value | Behavior |
|-------|----------|
| `"advisory"` (default) | Print warnings via **stdout** (so Claude sees them in context), never block |
| `"mandatory"` | Block edits outside workplan boundary (exit code 2), block destructive ops outside SHIP phase |

**Critical**: Warnings MUST use `stdout`, not `stderr`. Claude Code hooks only inject stdout into the agent's conversation context. Stderr goes to the user's terminal but is invisible to Claude, defeating the purpose of advisory warnings.

## Consequences

### Positive
- Every agent action is traceable to a workplan → ADR chain
- Stale agents cleaned up automatically via heartbeats
- Dashboard shows real-time, accurate development activity
- Workplan boundaries prevent accidental cross-adapter coupling

### Negative
- Hook latency adds ~5-10ms per tool invocation (acceptable given existing hook overhead)
- Requires nexus to be running for full enforcement (graceful degradation when offline)
- Advisory mode means enforcement is opt-in initially

### Risks
- Over-enforcement could frustrate developers doing exploratory work — mitigated by advisory default
- Session state file could become stale if process crashes — mitigated by heartbeat timeout cleanup

## Implementation

All changes in `hex-cli/src/commands/hook.rs`:

1. Extend `session_start` to load workplan context from HexFlo memory
2. Extend `session_end` to flush progress and update swarm task
3. Add heartbeat call to `route` (UserPromptSubmit handler)
4. Add workplan boundary validation to `pre_edit`
5. Add progress tracking to `post_edit`
6. Add destructive command detection to `pre_bash`
7. Extend session state file schema with lifecycle fields
