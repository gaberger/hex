# ADR-039: Nexus Agent Control Plane — OpenCode-Inspired Multi-Project Interface

**Status:** Accepted
**Accepted Date:** 2026-03-22
## Date: 2026-03-20
- **Informed by**: ADR-036 (sessions), ADR-037 (agent lifecycle), ADR-038 (Vite/Axum), OpenCode (anomalyco/opencode)
- **Authors**: Gary (architect), Claude (analysis)
- **Supersedes**: Current vanilla JS dashboard in `hex-nexus/assets/`

## Context

### The Vision

hex-nexus should be a **multi-project agent control plane** — a developer can open one browser tab and see agents working across multiple projects simultaneously, like a terminal multiplexer for AI development agents. They can spawn agents, route inference, watch live diffs, intervene when needed, and manage their entire AI development fleet from a single interface.

### What Exists Today

The backend is surprisingly mature:

| Capability | Status | Location |
|-----------|--------|----------|
| SpacetimeDB integration | ✅ Built | `hex-nexus/src/spacetime_bindings/` (10 modules) |
| Multi-project registry | ✅ Built | `POST /api/projects/register`, per-project routes |
| Inference routing | ✅ Built | Ollama/OpenAI/vLLM/llama-cpp, health checks, cost tracking |
| Session persistence | ✅ Built | SQLite + SpacetimeDB backends (ADR-036) |
| Agent lifecycle | ✅ Built | Heartbeat, stale/dead detection, task reclamation |
| HexFlo coordination | ✅ Built | Swarms, tasks, scoped memory, cleanup |
| Fleet management | ⚠️ Partial | SSH routes exist, less tested |
| WebSocket streaming | ✅ Built | `/ws/chat`, real-time events |
| hex-chat TUI | ✅ Built | ratatui 3-panel (fleet, chat, taskboard) |
| hex-chat web | ✅ Built | 2000+ lines vanilla JS, 16 modules |

### The Gap

Despite this backend maturity, the frontend fails the vision:

1. **Chat-centric, not agent-centric.** The dashboard centers on conversations. There's no split-pane view showing Project A's agent writing tests while Project B's agent does architecture analysis.

2. **No GUI-driven agent spawning.** You can't click a project and say "start a feature dev swarm here." Orchestration happens from CLI/Claude Code, not the GUI.

3. **No terminal-multiplexer UX.** hex-chat TUI has 3 panels but all serve one session. The vision is tmux for AI agents — multiple panes, each project, real-time diffs.

4. **Vanilla JS doesn't scale.** 2000+ lines of unstructured JavaScript across 16 files with no component model, no type safety, no state management. Adding the control plane UX on this foundation would be untenable.

5. **No command palette.** Power users expect Ctrl+P fuzzy-find for 60+ actions, not hunting through sidebar menus.

6. **No code display quality.** No syntax highlighting (Shiki), no diff viewer (Pierre), no file tree with change indicators.

### Why OpenCode Is the Right Reference

OpenCode (93K stars, 2.5M monthly developers) has solved the "AI coding interface" problem at scale with architecture decisions that map directly to hex-nexus:

| OpenCode Pattern | hex-nexus Mapping |
|-----------------|-------------------|
| Go backend + OpenAPI + SSE | Rust/axum backend + OpenAPI; **SpacetimeDB WebSocket replaces SSE** |
| SolidJS shared UI library | `@hex/ui` component library |
| Session-centric views | Already have sessions (ADR-036) |
| Build/Plan agent duality | hex-coder (build) + planner (plan) |
| Multi-session parallel agents | HexFlo swarm coordination |
| Pierre diffs + Shiki highlighting | Code display in browser |
| Command palette + slash + keybindings | Three access methods |
| Provider transformation layer | Inference routing (already built) |
| Vite dev + production embed | ADR-038 already decided this |

The key difference: **OpenCode manages one project per instance. hex-nexus manages many projects from one instance.** This is our differentiator — the multiplexer dimension.

### Why SpacetimeDB Changes the Architecture

OpenCode uses SSE for server→client event streaming because its Go backend owns all state. hex-nexus is different: **SpacetimeDB already owns the state and already provides real-time subscriptions via WebSocket.** Building a custom SSE event bus on top of SpacetimeDB would be redundant — we'd be re-implementing what SpacetimeDB gives us natively.

SpacetimeDB's client protocol:
- **WebSocket-only** for real-time data (binary BSATN or JSON sub-protocol)
- **SQL-based subscriptions**: client subscribes with SQL queries, receives initial snapshot (`SubscribeApplied`), then row-level deltas (`TransactionUpdate`) automatically on every transaction
- **Client-side cache**: SDK maintains an atomic, consistent local cache — no manual state sync
- **Typed codegen**: `spacetime generate --lang typescript` produces table types, reducer calls, and callback registrations from the WASM module
- **Browser-native**: TypeScript SDK opens WebSocket directly from browser to SpacetimeDB, no intermediate server needed

This means the browser can subscribe to `SELECT * FROM agents WHERE project_id = 'app-a'` and automatically receive row-level inserts/updates/deletes whenever any agent (on any machine) mutates that table. **No custom EventBus, no SSE endpoint, no polling.**

### What hex-nexus Already Has in SpacetimeDB

17 WASM modules exist in `spacetime-modules/` with tables for:
- `hexflo-coordination` — swarms, tasks, agents, heartbeats, scoped memory
- `agent-registry` — agent tracking, status, heartbeats
- `chat-relay` — message persistence
- `inference-gateway` — endpoint registry, health, rate limits
- `fleet-state` — compute node management
- `rl-engine` — Q-learning, pattern storage
- `workplan-state` — task execution state
- `skill-registry`, `hook-registry`, `agent-definition-registry` — metadata catalogs
- `secret-grant`, `conflict-resolver`, `file-lock-manager`, `architecture-enforcer`

Rust server-side bindings are generated for 8 modules. HTTP reducer calls work for RL, chat, and HexFlo. **What's missing: TypeScript browser bindings and the WebSocket subscription wiring.**

## Implementation Progress

| Component | Status | Notes |
|-----------|--------|-------|
| SolidJS dashboard rebuild | Done | `hex-nexus/assets/src/` with components/, stores/, hooks/, spacetimedb/ |
| SpacetimeDB WebSocket subscriptions | Done | Dashboard subscribes to swarm, task, agent, project tables |
| Multi-project views | Done | Project list, detail views, architecture health |
| Agent fleet panel | Partial | Agent list exists, but no GUI-driven spawning |
| Command palette (Ctrl+P) | Not started | No fuzzy-find command dispatch |
| Split-pane agent views | Not started | No tmux-style multi-agent monitoring |
| Code display (Shiki/Pierre) | Not started | No syntax-highlighted diffs in browser |
| GUI-driven agent spawning | Not started | Orchestration still requires CLI/Claude Code |

## Decision

### 1. Architecture: SolidJS Frontend, SpacetimeDB-Native State, hex-nexus Compute Layer

The fundamental insight: **SpacetimeDB IS the event bus.** The browser connects directly to SpacetimeDB for all live state (swarms, tasks, agents, sessions, inference). hex-nexus becomes a stateless compute layer for operations that need filesystem access (analyze, summarize, scaffold, agent process management).

```
┌─────────────────────────────────────────────────────────────┐
│                     Browser (SolidJS)                        │
│                                                             │
│  ┌──────────────────────┐    ┌───────────────────────────┐  │
│  │  SpacetimeDB SDK     │    │  hex-nexus REST            │  │
│  │  (WebSocket)         │    │  (stateless compute only)  │  │
│  │                      │    │                            │  │
│  │  SQL subscriptions:  │    │  POST /api/analyze         │  │
│  │  • swarms            │    │  POST /api/agents/spawn    │  │
│  │  • tasks             │    │  POST /api/summarize       │  │
│  │  • agents            │    │  POST /api/scaffold        │  │
│  │  • sessions          │    │  GET  /api/openapi.json    │  │
│  │  • memory            │    │  GET  /api/projects/files  │  │
│  │  • inference_endpoints│    │                            │  │
│  │  • fleet_nodes       │    │  WebSocket /ws/chat        │  │
│  │                      │    │  (bidirectional LLM stream) │  │
│  │  Auto-sync:          │    │                            │  │
│  │  onInsert / onUpdate │    │  (Axum — compute + proxy)  │  │
│  │  onDelete callbacks  │    │                            │  │
│  └──────────┬───────────┘    └──────────┬────────────────┘  │
│             │ ws://                      │ https://          │
└─────────────┼──────────────────────────┼────────────────────┘
              │                          │
    ┌─────────▼──────────┐    ┌──────────▼──────────┐
    │   SpacetimeDB      │    │   hex-nexus (Rust)   │
    │   (state + sync)   │◄───│   (compute + proxy)  │
    │                    │    │                      │
    │   17 WASM modules  │    │   hex analyze         │
    │   Tables + Reducers│    │   hex summarize       │
    │   Row-level deltas │    │   Agent process mgmt  │
    │   Client-side cache│    │   File system access   │
    │                    │    │   LLM bridge (chat)    │
    └────────────────────┘    └──────────────────────┘
              ▲                          ▲
              │                          │
    ┌─────────┴──────────┐    ┌──────────┴──────────┐
    │   hex-agent (Rust)  │    │   hex-chat TUI       │
    │   Writes state via  │    │   (ratatui)          │
    │   SpacetimeDB       │    │   Rust SpacetimeDB   │
    │   reducers          │    │   SDK subscriptions   │
    └────────────────────┘    └──────────────────────┘
```

#### Data Flow: How Real-Time Updates Work

```
1. hex-agent on bazzite completes a task
2. Agent calls SpacetimeDB reducer: complete_task(task_id, result)
3. SpacetimeDB updates `tasks` table row
4. SpacetimeDB pushes TransactionUpdate to ALL subscribed clients:
   - Browser (SolidJS) subscribed to "SELECT * FROM tasks WHERE swarm_id = 'auth-feat'"
   - hex-chat TUI subscribed to same query via Rust SDK
   - Other browser tabs, other machines — all get the delta
5. SpacetimeDB TypeScript SDK fires onUpdate callback
6. SolidJS signal updates → only the TaskNode component re-renders
```

No custom EventBus. No SSE endpoint. No polling. SpacetimeDB handles fan-out to every connected client automatically.

#### Two Transport Channels (Not Three)

| Channel | Protocol | Purpose | When |
|---------|----------|---------|------|
| **SpacetimeDB** | WebSocket (BSATN) | All state: swarms, tasks, agents, sessions, memory, inference, fleet | Always connected |
| **hex-nexus** | HTTP REST + WebSocket `/ws/chat` | Stateless compute (analyze, summarize) + bidirectional LLM chat streaming | On-demand |

**Why NOT SSE:** SpacetimeDB's WebSocket subscription model provides everything SSE would — real-time push, automatic reconnection, ordered delivery — plus it adds SQL-based filtering, client-side caching, and typed codegen. Adding SSE on top would be redundant complexity.

**Why WebSocket `/ws/chat` remains:** LLM chat streaming is bidirectional (user sends message → agent streams tokens back). This is a live conversation, not table state. SpacetimeDB stores the completed messages (via `chat-relay` module), but the streaming happens through hex-nexus's LLM bridge.

**Why SolidJS over React/Svelte:**
- OpenCode uses SolidJS for all browser surfaces — proven for this exact use case
- Fine-grained reactivity without virtual DOM — critical for real-time streaming updates
- Smaller bundle (~7KB vs React's ~40KB) — embedded in Rust binary
- JSX syntax familiar to React developers
- Signal-based state maps naturally to SpacetimeDB's `onInsert`/`onUpdate`/`onDelete` callbacks — each callback updates a SolidJS signal, triggering only the affected DOM nodes

### 2. Control Plane Layout — The Multiplexer

The primary interface is an **agent-centric multiplexer**, not a chat window:

```
┌─────────────────────────────────────────────────────────────────────┐
│  ⬡ HEX NEXUS                          [Ctrl+P] ░░░░░░░░  ⚙ │ ? │
├───────────┬────────────────────────────────────────┬────────────────┤
│           │                                        │                │
│ PROJECTS  │  ┌─ Project A ──────────────────────┐  │  INFERENCE     │
│ ────────  │  │ [hex-coder] Writing unit tests    │  │  ──────────   │
│ ● app-a   │  │                                   │  │  ollama:qwen  │
│ ○ app-b   │  │  src/auth/login.test.ts           │  │  ████░░ 67%   │
│ ○ app-c   │  │  +describe('login handler', () => │  │               │
│           │  │  +  it('rejects expired tokens',  │  │  vllm:70b     │
│ AGENTS    │  │  +    ...                         │  │  ██░░░░ 23%   │
│ ────────  │  │                                   │  │               │
│ ● coder-1 │  │  [3/7 tasks] ████████░░░ 43%     │  │  anthropic     │
│ ● planner │  ├───────────────────────────────────┤  │  █░░░░░ 10%   │
│ ○ tester  │  │ [planner] Decomposing auth feat   │  │               │
│           │  │                                   │  │  FLEET         │
│ SWARMS    │  │  Phase: ARCHITECTURE              │  │  ──────────   │
│ ────────  │  │  Tier 0: domain ✓                 │  │  local  ● 3   │
│ auth-feat │  │  Tier 1: adapters (in progress)   │  │  bazzite ● 1  │
│  3 tasks  │  │  Tier 2: usecases (pending)       │  │  cloud  ○ 0   │
│           │  │                                   │  │               │
│           │  └───────────────────────────────────┘  │  TOKENS        │
│           │                                        │  ──────────   │
│           │  ┌─ Project B ──────────────────────┐  │  In:  42.1K   │
│           │  │ [idle] Last: 2m ago               │  │  Out: 18.7K   │
│           │  │ Ready for next task               │  │  Cost: $0.43  │
│           │  └───────────────────────────────────┘  │               │
├───────────┴────────────────────────────────────────┴────────────────┤
│ > Type a message or / for commands...                    [Session 4]│
└─────────────────────────────────────────────────────────────────────┘
```

#### Layout Regions

| Region | Content | Data Source |
|--------|---------|-------------|
| **Left sidebar** | Projects, agents, swarms | SpacetimeDB: `SELECT * FROM projects`, `SELECT * FROM agents`, `SELECT * FROM swarms` (live subscriptions) |
| **Center panes** | Per-project agent activity, live diffs, task progress | SpacetimeDB: `tasks`, `session_messages` tables; WebSocket `/ws/chat` for live LLM streaming |
| **Right sidebar** | Inference load, fleet health, token costs | SpacetimeDB: `inference_endpoints`, `fleet_nodes`, `token_usage` tables (live subscriptions) |
| **Bottom bar** | Chat input, command input, session indicator | WebSocket `/ws/chat` (bidirectional LLM bridge) |
| **Command palette** | Ctrl+P overlay, fuzzy search all actions | Client-side action registry + SpacetimeDB reducer calls for mutations |

#### Pane System

Center panes use a **tiling window manager** model (inspired by tmux/i3):

- **Split horizontal/vertical** — `Ctrl+\` / `Ctrl+-`
- **Focus pane** — `Ctrl+[1-9]` or click
- **Maximize pane** — `Ctrl+Shift+Enter` (toggle)
- **Close pane** — `Ctrl+W`
- **Pane types**: Chat, Diff, TaskBoard, Terminal, FileTree, AgentLog
- **Pane state persisted** in SpacetimeDB KV (survives refresh)

### 3. Views — Six Core Views

#### 3.1 Chat View (default)

OpenCode-style session-centric conversation:

- Sticky message headers (role + model badge + timestamp)
- Auto-scroll during streaming, pause on manual scroll-up
- Inline tool call/result display (expandable)
- File parts rendered with Shiki syntax highlighting
- Diff parts rendered with Pierre
- Per-message model indicator (which inference endpoint generated this)
- Session controls: fork, compact, revert, archive (ADR-036)

```typescript
// SolidJS component sketch — SpacetimeDB provides live state, WS provides LLM stream
const ChatView: Component<{ sessionId: string }> = (props) => {
  const { db } = useSpacetimeDB();  // SpacetimeDB connection context

  // Messages come from SpacetimeDB subscription (auto-synced)
  // SQL: SELECT * FROM session_messages WHERE session_id = ? ORDER BY sequence
  const messages = createMemo(() =>
    db.sessionMessages.filter(m => m.sessionId === props.sessionId)
  );

  // Live LLM streaming comes via hex-nexus WebSocket (bidirectional)
  const { streamingMessage, send } = useChatWebSocket(props.sessionId);

  // When streaming completes, hex-nexus writes to SpacetimeDB via chat-relay reducer
  // → SpacetimeDB pushes the completed message to all subscribed clients
  // → messages() signal updates automatically

  return (
    <div class="chat-view">
      <SessionHeader session={session()} />
      <MessageList messages={messages()} streaming={streamingMessage()} />
      <ChatInput onSend={send} onCommand={handleSlashCommand} />
    </div>
  );
};
```

#### 3.2 Project Overview

Multi-project dashboard showing health at a glance:

- Project cards with: name, path, agent count, active swarm, last activity
- Health badge from `hex analyze` (green/yellow/red)
- Click to expand into split panes
- Drag to rearrange
- "New Project" registers via `POST /api/projects/register`
- File tree per project with change indicators from active agent diffs

#### 3.3 Swarm Monitor

Real-time swarm visualization:

- Task dependency graph (DAG) with live status coloring
- Agent assignments shown on each task node
- Phase indicator (SPECS → PLAN → CODE → VALIDATE → INTEGRATE)
- Token burn rate per agent
- Timeline view of task start/complete events
- Click task → expand to see agent's conversation

#### 3.4 Inference Control Plane

Manage inference fleet:

- Provider cards: Ollama, vLLM, OpenAI, Anthropic — each with health status
- Model list per provider with RPM/TPM meters
- Cost accumulator (per-session, per-project, total)
- "Register Endpoint" form
- Route table: which agents → which models
- Health check results with latency sparklines

#### 3.5 Agent Inspector

Deep-dive into a single agent:

- Live log stream (structured, filterable)
- Current task + progress
- Tool call history (collapsible timeline)
- Memory snapshot (HexFlo scoped memory)
- Resource usage (tokens consumed, files modified)
- Kill / restart / reassign controls

#### 3.6 Fleet View

Remote agent management:

- Node cards: hostname, OS, GPU info, agent count
- SSH connection status
- Deploy hex-agent to new node
- Agent-to-node assignment matrix
- Network topology diagram

### 4. Command System — Three Access Methods

Following OpenCode's proven pattern:

#### 4.1 Slash Commands (in chat input)

```
/project add <path>              Register a new project
/project switch <name>           Focus on project
/agent spawn <type> [project]    Spawn agent for project
/agent kill <id>                 Terminate agent
/swarm init <name> [topology]    Start a swarm
/model switch <model>            Change inference model
/session fork                    Fork current session
/session compact                 Summarize and compact
/analyze                         Run hex analyze on focused project
/diff                            Show current agent's pending changes
/approve                         Approve agent's pending file writes
/reject                          Reject and rollback
```

#### 4.2 Command Palette (Ctrl+P)

Fuzzy-searchable overlay listing ALL available actions:

```typescript
interface CommandEntry {
  id: string;
  label: string;
  category: 'project' | 'agent' | 'swarm' | 'inference' | 'session' | 'view' | 'settings';
  shortcut?: string;
  action: () => void | Promise<void>;
}
```

Categories:
- **Project**: add, remove, switch, analyze, browse files
- **Agent**: spawn, kill, inspect, reassign, list
- **Swarm**: init, teardown, add task, complete task
- **Inference**: register endpoint, health check, switch model, cost report
- **Session**: create, fork, compact, revert, archive, search, export
- **View**: split pane, maximize, close, toggle sidebar, toggle theme
- **Settings**: configure keybindings, set default model, manage secrets

#### 4.3 Keybindings (Leader Key)

Leader key: `Ctrl+X` (avoids conflicts with browser shortcuts)

| Binding | Action |
|---------|--------|
| `Ctrl+X, p` | Command palette |
| `Ctrl+X, n` | New session |
| `Ctrl+X, s` | Switch session |
| `Ctrl+X, 1-9` | Focus pane |
| `Ctrl+X, \|` | Split vertical |
| `Ctrl+X, -` | Split horizontal |
| `Ctrl+X, z` | Maximize pane |
| `Ctrl+X, a` | Spawn agent |
| `Ctrl+X, k` | Kill agent |
| `Ctrl+X, i` | Agent inspector |
| `Ctrl+X, f` | File tree |
| `Ctrl+X, d` | Diff view |
| `Ctrl+X, t` | Toggle theme |

### 5. Technology Stack

| Layer | Technology | Rationale |
|-------|-----------|-----------|
| **UI Framework** | SolidJS 1.x | OpenCode-proven, fine-grained reactivity, small bundle |
| **Build** | Vite 6 | ADR-038 already decided; HMR in dev, static build for prod |
| **Styling** | Tailwind CSS 4 + custom design tokens | Utility-first, theme-able, dark mode native |
| **Code display** | Shiki 1.x | Syntax highlighting (same as OpenCode, VS Code themes) |
| **Diff display** | Pierre | Diff component (same as OpenCode) |
| **Icons** | Lucide | Already used in hex-nexus dashboard |
| **Real-time state** | SpacetimeDB TypeScript SDK | WebSocket subscriptions with SQL filtering, auto client-side cache, typed codegen |
| **Local UI state** | SolidJS signals | Native reactivity for pane layout, command palette, UI preferences |
| **LLM streaming** | WebSocket `/ws/chat` | Bidirectional chat through hex-nexus LLM bridge |
| **Compute API** | Auto-generated from OpenAPI spec (utoipa) | Type-safe REST client for stateless operations (analyze, summarize, spawn) |
| **Terminal** | xterm.js (optional) | Embedded terminal for agent output |
| **Charts** | Lightweight (uPlot or custom SVG) | Sparklines, token burn, inference load |
| **Markdown** | marked + DOMPurify | Chat message rendering (secure) |

### 6. Backend Changes Required

#### 6.1 SpacetimeDB TypeScript Codegen (Critical Path)

Generate TypeScript client bindings for all 17 WASM modules:

```bash
# For each module, generate typed TypeScript bindings
spacetime generate --lang typescript \
  --out-dir hex-chat/ui/src/spacetimedb/ \
  --project-path spacetime-modules/hexflo-coordination

spacetime generate --lang typescript \
  --out-dir hex-chat/ui/src/spacetimedb/ \
  --project-path spacetime-modules/agent-registry

spacetime generate --lang typescript \
  --out-dir hex-chat/ui/src/spacetimedb/ \
  --project-path spacetime-modules/chat-relay

# ... repeat for all modules
```

This produces:
- Typed table definitions (`Swarm`, `Task`, `Agent`, `SessionMessage`, etc.)
- Typed reducer call functions (`completeTask()`, `spawnAgent()`, etc.)
- Callback registration (`onInsert`, `onUpdate`, `onDelete` per table)
- `DbConnection` builder with WebSocket connection management

The generated code replaces the need for a hand-written API client for all state operations.

#### 6.2 Complete Rust SpacetimeDB WebSocket Subscription Wiring

The `.connect()` method in `spacetime_state.rs` currently returns an error because codegen bindings aren't fully linked. Fix this:

```rust
// hex-nexus/src/adapters/spacetime_state.rs — fix connect()
pub async fn connect(&self) -> Result<(), StateError> {
    let conn = DbConnection::builder()
        .with_uri(&self.config.host)
        .with_module_name(&self.config.database)
        .with_token(&self.config.auth_token)
        .on_connect(|conn, _identity, _token| {
            // Subscribe to all tables needed for hex-nexus server-side operations
            conn.subscription_builder()
                .subscribe(["SELECT * FROM agents"])
                .subscribe(["SELECT * FROM swarms"])
                .subscribe(["SELECT * FROM tasks"])
                .subscribe(["SELECT * FROM fleet_nodes"]);
        })
        .on_disconnect(|_conn, err| {
            tracing::warn!("SpacetimeDB disconnected: {:?}", err);
        })
        .build()
        .map_err(|e| StateError::Connection(e.to_string()))?;

    *self.connection.lock().await = Some(conn);
    Ok(())
}
```

This gives hex-nexus server-side real-time state (for heartbeat monitoring, stale agent cleanup, etc.) while the browser independently subscribes via its own TypeScript SDK connection.

#### 6.3 OpenAPI Spec Generation (Stateless Routes Only)

Add `utoipa` to hex-nexus for the REST endpoints that remain (compute operations):

```rust
// hex-nexus/Cargo.toml
[dependencies]
utoipa = { version = "5", features = ["axum_extras"] }
utoipa-swagger-ui = { version = "8", features = ["axum"] }
```

Only stateless/compute routes need OpenAPI annotations:

```rust
#[utoipa::path(
    post, path = "/api/analyze",
    request_body = AnalyzeRequest,
    responses((status = 200, body = AnalyzeResult))
)]
async fn analyze(...) { ... }

#[utoipa::path(
    post, path = "/api/agents/spawn",
    request_body = SpawnRequest,
    responses((status = 200, body = SpawnResult))
)]
async fn spawn_agent(...) { ... }
```

Serve at `GET /api/openapi.json` and `GET /api/docs` (Swagger UI).

**Routes that move to SpacetimeDB reducers (no longer REST):**
- ~~GET /api/agents~~ → SpacetimeDB subscription: `SELECT * FROM agents`
- ~~GET /api/swarms~~ → SpacetimeDB subscription: `SELECT * FROM swarms`
- ~~GET /api/sessions~~ → SpacetimeDB subscription: `SELECT * FROM sessions`
- ~~POST /api/hexflo/memory~~ → SpacetimeDB reducer: `memory_store(key, value, scope)`
- ~~GET /api/inference~~ → SpacetimeDB subscription: `SELECT * FROM inference_endpoints`

**Routes that remain as REST (need filesystem/process access):**
- `POST /api/analyze` — runs `hex analyze` on project directory
- `POST /api/summarize` — runs tree-sitter AST summarization
- `POST /api/scaffold` — generates project files
- `POST /api/agents/spawn` — starts hex-agent subprocess
- `POST /api/agents/kill` — terminates agent process
- `GET /api/projects/files` — file tree listing
- `WebSocket /ws/chat` — bidirectional LLM streaming

#### 6.4 Agent Spawn Endpoint

New REST endpoint to spawn agents from the GUI (this MUST be REST — it starts a process on the host):

```
POST /api/agents/spawn
{
  "project_id": "app-a",
  "agent_type": "hex-coder",
  "model": "qwen3.5:27b",
  "task": "Write unit tests for auth module"
}
```

The spawned agent:
1. Starts as a `hex-agent` subprocess (local) or dispatches to fleet node (remote, ADR-037)
2. Registers itself with SpacetimeDB via `register_agent()` reducer
3. Sends heartbeats via `agent_heartbeat()` reducer every 15s
4. Updates task status via `complete_task()` reducer
5. All state changes automatically propagate to every subscribed browser

#### 6.5 SpacetimeDB Connection Proxy (Optional)

For deployments where SpacetimeDB isn't directly accessible from the browser (corporate firewalls, etc.), hex-nexus can act as a WebSocket proxy:

```
Browser ←WebSocket→ hex-nexus:5556/ws/stdb ←WebSocket→ SpacetimeDB:3000
```

This is opt-in. Default architecture assumes browser connects directly to SpacetimeDB.

### 7. Frontend Directory Structure

```
hex-chat/ui/
├── index.html
├── package.json
├── vite.config.ts
├── tsconfig.json
├── tailwind.config.ts
├── src/
│   ├── main.tsx                    # SolidJS entry
│   ├── App.tsx                     # Root layout + router
│   ├── spacetimedb/               # Auto-generated by `spacetime generate --lang typescript`
│   │   ├── hexflo_coordination/   # Swarm, Task, Agent, Memory tables + reducers
│   │   ├── agent_registry/        # Agent table + heartbeat reducers
│   │   ├── chat_relay/            # SessionMessage table + send_message reducer
│   │   ├── inference_gateway/     # InferenceEndpoint table + register/health reducers
│   │   ├── fleet_state/           # FleetNode table + reducers
│   │   ├── rl_engine/             # RL state tables + reducers
│   │   └── ...                    # Other module bindings
│   ├── api/
│   │   ├── client.ts              # Auto-generated from OpenAPI (stateless routes only)
│   │   └── ws.ts                  # WebSocket chat client (LLM streaming)
│   ├── components/
│   │   ├── layout/
│   │   │   ├── Sidebar.tsx        # Left sidebar (projects, agents, swarms)
│   │   │   ├── RightPanel.tsx     # Right sidebar (inference, fleet, tokens)
│   │   │   ├── PaneManager.tsx    # Tiling pane system
│   │   │   ├── Pane.tsx           # Individual pane wrapper
│   │   │   └── BottomBar.tsx      # Input + session indicator
│   │   ├── chat/
│   │   │   ├── ChatView.tsx       # Session conversation
│   │   │   ├── MessageList.tsx    # Scrollable message list
│   │   │   ├── Message.tsx        # Single message (sticky header)
│   │   │   ├── ChatInput.tsx      # Input with slash command support
│   │   │   ├── ToolCallPart.tsx   # Tool invocation display
│   │   │   └── FilePart.tsx       # Code file display (Shiki)
│   │   ├── project/
│   │   │   ├── ProjectCard.tsx    # Project summary card
│   │   │   ├── ProjectList.tsx    # Multi-project list
│   │   │   ├── FileTree.tsx       # File browser with change indicators
│   │   │   └── HealthBadge.tsx    # Architecture health indicator
│   │   ├── agent/
│   │   │   ├── AgentCard.tsx      # Agent status + controls
│   │   │   ├── AgentInspector.tsx # Deep agent view
│   │   │   ├── AgentLog.tsx       # Structured log stream
│   │   │   └── SpawnDialog.tsx    # GUI agent spawning form
│   │   ├── swarm/
│   │   │   ├── SwarmMonitor.tsx   # Task DAG visualization
│   │   │   ├── TaskNode.tsx       # Individual task in graph
│   │   │   └── PhaseIndicator.tsx # SPARC phase progress
│   │   ├── inference/
│   │   │   ├── ProviderCard.tsx   # Inference endpoint card
│   │   │   ├── ModelSelector.tsx  # Model picker dropdown
│   │   │   ├── CostTracker.tsx    # Token cost accumulator
│   │   │   └── HealthSpark.tsx    # Latency sparkline
│   │   ├── fleet/
│   │   │   ├── NodeCard.tsx       # Remote node card
│   │   │   └── FleetView.tsx      # Fleet topology
│   │   ├── command/
│   │   │   ├── CommandPalette.tsx # Ctrl+P overlay
│   │   │   ├── SlashMenu.tsx      # Slash command autocomplete
│   │   │   └── KeybindManager.tsx # Leader key handling
│   │   └── shared/
│   │       ├── DiffView.tsx       # Pierre wrapper
│   │       ├── CodeBlock.tsx      # Shiki wrapper
│   │       ├── Markdown.tsx       # marked + DOMPurify
│   │       ├── Sparkline.tsx      # SVG sparkline
│   │       └── Badge.tsx          # Status badges
│   ├── stores/
│   │   ├── connection.ts         # SpacetimeDB DbConnection singleton + auth
│   │   ├── projects.ts           # SpacetimeDB table → SolidJS signal bridge
│   │   ├── agents.ts             # SpacetimeDB table → SolidJS signal bridge
│   │   ├── sessions.ts           # SpacetimeDB table → SolidJS signal bridge
│   │   ├── swarms.ts             # SpacetimeDB table → SolidJS signal bridge
│   │   ├── inference.ts          # SpacetimeDB table → SolidJS signal bridge
│   │   ├── panes.ts              # Local pane layout state (SolidJS signals)
│   │   ├── commands.ts           # Command registry (local)
│   │   └── preferences.ts        # User preferences (SpacetimeDB KV or localStorage)
│   ├── hooks/
│   │   ├── useSpacetimeDB.ts     # SpacetimeDB connection + subscription management
│   │   ├── useTable.ts           # Reactive wrapper: SpacetimeDB table → SolidJS signal
│   │   ├── useWebSocket.ts       # WS chat hook (LLM streaming only)
│   │   └── useKeybindings.ts     # Keyboard shortcut hook
│   └── styles/
│       ├── tokens.css            # Design tokens (colors, spacing)
│       └── global.css            # Base styles
```

### 8. Hybrid LLM Bridge Architecture — Queue-Driven Inference

> **Update (ADR-2603300100, Phase 2.2 — 2026-03-30):**
> The `inference-gateway` WASM module now handles **agent inference** (code generation tasks)
> fully inside SpacetimeDB via `#[spacetimedb::procedure]`. The `execute_inference` procedure
> reads the request, calls the provider HTTP API via `ctx.http.send`, and writes the response
> row — no hex-nexus bridge involved for this path. `InferenceRequestProcessor` has been
> removed from hex-nexus.
>
> The analysis below about streaming and chat UX remains valid and applies only to the
> **interactive chat path** (`/ws/chat`). Agent inference and chat are separate paths.

The LLM bridge is the last piece of hex-nexus that resists migration to SpacetimeDB. Today, `/ws/chat` is a bidirectional WebSocket through hex-nexus that routes to inference providers and streams tokens back. This section describes how to make inference **SpacetimeDB-coordinated** while preserving token-by-token streaming UX.

#### Why Not Fully Inside SpacetimeDB? (chat streaming path only)

SpacetimeDB 1.0 procedures support `ctx.http.send()` for outbound HTTP — so a procedure *can*
call Ollama/OpenAI (and does, for agent inference). But for **chat streaming**:

- **No streaming**: Procedures are synchronous. User sees nothing until the entire response completes (5-30s blank screen).
- **Timeout risk**: Large local models (70B on Ollama) can take 30s+.

Token-by-token streaming is non-negotiable for chat UX. So the chat path uses a **hybrid: SpacetimeDB coordinates, external workers stream.**

#### Architecture: Queue Table + Bridge Workers

```
┌──────────────────────────────────────────────────────────────────────┐
│                        Browser (SolidJS)                             │
│                                                                      │
│  1. User sends message                                               │
│     → calls SpacetimeDB reducer: enqueue_inference(session, prompt)   │
│                                                                      │
│  2. Subscribes to stream_chunks table (SpacetimeDB WS)               │
│     → tokens appear as bridge writes them                            │
│                                                                      │
│  3. Subscribes to session_messages table (SpacetimeDB WS)            │
│     → completed message appears when bridge finalizes                │
└───────────┬──────────────────────────────────────┬───────────────────┘
            │ ws:// (SpacetimeDB)                  │
            ▼                                      │
┌───────────────────────────┐                      │
│       SpacetimeDB         │                      │
│                           │                      │
│  inference_queue table:   │                      │
│  ┌─────────────────────┐  │                      │
│  │ id: uuid            │  │                      │
│  │ session_id: string  │  │                      │
│  │ prompt: string      │  │                      │
│  │ model: string       │  │                      │
│  │ provider: string    │  │   (fallback: direct WS
│  │ status: pending     │  │    for streaming UX
│  │ claimed_by: null    │  │    during transition)
│  │ created_at: time    │  │                      │
│  └─────────────────────┘  │                      │
│                           │                      │
│  stream_chunks table:     │                      │
│  ┌─────────────────────┐  │                      │
│  │ request_id: uuid    │  │                      │
│  │ sequence: u32       │  │                      │
│  │ content: string     │  │                      │
│  │ finished: bool      │  │                      │
│  └─────────────────────┘  │                      │
│                           │                      │
│  session_messages table:  │                      │
│  (completed messages)     │                      │
└──────────┬────────────────┘                      │
           │ subscribes via Rust SDK               │
           ▼                                       │
┌─────────────────────────────────────────────────────────────────────┐
│                     hex-llm-bridge (Rust)                            │
│                     (one or more instances)                          │
│                                                                     │
│  Runs on: Mac (Anthropic API), bazzite (Ollama), cloud (vLLM)      │
│                                                                     │
│  Loop:                                                              │
│    1. Subscribe to inference_queue WHERE status = 'pending'         │
│    2. Claim request: call claim_inference(id, worker_id) reducer    │
│    3. Call inference provider (Ollama/OpenAI/vLLM) with streaming   │
│    4. For each token chunk:                                         │
│       → call append_stream_chunk(request_id, seq, content) reducer  │
│       → SpacetimeDB pushes to all subscribed browsers instantly     │
│    5. On completion:                                                │
│       → call finalize_inference(request_id, full_text, usage)       │
│       → writes completed message to session_messages                │
│       → marks inference_queue entry as 'completed'                  │
│       → cleans up stream_chunks for this request                    │
└─────────────────────────────────────────────────────────────────────┘
```

#### SpacetimeDB Module: `inference-bridge`

New WASM module (or extend `chat-relay`) with these tables and reducers:

```rust
// spacetime-modules/inference-bridge/src/lib.rs

#[spacetimedb::table(name = inference_queue, public)]
pub struct InferenceQueue {
    #[primary_key]
    #[auto_inc]
    pub id: u64,
    pub session_id: String,
    pub project_id: String,
    pub prompt: String,           // full prompt or message history JSON
    pub model: String,            // e.g. "qwen3.5:27b", "claude-sonnet-4-20250514"
    pub provider: String,         // e.g. "ollama", "anthropic", "vllm"
    pub preferred_node: Option<String>,  // route to specific fleet node
    pub status: String,           // pending | claimed | streaming | completed | failed
    pub claimed_by: Option<String>,      // worker_id that claimed this
    pub created_at: Timestamp,
    pub claimed_at: Option<Timestamp>,
    pub completed_at: Option<Timestamp>,
}

#[spacetimedb::table(name = stream_chunks, public)]
pub struct StreamChunk {
    #[primary_key]
    #[auto_inc]
    pub id: u64,
    pub request_id: u64,          // FK to inference_queue.id
    pub sequence: u32,            // ordering
    pub content: String,          // token(s) text
    pub finished: bool,           // true on final chunk
}

#[spacetimedb::reducer]
pub fn enqueue_inference(ctx: &ReducerContext, session_id: String, prompt: String, model: String, provider: String) {
    ctx.db.inference_queue().insert(InferenceQueue {
        id: 0, session_id, project_id: /* from context */, prompt, model, provider,
        preferred_node: None, status: "pending".into(), claimed_by: None,
        created_at: Timestamp::now(), claimed_at: None, completed_at: None,
    });
}

#[spacetimedb::reducer]
pub fn claim_inference(ctx: &ReducerContext, request_id: u64, worker_id: String) {
    if let Some(mut req) = ctx.db.inference_queue().id().find(request_id) {
        if req.status == "pending" {
            req.status = "claimed".into();
            req.claimed_by = Some(worker_id);
            req.claimed_at = Some(Timestamp::now());
            ctx.db.inference_queue().id().update(req);
        }
    }
}

#[spacetimedb::reducer]
pub fn append_stream_chunk(ctx: &ReducerContext, request_id: u64, sequence: u32, content: String, finished: bool) {
    ctx.db.stream_chunks().insert(StreamChunk {
        id: 0, request_id, sequence, content, finished,
    });
}

#[spacetimedb::reducer]
pub fn finalize_inference(ctx: &ReducerContext, request_id: u64, full_text: String, input_tokens: u32, output_tokens: u32) {
    // Write completed message to session_messages
    if let Some(req) = ctx.db.inference_queue().id().find(request_id) {
        ctx.db.session_messages().insert(SessionMessage {
            id: 0, session_id: req.session_id, role: "assistant".into(),
            content: full_text, model: req.model, input_tokens, output_tokens,
            sequence: /* next seq */, created_at: Timestamp::now(),
        });
        // Mark request completed
        let mut req = req;
        req.status = "completed".into();
        req.completed_at = Some(Timestamp::now());
        ctx.db.inference_queue().id().update(req);
    }
    // Cleanup stream chunks
    for chunk in ctx.db.stream_chunks().request_id().filter(request_id) {
        ctx.db.stream_chunks().id().delete(chunk.id);
    }
}
```

#### Browser SolidJS Integration

```typescript
// stores/inference.ts
import { createSignal, createMemo } from 'solid-js';
import { useSpacetimeDB } from '../hooks/useSpacetimeDB';

export function useInferenceStream(sessionId: string) {
  const { db, reducers } = useSpacetimeDB();
  const [activeRequestId, setActiveRequestId] = createSignal<number | null>(null);

  // Live stream: collect chunks as they arrive via SpacetimeDB subscription
  const streamedText = createMemo(() => {
    const reqId = activeRequestId();
    if (!reqId) return '';
    return db.streamChunks
      .filter(c => c.requestId === reqId)
      .sort((a, b) => a.sequence - b.sequence)
      .map(c => c.content)
      .join('');
  });

  const isStreaming = createMemo(() => {
    const reqId = activeRequestId();
    if (!reqId) return false;
    const req = db.inferenceQueue.find(r => r.id === reqId);
    return req?.status === 'claimed' || req?.status === 'streaming';
  });

  async function send(prompt: string, model: string, provider: string) {
    // Enqueue via SpacetimeDB reducer — bridge workers pick it up
    reducers.enqueueInference(sessionId, prompt, model, provider);
    // Watch for the new queue entry to track its ID
    db.inferenceQueue.onInsert((ctx, row) => {
      if (row.sessionId === sessionId) setActiveRequestId(row.id);
    });
  }

  return { streamedText, isStreaming, send };
}
```

#### Multi-Machine Inference Routing (Free)

The queue pattern unlocks **distributed inference routing without any custom load balancer**:

```
┌──────────────────┐   ┌──────────────────┐   ┌──────────────────┐
│  hex-llm-bridge  │   │  hex-llm-bridge  │   │  hex-llm-bridge  │
│  (Mac laptop)    │   │  (bazzite GPU)   │   │  (cloud VM)      │
│                  │   │                  │   │                  │
│  Anthropic API   │   │  Ollama local    │   │  vLLM cluster    │
│  claude-sonnet   │   │  qwen3.5:27b    │   │  llama-70b       │
└────────┬─────────┘   └────────┬─────────┘   └────────┬─────────┘
         │                      │                      │
         │   all subscribe to inference_queue           │
         └──────────────┬───────────────────────────────┘
                        │
              ┌─────────▼──────────┐
              │    SpacetimeDB     │
              │                    │
              │  inference_queue:  │
              │  provider=anthropic│ → Mac bridge claims
              │  provider=ollama   │ → bazzite bridge claims
              │  provider=vllm     │ → cloud bridge claims
              │                    │
              │  Unclaimed after   │
              │  30s? Any bridge   │
              │  can pick it up    │ → automatic failover
              └────────────────────┘
```

Each bridge subscribes with a filter: `SELECT * FROM inference_queue WHERE status = 'pending' AND (provider = 'ollama' OR provider = '*')`. First to call `claim_inference()` wins — SpacetimeDB's transactional semantics prevent double-claiming.

Benefits over current hex-nexus single-process routing:
- **Horizontal scaling**: add bridge instances on any machine
- **Provider affinity**: bridge on bazzite claims Ollama requests, Mac claims Anthropic
- **Automatic failover**: unclaimed requests after timeout get picked up by any available bridge
- **Cost routing**: UI can set `provider` based on user preference or cost policy
- **Zero configuration**: bridges self-register by subscribing to the queue

#### Transition Strategy

This is **not a breaking change** — it's additive:

1. **Phase 1-6** (current plan): `/ws/chat` through hex-nexus works as-is for LLM streaming
2. **Phase 7**: Deploy `inference-bridge` SpacetimeDB module, run first `hex-llm-bridge` worker alongside hex-nexus
3. **Phase 8**: Browser switches to SpacetimeDB queue-based inference (feature flag). `/ws/chat` remains as fallback.
4. **Phase 9**: Remove `/ws/chat` from hex-nexus. All inference is queue-driven.
5. **Phase 10**: hex-nexus is now filesystem adapter only. Can be replaced by a `hex-fs-bridge` if desired.

#### Future: SpacetimeDB Streaming Procedures

When SpacetimeDB adds streaming support to procedures (chunked writes within a single invocation), the external bridge becomes optional:

```rust
// Hypothetical — not yet available in SpacetimeDB
#[spacetimedb::procedure(streaming)]
fn inference_stream(ctx: &ProcedureContext, session_id: String, prompt: String, model: String) {
    let stream = ctx.http.fetch_streaming(&endpoint_url, request_body)?;
    for chunk in stream {
        ctx.db.stream_chunks().insert(StreamChunk {
            request_id: req_id, sequence: chunk.seq, content: chunk.text, finished: false,
        });
        // Each insert triggers subscription push to all clients
    }
    ctx.db.session_messages().insert(/* completed message */);
}
```

This would eliminate the last external service entirely. **SpacetimeDB becomes the only runtime** — filesystem access could be handled via a SpacetimeDB procedure calling a minimal `hex-fs` HTTP service, or via future WASI filesystem capabilities.

### 9. Implementation Phases

#### Phase 1: SpacetimeDB Bridge + SolidJS Foundation (Week 1-2)

**SpacetimeDB:**
- [ ] Run `spacetime generate --lang typescript` for all 17 WASM modules
- [ ] Fix `spacetime_state.rs` `.connect()` — complete Rust WebSocket subscription wiring
- [ ] Verify all modules are deployed and tables are queryable
- [ ] Test browser→SpacetimeDB direct WebSocket connection

**Backend:**
- [ ] Add `utoipa` to hex-nexus, annotate stateless routes for OpenAPI generation
- [ ] Add `POST /api/agents/spawn` endpoint
- [ ] Migrate state-read routes to "deprecated" (still functional, browser will use SpacetimeDB instead)

**Frontend:**
- [ ] Scaffold `hex-chat/ui/` with Vite + SolidJS + Tailwind (ADR-038)
- [ ] Implement `useSpacetimeDB` hook — connection management, auth, reconnection
- [ ] Implement `useTable` hook — bridge SpacetimeDB `onInsert`/`onUpdate`/`onDelete` → SolidJS signals
- [ ] Build basic 3-column layout shell (Sidebar, Center, RightPanel)
- [ ] Port chat functionality: SpacetimeDB for history, WebSocket `/ws/chat` for live LLM streaming

**Milestone:** Chat works in SolidJS with SpacetimeDB-backed message history and live subscriptions.

#### Phase 2: Multi-Project Panes (Week 3-4)

**Frontend:**
- [ ] Implement `PaneManager` (tiling window manager)
- [ ] Build `ProjectCard` and `ProjectList` with live health badges (SpacetimeDB `projects` table subscription)
- [ ] Implement `FileTree` with change indicators from agent diffs
- [ ] Per-project pane assignment (click project → opens pane)
- [ ] Pane state persistence via SpacetimeDB KV (`memory_store` reducer)

**SpacetimeDB:**
- [ ] Add `project_health` table/reducer for architecture analysis results
- [ ] Add `file_change` table for tracking agent file modifications

**Milestone:** Multiple projects visible simultaneously in split panes, all state live via SpacetimeDB.

#### Phase 3: Agent Control (Week 5-6)

**Frontend:**
- [ ] Build `SpawnDialog` — calls hex-nexus `POST /api/agents/spawn`, agent self-registers in SpacetimeDB
- [ ] Build `AgentInspector` with live state from SpacetimeDB `agents` table subscription
- [ ] Build `AgentCard` with status, controls (kill via REST, state via SpacetimeDB)
- [ ] Agent heartbeat visualization — SpacetimeDB `agent_heartbeats` table with `onUpdate` callbacks

**Backend:**
- [ ] Implement agent subprocess spawning from REST endpoint
- [ ] Agent log forwarding: agent writes structured logs to SpacetimeDB `agent_logs` table
- [ ] Add agent task reassignment reducer in SpacetimeDB

**Milestone:** Spawn agents from GUI, monitor via SpacetimeDB live subscriptions, kill via REST.

#### Phase 4: Swarm Visualization (Week 7-8)

**Frontend:**
- [ ] Build `SwarmMonitor` task DAG with SVG/Canvas rendering
- [ ] Live task status coloring (pending → in_progress → completed)
- [ ] Phase indicator (SPECS → PLAN → CODE → VALIDATE → INTEGRATE)
- [ ] Click-through from task node to agent conversation

**SpacetimeDB:**
- [ ] Add task dependency edges to `hexflo-coordination` module
- [ ] Add phase transition tracking (reducer writes phase changes, subscription delivers to UI)

**Milestone:** Visual swarm progress tracking with task graph — all state from SpacetimeDB subscriptions.

#### Phase 5: Command System + Polish (Week 9-10)

**Frontend:**
- [ ] Build `CommandPalette` (Ctrl+P) with fuzzy search
- [ ] Implement `SlashMenu` autocomplete in chat input
- [ ] Implement `KeybindManager` with leader key system
- [ ] Integrate Shiki for code syntax highlighting
- [ ] Integrate Pierre for diff display
- [ ] Dark/light theme toggle
- [ ] Responsive layout (collapse sidebars on narrow screens)

**Milestone:** Power-user command system, production-quality code display.

#### Phase 6: Fleet + Inference Control (Week 11-12)

**Frontend:**
- [ ] Build `FleetView` with node cards and topology
- [ ] Build inference `ProviderCard` with health sparklines
- [ ] Build `CostTracker` with per-project/per-session breakdown
- [ ] Model route table editor (which agents use which models)
- [ ] Deploy agent to remote node from GUI

**SpacetimeDB:**
- [ ] Inference health check results written to `inference_endpoints` table (subscription delivers to UI)
- [ ] Fleet node discovery via `fleet_state` module subscriptions
- [ ] Cost aggregation via SpacetimeDB SQL queries on `token_usage` table

**Milestone:** Full inference and fleet visibility from the GUI — zero polling, all SpacetimeDB subscriptions.

### 10. Migration Strategy

The current vanilla JS dashboard (`hex-nexus/assets/`) is NOT deleted immediately:

1. **Phase 1-2**: New SolidJS app runs on Vite dev server (port 5173), old dashboard on 5556
2. **Phase 3**: Feature parity achieved — new app handles chat + projects + agents
3. **Phase 4**: Old dashboard deprecated, new app served by Axum on 5556
4. **Phase 5-6**: Old `hex-nexus/assets/*.js` files removed, replaced by `hex-chat/ui/dist/`
5. **Final**: `rust-embed` bakes `dist/` into hex-nexus binary for single-binary deployment

### 11. Security Considerations

- **XSS prevention**: All user/agent content rendered via `textContent` or DOMPurify — never raw `innerHTML` (reinforces existing CLAUDE.md rule)
- **SpacetimeDB auth**: Browser authenticates with SpacetimeDB token (stored in localStorage, rotated on session start)
- **CSRF**: WebSocket connections require session token
- **Agent spawn authorization**: GUI spawn requires valid session with project access
- **Inference credentials**: Never exposed to frontend — proxied through hex-nexus
- **Fleet SSH keys**: Managed server-side, never sent to browser
- **Content Security Policy**: Strict CSP headers from Axum

### 12. Performance Targets

| Metric | Target | Rationale |
|--------|--------|-----------|
| Initial load (gzipped) | < 150KB | SolidJS (~7KB) + Tailwind (purged) + Shiki (lazy) |
| SpacetimeDB delta latency | < 50ms | Direct WebSocket, BSATN binary protocol, no intermediate proxy |
| Pane switch | < 16ms | SolidJS fine-grained updates, no re-render cascade |
| Chat scroll (1000 messages) | 60 FPS | Virtual scroll list, message recycling |
| Dashboard with 10 projects | < 100ms render | Signal-based updates, no prop drilling |
| Syntax highlighting | < 200ms per block | Shiki with WASM grammar loading |

## Consequences

### Positive

- **Closes the vision gap**: developers get the tmux-for-AI-agents experience
- **SpacetimeDB-native real-time**: no custom event bus, no SSE, no polling — row-level deltas push automatically to every connected client
- **Massive backend simplification**: hex-nexus drops ~15 state-read REST routes, becomes a thin compute layer
- **Cross-machine sync for free**: agent on bazzite completes task → every browser sees it instantly via SpacetimeDB
- **SolidJS + Vite**: modern, fast iteration, small bundle, proven by OpenCode at scale
- **Typed end-to-end**: SpacetimeDB codegen produces TypeScript types from Rust module definitions — single source of truth
- **Incremental migration**: old dashboard stays until parity, zero disruption
- **Simpler transport**: two channels (SpacetimeDB WS + chat WS) instead of three (REST + SSE + WS)

### Negative

- **Frontend rewrite**: 2000+ lines of vanilla JS replaced, ~12 weeks of work
- **SpacetimeDB as critical dependency**: browser connects directly — if SpacetimeDB is down, UI has no state (mitigated by client-side cache persistence)
- **New dependency surface**: SolidJS, Tailwind, Shiki, Pierre, Vite, SpacetimeDB TypeScript SDK
- **Codegen maintenance**: must re-run `spacetime generate` when WASM module schemas change
- **Bundle size growth**: Shiki grammars and themes add weight (mitigated by lazy loading)
- **SpacetimeDB SDK maturity**: TypeScript SDK is newer than the Rust SDK — may hit edge cases

### Risks

| Risk | Mitigation |
|------|-----------|
| SolidJS ecosystem smaller than React | OpenCode validates it at 93K-star scale; SolidJS 2.0 on the horizon |
| Feature creep during 12-week build | Strict phase gates — each phase has a clear milestone and no scope leakage |
| Pane manager complexity | Start with simple horizontal/vertical splits, defer floating/tabbed panes |
| SpacetimeDB offline / unavailable | Client-side cache survives brief disconnects; SQLite fallback in hex-nexus for degraded mode |
| SpacetimeDB WebSocket blocked by firewall | Optional proxy mode: hex-nexus proxies WS at `/ws/stdb` (Section 6.5) |
| Mobile/responsive layout | Defer — desktop-first for v1, responsive in v2 |
| Codegen drift (TS types out of sync with modules) | CI check: `spacetime generate` output must match committed bindings |

## Alternatives Considered

### 1. Enhance Vanilla JS Dashboard

**Rejected.** The current 2000-line unstructured JS cannot support pane management, command palette, or fine-grained reactivity without becoming unmaintainable.

### 2. React Instead of SolidJS

**Rejected.** Larger bundle, virtual DOM overhead during streaming updates, and OpenCode already validated SolidJS for exactly this use case. React's scheduler can cause visible jank during high-frequency SSE events.

### 3. Svelte / Vue

**Considered but rejected.** Both are viable, but SolidJS's signal model maps most naturally to SSE event streams, and OpenCode's shared component library demonstrates the pattern we want to follow.

### 4. Electron Desktop App

**Deferred.** Web-first via Axum embedding. Desktop (Tauri) can come later using the same SolidJS components, following OpenCode's multi-surface pattern.

### 5. Keep TUI as Primary Interface

**Rejected for this goal.** TUI (hex-chat tui) remains for terminal users, but the multi-project multiplexer with panes, diffs, and graphs needs a browser canvas. Both share the same SpacetimeDB subscriptions (Rust SDK for TUI, TypeScript SDK for browser).

### 6. Custom EventBus + SSE Instead of SpacetimeDB Direct

**Rejected.** Early draft of this ADR proposed building a Rust `EventBus` with `tokio::broadcast` and an SSE endpoint (`GET /api/events`). This would have hex-nexus act as a middleman: read SpacetimeDB → broadcast via SSE → browser subscribes.

This was rejected because:
- It re-implements what SpacetimeDB provides natively (real-time subscriptions, fan-out, client-side cache)
- Adds latency: mutation → SpacetimeDB → hex-nexus EventBus → SSE → browser, vs mutation → SpacetimeDB → browser
- Adds operational complexity: three transports (REST + SSE + WS) instead of two (SpacetimeDB WS + chat WS)
- Requires hand-coding event types that SpacetimeDB codegen produces automatically
- Doesn't support cross-machine fan-out (SSE only reaches clients connected to that hex-nexus instance; SpacetimeDB reaches all clients everywhere)

### 7. SpacetimeDB 2.0 Procedures for Server-Side Logic

**Adopted (strategic).** SpacetimeDB 2.0 introduces scheduled and triggered procedures that run inside the database itself — no external process needed. This is the key to the next major simplification of hex-nexus.

#### The Architectural Endgame

Right now hex-nexus runs background tasks in Rust: "check agent heartbeats every 45s, mark stale," "reclaim tasks from dead agents," "decay RL weights periodically." These are coordination concerns that currently require hex-nexus to be running, polling state, and writing back.

With 2.0 scheduled procedures, **that logic lives inside SpacetimeDB itself.** It runs on every transaction or on a cron-like schedule, with no external process needed. The state mutation triggers the subscription fan-out to all connected clients automatically.

This means hex-nexus could eventually shrink to **just one responsibility**:

1. **Filesystem adapter** — read/write files, run `hex analyze`, execute tree-sitter, spawn agent processes (things that need OS access)

Even the LLM bridge can be extracted into a queue-driven `hex-llm-bridge` microservice coordinated through SpacetimeDB (see Section 8: Hybrid LLM Bridge Architecture). Everything else — coordination, lifecycle, scheduling, state management, real-time sync, and inference routing — lives in SpacetimeDB.

```
BEFORE (current):
  hex-nexus = state proxy + compute + coordination + lifecycle + scheduling + LLM bridge
  SpacetimeDB = dumb persistence

PHASE 1 (this ADR, 2.0 procedures):
  hex-nexus = filesystem adapter + LLM bridge (thin compute shell)
  SpacetimeDB = state + coordination + lifecycle + scheduling + real-time sync

PHASE 2 (hybrid LLM bridge):
  hex-nexus = filesystem adapter only (~500-line Rust binary)
  hex-llm-bridge = stateless streaming workers (N instances across machines)
  SpacetimeDB = state + coordination + lifecycle + scheduling + sync + inference queue
```

#### Migration Candidates

| Current hex-nexus Logic | Location | SpacetimeDB 2.0 Replacement | Type |
|------------------------|----------|----------------------------|------|
| Agent stale/dead detection | `coordination/cleanup.rs` | Scheduled procedure: every 15s, check `last_heartbeat < now() - 45s`, update agent status to `stale`; `< now() - 120s` → `dead` | Scheduled |
| Task reclamation from dead agents | `coordination/cleanup.rs` | Triggered procedure: on agent status change to `dead`, set all assigned tasks back to `pending`, clear `assigned_agent_id` | Triggered |
| Inference health check scheduling | `routes/inference.rs` | Scheduled procedure: every 60s, ping registered endpoints, update `inference_endpoints.healthy` and `latency_ms` | Scheduled |
| Session compaction | `routes/sessions.rs` | Procedure: count messages in session, if > threshold, summarize via LLM call*, archive old messages, insert summary | Callable |
| RL reward decay | `spacetime_state.rs` | Scheduled procedure: daily `decay_all` — multiply all pattern weights by decay factor (0.95) | Scheduled |
| Cost aggregation | `routes/sessions.rs` | SQL materialized view or scheduled procedure: aggregate `token_usage` rows per project/session into `cost_summary` table | Scheduled |
| Swarm phase transitions | `coordination/mod.rs` | Triggered procedure: on all tasks in current tier completed, advance swarm phase, create next tier's tasks | Triggered |
| Worktree stale detection | future | Scheduled procedure: check worktree `last_commit_at`, flag if > 24h with no activity | Scheduled |

*Note: Session compaction requires an LLM call for summarization. This can be implemented as a SpacetimeDB procedure that calls hex-nexus's LLM bridge endpoint, or remains in hex-nexus with the trigger coming from SpacetimeDB.

#### Implementation Strategy

This migration is **not part of the 12-week Phase 1-6 plan** — it's a follow-on optimization. The control plane UI works with or without 2.0 procedures (hex-nexus handles the logic in Rust initially, SpacetimeDB provides the state and sync).

Recommended sequence:
1. **Phase 1-6**: Build the control plane UI with SpacetimeDB subscriptions + hex-nexus compute (this ADR)
2. **Phase 7** (post-launch): Migrate heartbeat/cleanup to SpacetimeDB scheduled procedures — delete `coordination/cleanup.rs`
3. **Phase 8**: Migrate task reclamation and phase transitions to triggered procedures
4. **Phase 9**: Migrate RL decay, cost aggregation, worktree detection
5. **Phase 10**: Evaluate remaining hex-nexus routes — if only filesystem + LLM bridge remain, hex-nexus becomes a ~500-line Rust binary

Each migration step:
- Implement the procedure in the SpacetimeDB WASM module
- Verify via subscription that the UI sees the same state changes
- Remove the corresponding Rust code from hex-nexus
- Run `spacetime generate` to update TypeScript bindings if table schema changed

#### Why This Matters for Multi-Project

The procedure model scales naturally across projects. A single SpacetimeDB instance handles heartbeat monitoring for agents across ALL registered projects — no per-project hex-nexus instances needed. One database, one set of procedures, N projects, M agents, all clients synced in real-time.

This is fundamentally different from the current model where each hex-nexus instance polls its own state. With 2.0 procedures, **coordination is truly centralized and autonomous** — it happens even if no hex-nexus instance is running, as long as SpacetimeDB is up.

## References

- [OpenCode](https://github.com/opencode-ai/opencode) — 93K-star AI coding tool, UX architecture reference
- [SpacetimeDB](https://spacetimedb.com/) — real-time database with WebSocket subscriptions
- [SpacetimeDB TypeScript SDK](https://spacetimedb.com/docs/sdks/typescript/quickstart/) — browser client
- [SpacetimeDB Subscription Semantics](https://spacetimedb.com/docs/subscriptions/semantics/) — SQL subscriptions, row-level deltas
- [SpacetimeDB Codegen](https://spacetimedb.com/docs/sdks/codegen/) — typed client generation from WASM modules
- [SpacetimeDB Procedures](https://spacetimedb.com/docs/procedures/) — 2.0 procedures with `ctx.http.fetch()` for outbound HTTP
- [SolidJS](https://www.solidjs.com/) — reactive UI framework
- [Pierre](https://github.com/nicolo-ribaudo/pierre) — diff component
- [Shiki](https://shiki.matsu.io/) — syntax highlighter
- [utoipa](https://github.com/juhaku/utoipa) — Rust OpenAPI generation
- ADR-025: IStatePort + SpacetimeDB backend
- ADR-027: HexFlo coordination
- ADR-036: Session architecture
- ADR-037: Agent lifecycle
- ADR-038: Vite/Axum split
