# SpacetimeDB Module Audit — Multi-host Necessity

**Investigation team task:** Determine which SpacetimeDB modules are genuinely required for multi-agent/multi-host coordination and which could be replaced by local SQLite or flat files.

---

## Methodology

Sources examined:
- `spacetime-modules/*/src/lib.rs` — all 19 WASM module definitions (tables + reducers)
- `hex-nexus/src/adapters/spacetime_state.rs` — every `call_reducer` and `call_reducer_on` call (the only file that actually talks to STDB)
- `hex-nexus/src/ports/state.rs` — full `IStatePort` trait surface
- `hex-agent/src/adapters/secondary/stdb_connection.rs` — WebSocket task claiming
- `hex-agent/src/adapters/secondary/stdb_task_poller.rs` — task polling strategy
- `hex-agent/src/adapters/secondary/stdb_inference.rs` — inference routing

---

## Key Finding: 5 of 19 Modules Are Actually Called

`spacetime_state.rs` routes to only **5 named databases** via HTTP reducer calls:

| Database name | Module | Called reducers |
|---|---|---|
| `hex` (default) | hexflo-coordination | swarm_init, task_create/assign/complete/fail, inference_task_*, agent_register/heartbeat/remove/mark_stale/mark_dead, memory_store/delete, project_register/remove/update_state, instance_*, worktree_lock_*, task_claim_*, notify_agent, quality_gate ops, fix_task ops, agent_connect |
| `agent-registry` | agent-registry | register_agent, update_status, remove_agent |
| `hex-chat-relay` | chat-relay | create_conversation, send_message |
| `rl-engine` | rl-engine | select_action, record_reward, store_pattern, decay_patterns |
| neural-lab `DB` const | neural-lab | experiment_create/start/complete/fail, config_create |

The remaining **14 modules** have WASM tables defined but are **never invoked** from the nexus state adapter.

hex-agent adds two more STDB usages independent of the nexus:
- `stdb_connection.rs` + `stdb_task_poller.rs`: WebSocket push subscribe to `code_gen_task` table in a `remote-agent-registry` module — but **this module does not exist** in `spacetime-modules/`. The agent falls back to REST polling unconditionally in production.
- `stdb_inference.rs`: Routes all LLM inference through `inference-gateway` module via WebSocket subscriptions with **no REST fallback** — only compiled with the `spacetimedb` feature flag.

---

## Module-by-Module Verdict

| Module | Multi-host needed? | Can be local? | Verdict |
|--------|-------------------|---------------|---------|
| **hexflo-coordination** | YES — atomic task claiming across agents on different hosts; real-time inbox push | No | **KEEP** |
| **agent-registry** | YES — dashboard and remote projects need live cross-host agent visibility | Single-host only | **KEEP** |
| **inference-gateway** | YES — hex-agent routes all LLM calls through this; no REST fallback when enabled | No | **KEEP** |
| **secret-grant** | YES — shared secrets across agents/hosts needs STDB as single authority with private tables | No — atomic ACL enforcement required | **KEEP** |
| **fleet-state** | YES for multi-host node registry | YES for single-host | **KEEP (multi-host)** |
| **file-lock-manager** | YES for multi-host distributed locking | YES for single-host (SQLite advisory locks) | **OPTIONAL** |
| **chat-relay** | Marginal — real-time multi-client chat | YES — local SQLite + SSE sufficient | **OPTIONAL** |
| **rl-engine** | No — per-project patterns; no cross-host consumers | YES — flat JSON or SQLite | **REMOVE or defer** |
| **neural-lab** | No — experiment tracking; no cross-host dependency | YES | **REMOVE or defer** |
| **hexflo-cleanup** | No — hexflo-coordination already has `agent_mark_stale`/`agent_mark_dead` reducers | YES | **REMOVE (duplicate)** |
| **hexflo-lifecycle** | No — never called from nexus | YES | **REMOVE** |
| **workplan-state** | No — workplan state is per-project | YES — SQLite | **REMOVE** |
| **conflict-resolver** | Potentially yes (multi-agent merges), but never called | YES | **REMOVE (not wired)** |
| **architecture-enforcer** | No — boundary rules are static config; already in hexflo-coordination as `enforcement_rule` | YES — flat files | **REMOVE (duplicate)** |
| **skill-registry** | No — skills are compiled-in YAML files | YES — flat files | **REMOVE (duplicate)** |
| **agent-definition-registry** | No — agent defs are YAML files; hexflo-coordination also has `agent_definition` table | YES — flat files | **REMOVE (duplicate)** |
| **hook-registry** | No — hooks are YAML files | YES — flat files | **REMOVE (duplicate)** |
| **inference-bridge** | No — overlaps with inference-gateway; never called from nexus | YES | **REMOVE (duplicate)** |
| **test-results** | No — empty stub | YES | **REMOVE** |

---

## Summary

**Must keep (4 modules):** hexflo-coordination, agent-registry, inference-gateway, secret-grant

**Keep for multi-host deployment (2 modules):** fleet-state, file-lock-manager

**Optional (1 module):** chat-relay

**Can remove — 14 modules** never wired or duplicated by hexflo-coordination.

---

## Critical Gap Found

`remote-agent-registry` module referenced in `hex-agent/src/adapters/secondary/stdb_connection.rs` **does not exist** in `spacetime-modules/`. The push-based WebSocket task claiming path in `StdbConnection` is dead code — all agents fall back to REST polling in production today.

---

## Answer to the Investigation Question

**SpacetimeDB is irreplaceable for multi-host/multi-agent coordination.** The 4 core modules (hexflo-coordination, agent-registry, inference-gateway, secret-grant) provide atomic distributed operations that Claude Code hooks fundamentally cannot replicate — hooks run locally on one machine, not across a distributed fleet. However, 14 of 19 modules are dead weight and should be removed.
