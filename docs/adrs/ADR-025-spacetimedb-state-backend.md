# ADR-025: SpacetimeDB as Distributed State Backend

**Status:** Accepted

## Context

ADR-024 (Hex-Hub Autonomous Nexus) introduced SQLite as the state backend for the RL engine, agent lifecycle, workplan execution, and coordination. This works for single-node operation but has limitations:

1. **No real-time sync**: Agents must poll hex-hub HTTP endpoints for state changes
2. **Manual coordination**: Lock/claim/heartbeat system is hand-rolled and fragile
3. **No automatic replication**: Chat UI, hex-agents, and hex-hub each need explicit API calls to stay in sync
4. **Schema rigidity**: Adding new state requires migration code in persistence.rs

SpacetimeDB (https://spacetimedb.com) is a Rust-native relational database that embeds application logic as WASM modules and provides automatic real-time state synchronization via WebSocket subscriptions.

## Decision

Introduce SpacetimeDB as an **alternative state backend** behind a port abstraction (`IStatePort`), alongside the existing SQLite backend. SpacetimeDB handles state management and real-time sync; hex-hub retains process management (spawning, SSH, OS operations).

### Architecture

```
SpacetimeDB Instance
├── WASM Modules (Rust)
│   ├── rl_engine      — Q-learning tables, experience tuples, pattern store
│   ├── workplan_state  — task status, phase tracking, gate results
│   ├── agent_registry  — agent lifecycle, metrics, heartbeats
│   ├── chat_relay      — message routing, conversation history
│   └── fleet_state     — compute node registry, health status
└── Auto-sync via WebSocket subscriptions

Clients (all connect directly to SpacetimeDB):
├── hex-hub     — subscribes to agent_registry, fleet_state; spawns processes
├── hex-agent   — subscribes to workplan_state; calls rl_engine reducers
└── Chat UI     — subscribes to chat_relay; sends messages via reducers
```

### Port Abstraction

```rust
// New port in hex-agent and hex-hub
trait StatePort: Send + Sync {
    // RL
    async fn rl_select_action(&self, state_key: &str) -> String;
    async fn rl_record_reward(&self, state_key: &str, action: &str, reward: f64);

    // Agent tracking
    async fn register_agent(&self, agent: AgentInfo) -> String;
    async fn update_agent_status(&self, id: &str, status: AgentStatus);
    async fn subscribe_agents(&self) -> impl Stream<Item = AgentEvent>;

    // Workplan
    async fn update_task_status(&self, task_id: &str, status: TaskStatus);
    async fn subscribe_workplan(&self, plan_id: &str) -> impl Stream<Item = TaskEvent>;

    // Chat
    async fn send_message(&self, msg: ChatMessage);
    async fn subscribe_messages(&self, conversation_id: &str) -> impl Stream<Item = ChatMessage>;
}

// Two implementations:
// 1. SqliteStateAdapter (current, default)
// 2. SpacetimeStateAdapter (new, opt-in)
```

### SpacetimeDB Module Structure

Each WASM module is a Rust crate compiled to `wasm32-unknown-unknown`:

```
spacetime-modules/
├── Cargo.toml (workspace)
├── rl-engine/
│   └── src/lib.rs       # Tables: experiences, q_table, patterns
├── workplan-state/
│   └── src/lib.rs       # Tables: workplans, phases, tasks
├── agent-registry/
│   └── src/lib.rs       # Tables: agents, metrics, heartbeats
├── chat-relay/
│   └── src/lib.rs       # Tables: conversations, messages
└── fleet-state/
    └── src/lib.rs       # Tables: nodes, deployments
```

### Migration Strategy

1. Phase 1 (current): SQLite backend — **done** (ADR-024)
2. Phase 2: Add `IStatePort` abstraction, refactor SQLite behind it
3. Phase 3: Implement SpacetimeDB adapter, deploy SpacetimeDB instance
4. Phase 4: Make SpacetimeDB the default, SQLite as offline fallback

## Consequences

### Positive
- **Automatic real-time sync** — eliminates polling, all clients see changes instantly
- **Transactional reducers** — atomic state transitions, no partial updates
- **Rust-native** — WASM modules written in Rust, same toolchain as hex-hub/hex-agent
- **Subscription-based** — agents subscribe to their task table, get pushed updates
- **Replaces hand-rolled coordination** — no more lock/claim/heartbeat code
- **In-memory performance** — faster than SQLite for hot-path reads

### Negative
- **Additional dependency** — SpacetimeDB server must be running
- **WASM limitations** — no filesystem or network access from modules (by design)
- **Single-node** — SpacetimeDB doesn't cluster yet (acceptable at our scale)
- **Learning curve** — reducer/subscription model is different from REST API

### Why Not Replace hex-hub Entirely?
SpacetimeDB WASM modules cannot:
- Spawn OS processes (hex-agent instances)
- SSH into remote nodes
- Access the filesystem
- Execute shell commands

hex-hub must remain as the **process orchestrator**. SpacetimeDB replaces only the **state layer**.

## Dependencies

- ADR-024 (Hex-Hub Autonomous Nexus) — prerequisite architecture
- SpacetimeDB v1.0+ (Rust SDK: `spacetimedb-sdk`)
- `wasm32-unknown-unknown` Rust target for module compilation
