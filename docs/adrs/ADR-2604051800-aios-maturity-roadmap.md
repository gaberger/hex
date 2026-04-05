# ADR-2604051800: AIOS Maturity Roadmap — Missing Primitives

**Status:** Accepted
**Date:** 2026-04-05
**Drivers:** Architectural audit revealed hex has evolved from an AAIDE (AI-Assisted Integrated Development Environment) into an AIOS (AI Operating System) with microkernel architecture, process lifecycle management, distributed state, and resource brokering. The system's capabilities now map 1:1 to traditional OS primitives, but several critical gaps prevent it from being a complete multi-tenant agent runtime.
**Supersedes:** ADR-052 (AIIDE — Hex Nexus as AI Integrated Development Environment)

## Context

### What hex is today

hex is a microkernel-based AI Operating System with the following verified primitives:

| OS Concept | hex Implementation | Status |
|---|---|---|
| Microkernel | 7 SpacetimeDB WASM modules (~130 reducers) | Solid |
| Syscall surface | 9 port traits in hex-core (27 total in hex-nexus) | Solid |
| Process lifecycle | spawn → heartbeat → stale → dead → evict + task reclaim | Solid |
| Resource locking | Worktree locks, task claims with CAS + TTL expiry | Solid |
| Secret brokering | TTL grants, encrypted vault, audit log | Solid |
| IPC | WebSocket broadcast, inbox notifications, chat relay | Working |
| Scheduling | HexFlo topology-aware dispatch (mesh/pipeline/hierarchical) | Working |
| Inference routing | RL Q-learning model selection + quantization tiers | Working |
| Constraint enforcement | Architecture validation, boundary checking, quality gates | Working |
| Sandboxing | Docker/microVM via ISandboxPort, worktree isolation | Partial |
| Multi-host fleet | Remote agent lifecycle, SSH tunneling, fleet node selection | Partial |

### The layering

```
Userland:         Skills, Agents, Swarm YAMLs, Workplans (hex-agent)
System services:  Agent Manager, Workplan Executor, Inference Router,
                  Secret Broker, Fleet Manager (hex-nexus)
Kernel:           9 port traits — ICoordination, ISandbox, IFileSystem,
                  ISecret, IInference, IEnforcement, IAgentRuntime,
                  IContextCompressor, IState (hex-core)
Microkernel:      7 WASM modules (SpacetimeDB)
```

The hexagonal architecture serves a dual purpose: code organization AND privilege boundary. Domain can't reach adapters. Adapters can't reach each other. Only the composition root wires them. This is structurally isomorphic to a capability-based security model.

### What's missing

Five critical OS primitives are absent or incomplete. Without them, hex remains a sophisticated orchestrator but not a complete multi-tenant agent runtime.

## Decision

We will address five missing primitives in priority order. Each is a discrete workstream that can be implemented independently.

### P1: Capability-Based Agent Authorization

**Gap:** Any agent with HTTP access to hex-nexus can call any of ~150 REST endpoints. There is no per-agent authorization beyond secret grants.

**Design:** Introduce agent capability tokens — signed JWTs issued at spawn time that encode what an agent is allowed to do.

```rust
// hex-core/src/domain/capability.rs
pub struct AgentCapabilityToken {
    pub agent_id: String,
    pub swarm_id: Option<String>,
    pub capabilities: Vec<Capability>,
    pub issued_at: u64,
    pub expires_at: u64,
    pub signature: String,
}

pub enum Capability {
    /// Can only complete tasks assigned to this agent
    TaskComplete { task_ids: Vec<String> },
    /// Can read/write only within this worktree
    FileSystem { root: PathBuf, read_only: bool },
    /// Can call inference with this model tier or below
    Inference { max_tier: QuantTier },
    /// Can read these memory scopes
    Memory { scopes: Vec<String> },
    /// Can send inbox notifications to these agents
    Notify { target_agents: Vec<String> },
    /// Full access (for orchestrator agents only)
    Admin,
}
```

**Enforcement:** Axum middleware extracts `X-Hex-Agent-Token` from every request. Endpoints check `has_capability()` before executing. Agents spawned by hex-nexus receive their token via environment variable.

**Scope:** hex-nexus routes, IFileSystemPort adapter, IStatePort adapter.

### P2: Preemptive Agent Termination

**Gap:** hex can mark agents stale/dead but cannot actually kill a running process. It relies on cooperative shutdown (agent polls for cancellation or heartbeat timeout triggers task reclamation). A rogue or looping agent continues consuming resources until it exits on its own.

**Design:** Two-phase termination protocol.

1. **Graceful:** Write a cancellation signal to `~/.hex/sessions/agent-{id}.cancel`. Agent hooks check this file on every `UserPromptSubmit` and exit cleanly. Set a 30-second deadline.
2. **Forceful:** If the agent hasn't exited after the deadline, `kill -TERM` the process (PID tracked in agent registry). After another 10 seconds, `kill -9`.
3. **Remote:** For SSH-tunneled agents, send a `terminate` command over the WebSocket control channel. The remote hex-agent process handles local kill.

```rust
// hex-nexus/src/orchestration/agent_manager.rs
pub async fn terminate_agent(&self, agent_id: &str, force: bool) -> Result<(), AgentError> {
    // Phase 1: signal cancellation
    self.write_cancel_signal(agent_id).await?;
    if !force {
        tokio::time::sleep(Duration::from_secs(30)).await;
        if self.is_agent_alive(agent_id).await? { return Ok(()); }
    }
    // Phase 2: SIGTERM
    if let Some(pid) = self.get_agent_pid(agent_id).await? {
        nix::sys::signal::kill(Pid::from_raw(pid), Signal::SIGTERM)?;
        tokio::time::sleep(Duration::from_secs(10)).await;
        // Phase 3: SIGKILL
        if self.is_agent_alive(agent_id).await? {
            nix::sys::signal::kill(Pid::from_raw(pid), Signal::SIGKILL)?;
        }
    }
    self.reclaim_agent_tasks(agent_id).await?;
    Ok(())
}
```

**CLI/MCP:** `hex agent terminate <id> [--force]`, `mcp__hex__hex_agent_terminate`.

### P3: Adaptive Scheduling

**Gap:** Swarm YAMLs define topology and agent cardinality statically. The RL engine selects models but doesn't rebalance workload, migrate tasks, or auto-scale agent count based on queue depth.

**Design:** Add a scheduling loop to HexFlo that runs every 30 seconds:

1. **Queue pressure:** Count pending tasks per swarm. If pending > 2x active agents, emit a `scale_up` recommendation.
2. **Starvation detection:** If any task has been pending > 5 minutes with no agent claiming it, flag it.
3. **Load rebalancing:** If one agent has 3+ tasks in-progress while another has 0, reassign the oldest pending task.
4. **Auto-scale response:** When `scale_up` fires, spawn additional agents up to the swarm's `max_cardinality` from the YAML definition.

```rust
// hex-nexus/src/coordination/scheduler.rs
pub struct SchedulerConfig {
    pub check_interval: Duration,       // 30s default
    pub scale_up_threshold: f32,        // pending/active ratio
    pub starvation_timeout: Duration,   // 5min default
    pub max_rebalance_per_tick: u32,    // 2
}
```

State stored in SpacetimeDB: `scheduler_event` table for audit trail. Scheduler decisions are advisory by default — auto-scale requires opt-in via swarm YAML `auto_scale: true`.

### P4: Virtual Filesystem with Namespace Isolation

**Gap:** Worktree isolation helps but agents sharing a host can still read each other's files. There is no mount namespace equivalent.

**Design:** Extend `IFileSystemPort` with a namespace-scoped adapter:

```rust
// hex-core/src/ports/file_system.rs (new adapter)
pub struct NamespacedFileSystem {
    inner: Arc<dyn IFileSystemPort>,
    allowed_roots: Vec<PathBuf>,    // ["/project/feat/my-worktree", "/tmp/hex-agent-xyz"]
    deny_patterns: Vec<Glob>,       // ["**/.env", "**/credentials*"]
}
```

All file operations check `path.starts_with(allowed_root)` and reject paths matching deny patterns. The path traversal protection in `safePath()` already exists — this extends it to per-agent scoping.

For Docker-sandboxed agents, this maps to Docker bind mount configuration. For local agents, it's enforced in the adapter layer.

**Enforcement:** `AgentCapabilityToken::FileSystem { root, read_only }` (from P1) controls which namespace an agent gets.

### P5: Control Plane Partition Tolerance

**Gap:** SpacetimeDB is a single point of failure. SQLite fallback preserves local operation but swarm coordination stops.

**Design:** Graceful degradation with explicit mode signaling:

| SpacetimeDB State | Behavior |
|---|---|
| **Connected** | Full coordination — all state via SpacetimeDB WebSocket |
| **Disconnected < 60s** | Buffer writes locally, read from SQLite cache. Retry connection. |
| **Disconnected > 60s** | Enter **local mode**: SQLite becomes primary, swarm operations pause, single-agent operation continues |
| **Reconnected** | Reconcile: push buffered writes to SpacetimeDB, resume swarm subscriptions |

```rust
// hex-nexus/src/adapters/state_router.rs
pub enum ConnectionMode {
    Connected,
    Buffering { since: Instant, buffer: Vec<BufferedWrite> },
    LocalMode { since: Instant },
}
```

Critical invariant: an agent must never lose work due to SpacetimeDB going down mid-task. The write buffer captures all state mutations during disconnection and replays them on reconnect.

**NOT in scope:** Multi-SpacetimeDB consensus or active-active replication. hex-nexus remains single-authority; this is about graceful degradation, not distributed consensus.

## Consequences

**Positive:**
- P1 enables multi-tenant operation — untrusted agents can be sandboxed with specific capabilities
- P2 closes the "rogue agent" gap — operators can forcefully stop misbehaving processes
- P3 eliminates manual cardinality tuning — swarms self-scale to match workload
- P4 prevents information leakage between agents on shared hosts
- P5 eliminates the single-point-of-failure for SpacetimeDB outages

**Negative:**
- P1 adds authentication overhead to every request (~1ms JWT verification)
- P2 introduces platform-specific code (POSIX signals, Windows `TerminateProcess`)
- P3 auto-scaling can overshoot — spawning too many agents wastes inference budget
- P4 filesystem namespace enforcement adds latency to every file operation
- P5 reconciliation on reconnect can produce conflicts if two agents modified overlapping state

**Mitigations:**
- P1: Cache verified tokens in-memory with TTL; overhead drops to ~10us for repeat requests
- P2: Abstract platform kill behind `IProcessPort` trait; test on CI for both platforms
- P3: Exponential backoff on scale-up; hard cap from swarm YAML `max_cardinality`
- P4: Namespace check is O(1) prefix match; deny patterns compiled once at spawn
- P5: Buffered writes carry vector clocks; conflicts detected at replay time and surfaced to operator

## Implementation

| Phase | Description | Priority | Status |
|-------|------------|----------|--------|
| P1 | Capability-based agent authorization | Critical | Pending |
| P2 | Preemptive agent termination | High | Pending |
| P3 | Adaptive scheduling | Medium | Pending |
| P4 | Virtual filesystem with namespace isolation | Medium | Pending |
| P5 | Control plane partition tolerance | Low | Pending |

### Dependency order

P1 (capabilities) is a prerequisite for P4 (namespace isolation) — the capability token determines which filesystem namespace an agent receives.

P2 (termination) is independent and can be implemented at any time.

P3 (adaptive scheduling) requires only the existing HexFlo primitives.

P5 (partition tolerance) requires careful testing and should follow P1-P4 to avoid complicating the state layer during active development.

### Estimated scope

| Phase | Files modified | New files | Lines (est.) |
|-------|---------------|-----------|-------------|
| P1 | ~8 (routes, middleware, agent_manager) | 2 (capability.rs, auth middleware) | ~600 |
| P2 | ~3 (agent_manager, routes, CLI) | 0 | ~200 |
| P3 | ~4 (coordination, swarm YAMLs) | 1 (scheduler.rs) | ~400 |
| P4 | ~3 (file_system adapter, sandbox) | 1 (namespaced_fs.rs) | ~300 |
| P5 | ~5 (state adapters, connection manager) | 1 (state_router.rs) | ~500 |

## References

- ADR-052: AIIDE — Hex Nexus as AI Integrated Development Environment (superseded by this ADR)
- ADR-025: SpacetimeDB as Distributed State Backend
- ADR-026: Secure Secret Distribution via SpacetimeDB Coordination
- ADR-027: HexFlo — Replace Ruflo with Native Swarm Coordination
- ADR-042: Multi-Instance Coordination — Locks, Claims, Cleanup
- ADR-060: Agent Notification Inbox
- ADR-2604050900: SpacetimeDB Right-Sizing and IStatePort Sub-Trait Split
