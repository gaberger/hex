# Component: hex Clients

## One-Line Summary

User-facing interfaces — CLI, web, desktop, and chat — that connect to SpacetimeDB for real-time state and hex-nexus for filesystem operations, providing multiple ways to interact with the hex AAIDE.

## Key Facts

- 4 client types: CLI (Rust), Web (dashboard SPA), Desktop (Tauri), Chat (Tauri)
- All connect to SpacetimeDB via WebSocket for real-time state
- All delegate filesystem operations to hex-nexus REST API
- CLI also serves MCP tools via `hex mcp` (same binary, same backend)
- Model-agnostic — inference routing is handled by SpacetimeDB, not clients

## Clients

### hex-cli — Canonical CLI

**Crate:** `hex-cli/`
**Technology:** Rust (clap)
**Purpose:** The primary user interface. ALL hex commands go through this binary.

**Key characteristics:**
- 19 subcommands covering analysis, swarm coordination, secrets, git, ADRs
- MCP server mode (`hex mcp`) for IDE integration — same backend as CLI
- Delegates to hex-nexus REST API for all operations
- Queries SpacetimeDB directly for state where needed

**Commands:**
```
hex nexus [start|stop|status]        # Manage hex-nexus daemon
hex agent [spawn|list|kill]          # Manage agents
hex secrets [has|get|set]            # Secret management
hex stdb [start|stop|publish]        # Local SpacetimeDB management
hex swarm [init|status]              # Swarm coordination
hex task [create|list|complete]      # Task management
hex memory [store|get|search]        # Persistent memory
hex adr [list|status|search|abandoned]  # ADR lifecycle
hex analyze [path]                   # Architecture health check
hex plan [requirements]              # Generate workplan
hex project [list|init]              # Project management
hex status                           # Project overview
hex mcp                              # Start MCP stdio server
```

**MCP tools** (served by `hex mcp`):
All MCP tool names map 1:1 to CLI commands:
```
mcp__hex__hex_analyze      → hex analyze [path]
mcp__hex__hex_status       → hex status
mcp__hex__hex_swarm_init   → hex swarm init
mcp__hex__hex_task_create  → hex task create
mcp__hex__hex_adr_list     → hex adr list
mcp__hex__hex_nexus_start  → hex nexus start
... (17+ tools total)
```

**IMPORTANT:** Never recommend commands that don't exist in `hex --help`.

### hex-dashboard — Web Control Plane

**Source:** `hex-nexus/assets/`
**Technology:** Solid.js + TailwindCSS SPA
**Purpose:** Browser-based control plane for multi-project management

See [hex-dashboard.md](hex-dashboard.md) for full details.

**Access:** `http://localhost:5555` (served by hex-nexus)

### hex-desktop — Native Desktop App

**Crate:** `hex-desktop/`
**Technology:** Tauri wrapper
**Purpose:** Native desktop application that wraps the hex-dashboard web UI

Provides native OS integration (system tray, notifications, file dialogs) while rendering the same Solid.js dashboard. Useful for developers who prefer a dedicated app window over a browser tab.

### hex-chat — Conversational Interface

**Crate:** `hex-chat/`
**Technology:** Tauri + TypeScript
**Purpose:** Conversational chat interface for agent interaction

Provides a chat-style interface for interacting with hex agents. Messages are routed through the `chat-relay` SpacetimeDB module, enabling human-agent and agent-agent communication.

## Connection Architecture

All clients follow the same pattern:

```
Client
  ├── WebSocket → SpacetimeDB (real-time state subscriptions)
  │                ├── hexflo-coordination (swarms, tasks, agents)
  │                ├── agent-registry (agent lifecycle)
  │                ├── inference-gateway (LLM requests)
  │                └── fleet-state (compute nodes)
  │
  └── HTTP → hex-nexus REST API (filesystem operations)
              ├── /api/analyze (architecture analysis)
              ├── /api/{project}/git/* (git operations)
              ├── /api/files (file read/write)
              └── /api/swarms (swarm management)
```

## Depends On

- **SpacetimeDB** — real-time state via WebSocket
- **hex-nexus** — REST API for filesystem, analysis, git operations
- **hex-core** — shared domain types (Rust clients)

## Depended On By

- Developers and AI agents (consumption only)

## Related ADRs

- ADR-010: TypeScript-to-Rust Migration (hex-cli rewrite)
- ADR-019: CLI-MCP Parity (CLI and MCP tools share same backend)
