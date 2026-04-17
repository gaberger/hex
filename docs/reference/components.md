# System Components

hex has five deployment units plus two composition modes.

## SpacetimeDB — Coordination & State Core (REQUIRED)

**Must always be running.** Backbone for all clients (web, CLI, desktop) via WebSocket.

- 7 WASM modules in `spacetime-modules/` (ADR-2604050900, right-sized from 19)
- Transactional reducers for swarms, agents, inference, secrets
- Real-time subscriptions replace polling
- **Limitation**: WASM can't touch filesystem / spawn processes / make network calls → why hex-nexus exists

## hex-nexus — Filesystem Bridge Daemon (`hex-nexus/`)

Bridges SpacetimeDB (sandboxed WASM) with the local OS.

- Reads/writes files, runs architecture analysis (tree-sitter), manages git
- Syncs repo files → SpacetimeDB on startup (ADR-044)
- Serves dashboard at `http://localhost:5555` (assets baked via `rust-embed`)
- Exposes REST API that CLI + MCP tools delegate to
- Editing `hex-nexus/assets/` requires `cd hex-nexus && cargo build --release`
- Remote agent state synced for cross-host fleet visibility

**State**: SpacetimeDB primary (real-time WebSocket), SQLite fallback (`~/.hex/hub.db`). Multi-instance coordination via `ICoordinationPort` with filesystem locks + heartbeats (ADR-011).

## hex-agent — Architecture Enforcement Runtime (`hex-agent/`)

Must be present (locally or remotely) on any host running hex dev agents.

- Skills: slash commands guiding compliant codegen
- Hooks: pre/post op validation, formatting, pattern training
- ADRs: decision records
- Workplans: adapter-bounded task decomposition
- HexFlo dispatchers: native Rust swarm coordination
- Agent definitions: YAML roles (planner, coder, reviewer)

## hex-dashboard — Developer Control Plane (`hex-nexus/assets/`)

Nexus of data + control across projects/systems.

- Multi-project management with freshness indicators
- Agent fleet control (status, heartbeats, assignments)
- Architecture health ring + violation breakdown
- Command dispatch from browser
- Inference monitoring (requests, tokens)
- Solid.js + TailwindCSS, real-time via SpacetimeDB subscriptions

## Inference — Model Integration

- `inference-gateway` WASM routes requests
- `inference-bridge` WASM handles model integration
- hex-nexus performs actual HTTP (WASM can't)
- Model-agnostic: Anthropic / OpenAI / Ollama / any provider

## Standalone Mode (ADR-2604112000)

No Claude Code required. When `CLAUDE_SESSION_ID` is unset, hex-nexus wires `AgentManager` + `OllamaInferenceAdapter`.

```bash
hex nexus start && hex plan execute wp-foo.json   # no Claude CLI needed
hex doctor composition                            # diagnose active variant
hex ci --standalone-gate                          # validate standalone path (P2/P3/P6 suites)
```

Claude-integrated path remains the fast path when `CLAUDE_SESSION_ID` is present.
