# hex — AI-Assisted Integrated Development Environment (AAIDE)

## What This Project Is

hex is an **AAIDE** (AI-Assisted Integrated Development Environment) — an opinionated development framework built around **hexagonal architecture** (Ports & Adapters). It is not an application you deploy; it is the framework + CLI toolchain that gets **installed into target projects** to enforce architecture and coordinate AI-driven development.

**Critical**: Everything in this repo (settings, hooks, statuslines, agents, skills) exists to be instantiated INTO a target project. The `examples/` directory contains sample target projects that use hex as an installed dependency. When working on examples, you are testing hex as a consumer would use it — the example IS the project, hex is the tool.

hex provides token-efficient code summaries via tree-sitter, swarm coordination via HexFlo (native Rust, ADR-027), a specs-first development pipeline, and a control plane dashboard for multi-project management.

## System Components

hex is composed of five deployment units. Understanding these is essential for working on the codebase:

### SpacetimeDB — Coordination & State Core (REQUIRED)

**SpacetimeDB must always be running to use hex.** It is the backbone — all clients (web, CLI, desktop) connect via WebSocket for real-time state synchronization.

- **18 WASM modules** in `spacetime-modules/` provide transactional reducers for swarm coordination, agent lifecycle, inference routing, chat relay, and more
- Replaces polling with instant WebSocket subscriptions — when one agent completes a task, all clients see it immediately
- **Critical limitation**: WASM modules cannot access filesystems, spawn processes, or make network calls — this is why hex-nexus exists

### hex-nexus — Filesystem Bridge Daemon (`hex-nexus/`)

hex-nexus bridges the gap between SpacetimeDB (sandboxed WASM) and the local operating system:

- **Reads/writes files** on behalf of SpacetimeDB operations
- **Runs architecture analysis** (tree-sitter, boundary checking, cycle detection)
- **Manages git** (blame, diff, worktree management)
- **Syncs config** from repo files → SpacetimeDB tables on startup (ADR-044)
- **Serves the dashboard** frontend (assets baked in via `rust-embed`)
- **Exposes REST API** that CLI and MCP tools delegate to
- Editing `hex-nexus/assets/` requires rebuilding: `cd hex-nexus && cargo build --release`
- State fallback: SQLite (`~/.hex/hub.db`) when SpacetimeDB unavailable (ADR-025)

### hex-agent — Architecture Enforcement Runtime (`hex-agent/`)

hex-agent **must always be present** (locally or remotely) on any system running hex development agents. It is the runtime environment for hex's AI agents, enforcing hexagonal architecture through:

- **Skills**: Slash commands that guide AI agents to produce compliant code
- **Hooks**: Pre/post operation hooks for boundary validation, formatting, pattern training
- **ADRs**: Architecture Decision Records documenting design choices
- **Workplans**: Structured task decomposition into adapter-bounded steps
- **HexFlo dispatchers**: Native Rust coordination for multi-agent swarm execution
- **Agent definitions**: YAML-defined roles (planner, coder, reviewer) with specific boundaries

### hex-dashboard — Developer Control Plane (`hex-nexus/assets/`)

The dashboard is the **nexus of data and control** for developers using hex across many projects and systems:

- **Multi-project management** with live freshness indicators
- **Agent fleet control** — status, heartbeats, task assignments across systems
- **Architecture health** — real-time score ring with violation breakdown
- **Command dispatch** — send commands to any connected project from the browser
- **Inference monitoring** — track model requests and token consumption
- Tech stack: Solid.js + TailwindCSS, real-time via SpacetimeDB WebSocket subscriptions
- Served at `http://localhost:5555` by hex-nexus

### Inference — Model Integration

hex interfaces with external inference through SpacetimeDB procedures and reducers. It can leverage **local models, free models, or frontier models**:

- `inference-gateway` WASM module routes requests
- `inference-bridge` WASM module handles model integration
- hex-nexus performs actual HTTP calls (WASM can't make network requests)
- Model-agnostic — works with any LLM provider (Anthropic, OpenAI, Ollama, etc.)

## Behavioral Rules

- **Workplans are autonomous**: When executing a workplan, complete ALL phases without asking. Do not pause between phases to ask "want me to continue?" — just keep going until done. Use HexFlo swarm tracking and background agents to parallelize where possible.
- Do what has been asked; nothing more, nothing less
- ALWAYS read a file before editing it
- NEVER save files to the root folder — use the directories below
- NEVER commit secrets, credentials, or .env files
- ALWAYS run `bun test` after making code changes
- ALWAYS run `bun run build` before committing
- NEVER use `mock.module()` in tests — use dependency injection via the Deps pattern instead (ADR-014)

## Hexagonal Architecture Rules (ENFORCED)

These rules are checked by `hex analyze .` and the dead-code-analyzer agent:

1. **domain/** must only import from **domain/** (value-objects, entities)
2. **ports/** may import from **domain/** (for value types) but nothing else
3. **usecases/** may import from **domain/** and **ports/** only
4. **adapters/primary/** may import from **ports/** only
5. **adapters/secondary/** may import from **ports/** only
6. **adapters must NEVER import other adapters** (cross-adapter coupling)
7. **composition-root.ts** is the ONLY file that imports from adapters — this is by design
8. All relative imports MUST use `.js` extensions (NodeNext module resolution)

## File Organization

```
# ── Rust Workspace (6 crates) ──────────────────────────────────────────────
hex-cli/                 # CLI binary — canonical user entry point (all hex commands)
hex-nexus/               # Filesystem bridge daemon + dashboard (axum, port 5555)
  src/
    analysis/            #   Architecture analysis (tree-sitter, boundary checking)
    coordination/        #   HexFlo swarm coordination (ADR-027)
    adapters/            #   SpacetimeDB + SQLite state adapters
    config_sync.rs       #   Repo → SpacetimeDB config sync on startup (ADR-044)
    git/                 #   Git introspection (blame, diff, worktree mgmt)
    orchestration/       #   Agent manager, constraint enforcer, workplan executor
  assets/                #   Dashboard frontend (Solid.js, baked in via rust-embed)
    src/spacetimedb/     #     Auto-generated SpacetimeDB client bindings
hex-core/                # Shared domain types & port traits (zero external deps)
hex-agent/               # Architecture enforcement runtime (agent runtime for AI dev agents)
hex-desktop/             # Desktop app (Tauri wrapper for dashboard)
hex-parser/              # Code parsing utilities

# ── SpacetimeDB WASM Modules ──────────────────────────────────────────────
spacetime-modules/       # 18 WASM modules (wasm32-unknown-unknown)
  hexflo-coordination/   #   Core: swarms, tasks, agents, memory, projects, config
  agent-registry/        #   Agent lifecycle + heartbeats
  inference-gateway/     #   LLM request routing (model-agnostic)
  workplan-state/        #   Task status + phase tracking
  chat-relay/            #   Message routing
  fleet-state/           #   Compute node registry
  architecture-enforcer/ #   Server-side boundary validation
  # ... + 11 more modules

# ── TypeScript Library ─────────────────────────────────────────────────────
src/
  core/
    domain/              # Pure business logic, zero external deps
      value-objects.ts   #   Shared types (Language, ASTSummary, etc.)
      entities.ts        #   Domain events, QualityScore, FeedbackLoop, TaskGraph
    ports/               # Typed interfaces — contracts between layers (31 files)
    usecases/            # Application logic composing ports
  adapters/
    primary/             # Driving adapters (CLI, MCP, dashboard, notifications)
    secondary/           # Driven adapters (FS, Git, LLM, tree-sitter, HexFlo, secrets)
  infrastructure/        # Cross-cutting (tree-sitter queries)
  composition-root.ts    # Wires adapters → ports (single DI point)
  cli.ts                 # CLI entry point
  index.ts               # Library public API

# ── Supporting ─────────────────────────────────────────────────────────────
tests/
  unit/                  # London-school mock-first tests
  integration/           # Real adapter tests
examples/                # Reference apps (flappy-bird, weather, rust-api, todo-app, etc.)
agents/                  # Agent definitions (14 YAML files, shipped in npm package)
skills/                  # Skill definitions (6 Markdown files, shipped in npm package)
.claude/
  skills/                # IDE skills (.md) — /hex-scaffold, /hex-generate, etc.
  agents/                # IDE agent definitions
docs/
  adrs/                  # 37 Architecture Decision Records
  specs/                 # Behavioral specifications
  workplans/             # Feature workplans
  analysis/              # Adversarial review reports
config/                  # Language configs, tree-sitter settings
scripts/                 # Build and setup scripts
```

## Build & Test

```bash
# Rust CLI (the primary CLI — what users run)
cargo build -p hex-cli --release    # Build hex CLI binary
cargo build -p hex-nexus --release  # Build hex-nexus daemon

# TypeScript library (secondary — ports, adapters, tree-sitter)
bun run build        # Bundle TS library to dist/
bun test             # Run all tests (unit + property + smoke)
bun run check        # TypeScript type check (no emit)

# hex CLI commands (Rust binary)
hex analyze .        # Architecture health check
hex nexus start      # Start the nexus daemon
hex nexus status     # Check daemon health
hex adr list         # List all ADRs with status
hex adr status <id>  # Show ADR detail
hex adr search <q>   # Search ADRs by keyword
hex adr abandoned    # Detect stale/abandoned ADRs
hex swarm init       # Initialize a swarm
hex task list        # List tasks
hex memory store     # Store key-value
hex status           # Project status overview
```

### hex-cli (Rust CLI — the canonical CLI)

**hex-cli** (`hex-cli/`) is the CLI binary users run. ALL hex commands go through this binary. The MCP server (`hex mcp`) is also served from this binary, ensuring MCP tools and CLI commands share the same backend.

**IMPORTANT**: Never recommend commands that don't exist in `hex --help`. If a command isn't in the Rust CLI, it doesn't exist.

### hex-nexus (Filesystem Bridge — Library + Binary)

**hex-nexus** (`hex-nexus/`) is the daemon that bridges SpacetimeDB (sandboxed WASM) with the local OS. It provides REST API endpoints for filesystem ops, architecture analysis, swarm coordination, and fleet management. It uses `rust-embed` to bake `hex-nexus/assets/*` (HTML, CSS, JS) into the binary at compile time.

- **SpacetimeDB is required** — hex-nexus connects to it for state sync, coordination, and real-time subscriptions (ADR-025)
- **Editing `hex-nexus/assets/index.html`** (or any asset) requires rebuilding the Rust binary:
  ```bash
  cd hex-nexus && cargo build --release
  ```
- Then restart the nexus daemon and hard-refresh the browser (Cmd+Shift+R)
- **Primary state**: SpacetimeDB (real-time sync via WebSocket)
- **Fallback state**: SQLite (`~/.hex/hub.db`) for offline/single-node operation
- Multi-instance coordination uses `ICoordinationPort` with filesystem-based locking and heartbeats (ADR-011)
- HexFlo coordination module provides native swarm orchestration (ADR-027)
- Config sync on startup pushes repo files → SpacetimeDB tables (ADR-044)

## Development Pipeline (Specs-First)

When building new features or example applications, follow this order:

1. **Decide** — If the change involves new ports, adapters, or external dependencies, write an ADR in `docs/adrs/`
2. **Specify** — Write behavioral specs BEFORE code (what "correct" looks like)
3. **Build** — Generate code following hex architecture rules
4. **Test** — Unit tests + property tests + smoke tests (3 levels)
5. **Validate** — Run `hex analyze` + validation judge
6. **Ship** — README + start scripts + commit

## Feature Development Workflow

In hex architecture, a "feature" is NOT a vertical slice. It decomposes inside-out across layers, with each adapter boundary getting its own git worktree for isolation.

### How to Start a Feature

Use `/hex-feature-dev` or run the shell script directly:

```bash
# Interactive (via Claude Code skill)
/hex-feature-dev

# Shell script for worktree lifecycle
./scripts/feature-workflow.sh setup <feature-name>     # Create worktrees from workplan
./scripts/feature-workflow.sh status <feature-name>     # Show progress
./scripts/feature-workflow.sh merge <feature-name>      # Merge in dependency order
./scripts/feature-workflow.sh cleanup <feature-name>    # Remove worktrees + branches
./scripts/feature-workflow.sh list                      # List all feature worktrees
./scripts/feature-workflow.sh stale                     # Find abandoned worktrees
```

### Feature Lifecycle (7 Phases)

```
Phase 1: SPECS       behavioral-spec-writer → docs/specs/<feature>.json
Phase 2: PLAN        planner → docs/workplans/feat-<feature>.json
Phase 3: WORKTREES   feature-workflow.sh setup → one worktree per adapter
Phase 4: CODE        hex-coder agents (parallel, TDD) in isolated worktrees
Phase 5: VALIDATE    validation-judge → PASS/FAIL verdict (BLOCKING)
Phase 6: INTEGRATE   merge worktrees in dependency order → run full suite
Phase 7: FINALIZE    cleanup worktrees, commit, report
```

### Worktree Conventions

- **Naming**: `feat/<feature-name>/<layer-or-adapter>`
- **Max concurrent**: 8 worktrees
- **Merge order**: domain → ports → secondary adapters → primary adapters → usecases → integration
- **Cleanup**: Always remove worktrees after successful merge
- **Stale detection**: Worktrees older than 24h with no commits are flagged

### Dependency Tiers (What Runs When)

| Tier | Layer | Depends On | Agent |
|------|-------|------------|-------|
| 0 | Domain + Ports | Nothing | hex-coder |
| 1 | Secondary adapters | Tier 0 | hex-coder |
| 2 | Primary adapters | Tier 0 | hex-coder |
| 3 | Use cases | Tiers 0-2 | hex-coder |
| 4 | Composition root | Tiers 0-3 | hex-coder |
| 5 | Integration tests | Everything | integrator |

### Development Modes

| Mode | When to Use |
|------|------------|
| **Swarm** (default) | Features spanning 2+ adapters — parallel worktrees |
| **Interactive** | Critical features needing human review at each phase |
| **Single-agent** | Small changes within one adapter boundary |

## Available Skills (Claude Code slash commands)

| Skill | Trigger |
|-------|---------|
| `/hex-feature-dev` | Start feature development with hex decomposition |
| `/hex-scaffold` | Scaffold a new hex project |
| `/hex-generate` | Generate code within an adapter boundary |
| `/hex-summarize` | Token-efficient AST summaries (L0-L3) |
| `/hex-analyze-deps` | Dependency analysis + tech stack recommendation |
| `/hex-analyze-arch` | Architecture health check |
| `/hex-validate` | Post-build semantic validation |

## Available Agents

| Agent | Role |
|-------|------|
| `feature-developer` | Orchestrates full feature lifecycle (specs → code → validate → merge) |
| `planner` | Decomposes requirements into adapter-bounded tasks |
| `hex-coder` | Codes within one adapter with TDD loop |
| `integrator` | Merges worktrees, integration tests |
| `swarm-coordinator` | Orchestrates full lifecycle via HexFlo |
| `dependency-analyst` | Recommends tech stack + runtime requirements |
| `dead-code-analyzer` | Finds dead exports + hex boundary violations |
| `scaffold-validator` | Ensures projects are runnable (README, scripts, dev server) |
| `behavioral-spec-writer` | Writes acceptance specs before code generation |
| `validation-judge` | Post-build semantic validation (BLOCKING gate) |
| `status-monitor` | Swarm progress monitoring |

## Key Lessons (from adversarial review)

- **Tests can mirror bugs**: When the same LLM writes code AND tests, tests may encode the LLM's misunderstanding. Use property tests and behavioral specs as independent oracles.
- **Sign conventions matter**: For physics/math domains, document coordinate systems explicitly. `flapStrength` must be NEGATIVE (upward force in screen coords).
- **"It compiles" ≠ "it works"**: Always include runtime validation — can a user actually start the app?
- **Browser TypeScript needs a dev server**: Any project with HTML + TypeScript MUST include Vite (or equivalent).

## Swarm Coordination (HexFlo — ADR-027)

HexFlo is the native Rust coordination layer built into hex-nexus. It replaces ruflo with zero external dependencies. State is persisted in SpacetimeDB via the `hexflo-coordination` WASM module, with SQLite fallback for offline use.

### Architecture

```
hex-nexus/src/coordination/
  mod.rs           # HexFlo struct — unified API for swarm/task/agent ops
  memory.rs        # Key-value persistent memory (scoped: global, per-swarm, per-agent)
  cleanup.rs       # Heartbeat timeout + dead agent task reclamation

spacetime-modules/hexflo-coordination/
  src/lib.rs       # SpacetimeDB tables: swarm, swarm_task, swarm_agent, hexflo_memory
                   # Reducers: swarm_init, task_create, task_assign, task_complete,
                   #           agent_register, agent_heartbeat, memory_store
```

### API Surface

| Operation | HexFlo API | REST Endpoint |
|-----------|-----------|---------------|
| Init swarm | `HexFlo::swarm_init(name, topology)` | `POST /api/swarms` |
| Swarm status | `HexFlo::swarm_status()` | `GET /api/swarms` |
| Create task | `HexFlo::task_create(swarm_id, title)` | `POST /api/swarms/:id/tasks` |
| Complete task | `HexFlo::task_complete(id, result)` | `PATCH /api/swarms/tasks/:id` |
| Store memory | `HexFlo::memory_store(key, value, scope)` | `POST /api/hexflo/memory` |
| Retrieve memory | `HexFlo::memory_retrieve(key)` | `GET /api/hexflo/memory/:key` |
| Search memory | `HexFlo::memory_search(query)` | `GET /api/hexflo/memory/search` |
| Cleanup stale | `HexFlo::cleanup_stale_agents()` | `POST /api/hexflo/cleanup` |

### MCP Tools (Claude Code integration)

MCP tools are served by `hex mcp` (Rust binary). Tool names map 1:1 to CLI commands:

```
mcp__hex__hex_analyze          → hex analyze [path]
mcp__hex__hex_status           → hex status
mcp__hex__hex_swarm_init       → hex swarm init
mcp__hex__hex_swarm_status     → hex swarm status
mcp__hex__hex_task_create      → hex task create
mcp__hex__hex_task_list        → hex task list
mcp__hex__hex_task_complete    → hex task complete
mcp__hex__hex_memory_store     → hex memory store
mcp__hex__hex_memory_retrieve  → hex memory get
mcp__hex__hex_memory_search    → hex memory search
mcp__hex__hex_adr_list         → hex adr list
mcp__hex__hex_adr_search       → hex adr search
mcp__hex__hex_adr_status       → hex adr status
mcp__hex__hex_adr_abandoned    → hex adr abandoned
mcp__hex__hex_nexus_status     → hex nexus status
mcp__hex__hex_nexus_start      → hex nexus start
mcp__hex__hex_secrets_status   → hex secrets status
mcp__hex__hex_secrets_has      → hex secrets has
```

All tools delegate to the same hex-nexus REST API as the CLI commands.

### CLI Commands

```bash
hex swarm init <name> [topology]    # Initialize a swarm
hex swarm status                    # Show active swarms
hex task create <swarm-id> <title>  # Create a task
hex task list                       # List all tasks
hex task complete <id> [result]     # Mark task done
hex memory store <key> <value>      # Store key-value
hex memory get <key>                # Retrieve value
hex memory search <query>           # Search memory
```

### Heartbeat Protocol

- Agents send heartbeat on every `UserPromptSubmit` (via `hex hook route`)
- Hub marks agents as `stale` after 45 seconds without heartbeat
- Hub marks agents as `dead` after 120 seconds and reclaims their tasks

```bash
# Always use background agents with bypassPermissions for file writes
Agent tool: { subagent_type: "coder", mode: "bypassPermissions", run_in_background: true }
```

### Task State Synchronization (ADR-048)

When spawning subagents for HexFlo swarm tasks, include `HEXFLO_TASK:{task_id}` in the agent prompt. The hooks (`hex hook subagent-start` / `hex hook subagent-stop`) receive the prompt/result via **stdin** and automatically:
1. `SubagentStart`: Extracts the task ID from stdin, calls PATCH `/api/hexflo/tasks/{task_id}` with `agent_id` → server sets status to `in_progress`
2. `SubagentStop`: Reads `current_task_id` from session state, calls PATCH with `status: "completed"` and first 200 chars of subagent output as result
3. Both hooks persist task tracking state in `~/.hex/sessions/agent-{CLAUDE_SESSION_ID}.json`

```bash
# Example: spawn a coder agent with task tracking
Agent tool: {
  prompt: "HEXFLO_TASK:88bb424c-591a-482e-ac4f-55969549b7cf\nImplement the port interface for...",
  subagent_type: "coder",
  mode: "bypassPermissions",
  run_in_background: true
}
```

The `agent_id` is auto-resolved from `~/.hex/sessions/agent-{CLAUDE_SESSION_ID}.json` (written by `hex hook session-start`). The MCP tool `hex_hexflo_task_assign` also auto-resolves agent_id from this file when not explicitly provided.

## Security

- `FileSystemAdapter` has path traversal protection via `safePath()`
- API keys loaded only in `composition-root.ts` from env vars
- Never commit `.env` files — use `.env.example`
- Primary adapters MUST NOT use `innerHTML`/`outerHTML`/`insertAdjacentHTML` with any data that originates outside the domain layer. Use `textContent` or DOM APIs (`createElement`) instead.
