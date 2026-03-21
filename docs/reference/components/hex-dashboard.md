# Component: hex-dashboard

## One-Line Summary

Developer control plane — a single interface for managing AI-driven development across multiple projects and systems, providing agent fleet control, architecture health monitoring, command dispatch, and inference monitoring via real-time SpacetimeDB WebSocket subscriptions.

## Key Facts

- Solid.js + TailwindCSS single-page application
- Baked into hex-nexus binary at compile time via `rust-embed`
- Served at `http://localhost:5555`
- Real-time data via SpacetimeDB WebSocket subscriptions (not polling)
- Source in `hex-nexus/assets/`
- Multiple SpacetimeDB database connections: hexflo-coordination, agent-registry, inference-gateway, fleet-state
- Modern dark theme optimized for extended developer use

## Why It's a Control Plane

hex-dashboard is not just a monitoring UI — it is a **control plane** that lets developers:

1. **Observe** — See architecture health, agent status, task progress, and inference costs across all projects
2. **Command** — Send commands to any connected project (analyze, build, spawn agents)
3. **Coordinate** — Manage agent fleet, resolve decisions, handle conflicts
4. **Configure** — Manage secrets, environment variables, project settings

This is the "mission control" for developers using hex at scale across many projects and systems.

## Views

| View | Purpose | Data Source |
|:-----|:--------|:-----------|
| **Projects** | Multi-project tabs with live freshness indicators | project table (hexflo-coordination) |
| **Architecture Health** | Real-time score ring, violation/dead-export breakdown | hex-nexus analysis API |
| **Agent Fleet** | Agent list, heartbeat status, task assignments, pulse animations | agent table (agent-registry) |
| **Swarm Status** | Task progress, agent topology visualization | swarm/swarm_task/swarm_agent (hexflo-coordination) |
| **Dependency Graph** | Interactive canvas, hexagonal ring layout, zoom/pan, violation highlighting | hex-nexus analysis API |
| **Token Efficiency** | L0–L3 compression bars per file | hex-nexus summarize API |
| **Command Chat** | Send commands to any connected project from browser | chat-relay + hex-nexus command API |
| **Inference Monitor** | Track model requests, token consumption, costs | inference_request/response (inference-gateway) |
| **Config Page** | Secrets, environment variables, project settings | project_config (hexflo-coordination) |
| **Event Log** | Filterable real-time stream (errors, decisions, milestones) | hex-nexus WebSocket |
| **Decision Modal** | Interactive prompts for agent decisions requiring human input | hex-nexus command API |
| **ADR Browser** | Browse and edit Architecture Decision Records | hex-nexus ADR API |

## Source Structure

```
hex-nexus/assets/
├── index.html                  # Entry point
├── package.json                # Vite, Solid.js, TailwindCSS, SpacetimeDB SDK
├── vite.config.ts              # Dev server on port 5174
├── tailwind.config.js
└── src/
    ├── app/
    │   └── App.tsx             # Main component — pane management, keyboard shortcuts
    ├── components/
    │   ├── ControlPlane.tsx     # Primary dashboard layout
    │   ├── AgentFleet.tsx       # Agent management view
    │   ├── ProjectDetail.tsx    # Per-project detail view
    │   ├── ADRBrowser.tsx       # ADR browsing and editing
    │   └── ConfigPage.tsx       # Configuration management
    ├── hooks/                  # Reactive Solid.js hooks
    ├── stores/
    │   ├── connection.ts       # SpacetimeDB WebSocket connections
    │   ├── router.ts           # Client-side routing
    │   ├── ui.ts               # UI state management
    │   ├── chat.ts             # Chat state
    │   ├── hexflo-monitor.ts   # HexFlo monitoring
    │   └── nexus-health.ts     # Nexus health tracking
    └── spacetimedb/            # Auto-generated SpacetimeDB client bindings
        ├── hexflo-coordination/
        ├── agent-registry/
        ├── inference-gateway/
        └── fleet-state/
```

## SpacetimeDB Connections

The dashboard connects to multiple SpacetimeDB databases simultaneously:

```typescript
// From hex-nexus/assets/src/stores/connection.ts
const connections = {
  "hexflo-coordination": {  // Swarms, tasks, agents, memory, config
    uri: "ws://localhost:3000",
    subscriptions: ["SELECT * FROM swarm", "SELECT * FROM swarm_task", ...]
  },
  "agent-registry": {       // Agent lifecycle and heartbeats
    uri: "ws://localhost:3000",
    subscriptions: ["SELECT * FROM agent", "SELECT * FROM agent_heartbeat"]
  },
  "inference-gateway": {    // Model requests and responses
    uri: "ws://localhost:3000",
    subscriptions: ["SELECT * FROM inference_request", ...]
  },
  "fleet-state": {          // Compute node status
    uri: "ws://localhost:3000",
    subscriptions: ["SELECT * FROM compute_node"]
  }
}
```

Token persistence in localStorage. Auto-reconnection with retry logic.

## Development

**Dev server (hot reload):**
```bash
cd hex-nexus/assets
bun install
bun run dev          # Vite dev server on port 5174
```

**Production build:**
```bash
cd hex-nexus/assets
bun run build        # Output to hex-nexus/assets/dist/
cd ..
cargo build --release  # Bakes dist/ into binary via rust-embed
```

**Important:** Any change to dashboard assets requires rebuilding the hex-nexus Rust binary. Then restart the daemon and hard-refresh the browser (Cmd+Shift+R).

## Depends On

- **SpacetimeDB** — real-time data via WebSocket subscriptions
- **hex-nexus** — serves the SPA, provides REST API for commands and analysis

## Depended On By

- Developers (consumption only — no other component depends on the dashboard)
- hex-desktop (wraps the dashboard in a Tauri native app)

## Related ADRs

- ADR-024: Hex-Hub Autonomous Nexus (original dashboard concept)
- ADR-032: Deprecate hex-hub (migration to hex-nexus-served dashboard)
