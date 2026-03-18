# ADR-027: HexFlo — Replace Ruflo with Native Swarm Coordination

## Status

Proposed

## Context

hex currently depends on **ruflo** (`@claude-flow/cli`), a Node.js CLI tool, as an external dependency for swarm coordination (ADR-009). Ruflo provides:

- Task tracking (create, complete, assign)
- Swarm topology management (hierarchical, mesh)
- Agent lifecycle (spawn, terminate)
- Persistent memory (key-value store)
- Session continuity

However, ruflo has significant limitations for hex's evolving architecture:

1. **Language mismatch**: hex-hub and hex-agent are Rust binaries. Shelling out to a Node.js CLI for every coordination call adds ~200ms latency per operation and requires Node.js on the host.

2. **Parallel registries**: hex-hub already tracks agents, tasks, and state via `IStatePort` (SQLite/SpacetimeDB). Ruflo maintains a separate, disconnected registry. Neither knows about the other, causing split-brain state.

3. **No hex-agent integration**: hex-agent (Rust) cannot call ruflo without spawning a subprocess. The hub's WebSocket-based coordination is the natural control plane.

4. **Operational complexity**: Users must have both `bun`/`node` AND Rust binaries installed. Ruflo's task list has bugs (empty results, crashes on `task list`).

5. **Feature overlap**: hex-hub already implements everything ruflo provides — agent spawning, task CRUD, heartbeat monitoring, workplan execution, RL-guided optimization — but natively in Rust with SQLite persistence.

## Decision

Replace ruflo with **HexFlo** — a native coordination layer built into hex-nexus that provides the same swarm orchestration API surface as ruflo but implemented in Rust, using `IStatePort` as the persistence backend.

### HexFlo Architecture

```
hex-nexus/src/
  coordination/
    mod.rs              # HexFlo public API
    swarm.rs            # Swarm lifecycle (init, status, teardown)
    task_tracker.rs     # Task CRUD with status machine
    agent_registry.rs   # Agent spawn/heartbeat/terminate
    topology.rs         # Hierarchical, mesh, adaptive topologies
    memory.rs           # Key-value persistent memory
```

### API Surface (mirrors ruflo)

| Ruflo Command | HexFlo Equivalent | Implementation |
|---------------|-------------------|----------------|
| `ruflo swarm init` | `HexFlo::swarm_init(name, topology)` | IStatePort + broadcast |
| `ruflo task create` | `HexFlo::task_create(type, desc, priority)` | IStatePort |
| `ruflo task list` | `HexFlo::task_list(filter)` | IStatePort query |
| `ruflo task complete` | `HexFlo::task_complete(id, result, commit)` | IStatePort + event |
| `ruflo memory store` | `HexFlo::memory_store(key, value)` | IStatePort |
| `ruflo memory retrieve` | `HexFlo::memory_retrieve(key)` | IStatePort query |
| `ruflo swarm status` | `HexFlo::swarm_status()` | IStatePort + heartbeats |

### Access Patterns

1. **From Claude Code (TypeScript)**: MCP tools (`mcp__hex__hexflo_*`) that call hex-hub's REST API
2. **From hex-agent (Rust)**: Direct function calls via hex-nexus library, or REST API when running as separate process
3. **From hex-hub dashboard**: WebSocket events for real-time swarm monitoring
4. **From CLI**: `hex swarm init`, `hex task create`, etc.

### REST API Endpoints

```
POST   /api/swarm/init          { name, topology }
GET    /api/swarm/status
DELETE /api/swarm/teardown

POST   /api/tasks               { type, description, priority }
GET    /api/tasks               ?status=pending&limit=20
GET    /api/tasks/:id
PATCH  /api/tasks/:id           { status, result, commit_hash }
DELETE /api/tasks/:id

POST   /api/agents/register     { agent_id, agent_name, project_dir }
GET    /api/agents
DELETE /api/agents/:id
POST   /api/agents/:id/heartbeat

POST   /api/memory              { key, value }
GET    /api/memory/:key
GET    /api/memory/search       ?query=...
```

### MCP Tools (replace ruflo MCP tools)

```
mcp__hex__hexflo_swarm_init      → POST /api/swarm/init
mcp__hex__hexflo_swarm_status    → GET  /api/swarm/status
mcp__hex__hexflo_task_create     → POST /api/tasks
mcp__hex__hexflo_task_list       → GET  /api/tasks
mcp__hex__hexflo_task_complete   → PATCH /api/tasks/:id
mcp__hex__hexflo_memory_store    → POST /api/memory
mcp__hex__hexflo_memory_retrieve → GET  /api/memory/:key
mcp__hex__hexflo_memory_search   → GET  /api/memory/search
```

### Migration Path

1. **Phase 1**: Implement HexFlo coordination module in hex-nexus
2. **Phase 2**: Add MCP tools and REST endpoints
3. **Phase 3**: Update CLAUDE.md to reference HexFlo instead of ruflo
4. **Phase 4**: Remove ruflo from dependencies (`package.json`)
5. **Phase 5**: Update all skills/agents that reference ruflo commands

### Topology Support

HexFlo supports the same topologies as ruflo:

- **Hierarchical**: Leader coordinates workers. Best for feature development.
- **Mesh**: Peer-to-peer. Best for parallel independent tasks.
- **Adaptive**: Starts mesh, promotes leader when coordination needed.

Topology is stored in IStatePort and affects task assignment strategy.

### Heartbeat Protocol

- Agents send heartbeat every 15 seconds (already implemented in hex-agent)
- Hub marks agents as `stale` after 45 seconds without heartbeat
- Hub marks agents as `dead` after 120 seconds and reclaims their tasks
- Dashboard shows real-time agent status via WebSocket events

## Consequences

### Positive

- **Zero external dependencies**: No Node.js/bun required for coordination
- **Single registry**: One source of truth for all agent and task state
- **Lower latency**: Native Rust calls vs subprocess spawn (~200ms → <1ms)
- **hex-agent native integration**: Direct library calls from hex-agent
- **SpacetimeDB ready**: HexFlo uses IStatePort, so it automatically works with both SQLite and SpacetimeDB backends
- **Dashboard integration**: Real-time swarm visualization through existing WebSocket infrastructure

### Negative

- **Migration effort**: Skills, agents, and CLAUDE.md all reference ruflo commands
- **Feature parity risk**: Must ensure all ruflo features are covered before removing dependency
- **Claude Code integration**: The `Agent` tool in Claude Code spawns subagents natively — HexFlo needs MCP tools that Claude Code can call as a replacement

### Neutral

- Ruflo can remain as an optional dependency during transition
- Existing ruflo memory data can be imported via a one-time migration script

## Related

- ADR-009: Ruflo as Required Dependency (superseded by this ADR)
- ADR-024: Hex-Hub Autonomous Nexus Architecture
- ADR-025: SpacetimeDB as Distributed State Backend
