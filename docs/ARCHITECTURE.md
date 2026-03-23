# hex System Architecture

hex is composed of five deployment units that work together.

```
┌──────────────────────────────────────────────────────────────────────────────┐
│                           hex System Architecture                           │
│                                                                              │
│  ┌─────────────────┐     WebSocket      ┌─────────────────────────────────┐  │
│  │  hex-dashboard   │◄──────────────────►│         SpacetimeDB            │  │
│  │  (Control Plane) │    real-time sub    │  (Coordination & State Core)   │  │
│  │                  │                    │                                 │  │
│  │  Multi-project   │                    │  19 WASM modules:              │  │
│  │  monitoring,     │                    │  • hexflo-coordination         │  │
│  │  agent fleet     │                    │  • agent-registry              │  │
│  │  management,     │                    │  • inference-gateway           │  │
│  │  architecture    │                    │  • workplan-state              │  │
│  │  health views    │                    │  • test-results                │  │
│  │                  │                    │  • + 14 more                   │  │
│  └─────────────────┘                    │                                 │  │
│                                          │  Provides:                     │  │
│  ┌─────────────────┐     WebSocket      │  • WebSocket pub/sub           │  │
│  │  hex Clients     │◄──────────────────►│  • Transactional reducers      │  │
│  │  (CLI, MCP,      │    real-time sub    │  • SQL query interface          │  │
│  │   Desktop)       │                    │  • Automatic state replication  │  │
│  └─────────────────┘                    └─────────────────────────────────┘  │
│                                                                              │
│  ┌─────────────────┐     REST API       ┌─────────────────────────────────┐  │
│  │  hex-nexus       │◄─────────────────►│  hex-agent                      │  │
│  │  (Filesystem     │                    │  (Architecture Enforcement)     │  │
│  │   Bridge)        │                    │                                 │  │
│  │                  │                    │  Enforces hex architecture via: │  │
│  │  Bridges STDB    │                    │  • Skills & hooks               │  │
│  │  and local OS    │                    │  • ADRs & workplans             │  │
│  └─────────────────┘                    │  • HexFlo dispatchers           │  │
│                                          └─────────────────────────────────┘  │
└──────────────────────────────────────────────────────────────────────────────┘
```

## SpacetimeDB — Coordination & State Core

**SpacetimeDB must always be running to use hex.** It is the backbone — every client connects via WebSocket for real-time state synchronization.

| Capability | How It Works |
|:-----------|:-------------|
| Real-time state sync | All clients subscribe via WebSocket — no polling |
| Transactional coordination | Reducers (WASM stored procedures) provide atomic transitions |
| Swarm orchestration | `hexflo-coordination` module tracks swarms, tasks, agents, memory |
| Inference routing | `inference-gateway` module routes LLM requests |
| Agent lifecycle | `agent-registry` module tracks heartbeats and status |
| Test tracking | `test-results` module persists test session outcomes |

## hex-nexus — Filesystem Bridge

hex-nexus bridges SpacetimeDB (sandboxed WASM) with the local OS:

- Reads/writes files on behalf of SpacetimeDB operations
- Runs architecture analysis (tree-sitter, boundary checking)
- Manages git operations (blame, diff, worktree management)
- Serves the dashboard (assets baked in via `rust-embed`)
- Exposes REST API that CLI and MCP tools delegate to
- Enforces operations via axum middleware (ADR-2603221959)

## hex-agent — Architecture Enforcement Runtime

hex-agent must always be present on any system running hex development agents:

| Mechanism | What It Enforces |
|:----------|:----------------|
| Skills | Claude Code slash commands for architecture-compliant code |
| Hooks | Pre/post operation hooks for boundary validation |
| ADRs | Architecture Decision Records documenting design choices |
| Workplans | Structured task decomposition into adapter-bounded steps |
| HexFlo | Native Rust coordination for multi-agent swarm execution |

## hex-dashboard — Developer Control Plane

Solid.js + TailwindCSS frontend served by hex-nexus at `http://localhost:5555`:

- Multi-project management with live freshness indicators
- Agent fleet control — status, heartbeats, task assignments
- Architecture health — real-time score ring with violations
- Swarm visualization — task progress, dependency graphs
- Test health — pass rate trends, flake detection

## Hexagonal Architecture Layers

| Layer | May Import From | Purpose |
|:------|:---------------|:--------|
| `domain/` | `domain/` only | Pure business logic, zero external deps |
| `ports/` | `domain/` only | Typed interfaces — contracts between layers |
| `usecases/` | `domain/` + `ports/` | Application logic composing ports |
| `adapters/primary/` | `ports/` only | Driving: CLI, HTTP, MCP, Dashboard |
| `adapters/secondary/` | `ports/` only | Driven: FS, Git, LLM, TreeSitter |
| `composition-root` | Everything | The ONLY file that imports adapters |

**The golden rule:** Adapters NEVER import other adapters.
