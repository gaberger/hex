# hex System Architecture

> **One-Line Summary:** hex is an AAIDE with 5 deployment units — SpacetimeDB (state backbone), hex-nexus (filesystem bridge), hex-agent (enforcement runtime), hex-dashboard (control plane), and hex clients (CLI/web/desktop/chat) — coordinated via real-time WebSocket subscriptions.

---

## Architecture Overview

```
┌──────────────────────────────────────────────────────────────────────────────────┐
│                              hex System Architecture                             │
│                                                                                  │
│                        ┌───────────────────────────────┐                         │
│                        │        SpacetimeDB            │                         │
│                        │   (Coordination & State Core) │                         │
│                        │                               │                         │
│                        │   18 WASM Modules:            │                         │
│                        │   ┌─────────────────────────┐ │                         │
│                        │   │ hexflo-coordination     │ │                         │
│                        │   │ agent-registry          │ │                         │
│                        │   │ inference-gateway       │ │                         │
│                        │   │ workplan-state          │ │                         │
│                        │   │ architecture-enforcer   │ │                         │
│                        │   │ chat-relay              │ │                         │
│                        │   │ fleet-state             │ │                         │
│                        │   │ + 11 more               │ │                         │
│                        │   └─────────────────────────┘ │                         │
│                        │                               │                         │
│                        │   ⚠ WASM sandbox:             │                         │
│                        │   NO filesystem access        │                         │
│                        │   NO process spawning         │                         │
│                        │   NO network calls            │                         │
│                        └──────────┬────────────────────┘                         │
│                            WebSocket│(real-time subscriptions)                   │
│                    ┌───────────────┼───────────────────┐                         │
│                    │               │                   │                         │
│               ┌────▼────┐    ┌────▼────┐         ┌────▼────┐                    │
│               │hex-nexus│    │  hex    │         │  hex    │                    │
│               │(FS      │    │ clients │         │dashboard│                    │
│               │ Bridge) │    │CLI/Web/ │         │(Control │                    │
│               │         │    │Desktop/ │         │ Plane)  │                    │
│               │ REST API│    │Chat     │         │         │                    │
│               │ :5555   │    └─────────┘         └─────────┘                    │
│               └────┬────┘                                                       │
│                    │                                                             │
│               ┌────▼──────────────────┐    ┌──────────────────────────┐          │
│               │   Local OS            │    │      hex-agent         │          │
│               │   • Filesystem        │    │  (Enforcement Runtime)   │          │
│               │   • Git repositories  │    │                          │          │
│               │   • Processes         │    │  Skills · Hooks · ADRs   │          │
│               │   • Shell             │    │  Workplans · HexFlo      │          │
│               └───────────────────────┘    │  Agent Definitions       │          │
│                                            └──────────────────────────┘          │
└──────────────────────────────────────────────────────────────────────────────────┘
```

## Component Details

### 1. SpacetimeDB — Coordination & State Core

**Role:** The backbone of the entire system. Every hex client connects to SpacetimeDB via WebSocket for real-time state synchronization. It replaces polling, hand-rolled coordination, and manual state management.

**Must always be running.** Without SpacetimeDB, hex operates in degraded mode with SQLite fallback (single-node, no real-time sync).

**Key Facts:**
- Rust-native relational database with embedded WASM application logic
- 18 WASM modules define tables and transactional reducers
- WebSocket subscriptions push state changes to all connected clients instantly
- Reducers execute atomically — no partial updates
- WASM sandbox: modules cannot access filesystem, spawn processes, or make network calls

**WASM Modules (18 total):**

| Module | Purpose | Key Tables | Key Reducers |
|:-------|:--------|:-----------|:-------------|
| `hexflo-coordination` | Core swarm orchestration | swarm, swarm_task, swarm_agent, hexflo_memory, project, project_config, skill_registry, agent_definition | swarm_init, task_create, task_complete, agent_register, agent_heartbeat, memory_store |
| `agent-registry` | Agent lifecycle tracking | agent, agent_heartbeat | register_agent, heartbeat, update_status |
| `inference-gateway` | LLM request routing | inference_request, inference_response, inference_provider, agent_budget | request_inference, complete_inference, register_provider, set_agent_budget |
| `inference-bridge` | Model integration | inference_queue, inference_result, provider_route | submit_inference, claim_inference, complete_inference |
| `workplan-state` | Task status tracking | workplan_execution, workplan_task | start_workplan, update_task, advance_phase |
| `chat-relay` | Message routing | conversation, message | create_conversation, send_message |
| `fleet-state` | Compute node registry | compute_node | register_node, update_health |
| `architecture-enforcer` | Boundary rule validation | boundary_rule, write_validation | seed_default_rules, validate_write |
| `rl-engine` | Reinforcement learning | rl_experience, rl_q_entry, rl_pattern | select_action, record_reward, store_pattern |
| `skill-registry` | Skill metadata | skill, skill_trigger_index | register_skill, search_skills |
| `agent-definition-registry` | Agent definition metadata | agent_definition, agent_definition_version | register_definition, update_definition |
| `hook-registry` | Hook management | hook, hook_execution_log | register_hook, toggle_hook, log_execution |
| `file-lock-manager` | Distributed file locks | file_lock | acquire_lock, release_lock, expire_stale_locks |
| `conflict-resolver` | State conflict resolution | conflict_event | report_conflict, resolve_conflict |
| `secret-grant` | Secret distribution (ADR-026) | secret_grant, inference_endpoint, secret_vault, secret_audit_log | grant_secret, claim_grant, store_secret |
| `hexflo-cleanup` | Stale agent detection | agent_health, reclaimable_task, cleanup_log | run_cleanup (scheduled every 30s) |
| `hexflo-lifecycle` | Swarm phase transitions | swarm_lifecycle, lifecycle_task, phase_transition_log | on_task_complete (triggers phase advance) |

**Connection pattern:**
```
Client → ws://localhost:3000 → SpacetimeDB
         DbConnection.builder()
           .withUri("ws://localhost:3000")
           .withDatabaseName("hexflo-coordination")
           .onConnect(subscribe)
           .build()
```

**Config:** `.hex/state.json`
```json
{
  "backend": "spacetimedb",
  "spacetimedb": {
    "host": "localhost:3000",
    "database": "hex-nexus"
  }
}
```

---

### 2. hex-nexus — Filesystem Bridge Daemon

**Role:** Bridges SpacetimeDB's sandboxed WASM environment with the local operating system. Anything that touches the filesystem, git, or processes goes through hex-nexus.

**Key Facts:**
- Rust binary (axum), runs on port 5555
- 95+ REST API endpoints
- Serves dashboard frontend (Solid.js SPA, baked in via `rust-embed`)
- Syncs repo config → SpacetimeDB tables on startup (ADR-044)
- State: SpacetimeDB (primary), SQLite fallback (`~/.hex/hub.db`)
- HexFlo coordination module for swarm orchestration (ADR-027)

**Why it exists:** SpacetimeDB WASM modules cannot access the filesystem, spawn processes, or make network calls. hex-nexus performs these operations on behalf of the system.

**REST API surface (95+ endpoints):**

| Resource | Endpoints | Purpose |
|:---------|:----------|:--------|
| Projects | 4 | Register, list, init, unregister projects |
| Analysis | 5 | Architecture analysis, ADR compliance |
| Swarms | 7 | Create swarms, manage tasks |
| Coordination | 12 | Multi-instance locks, task claims, activity |
| RL Engine | 7 | Action selection, rewards, pattern store |
| Agents | 5 | Spawn, list, terminate agents |
| Workplans | 7 | Execute, pause, resume, report |
| Fleet | 7 | Register nodes, deploy, health check |
| Secrets | 7 | Grant, claim, revoke secrets, vault ops |
| Inference | 5 | Register providers, completions |
| Git | 12 | Status, log, diff, branches, worktrees, blame |
| ADRs | 3 | List, get, save ADRs |
| HexFlo Memory | 5 | Store, retrieve, search, delete memory |
| Sessions | 12 | Chat session management (sqlite-session feature) |
| WebSocket | 2 | Real-time events + chat |
| Meta | 3 | OpenAPI spec, docs, version |

**Config sync on startup (ADR-044):**
```
.hex/blueprint.json         → project_config table
.claude/settings.json        → project_config (MCP servers, hooks)
.claude/skills/*.md          → skill_registry table
.claude/agents/*.yml         → agent_definition table
```

**Depends on:** SpacetimeDB, hex-core
**Depended on by:** hex-cli, hex-dashboard, hex-agent

---

### 3. hex-agent — Architecture Enforcement Runtime

**Role:** The component that **must always be present** (locally or remotely) on any system running hex development agents. It is the software that makes AI agents produce architecture-compliant code.

> Not to be confused with the hexagonal architecture concept of "adapter." hex-agent is the *agent runtime* — the software that makes AI development agents produce architecture-compliant code.

**Key Facts:**
- Runs locally or on remote compute nodes
- Connects to SpacetimeDB for coordination
- Uses hex-nexus for filesystem operations
- Enforces architecture through 6 mechanisms (see below)

**Enforcement mechanisms:**

| Mechanism | What It Does | Where Defined |
|:----------|:-------------|:-------------|
| **Skills** | Slash commands that guide AI agents through hex-compliant workflows | `.claude/skills/`, `skills/` |
| **Hooks** | Pre/post operation triggers — validate boundaries, auto-format, train patterns | `.claude/settings.json` |
| **ADRs** | Architecture Decision Records — document and track design choices | `docs/adrs/` |
| **Workplans** | Structured task decomposition into adapter-bounded steps | `docs/workplans/` |
| **HexFlo dispatchers** | Native Rust coordination for multi-agent swarm execution | `hex-nexus/src/coordination/` |
| **Agent definitions** | YAML roles (planner, coder, reviewer) with boundaries and constraints | `agents/`, `.claude/agents/` |

**14 agent definitions:**

| Agent | Role |
|:------|:-----|
| `feature-developer` | Orchestrates full feature lifecycle |
| `planner` | Decomposes requirements into tasks |
| `hex-coder` | Codes within one adapter boundary (TDD) |
| `integrator` | Merges worktrees, integration tests |
| `swarm-coordinator` | Orchestrates lifecycle via HexFlo |
| `behavioral-spec-writer` | Writes acceptance specs before code |
| `validation-judge` | Post-build validation (BLOCKING gate) |
| `dead-code-analyzer` | Finds dead exports + boundary violations |
| `scaffold-validator` | Ensures projects are runnable |
| `dependency-analyst` | Recommends tech stack |
| `status-monitor` | Swarm progress monitoring |
| `adr-reviewer` | Reviews ADR compliance |
| `rust-refactorer` | Rust-specific refactoring |
| `dev-tracker` | Development activity tracking |

**Depends on:** SpacetimeDB, hex-nexus, hex-core
**Depended on by:** Any system running hex development agents

---

### 4. hex-dashboard — Developer Control Plane

**Role:** The nexus of data and control for developers using hex across many projects and systems. A single interface for monitoring, managing, and commanding the entire hex fleet.

**Key Facts:**
- Solid.js + TailwindCSS SPA
- Baked into hex-nexus binary via `rust-embed`
- Real-time data via SpacetimeDB WebSocket subscriptions (not polling)
- Available at `http://localhost:5555`
- Source in `hex-nexus/assets/`

**Views and capabilities:**

| View | Purpose |
|:-----|:--------|
| **Projects** | Multi-project tabs with live freshness indicators |
| **Architecture Health** | Real-time score ring, violation/dead-export breakdown |
| **Agent Fleet** | Agent list, heartbeat status, task assignments |
| **Swarm Status** | Task progress, topology visualization |
| **Dependency Graph** | Interactive canvas, hexagonal ring layout, violation highlighting |
| **Command Dispatch** | Send commands to any connected project from browser |
| **Chat** | Conversational interface with agents via WebSocket |
| **Config** | Secrets, environment variables, project settings |
| **Event Log** | Filterable real-time stream (errors, decisions, milestones) |
| **Decision Modal** | Interactive prompts for agent decisions requiring human input |

**SpacetimeDB connections (from dashboard frontend):**
```
hexflo-coordination  → swarms, tasks, agents, memory
agent-registry       → agent lifecycle and heartbeats
inference-gateway    → model requests and responses
fleet-state          → compute node status
```

**Depends on:** SpacetimeDB (real-time data), hex-nexus (serves the SPA)
**Depended on by:** Developers (consumption only)

---

### 5. hex Clients — CLI, Web, Desktop, Chat

**Role:** User-facing interfaces that connect to SpacetimeDB for state and hex-nexus for filesystem operations.

| Client | Crate | Technology | Purpose |
|:-------|:------|:-----------|:--------|
| **hex-cli** | `hex-cli/` | Rust (clap) | Canonical CLI — all hex commands. Also serves MCP tools via `hex mcp`. |
| **hex-dashboard** | `hex-nexus/assets/` | Solid.js SPA | Browser-based control plane (see above). |
| **hex-desktop** | `hex-desktop/` | Tauri wrapper | Native desktop app wrapping the web dashboard. |
**CLI commands (19 subcommands):**
```
hex nexus [start|stop|status]    # Manage hex-nexus daemon
hex agent [spawn|list|kill]      # Manage agents
hex secrets [has|get|set]        # Secret management
hex stdb [start|stop|publish]    # Local SpacetimeDB management
hex swarm [init|status]          # Swarm coordination
hex task [create|list|complete]  # Task management
hex memory [store|get|search]    # Persistent memory
hex adr [list|status|search|abandoned]  # ADR lifecycle
hex analyze [path]               # Architecture health check
hex plan [requirements]          # Generate workplan
hex project [list|init]          # Project management
hex status                       # Project overview
hex mcp                          # Start MCP stdio server
```

**MCP tools** (`hex mcp`) share the same backend as CLI commands — both delegate to hex-nexus REST API.

---

## Data Flow

### 1. Agent completing a task

```
Agent (hex-agent)
  ↓ writes code in git worktree
hex-nexus REST API
  ↓ POST /api/swarms/:id/tasks/:task_id (status=completed)
hex-nexus calls SpacetimeDB reducer
  ↓ task_complete(task_id, result)
SpacetimeDB updates swarm_task table
  ↓ WebSocket subscription push
All connected clients see update instantly
  ├── hex-dashboard: task badge turns green
  ├── hex-cli: status command shows completion
  └── Other agents: may unblock dependent tasks
```

### 2. Inference request

```
Agent needs LLM completion
  ↓ SpacetimeDB reducer: request_inference(...)
inference-gateway records request in inference_request table
  ↓ hex-nexus watches for pending requests
hex-nexus makes HTTP call to provider (Anthropic/OpenAI/Ollama/etc.)
  ↓ receives response
hex-nexus calls SpacetimeDB reducer: complete_inference(...)
  ↓ WebSocket subscription push
Agent receives inference_response via subscription
```

### 3. Config sync on startup

```
hex nexus start
  ↓ reads local config files
hex-nexus/src/config_sync.rs
  ↓ .hex/blueprint.json → register_project + sync_config reducers
  ↓ .claude/settings.json → sync_config reducer
  ↓ .claude/skills/*.md → sync_skill reducer (per skill)
  ↓ .claude/agents/*.yml → sync_agent_def reducer (per agent)
SpacetimeDB tables updated
  ↓ WebSocket push to all clients
Dashboard shows current project config
```

---

## Deployment Topologies

### Local Development (Single Machine)

```
┌─────────────────────────────┐
│  Developer Machine          │
│                             │
│  SpacetimeDB (localhost:3000)│
│  hex-nexus   (localhost:5555)│
│  hex-agent (local)        │
│  hex-cli     (terminal)     │
│  Browser     (dashboard)    │
└─────────────────────────────┘
```

### Team Development (Shared SpacetimeDB)

```
┌──────────────┐  ┌──────────────┐  ┌──────────────┐
│ Dev Machine 1│  │ Dev Machine 2│  │ Dev Machine 3│
│ hex-nexus    │  │ hex-nexus    │  │ hex-nexus    │
│ hex-agent  │  │ hex-agent  │  │ hex-agent  │
│ hex-cli      │  │ hex-cli      │  │ hex-cli      │
└──────┬───────┘  └──────┬───────┘  └──────┬───────┘
       │                 │                 │
       └────────────┬────┴────────────────┘
                    │
          ┌─────────▼──────────┐
          │  Shared SpacetimeDB │
          │  (team server)      │
          └────────────────────┘
```

### Fleet Development (Remote Compute)

```
┌──────────────┐        ┌─────────────────────┐
│ Dev Machine  │        │  Compute Fleet      │
│ hex-cli      │        │  ┌───────────────┐  │
│ Browser      ├──WS───►│  │ Node 1        │  │
│              │        │  │ hex-agent    │  │
└──────────────┘        │  │ hex-nexus      │  │
                        │  └───────────────┘  │
                        │  ┌───────────────┐  │
                        │  │ Node 2        │  │
          ┌─────────┐   │  │ hex-agent    │  │
          │SpacetimeDB│◄──│ hex-nexus      │  │
          │(central) │   │  └───────────────┘  │
          └─────────┘   └─────────────────────┘
```

---

## Key Design Principles

1. **SpacetimeDB is the source of truth** — All state lives in SpacetimeDB. hex-nexus and SQLite are bridges/fallbacks, not primary stores.

2. **WASM sandbox is a feature, not a limitation** — Sandboxed reducers guarantee deterministic, transactional state transitions. The filesystem bridge (hex-nexus) is the explicit boundary between pure state and side effects.

3. **Real-time over polling** — Every client subscribes via WebSocket. State changes propagate instantly. No polling loops, no stale data.

4. **Model-agnostic inference** — hex works with any LLM provider. The inference gateway routes requests without being coupled to a specific model or vendor.

5. **Mechanical enforcement over prompt engineering** — Architecture rules are checked by static analysis (`hex analyze`), validated server-side (`architecture-enforcer` module), and enforced at write time (hooks). Prompts guide agents, but machines verify.

6. **Single composition root** — Only one file wires adapters to ports. Adapter swaps are one-line changes. No adapter imports another adapter.

---

## Related ADRs

| ADR | Title | Components Affected |
|:----|:------|:-------------------|
| ADR-001 | Hexagonal Architecture | All |
| ADR-010 | TypeScript-to-Rust Migration | hex-cli, hex-nexus |
| ADR-011 | Multi-Instance Coordination | hex-nexus, hex-agent |
| ADR-019 | CLI-MCP Parity | hex-cli |
| ADR-025 | SpacetimeDB as State Backend | SpacetimeDB, hex-nexus |
| ADR-026 | Secret Management | hex-nexus, SpacetimeDB |
| ADR-027 | HexFlo Swarm Coordination | hex-nexus, SpacetimeDB |
| ADR-034 | Migrate Analyzer to Rust | hex-nexus |
| ADR-044 | Config Sync to SpacetimeDB | hex-nexus, SpacetimeDB |
| ADR-045 | ADR Compliance Enforcement | hex-nexus |
| ADR-047 | Internal Documentation System | All |
