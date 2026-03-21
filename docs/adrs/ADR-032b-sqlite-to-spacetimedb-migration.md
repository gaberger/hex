# ADR-032: Deprecate SQLite, Migrate HexFlo to SpacetimeDB

**Status:** Accepted
**Date:** 2026-03-18
**Deciders:** Gary
**Relates to:** ADR-025 (SpacetimeDB State Backend), ADR-027 (HexFlo Coordination)

## Context

HexFlo coordination (swarms, tasks, agents, memory) currently uses **SQLite directly** via `SwarmDb` in `hex-nexus/src/persistence.rs`. This creates two problems:

1. **Architectural violation**: ADR-027 claims "HexFlo uses `IStatePort`, so it automatically works with both SQLite and SpacetimeDB backends." In reality, the coordination module makes **direct rusqlite calls**, bypassing `IStatePort` entirely.

2. **Duplicate state management**: The project already has 9 SpacetimeDB modules covering RL, agents, chat, skills, hooks, fleet, secrets, and workplans — but swarm coordination (the most critical operational state) still lives in SQLite. This means:
   - No real-time subscriptions for swarm status changes
   - No cross-node replication for distributed swarms
   - State is locked to a single `~/.hex/hub.db` file
   - No reducer-level validation or constraint enforcement

### Current SQLite Tables (to migrate)

```sql
-- HexFlo coordination (persistence.rs)
swarms (id, project_id, name, topology, status, created_at, updated_at)
swarm_tasks (id, swarm_id, title, status, agent_id, result, created_at, completed_at)
swarm_agents (id, swarm_id, name, role, status, worktree_path)
hexflo_memory (key, value, scope, updated_at)

-- RL engine (rl/schema.rs) — already has SpacetimeDB module
rl_experiences (id, state, action, reward, next_state, timestamp, task_type)
rl_q_table (state_key, action, q_value, visit_count, last_updated)
rl_patterns (id, category, content, confidence, ...)
```

### Existing SpacetimeDB Modules

| Module | Tables | Status |
|--------|--------|--------|
| rl-engine | rl_experience, rl_q_entry, rl_pattern | Exists, not wired to coordination |
| agent-registry | agent, agent_heartbeat | Exists |
| chat-relay | conversation, message | Exists |
| skill-registry | skill, skill_trigger | Exists |
| hook-registry | hook, hook_execution_log | Exists |
| fleet-state | compute_node | Exists |
| secret-grant | inference_endpoint, secret_grant, ... | Exists |
| workplan-state | workplan_task, task_assignment | Exists |
| **hexflo-coordination** | — | **MISSING** |

## Decision

### Phase 1: Create `hexflo-coordination` SpacetimeDB Module

New module in `spacetime-modules/hexflo-coordination/` with tables and reducers mirroring the SQLite schema:

#### Tables

```rust
#[spacetimedb::table(public, name = swarm)]
pub struct Swarm {
    #[primary_key]
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub topology: String,   // "hierarchical", "mesh", "pipeline"
    pub status: String,     // "active", "completed", "failed"
    pub created_at: String,
    pub updated_at: String,
}

#[spacetimedb::table(public, name = swarm_task)]
pub struct SwarmTask {
    #[primary_key]
    pub id: String,
    pub swarm_id: String,
    pub title: String,
    pub status: String,     // "pending", "in_progress", "completed", "failed"
    pub agent_id: Option<String>,
    pub result: Option<String>,
    pub created_at: String,
    pub completed_at: Option<String>,
}

#[spacetimedb::table(public, name = swarm_agent)]
pub struct SwarmAgent {
    #[primary_key]
    pub id: String,
    pub swarm_id: String,
    pub name: String,
    pub role: String,
    pub status: String,     // "active", "stale", "dead", "disconnected"
    pub worktree_path: Option<String>,
    pub last_heartbeat: String,
}

#[spacetimedb::table(public, name = hexflo_memory)]
pub struct HexFloMemory {
    #[primary_key]
    pub key: String,
    pub value: String,
    pub scope: String,      // "global", "swarm:<id>", "agent:<id>"
    pub updated_at: String,
}
```

#### Reducers

```rust
// Swarm lifecycle
#[reducer] fn swarm_init(ctx, id, name, topology, project_id)
#[reducer] fn swarm_complete(ctx, id)
#[reducer] fn swarm_fail(ctx, id, reason)

// Task management
#[reducer] fn task_create(ctx, id, swarm_id, title)
#[reducer] fn task_assign(ctx, task_id, agent_id)
#[reducer] fn task_complete(ctx, task_id, result)
#[reducer] fn task_fail(ctx, task_id, reason)
#[reducer] fn task_reclaim(ctx, agent_id)  // reclaim from dead agent

// Agent lifecycle
#[reducer] fn agent_register(ctx, id, swarm_id, name, role)
#[reducer] fn agent_heartbeat(ctx, id)
#[reducer] fn agent_mark_stale(ctx, threshold_secs)
#[reducer] fn agent_mark_dead(ctx, threshold_secs)

// Memory
#[reducer] fn memory_store(ctx, key, value, scope)
#[reducer] fn memory_delete(ctx, key)
#[reducer] fn memory_clear_scope(ctx, scope)
```

### Phase 2: Refactor Coordination Layer to Use IStatePort

Replace direct rusqlite calls in `coordination/{mod,memory,cleanup}.rs` with `IStatePort` method calls:

```rust
// Before (direct SQLite):
self.db.conn.execute("INSERT INTO swarms ...", params![...])?;

// After (via IStatePort):
self.state.swarm_init(id, name, topology, project_id).await?;
```

This requires extending `IStatePort` (in `hex-nexus/src/ports.rs`) with swarm/task/memory methods:

```rust
pub trait IStatePort: Send + Sync {
    // ... existing methods ...

    // HexFlo coordination (new)
    async fn swarm_init(&self, id: &str, name: &str, topology: &str, project_id: &str) -> Result<()>;
    async fn swarm_status(&self) -> Result<Vec<SwarmInfo>>;
    async fn swarm_complete(&self, id: &str) -> Result<()>;
    async fn task_create(&self, id: &str, swarm_id: &str, title: &str) -> Result<()>;
    async fn task_complete(&self, id: &str, result: &str) -> Result<()>;
    async fn task_list(&self, swarm_id: Option<&str>) -> Result<Vec<TaskInfo>>;
    async fn agent_heartbeat(&self, id: &str) -> Result<()>;
    async fn cleanup_stale_agents(&self, stale_secs: u64, dead_secs: u64) -> Result<CleanupReport>;
    async fn memory_store(&self, key: &str, value: &str, scope: &str) -> Result<()>;
    async fn memory_retrieve(&self, key: &str) -> Result<Option<String>>;
    async fn memory_search(&self, query: &str) -> Result<Vec<(String, String)>>;
}
```

### Phase 3: Wire SpacetimeDB Adapter

Implement the new `IStatePort` methods in `SpacetimeStateAdapter`:
- Call SpacetimeDB reducers via the generated client bindings
- Subscribe to swarm/task/agent tables for real-time updates
- Use `on_insert`/`on_update` callbacks for event-driven status propagation

### Phase 4: Deprecate SwarmDb

1. Remove `persistence.rs` (SwarmDb)
2. Remove SQLite schema migrations for swarm tables
3. Keep SQLite as a **fallback-only** backend via `SqliteStateAdapter` for offline/air-gapped use
4. SpacetimeDB becomes the **default and primary** backend

### Migration Path for Existing Data

```bash
# One-time migration script
hex nexus migrate-state --from sqlite --to spacetimedb

# This reads ~/.hex/hub.db and calls SpacetimeDB reducers for each row
```

## Files Changed

### New
- `spacetime-modules/hexflo-coordination/` — New SpacetimeDB module (tables + reducers)
- `hex-nexus/src/spacetime_bindings/hexflo_coordination/` — Generated client bindings

### Modified
- `hex-nexus/src/ports.rs` — Extend `IStatePort` with swarm/task/memory methods
- `hex-nexus/src/adapters/spacetime_state.rs` — Implement new IStatePort methods
- `hex-nexus/src/adapters/sqlite_state.rs` — Implement new IStatePort methods (fallback)
- `hex-nexus/src/coordination/mod.rs` — Replace SwarmDb calls with IStatePort
- `hex-nexus/src/coordination/memory.rs` — Replace SwarmDb calls with IStatePort
- `hex-nexus/src/coordination/cleanup.rs` — Replace SwarmDb calls with IStatePort

### Deprecated
- `hex-nexus/src/persistence.rs` — SwarmDb (to be removed after migration)

## Consequences

### Positive
- **Real-time subscriptions**: Dashboard gets live swarm/task updates without polling
- **Cross-node replication**: Distributed swarms can share state via SpacetimeDB
- **Single state backend**: No more split between SQLite (coordination) and SpacetimeDB (everything else)
- **Reducer validation**: SpacetimeDB enforces state transitions server-side
- **Event-driven**: `on_insert`/`on_update` callbacks replace polling-based cleanup

### Negative
- **SpacetimeDB required for full functionality**: Offline mode degrades to SQLite fallback
- **Migration complexity**: Existing deployments need one-time data migration
- **Module deployment**: hexflo-coordination module must be deployed to SpacetimeDB server

### Risks
- SpacetimeDB `1.0` SDK compatibility — modules currently use `spacetimedb = "1.0"`, upgrade path unclear
- Heartbeat cleanup via reducers may have different timing characteristics than SQLite triggers
- Generated client bindings add compile-time overhead

## Implementation Order

1. **hexflo-coordination module** — tables + reducers (can be tested independently)
2. **IStatePort extension** — add methods, implement in both SQLite and SpacetimeDB adapters
3. **Coordination refactor** — swap SwarmDb → IStatePort in mod/memory/cleanup
4. **Migration script** — SQLite → SpacetimeDB data transfer
5. **Remove SwarmDb** — delete persistence.rs, update composition root

## References
- ADR-025: SpacetimeDB State Backend (original decision to adopt SpacetimeDB)
- ADR-027: HexFlo Swarm Coordination (current coordination design)
- [SpacetimeDB Rust Module SDK](https://spacetimedb.com/docs)
