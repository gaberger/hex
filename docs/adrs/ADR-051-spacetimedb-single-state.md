# ADR-051: SpacetimeDB as Single Source of State

**Status:** Accepted
**Accepted Date:** 2026-03-22
**Date:** 2026-03-21

> **Implementation Evidence:** Database target unified to `hexflo-coordination` in state_config.rs and spacetime_state.rs. AppState contains no in-memory HashMaps for persistent state — all coordination, inference, projects, and sessions go through SpacetimeDB (with feature-gated SQLite fallback for sessions only).
**Drivers:** UX dashboard redesign revealed state fragmentation across 4+ backends

## Context

The hex-nexus system currently stores coordination state in **multiple disconnected backends**:

| Backend | What it stores | Who reads | Who writes |
|---------|---------------|-----------|------------|
| `hex-nexus` SpacetimeDB database | Swarms, tasks, agents, memory | IStatePort (Rust) | CLI, MCP tools |
| `hexflo-coordination` SpacetimeDB database | Swarms, tasks, agents, memory | Dashboard (SolidJS WebSocket) | Manual test data only |
| `inference-gateway` SpacetimeDB database | Inference providers, requests | Dashboard (SolidJS WebSocket) | Register/remove reducers |
| `agent-registry` SpacetimeDB database | Agent heartbeats, status | Dashboard (SolidJS WebSocket) | Agent processes |
| `fleet-state` SpacetimeDB database | Compute nodes | Dashboard (SolidJS WebSocket) | Fleet register |
| In-memory `HashMap` (Rust) | Inference endpoints, instances | REST API handlers | REST API handlers |
| SQLite `hub.db` | Chat sessions, project registry | Session routes | Session routes |

**Problem:** The dashboard subscribes to `hexflo-coordination` but CLI/MCP writes to `hex-nexus`. These are different databases with the same schema but different data. Tasks created via MCP never appear in the dashboard. Inference providers exist in both in-memory maps AND SpacetimeDB with different behavior for CRUD operations.

## Decision

**Consolidate ALL coordination state into the 4 canonical SpacetimeDB modules:**

1. `hexflo-coordination` — swarms, tasks, agents, memory (THE coordination database)
2. `inference-gateway` — providers, requests, budgets, streaming
3. `agent-registry` — agent lifecycle, heartbeats
4. `fleet-state` — compute nodes

**Eliminate:**
- `hex-nexus` SpacetimeDB database (merge into `hexflo-coordination`)
- All in-memory `HashMap` state in Rust `AppState`
- SQLite `hub.db` for session storage (move to SpacetimeDB or `chat-relay` module)

**hex-nexus binary becomes stateless compute:**
- Filesystem operations (analyze, file browse, project scan)
- Process management (agent spawn/kill)
- Outbound HTTP (inference health checks, webhook calls)
- Static asset serving (dashboard HTML/JS/CSS)
- WebSocket proxy for chat (bridges LLM APIs to SpacetimeDB)

## Implementation Progress

| Phase | Status | Notes |
|-------|--------|-------|
| Phase 1: Unify HexFlo coordination | Partial | `hexflo-coordination` SpacetimeDB module exists with swarm/task/agent/memory tables. Dashboard subscribes to it. Gap: `SpacetimeStateAdapter` may still write to a separate database. |
| Phase 2: Eliminate in-memory inference | Not started | `state.inference_endpoints` HashMap still exists in AppState |
| Phase 3: Eliminate in-memory coordination | Not started | `state.hexflo`, `state.instances`, `state.worktree_locks` still in-memory |
| Phase 4: Projects in SpacetimeDB | Partial | Project table exists, dashboard reads from it. REST register route still writes to HashMap. |
| Phase 5: Sessions in SpacetimeDB | Not started | Sessions still in SQLite `hub.db` |

## Implementation Plan

### Phase 1: Unify HexFlo coordination (IStatePort → hexflo-coordination)

The `SpacetimeStateAdapter` currently writes to a database named from `.hex/state.json` (`hex-nexus`). Change it to write to `hexflo-coordination` directly.

**Files to modify:**
- `hex-nexus/src/adapters/spacetime_state.rs` — change database target
- `hex-nexus/src/state_config.rs` — update default database name
- `.hex/state.json` — change `database` to `hexflo-coordination`

**Verification:** After this change, `hex swarm init` creates a swarm visible in the dashboard sidebar within 1 second (via SpacetimeDB subscription).

### Phase 2: Eliminate in-memory inference state

Remove `state.inference_endpoints` HashMap. All inference CRUD goes through SpacetimeDB `inference-gateway` module.

**Files to modify:**
- `hex-nexus/src/routes/secrets.rs` — `register_inference`, `remove_inference`, `check_inference_health` use SpacetimeDB client only
- `hex-nexus/src/state.rs` — remove `inference_endpoints` field
- `hex-nexus/src/routes/chat.rs` — read provider list from SpacetimeDB for routing

### Phase 3: Eliminate in-memory coordination state

Remove `state.hexflo` (HexFlo struct) and `state.instances`, `state.worktree_locks`, `state.task_claims` HashMaps.

**Files to modify:**
- `hex-nexus/src/coordination/` — replace with thin SpacetimeDB reducer calls
- `hex-nexus/src/routes/hexflo.rs` — memory store/retrieve via SpacetimeDB
- `hex-nexus/src/routes/coordination.rs` — read from SpacetimeDB subscriptions

### Phase 4: Projects in SpacetimeDB

Add a `project` table to `hexflo-coordination` module. Register/list/remove projects via reducers.

**SpacetimeDB module changes:**
- Add `Project` table with `project_id`, `name`, `path`, `registered_at`
- Add `register_project`, `remove_project` reducers

**Dashboard changes:**
- Subscribe to `project` table
- Remove `stores/projects.ts` HTTP fetch, use subscription signal

### Phase 5: Sessions in SpacetimeDB

Move chat session storage from SQLite to `chat-relay` SpacetimeDB module.

## Consequences

**Positive:**
- Single source of truth — all Claude Code sessions, MCP tools, CLI, and dashboard see the same state
- Real-time reactivity — any state change propagates to all subscribers via WebSocket within milliseconds
- Multi-instance support — multiple hex-nexus processes can coordinate through shared SpacetimeDB state
- Simpler hex-nexus binary — no state management code, just compute adapters

**Negative:**
- SpacetimeDB becomes a hard dependency (currently gracefully degrades without it)
- Network latency for state operations (vs in-memory HashMap)
- Need to handle SpacetimeDB reconnection gracefully in all code paths

**Mitigations:**
- SpacetimeDB runs locally (localhost:3000), latency is sub-millisecond
- Reducer calls are fire-and-forget for writes; reads come from local subscription cache
- Auto-reconnect with exponential backoff already implemented in dashboard
