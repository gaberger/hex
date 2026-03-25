# ADR-2603241900: Agent-Swarm Ownership Hierarchy with Conflict Detection

**Status:** Accepted
**Date:** 2026-03-24
**Deciders**: Gary
**Required by**: ADR-2603240130 (Declarative Swarm Agent Behavior from YAML)

---

## Context

The current data model has an ambiguous relationship between agents, swarms, and projects:

- `Swarm.project_id` — swarm belongs to a project
- `Swarm.created_by` — agent ID stored as a plain string (no FK enforcement, no exclusivity)
- `HexAgent.swarm_id` — agent optionally assigned to a swarm
- `SwarmAgent` — bridge table linking agents to swarms (implies m:many)

This ambiguity causes two problems:

1. **No ownership semantics**: Any agent can call `swarm_init` for any project. There is no concept of an agent *owning* a swarm exclusively. Remote agents on different machines can claim the same swarm context.

2. **No conflict detection**: Task assignment (`task_assign`) does a simple write with no version check. If two agents on different nodes both fetch a `pending` task and attempt to assign themselves, the last writer wins silently.

The intended hierarchy is:

```
Project (root)
  └─ HexAgent    (belongs to project; 1:many per project)
       └─ Swarm  (belongs to agent; 1:1 active per agent)
            └─ SwarmTask  (belongs to swarm)
```

---

## Decision

### 1. Formalise 1:1 Agent ↔ Swarm ownership

- `Swarm` gains an `owner_agent_id` field (replaces the informal `created_by`).
- **Constraint**: an agent may have at most one swarm in `status = "active"` at a time. `swarm_init` rejects if the calling agent already owns an active swarm.
- `Swarm.project_id` is derived from `HexAgent.project_id` at creation time (denormalized for query convenience, but the agent is the authoritative owner).
- The `SwarmAgent` bridge table is **retained** for participant membership (an agent can *participate* in a swarm it does not own), but ownership is tracked separately via `owner_agent_id`.

### 2. Swarm-level conflict detection

- `swarm_init` checks `hex_agent` table: if `agent.swarm_id` is non-empty and the referenced swarm is still `active`, the reducer returns an error.
- Swarm ownership transfer (handoff to another agent) requires explicit `swarm_transfer(swarm_id, new_owner_agent_id)` reducer — cannot happen implicitly.

### 3. Task-level optimistic locking

`SwarmTask` gains two new fields:

```rust
pub version: u64,           // incremented on every status change
pub claimed_by: String,     // agent_id that last claimed this task (for CAS)
```

The `task_assign` reducer becomes a **Compare-And-Swap**:

```
task_assign(task_id, agent_id, expected_version) -> Result<SwarmTask, ConflictError>
```

- If `task.version != expected_version` → return `ConflictError::VersionMismatch`
- If `task.status != "pending"` → return `ConflictError::AlreadyClaimed { by: task.claimed_by }`
- Otherwise: set `agent_id`, `status = "in_progress"`, increment `version`

Callers (CLI, MCP tool, REST API) must read the task, note the version, then pass it in the assign call. On conflict they re-fetch and retry or back off.

### 4. Schema changes summary

**`Swarm`** (spacetime-modules + SQLite fallback):
- Add `owner_agent_id: String` (the agent that created and owns the swarm)
- Keep `created_by: String` as an alias during migration, remove after

**`SwarmTask`**:
- Add `version: u64` (default 0)
- Add `claimed_by: String` (agent_id, empty if unassigned)

**`HexAgent`**:
- `swarm_id` field meaning changes: it now points to the agent's **owned** swarm (not membership). Participation in foreign swarms is tracked via `SwarmAgent` only.

### 5. REST API changes

| Endpoint | Change |
|---|---|
| `POST /api/swarms` | Reads `x-hex-agent-id` header; rejects if agent already has active swarm |
| `PATCH /api/swarms/tasks/:id` (assign) | Requires `version` in body; returns 409 on CAS failure |
| `GET /api/agents/:id/swarm` | New: returns agent's owned swarm (if any) |
| `POST /api/swarms/:id/transfer` | New: transfer ownership to another agent |

### 6. MCP tool changes

- `hex_hexflo_task_assign` gains optional `expected_version` param; returns conflict details on 409
- `hex_hexflo_swarm_init` returns error if calling agent already owns an active swarm

---

## Consequences

**Positive:**
- Clear ownership semantics — every swarm has exactly one authoritative agent
- Remote node conflicts are detectable and reportable, not silent
- Task double-assignment is prevented without distributed locks

**Negative:**
- `SwarmTask.version` must be read before every assign call — slight extra round-trip
- Migration needed: existing swarms get `owner_agent_id` backfilled from `created_by`
- `swarm_transfer` adds surface area for future complexity

**Neutral:**
- `SwarmAgent` bridge table is kept — it serves participant membership, not ownership
- `project_id` on `SwarmTask` stays derived (via join) per decision above

---

## Workplan

See `docs/workplans/feat-agent-swarm-ownership.json`.

---

## Related ADRs

- ADR-058: Unified agent identity (`hex_agent` table)
- ADR-027: HexFlo native swarm coordination
- ADR-2603241800: Swarm lifecycle management
- **ADR-2603240130**: Declarative Swarm Agent Behavior — this ADR is a prerequisite. The swarm composition YAML (section 8 of ADR-2603240130) declares which agent roles participate in a swarm. This ADR provides the ownership and conflict-safety guarantees that make declarative multi-agent swarms safe to execute across remote nodes.
