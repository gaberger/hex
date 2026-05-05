# Component: SpacetimeDB

## One-Line Summary

Coordination & state core — the single transactional backbone every hex unit subscribes to.

## Key Facts

- Required service. No SQLite fallback (ADR-025).
- Hosts 7 server-side WASM modules in `spacetime-modules/`.
- All clients connect via WebSocket (typically `ws://localhost:3000`).
- Reducers are transactional stored procedures; tables are subscribed to in real time.
- WASM modules **cannot** access the filesystem, spawn processes, or make network calls — that is why hex-nexus exists.

## WASM module catalog

| Module | Role | Doc |
|--------|------|-----|
| `hexflo-coordination` | Swarms, tasks, agents, memory, fleet — primary swarm-state surface (ADR-027) | `spacetime-modules/hexflo-coordination/README.md` |
| `agent-registry` | Agent lifecycle + heartbeats (45 s stale, 120 s dead) + cleanup audit | `spacetime-modules/agent-registry/README.md` |
| `inference-gateway` | LLM request routing — Anthropic / OpenAI / Ollama / vLLM / OpenRouter | `spacetime-modules/inference-gateway/README.md` |
| `secret-grant` | TTL-based API-key distribution to agents (ADR-026) | `spacetime-modules/secret-grant/README.md` |
| `rl-engine` | Reinforcement-learning trajectories + verdicts | `spacetime-modules/rl-engine/README.md` |
| `chat-relay` | Conversational chat surface (`hex chat`) | `spacetime-modules/chat-relay/README.md` |
| `neural-lab` | Neural-net training experiments | `spacetime-modules/neural-lab/README.md` |

## API Surface

Clients interact via the SpacetimeDB SDK (Rust / TypeScript / etc.) using two primitives:

| Primitive | Purpose |
|-----------|---------|
| **Reducer call** | Transactional state mutation. e.g. `register_agent`, `request_inference`, `claim_task`. |
| **Subscription** | SQL-like `SELECT` query that streams updates to the client. Replaces polling. |

There is **no REST API on SpacetimeDB itself** — REST is provided by hex-nexus, which proxies into reducer calls.

### Sample subscriptions

```sql
-- agents currently registered + heartbeating
SELECT * FROM agent WHERE status IN ('registered', 'active', 'stale')
SELECT * FROM agent_heartbeat

-- inference responses for the agent I am
SELECT * FROM inference_response WHERE request_id IN (
  SELECT request_id FROM inference_request WHERE agent_id = 'me'
)

-- HexFlo swarm + tasks
SELECT * FROM swarm WHERE project_id = '...'
SELECT * FROM task WHERE swarm_id IN (...)
```

## Configuration

Configured via env when starting the SpacetimeDB host:

| Var | Default | Purpose |
|-----|---------|---------|
| `SPACETIMEDB_HOST` | `localhost:3000` | WebSocket endpoint |
| `SPACETIME_DATABASE` | `hex` | DB name (one DB hosts all 7 modules) |

WASM module sources live in `spacetime-modules/<module>/src/lib.rs`. Build + publish:

```bash
cd spacetime-modules/<module> && spacetime publish hex
```

The hex-nexus `hex-publish-module` skill scripts this for each module.

## Depends On

- The `spacetimedb` host process (external — install separately).

## Depended On By

- **hex-nexus** — config sync (ADR-044), HexFlo coordination, inference routing.
- **hex-agent** — heartbeats, task claiming, inference requests.
- **hex-dashboard** — every panel subscribes to one or more tables.
- **hex CLI** — `hex swarm`, `hex task`, `hex memory`, `hex inference`, `hex agent` all delegate to reducers via nexus.

## See also

- `docs/adrs/ADR-025-spacetimedb-state-backend.md` — original decision.
- `docs/adrs/ADR-027-hexflo-swarm-coordination.md` — coordination layer on top.
- `docs/adrs/ADR-2604050900-spacetime-module-rationalization.md` — module count and boundaries.
- `docs/reference/system-architecture.md` — where SpacetimeDB sits in the wider system.
