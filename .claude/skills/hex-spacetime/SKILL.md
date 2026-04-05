---
name: hex-spacetime
description: Guide SpacetimeDB WASM module development for hex. Use when the user asks to "create module", "spacetimedb", "wasm module", "new reducer", "spacetime table", "add spacetime module", or works in spacetime-modules/.
---

# Hex SpacetimeDB — WASM Module Development Guide

SpacetimeDB is hex's coordination backbone — 18+ WASM modules provide transactional state management for swarms, agents, inference, chat, architecture enforcement, and more. All clients (web dashboard, CLI, desktop) connect via WebSocket for real-time synchronization.

## Critical Constraints

SpacetimeDB WASM modules run in a **sandboxed environment**. Understanding these constraints is essential:

| Capability | Allowed? | Workaround |
|-----------|----------|------------|
| Read/write files | NO | Delegate to hex-nexus REST API |
| Network requests | NO | Delegate to hex-nexus (inference-bridge pattern) |
| Spawn processes | NO | Delegate to hex-nexus |
| Access env vars | NO | Pass config via reducer arguments |
| Persistent state | YES | SpacetimeDB tables (automatic persistence) |
| WebSocket push | YES | Client subscriptions (automatic) |
| Cross-module calls | LIMITED | Via shared tables, not direct function calls |

**Key implication**: Any operation requiring filesystem, network, or process access must go through hex-nexus as a bridge. The WASM module stores intent/state, and hex-nexus reads that state via WebSocket subscription and performs the actual I/O.

## Module Structure

Every SpacetimeDB module follows this pattern:

```
spacetime-modules/<module-name>/
  Cargo.toml          # Depends on spacetimedb crate
  src/
    lib.rs            # Tables + reducers
```

### Cargo.toml Template

```toml
[package]
name = "<module-name>"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
spacetimedb = "1.0"
log = "0.4"

[profile.release]
opt-level = "s"       # Optimize for size (WASM binary)
lto = true
```

### lib.rs Template

```rust
use spacetimedb::{table, reducer, Table, ReducerContext, Identity, Timestamp};

// ── Tables ──────────────────────────────────────────────────────────
// Tables are automatically persisted and subscribable via WebSocket.

#[table(name = my_entity, public)]
pub struct MyEntity {
    #[primary_key]
    #[auto_inc]
    pub id: u64,
    pub name: String,
    pub status: String,
    pub created_at: Timestamp,
    pub owner: Identity,
}

// ── Reducers ────────────────────────────────────────────────────────
// Reducers are transactional functions callable by clients.
// They run atomically — if they panic, the transaction is rolled back.

#[reducer]
pub fn create_entity(ctx: &ReducerContext, name: String) -> Result<(), String> {
    let entity = MyEntity {
        id: 0,  // auto_inc fills this
        name,
        status: "active".to_string(),
        created_at: ctx.timestamp,
        owner: ctx.sender,
    };
    ctx.db.my_entity().insert(entity);
    Ok(())
}

#[reducer]
pub fn update_entity_status(ctx: &ReducerContext, id: u64, status: String) -> Result<(), String> {
    let entity = ctx.db.my_entity()
        .id()
        .find(id)
        .ok_or_else(|| format!("Entity {} not found", id))?;

    ctx.db.my_entity().id().delete(id);
    ctx.db.my_entity().insert(MyEntity { status, ..entity });
    Ok(())
}

// ── Init reducer ────────────────────────────────────────────────────
// Called once when the module is first published.

#[reducer(init)]
pub fn init(ctx: &ReducerContext) {
    log::info!("Module initialized at {:?}", ctx.timestamp);
}
```

## Existing Modules Reference

| Module | Purpose | Key Tables |
|--------|---------|------------|
| `hexflo-coordination` | Core swarm/task/agent/memory/project/config/fleet/lifecycle/cleanup state | swarm, swarm_task, swarm_agent, hexflo_memory, compute_node, remote_agent |
| `agent-registry` | Agent lifecycle + heartbeats + scheduled cleanup | agent, agent_heartbeat, agent_cleanup_log |
| `inference-gateway` | LLM request routing + procedure-based inference | inference_request, inference_response, inference_provider |
| `secret-grant` | TTL-based key distribution to sandboxed agents | secret_grant, grant_audit |
| `rl-engine` | Reinforcement learning model selection | model_score, selection_event |
| `chat-relay` | Message routing between agents/users | chat_message, chat_channel |
| `neural-lab` | Experimental neural patterns | neural_pattern, experiment |

## Creating a New Module

### Step 1: Scaffold

```bash
mkdir -p spacetime-modules/<module-name>/src
```

Create `Cargo.toml` and `src/lib.rs` using the templates above.

### Step 2: Add to Workspace

Edit the root `Cargo.toml` to add the new module:
```toml
[workspace]
members = [
    # ... existing members ...
    "spacetime-modules/<module-name>",
]
```

### Step 3: Build

```bash
cargo build -p <module-name> --target wasm32-unknown-unknown --release
```

The WASM binary will be at:
```
target/wasm32-unknown-unknown/release/<module_name>.wasm
```

### Step 4: Publish to SpacetimeDB

```bash
spacetime publish <module-name> \
  --project-path spacetime-modules/<module-name> \
  --clear-database  # Only on first publish or schema changes
```

### Step 5: Generate Client Bindings

For the dashboard (TypeScript):
```bash
spacetime generate --lang typescript \
  --out-dir hex-nexus/assets/src/spacetimedb/<module-name> \
  --project-path spacetime-modules/<module-name>
```

For hex-nexus (Rust):
```bash
# hex-nexus connects via the spacetimedb-sdk crate
# Add the module's types to hex-nexus/src/adapters/spacetimedb/
```

## Design Patterns

### Bridge Pattern (for I/O operations)

When a WASM module needs to trigger filesystem or network operations:

```
1. Client calls reducer → stores "intent" in a table
2. hex-nexus subscribes to that table via WebSocket
3. hex-nexus sees new row → performs the I/O
4. hex-nexus calls another reducer to store the result
5. Original client sees the result via subscription
```

Example: Inference requests
```
Client → inference_gateway.request_inference(prompt)
  → Inserts row in inference_request table (status: "pending")

hex-nexus subscription fires → sees pending request
  → Makes HTTP call to LLM API
  → Calls inference_gateway.complete_inference(id, response)
    → Updates inference_request status to "completed"

Client subscription fires → sees completed request → reads response
```

### Shared Table Pattern (for cross-module communication)

Modules cannot call each other's reducers directly. Instead, they share state via tables:

```
Module A writes to shared_events table
Module B subscribes to shared_events table
Module B reads new events and acts on them
```

### Identity-Scoped Access

Use `ctx.sender` (Identity) to scope data access:

```rust
#[reducer]
pub fn get_my_tasks(ctx: &ReducerContext) -> Result<(), String> {
    let tasks: Vec<_> = ctx.db.swarm_task()
        .iter()
        .filter(|t| t.assignee == ctx.sender)
        .collect();
    // Tasks are returned via subscription, not return value
    Ok(())
}
```

## Testing

SpacetimeDB modules are tested via integration tests that publish to a local instance:

```bash
# Start local SpacetimeDB
spacetime start

# Publish and test
spacetime publish <module> --project-path spacetime-modules/<module>

# Run integration tests
cargo test -p <module> --test integration
```

Unit tests for pure logic can run normally:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_status_transition() {
        assert!(is_valid_transition("pending", "active"));
        assert!(!is_valid_transition("completed", "pending"));
    }
}
```

## Common Mistakes

| Mistake | Why it fails | Fix |
|---------|-------------|-----|
| `std::fs::read()` in reducer | WASM sandbox blocks filesystem | Use bridge pattern via hex-nexus |
| `reqwest::get()` in reducer | WASM sandbox blocks network | Use bridge pattern via hex-nexus |
| `std::env::var()` in reducer | No env vars in WASM | Pass config via reducer arguments |
| Large WASM binary | Slow publish, memory limits | Use `opt-level = "s"` and `lto = true` |
| Direct cross-module calls | Not supported | Use shared table pattern |
| Returning data from reducer | Reducers don't return data to caller | Use table subscriptions |

## Quick Reference

| Task | Command |
|------|---------|
| Build module | `cargo build -p <module> --target wasm32-unknown-unknown --release` |
| Publish module | `spacetime publish <module> --project-path spacetime-modules/<module>` |
| Generate TS bindings | `spacetime generate --lang typescript --out-dir hex-nexus/assets/src/spacetimedb/<module>` |
| View logs | `spacetime logs <module>` |
| List tables | `spacetime sql <module> "SELECT name FROM st_table"` |
| Query table | `spacetime sql <module> "SELECT * FROM <table>"` |
