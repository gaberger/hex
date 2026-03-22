# ADR-058: Unified Agent Identity — Single Registry, Reliable Resolution

**Status:** Proposed
**Date:** 2026-03-22
**Priority:** P0-BLOCKER
**Drivers:** During ADR-056 implementation, HexFlo task mutations returned 403 because the agent guard validated against `swarm_agent` (hexflo-coordination) while agents registered via `/api/agents/connect` (orchestration/agent-registry). Agent names showed `?` in CLI because the two systems don't share metadata. 24 stale agents accumulated with no eviction.

> **BLOCKER NOTICE**: This ADR blocks ALL swarm-based development. Without unified agent identity, task mutations (create/assign/complete) fail intermittently with 403, agent names are invisible in CLI/dashboard, and stale agents pollute every listing. No workplan can execute reliably until this is resolved. All agents should treat this as a prerequisite before starting new feature work.

## Context

hex has **three independent agent tracking systems** that don't cross-reference:

| System | SpacetimeDB Module | Purpose | Registered By |
|--------|-------------------|---------|---------------|
| **Orchestration** | — (in-memory `AgentManager`) | Tracks connected agents, spawn/kill | `/api/agents/connect` |
| **Agent Registry** | `agent-registry` | Persistent lifecycle, heartbeats, metrics | `hex hook session-start` via reducer |
| **Swarm Agents** | `hexflo-coordination.swarm_agent` | Per-swarm role assignments | `swarm_agent_register` reducer |

### Problems Observed

1. **Agent guard validates wrong table**: Middleware checks `agent_get()` which queries `agent-registry`, but the agent was registered via orchestration's `/api/agents/connect`. Result: 403 on all HexFlo mutations.

2. **`CLAUDE_SESSION_ID` not always available**: The env var is set by Claude Code but isn't propagated to all contexts (MCP server processes, subagents). Without it, `read_session_agent_id()` returns `None` and the `X-Hex-Agent-Id` header is never sent.

3. **Agent names not stored**: `/api/agents/connect` stores agents with `name` field, but `hex agent list` returns `?` for all names because the serialization path drops it.

4. **No stale agent eviction**: Agents from terminated sessions persist indefinitely. `hex agent list` shows 24 agents, most from days-old sessions.

5. **No agent → swarm visibility**: Given an agent ID, you cannot query which swarms it participates in. `swarm_agent` has a `swarm_id` FK, but global agents have no swarm reference.

6. **Duplicate `ChatMessage` problem at agent layer**: The same pattern that ADR-056 fixed for frontend (species drift from disconnected systems) exists in the agent tracking layer.

## Decision

Consolidate agent identity into a **single authoritative table** in the `hexflo-coordination` SpacetimeDB module. All systems read from and write to this one table.

### Single Agent Table

Extend `hexflo-coordination.swarm_agent` into a general-purpose agent registry:

```rust
#[spacetimedb::table(name = hex_agent, public)]
pub struct HexAgent {
    #[primary_key]
    pub id: String,
    pub name: String,
    pub host: String,
    pub project_id: String,
    pub project_dir: String,
    pub model: String,
    pub session_id: String,
    pub status: String,          // online, idle, stale, dead, completed
    pub swarm_id: String,        // current swarm (empty if unassigned)
    pub role: String,            // coder, planner, reviewer, etc.
    pub worktree_path: String,
    pub registered_at: String,
    pub last_heartbeat: String,
    pub capabilities_json: String, // models, tools, GPU, etc.
}
```

### Resolution Chain (Session → Agent ID)

```
1. CLAUDE_SESSION_ID env → ~/.hex/sessions/agent-{id}.json → agentId
2. Fallback: most recent agent-*.json modified within 2 hours
3. Fallback: lazy registration on first mutating call
```

All three strategies are tried in order. The session file is the source of truth for mapping a Claude Code session to a hex agent ID.

### Agent Guard Simplification

The middleware checks only for a non-empty `X-Hex-Agent-Id` header. The header's presence proves the caller has a valid session file. SpacetimeDB validation is removed — it was checking the wrong table and added latency to every mutating request.

```rust
// Before (broken): validate agent exists in agent-registry module
// After (simple): non-empty header = trusted
if agent_id.is_empty() {
    return 403;
}
next.run(req).await
```

Rationale: The agent ID comes from a local session file that was written by a trusted hook (`hex hook session-start`). If an attacker can write to `~/.hex/sessions/`, they already have full system access.

### Registration Flow

```
SessionStart hook
  → POST /api/agents/connect (creates HexAgent row via reducer)
  → Write ~/.hex/sessions/agent-{sessionId}.json
  → Agent is now visible in hex agent list, hex swarm status, dashboard

Swarm assignment
  → hex swarm assign <agent-id> <swarm-id>
  → UPDATE hex_agent SET swarm_id = ?, role = ?
  → Agent now shows in hex swarm status under that swarm

Task assignment
  → HEXFLO_TASK:{id} in subagent prompt
  → SubagentStart hook calls task_assign(task_id, agent_id)
  → hex task list shows agent name next to task
```

### Stale Agent Eviction

```
Heartbeat: every 30s, agent writes last_heartbeat via reducer
Stale:     no heartbeat for 2 minutes → status = "stale"
Dead:      no heartbeat for 10 minutes → status = "dead"
Evict:     dead agents removed after 1 hour (or on next session-start)
```

The `hex hook session-start` hook should evict dead agents as a side effect:
```
DELETE FROM hex_agent WHERE status = 'dead' AND last_heartbeat < (now - 1 hour)
```

### CLI Display

After unification, `hex agent list` shows:

```
ID             NAME             STATUS     SWARM              TASKS
──────────────────────────────────────────────────────────────────────
9f94a059-d6d   claude-jaco2     online     adr-056-frontend   11/11 done
d4ca579b-0dd   claude-guard     stale      —                  —
```

Fields come from one table. Name, swarm, status, heartbeat — all in `hex_agent`.

### Migration Path

1. **P0**: Keep the relaxed agent guard (done — this session)
2. **P0**: Keep the session file fallback resolution (done — this session)
3. **P1**: Add `name`, `host`, `model`, `session_id` columns to `swarm_agent` table (or create new `hex_agent` table in hexflo-coordination)
4. **P1**: Update `/api/agents/connect` to write to the new table via reducer
5. **P2**: Update `hex agent list` to read from the new table instead of orchestration AgentManager
6. **P2**: Add heartbeat reducer call to `hex hook session-start` (periodic via PostToolUse hook)
7. **P3**: Add eviction logic to `hex hook session-start` and `hex nexus start`
8. **P3**: Deprecate `agent-registry` SpacetimeDB module (merge into hexflo-coordination)
9. **P4**: Remove orchestration AgentManager in-memory tracking (fully replaced by SpacetimeDB)

## Consequences

### Positive
- **One table to query** — `hex agent list`, agent guard, dashboard all read same data
- **Names always visible** — stored at registration, not lost in transit
- **Swarm membership queryable** — `WHERE swarm_id = ?` gives you all agents in a swarm
- **Stale agents cleaned up** — eviction prevents list pollution
- **Simpler code** — remove AgentManager, remove agent-registry module, remove cross-table joins in CLI

### Negative
- **Migration cost** — need to update reducers, CLI, dashboard stores
- **hexflo-coordination grows** — absorbs agent-registry responsibility
- **Breaking change** — agent-registry module consumers need to migrate

### Risks
- **Data loss during migration** — existing agent-registry data needs migration script
- **Module size** — hexflo-coordination is already the largest module; adding agents adds more surface area

## References
- ADR-037: Agent Lifecycle — Local Default + Remote Connect
- ADR-048: Task State Synchronization (SubagentStart/Stop hooks)
- ADR-050: Hook-Enforced Agent Lifecycle Pipeline
- ADR-025: SpacetimeDB as Distributed State Backend
- Session file format: `~/.hex/sessions/agent-{sessionId}.json`
