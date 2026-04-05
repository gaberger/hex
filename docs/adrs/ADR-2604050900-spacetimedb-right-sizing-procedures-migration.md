# ADR-2604050900: SpacetimeDB Right-Sizing — Procedures Migration for Multi-Host Dispatch

**Status:** Accepted
**Date:** 2026-04-05
**Drivers:** 14 of 19 WASM modules are never invoked; hex-nexus acts as a monolithic bridge due to WASM side-effect limitations; SpacetimeDB procedures now allow HTTP calls, enabling direct LLM invocation from reducers; multi-host agent dispatch and Docker sandbox coordination require SpacetimeDB as the real-time coordination plane but not as a registry for static config.
**Supersedes:** ADR-025 (SQLite fallback, already superseded by ADR-2604020900), partially supersedes ADR-051 (narrows "single source of state" to coordination state only)

## Context

hex uses SpacetimeDB as its distributed state backbone with **19 WASM modules** organized into 5 tiers (Tier 0-4). This architecture was designed for a future where dozens of agents coordinate across multiple hosts and Docker sandbox microVMs.

**The architecture is strategically correct for multi-host dispatch.** When a hex-agent runs inside a Docker AI Sandbox on a remote host, it has no access to hex-nexus's process memory, no shared filesystem with the orchestrator, and network restricted to a sandbox.yml allowlist. SpacetimeDB's WebSocket subscriptions are the only viable push-based coordination mechanism across these boundaries:

```
Host A (orchestrator)     Host B (GPU worker)      Host C (sandbox farm)
  hex-nexus                 hex-agent (bare)         Docker sandbox
  hex-cli                   SSH tunnel -> A            hex-agent daemon
  dashboard                                            MCP server
       |                         |                         |
       +-------------------------+-------------------------+
                                 |
                        SpacetimeDB (WebSocket)
                        task claimed -> push
                        result written -> push
                        heartbeat -> push
```

**However, execution has drifted from this vision in three ways:**

### 1. Module Proliferation (19 modules, 14 never invoked)

Only 5 modules are actually called from hex-nexus:

| Module | Lines | Status |
|--------|-------|--------|
| hexflo-coordination | 1,915 | **Active** — swarms, tasks, agents, memory |
| agent-registry | 259 | **Active** — lifecycle, heartbeats |
| inference-gateway | 1,323 | **Active** — provider routing, procedures |
| secret-grant | 856 | **Active** — TTL key distribution |
| rl-engine | 624 | **Active** — model selection feedback |

14 modules (4,841 lines) are deployed but never invoked:

| Module | Lines | Why Dead |
|--------|-------|----------|
| fleet-state | 111 | FleetManager uses in-memory HashMap instead |
| file-lock-manager | 228 | Filesystem locking done in hex-nexus directly |
| inference-bridge | 295 | Duplicate of inference-gateway |
| architecture-enforcer | 358 | MCP server boundary checks + hooks replace this |
| skill-registry | 176 | Local YAML files synced at startup |
| hook-registry | 196 | Local YAML files synced at startup |
| agent-definition-registry | 217 | Local YAML files synced at startup |
| workplan-state | 345 | hexflo-coordination already tracks tasks |
| chat-relay | 375 | SSE from hex-nexus sufficient |
| hexflo-lifecycle | 235 | Should merge into hexflo-coordination |
| hexflo-cleanup | 212 | Duplicate of agent_mark_stale/dead in hexflo-coordination |
| conflict-resolver | 184 | Never wired |
| neural-lab | 908 | Per-project local storage sufficient |
| test-results | 1 | Empty stub |

**Cost:** 20,353 lines of auto-generated bindings in `hex-nexus/src/spacetime_bindings/` (118 files), 1,775-line `spacetime_state.rs` adapter with HTTP-to-WASM boilerplate, and a 19-module tiered publication pipeline.

### 2. hex-nexus as Monolithic Bridge

WASM reducers cannot make HTTP calls, access filesystems, or spawn processes. This forced hex-nexus into a bridge role where it proxies every operation through HTTP to localhost SpacetimeDB. The adapter pattern is:

```rust
async fn some_operation(&self, args...) -> Result<T, StateError> {
    // 1. Format arguments as JSON
    // 2. POST to http://localhost:3033/v1/database/{module}/call/{reducer}
    // 3. Parse response (often empty)
    // 4. Query table to get results
    // 5. Parse SpacetimeDB's nested schema response
    // 6. Convert to Rust types
}
```

This adds ~10-15ms latency per operation and creates a single point of failure — if hex-nexus is down, no coordination is possible even though SpacetimeDB is still running.

### 3. Remote Agent State Not in SpacetimeDB

`RemoteRegistryAdapter` stores agent state in `Arc<RwLock<HashMap>>`. When an agent spawns on Host B:
- Host A's hex-nexus knows (in-memory)
- The dashboard must poll `/api/remote-agents` (HTTP, not push)
- Host C's agents have no visibility
- If hex-nexus restarts, all remote agent state is lost

This directly contradicts the multi-host dispatch goal where agents on any host should see fleet state in real-time.

### 4. SpacetimeDB Procedures Now Available

SpacetimeDB 2.0+ introduces **procedures** — server-side functions that can perform side effects (HTTP calls, scheduled execution). The `inference-gateway` module already has one active procedure (`execute_inference`). This unlocks:

- Direct LLM API calls from SpacetimeDB (no hex-nexus proxy needed)
- Scheduled agent cleanup without hex-nexus background tasks
- Secret resolution inside procedures (no bridge for key distribution)
- Task result aggregation server-side

## Decision

**We will right-size SpacetimeDB to 7 focused modules optimized for multi-host dispatch, migrate inference to procedures, and eliminate the bridge pattern for coordination operations.**

### Module Topology (19 -> 7)

**Keep (5 modules, already active):**

| Module | Database | Role |
|--------|----------|------|
| hexflo-coordination | hex | Swarm tasks, agent memory, atomic task claiming |
| agent-registry | agent-registry | Agent lifecycle, heartbeats, stale/dead detection |
| inference-gateway | inference-gateway | Provider routing; migrate to procedures for direct LLM calls |
| secret-grant | secret-grant | TTL-based key distribution to sandboxed agents |
| rl-engine | rl-engine | Model selection feedback loop |

**Absorb into hexflo-coordination (2 modules):**

| Module | Absorption Target | Rationale |
|--------|-------------------|-----------|
| fleet-state | hexflo-coordination | Add `compute_node` table; fleet is part of swarm coordination |
| hexflo-lifecycle | hexflo-coordination | Phase transitions belong with swarm state |

**Delete (12 modules):**

| Module | Replacement |
|--------|-------------|
| file-lock-manager | In-process locking in hex-nexus (single-host) or advisory locks via hexflo-coordination reducer (multi-host) |
| inference-bridge | Redundant with inference-gateway |
| architecture-enforcer | MCP server boundary checks (hex-agent) + Claude Code hooks (ADR-2604012110) |
| skill-registry | Local YAML files; `config_sync.rs` already syncs to hexflo-coordination |
| hook-registry | Local YAML files; same config_sync path |
| agent-definition-registry | Local YAML files; same config_sync path |
| workplan-state | hexflo-coordination task tables already track this |
| chat-relay | SSE from hex-nexus; optional future: add chat tables to hexflo-coordination |
| hexflo-cleanup | Merge cleanup reducers into hexflo-coordination |
| conflict-resolver | Never wired; merge conflict resolution is a hex-nexus concern |
| neural-lab | Per-project local storage; dashboard reads via hex-nexus REST |
| test-results | Empty stub; delete |

### Procedures Migration

**Inference (P2):**
- Migrate `inference-gateway` from reducer-only to procedure-based
- `execute_inference` procedure makes HTTP calls to LLM providers directly
- Agents write to `inference_request` table -> procedure fires -> result appears in `inference_response` table
- Eliminates hex-nexus as inference proxy for sandboxed agents
- hex-nexus retains direct inference path for local agents (lower latency)

**Agent Cleanup (P3):**
- Migrate stale/dead agent detection to a scheduled procedure in agent-registry
- Procedure runs every 30s, marks agents as stale (45s) / dead (120s)
- Reclaims tasks from dead agents automatically
- Eliminates hex-nexus background cleanup task

**Secret Resolution (P4, future):**
- Procedure in secret-grant resolves API keys and injects into task payloads
- Sandboxed agents receive keys via SpacetimeDB subscription, not HTTP polling

### Remote Agent State Migration

- Replace `RemoteRegistryAdapter` (HashMap) with SpacetimeDB writes to hexflo-coordination
- Add `remote_agent` and `compute_node` tables (absorbed from fleet-state)
- Agent heartbeats write directly to SpacetimeDB (not hex-nexus memory)
- Dashboard subscribes to `remote_agent` table for real-time fleet visibility
- hex-nexus maintains local cache for fast queries, backed by subscription

### hex-nexus Role After Migration

hex-nexus becomes a focused daemon with three responsibilities:
1. **Filesystem bridge** — tree-sitter analysis, git introspection, file I/O (WASM can't do this even with procedures)
2. **Dashboard server** — serves Solid.js frontend, SSE for events
3. **Agent spawner** — SSH tunnels, Docker sandbox creation, binary provisioning

hex-nexus is no longer the coordination router. Agents coordinate directly via SpacetimeDB subscriptions.

## Consequences

**Positive:**
- 63% reduction in WASM modules (19 -> 7), 55% reduction in module code (8,818 -> ~3,977 lines)
- Eliminates ~15,000 lines of auto-generated bindings (118 -> ~40 files)
- Simplifies `spacetime_state.rs` from 1,775 lines to ~600 lines (only hexflo-coordination + agent-registry calls)
- Sandboxed agents on remote hosts coordinate directly via SpacetimeDB without hex-nexus proxying
- Dashboard gets real-time fleet visibility via subscriptions instead of HTTP polling
- Inference requests from Docker sandboxes don't need hex-nexus as intermediary
- Publication pipeline drops from 5 tiers to 2 tiers (foundation + services)

**Negative:**
- Procedures are newer SpacetimeDB feature; less battle-tested than reducers
- Agents on hosts without SpacetimeDB connectivity cannot function (no offline mode)
- Migration requires careful data preservation for active swarms during transition

**Mitigations:**
- Phase procedures migration incrementally: inference first (already has procedure), then cleanup, then secrets
- SpacetimeDB connectivity is already required (CLAUDE.md, ADR-2604020900)
- Migration phases include backward-compatible transition periods where both paths work

## Implementation

| Phase | Description | Status |
|-------|-------------|--------|
| P0 | Absorb fleet-state + hexflo-lifecycle tables into hexflo-coordination | **Done** — `compute_node`, `remote_agent` tables and lifecycle reducers added to hexflo-coordination; fleet-state and hexflo-lifecycle directories deleted |
| P1 | Delete 12 dead modules; update STDB_MODULE_DATABASES constant; prune spacetime_bindings | **Done** — 12 modules deleted, STDB_MODULE_DATABASES (hex-core), MODULE_TIERS (hex-cli, hex-nexus) all list exactly 7 modules |
| P2 | Migrate inference-gateway to procedure-based LLM calls; add REST fallback | **Done** — `execute_inference` is `#[spacetimedb::procedure]` with HTTP calls; `complete_inference` reducer fallback for hex-nexus retained |
| P3 | Migrate agent cleanup to scheduled procedure in agent-registry | **Deferred** — `run_agent_cleanup` remains a `#[reducer]` called by hex-nexus; SpacetimeDB scheduled-procedure maturity insufficient for cron-style triggers. Acceptance criteria: SpacetimeDB supports `#[table(scheduled(...))]` with cron syntax → convert reducer to procedure, delete hex-nexus cleanup polling loop |
| P4 | Replace RemoteRegistryAdapter with SpacetimeDB-backed state | **Done** — `list_agents()` and `get_agent()` now query SpacetimeDB SQL endpoint first (`SELECT * FROM remote_agent`), falling back to local HashMap cache when unreachable. Writes remain dual: HashMap + fire-and-forget reducer call. 5s HTTP timeout, shared connection pool. |
| P5 | Regenerate spacetime_bindings for 7 modules only; update spacetime_launcher | **Partial** — Stale bindings pruned; Rust bindings exist for 5/7 (missing hexflo-coordination, neural-lab); TS bindings exist for 4/7 (missing rl-engine, secret-grant, neural-lab). Remaining: run `scripts/generate-ts-bindings.sh` and `spacetime generate` for missing modules |
| P6 | Simplify spacetime_state.rs — remove dead module calls, reduce boilerplate | **Done** — IStatePort god-trait split into 16 focused sub-traits (IRlStatePort, IPatternStatePort, IAgentStatePort, IWorkplanStatePort, IChatStatePort, ISkillStatePort, IAgentDefStatePort, ISwarmStatePort, IInferenceTaskStatePort, IHexFloMemoryStatePort, IQualityGateStatePort, IProjectStatePort, ICoordinationStatePort, IHexAgentStatePort, IInboxStatePort, INeuralLabStatePort). IStatePort remains as super-trait for backward compatibility. 11 dead methods (fleet_*, hook_*) deleted. Adapter impls split into per-sub-trait blocks. |
| P7 | Integration test: Docker sandbox agent on remote host coordinates via SpacetimeDB | **Blocked** — `test_docker_sandbox_agent_registers_in_spacetimedb` exists (`#[ignore]`); requires Docker daemon + SpacetimeDB + hex-nexus running. Acceptance criteria: docker-compose test environment that provisions all dependencies |

## References

- ADR-025: SQLite fallback (superseded by ADR-2604020900)
- ADR-027: HexFlo — Replace Ruflo with Native Swarm Coordination
- ADR-035: Hex Architecture V2 — Rust-First, SpacetimeDB-Native
- ADR-040: Remote Agent Transport — WebSocket over SSH
- ADR-051: SpacetimeDB as Single Source of State (narrowed by this ADR)
- ADR-063: Deprecate SQLite, Migrate HexFlo to SpacetimeDB
- ADR-2603231400: SpacetimeDB Operational Resilience
- ADR-2603282000: hex-agent as Claude Code-Independent Runtime in Docker AI Sandbox
- ADR-2603291900: Docker Worker First-Class Execution
- ADR-2604012110: Hooks-First Architecture Enforcement
- ADR-2604020900: Eliminate SQLite Fallback
