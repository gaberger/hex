# hex — AI Operating System (AIOS)

## What This Project Is

hex is an **AIOS** (AI Operating System) — a microkernel-based runtime built around **hexagonal architecture** (Ports & Adapters) that manages AI agent processes, coordinates distributed workloads, and enforces architectural constraints. It is not an application you deploy; it is the operating system layer that gets **installed into target projects** to orchestrate AI-driven development. Agents are the users. Developers are the sysadmins.

**Critical**: Everything in this repo (settings, hooks, statuslines, agents, skills) exists to be instantiated INTO a target project. The `examples/` directory contains sample target projects that use hex as an installed dependency. When working on examples, you are testing hex as a consumer would use it — the example IS the project, hex is the tool.

hex provides token-efficient code summaries via tree-sitter, swarm coordination via HexFlo (native Rust, ADR-027), a specs-first development pipeline, and a control plane dashboard for multi-project management.

## System Components

hex is composed of five deployment units. Understanding these is essential for working on the codebase:

### SpacetimeDB — Coordination & State Core (REQUIRED)

**SpacetimeDB must always be running to use hex.** It is the backbone — all clients (web, CLI, desktop) connect via WebSocket for real-time state synchronization.

- **7 WASM modules** in `spacetime-modules/` provide transactional reducers for swarm coordination, agent lifecycle, inference routing, secret management, and more (ADR-2604050900: right-sized from 19 to 7)
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
- Remote agent state synced to SpacetimeDB for cross-host fleet visibility (ADR-2604050900)

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

### Standalone Mode (ADR-2604112000)

hex supports a standalone composition path that does not require Claude Code. When `CLAUDE_SESSION_ID` is unset, hex-nexus wires an `AgentManager` backed by HexFlo dispatch + the `OllamaInferenceAdapter` (default inference for standalone mode). This enables `hex nexus start && hex plan execute wp-foo.json` on any host with Ollama installed -- no Claude CLI needed.

- `hex doctor composition` diagnoses which variant is active and what prerequisites are met.
- `hex ci --standalone-gate` validates the standalone path by running the P2/P3/P6 test suites.
- The Claude-integrated path remains the fast path when `CLAUDE_SESSION_ID` is present.

### Tiered Inference Routing (ADR-2604120202 + ADR-2604131630)

Tasks are classified into inference tiers that map to progressively more capable models:

| Tier | Default Model | Use Case |
|------|--------------|----------|
| T1 | `qwen3:4b` | Scaffold, transform, script — boilerplate generation |
| T2 | `qwen2.5-coder:32b` | Standard codegen — adapter implementations, tests |
| T2.5 | `devstral-small-2:24b` | Complex reasoning — cross-adapter wiring, architecture |
| T3 | Claude (frontier) | Frontier tasks — bypasses scaffolded dispatch entirely |

**Tier selection**: WorkplanTask `strategy_hint` controls tier directly (`scaffold`/`transform`/`script` = T1, `codegen` = T2, `inference` = T2.5). When `strategy_hint` is absent, heuristics classify based on layer depth and dependency count.

**Scaffolded dispatch** (T1/T2/T2.5): Best-of-N completions where each candidate must pass a compile gate (`cargo check` / `tsc --noEmit`) before acceptance. T3 bypasses scaffolding — frontier models produce single-shot output.

**Configuration**: Override default models per-tier in `.hex/project.json`:
```json
{ "inference": { "tier_models": { "t1": "qwen3:4b", "t2": "qwen2.5-coder:32b", "t2_5": "devstral-small-2:24b" } } }
```

**Monitoring**: Run `hex inference escalation-report` to see how often tasks escalate from lower tiers to higher ones — high escalation rates indicate tier thresholds need tuning.

## Tool Precedence (IMPORTANT)

**hex MCP tools take precedence over all third-party plugins** (including `plugin:context-mode`, `ruflo`, etc.):

| Operation | Use |
|---|---|
| Execute a workplan | `mcp__hex__hex_plan_execute` |
| Search codebase / run commands | `mcp__hex__hex_batch_execute` + `mcp__hex__hex_batch_search` |
| Swarm + task tracking | `mcp__hex__hex_hexflo_*` |
| Architecture analysis | `mcp__hex__hex_analyze` |
| ADR search/list | `mcp__hex__hex_adr_search`, `mcp__hex__hex_adr_list` |
| Memory | `mcp__hex__hex_hexflo_memory_store/retrieve/search` |

`plugin:context-mode` tools (`ctx_batch_execute`, `ctx_search`, etc.) may be used **only** for operations that have no hex equivalent (e.g. fetching external URLs). Never use them as a substitute for hex MCP tools on this project.

## Behavioral Rules

- **Workplans are autonomous**: When executing a workplan, complete ALL phases without asking. Do not pause between phases to ask "want me to continue?" — just keep going until done. Use HexFlo swarm tracking and background agents to parallelize where possible.
- **Inbox notifications are priority** (ADR-060): When a critical notification (priority 2) appears in hook output, STOP current work, save state (session file + hex memory store), acknowledge the notification (`hex inbox ack <id>`), and inform the user. This takes precedence over all other work. The `route` hook checks the inbox on every user interaction.
- Do what has been asked; nothing more, nothing less
- ALWAYS read a file before editing it
- NEVER save files to the root folder — use the directories below
- NEVER commit secrets, credentials, or .env files
- ALWAYS run `bun test` after making code changes
- ALWAYS run `bun run build` before committing
- NEVER use `mock.module()` in tests — use dependency injection via the Deps pattern instead (ADR-014)

## Task Tier Routing (ADR-2604110227)

hex classifies every user prompt into one of three **tiers** and routes the work
to the right artifact — matching the ergonomics of Claude's `TodoWrite` while
preserving hex's specs-first guarantees.

| Tier | Intent signal | Artifact | What happens |
|------|---------------|----------|--------------|
| **T1 Todo** | Questions, trivial edits (typos, renames, comments), confirmatory replies, reformats | Claude `TodoWrite` | Silent — host agent handles it |
| **T2 Mini-plan** | Work-sized within a single adapter boundary | In-session note | One-line suggestion printed to hook output |
| **T3 Workplan** | Feature-sized / cross-adapter (`implement X`, `add support for Y`, subsystem nouns) | `docs/workplans/drafts/draft-*.json` | **Auto-invokes `hex plan draft`** — writes a stub, surfaces it in context, user picks it up via `/hex-feature-dev` |

The classifier lives in `hex-cli/src/commands/hook.rs::classify_work_intent`
and runs inside `hex hook route` on every `UserPromptSubmit`. The scoring is
conservative — false negatives (T3 missed → T2) are cheap, false positives
(T1 → T3) would spawn unwanted drafts, so the threshold errs high.

### Auto-invocation — what it does NOT do

Auto-invocation on T3 creates a **draft stub only**. It does **not**:

- Create worktrees (still gated on `hex plan drafts approve` + `/hex-feature-dev`)
- Dispatch coder agents (still gated on `hex plan execute`)
- Write specs or steps (the draft contains only the original prompt)
- Commit anything

The existing specs-first hook (`hex-specs-required`) and phase gates stay in
place. All the draft does is save the user the "which slash command do I
type?" friction.

### Controls & opt-outs

- `HEX_AUTO_PLAN=0` — disable auto-invocation via env var (highest precedence)
- `.hex/project.json` → `workplan.auto_invoke.enabled: false` — per-project
  config toggle
- `hex skip plan` in the prompt — per-prompt escape hatch (returns T1 regardless)
- Questions (`?`, `how`, `why`, `what`) — always classified T1
- Trivial phrases (`fix typo`, `rename`, `add a comment`, `run rustfmt`) —
  always T1 regardless of other signals

### Draft management

```bash
hex plan draft <prompt>           # Create a draft (normally auto-invoked)
hex plan drafts list              # List pending drafts
hex plan drafts approve <name>    # Promote draft → docs/workplans/approved-*
hex plan drafts clear [--name N]  # Delete all (or one) drafts
hex plan drafts gc --days 7       # Garbage-collect drafts older than N days
```

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
spacetime-modules/       # 7 WASM modules (ADR-2604050900, right-sized from 19)
  hexflo-coordination/   #   Core: swarms, tasks, agents, memory, fleet, lifecycle, cleanup
  agent-registry/        #   Agent lifecycle + heartbeats + cleanup
  inference-gateway/     #   LLM request routing + procedure-based inference
  secret-grant/          #   TTL-based key distribution to sandboxed agents
  rl-engine/             #   Reinforcement learning model selection
  chat-relay/            #   Message routing
  neural-lab/            #   Experimental neural patterns

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

# ── hex-cli/assets — Embedded Templates (rust-embed, baked at compile) ─────
#    All templates live here; hex-nexus also embeds from this directory.
#    hex-cli/assets/ structure:
#      agents/hex/hex/    Agent YAML definitions (14 files, deployed to .claude/agents/)
#      skills/            Skill definitions (21+ .md files, deployed to .claude/skills/)
#      hooks/hex/         Hook YAML definitions (boundary-check, lifecycle, etc.)
#      helpers/           Runtime scripts (statusline, hook-handler, agent-register)
#      swarms/            Swarm behavior YAMLs — declarative pipelines (ADR-2603240130)
#      mcp/               MCP config + claude settings template (ADR-049)
#      schemas/           JSON schemas (workplan, mcp-tools)
#      templates/         Init templates (CLAUDE.md, settings)

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
hex inbox list       # Check agent notification inbox (ADR-060)
hex inbox notify     # Send notification to agent/project
hex inbox ack <id>   # Acknowledge a notification
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
| `/cargo-fast` | Apply ADR-064 Rust compilation optimizations (lld, sccache, nextest, dev profile) |

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
| `adversarial-reviewer` | Post-migration/post-feature adversarial review — hunts dangling refs, stale config, build breakage |
| `adr-reviewer` | ADR structure validation, cross-reference integrity, compliance checking |
| `rust-refactorer` | Rust-specific refactoring with cross-crate dependency awareness |

## Key Lessons (from adversarial review)

- **Tests can mirror bugs**: When the same LLM writes code AND tests, tests may encode the LLM's misunderstanding. Use property tests and behavioral specs as independent oracles.
- **Sign conventions matter**: For physics/math domains, document coordinate systems explicitly. `flapStrength` must be NEGATIVE (upward force in screen coords).
- **"It compiles" ≠ "it works"**: Always include runtime validation — can a user actually start the app?
- **Browser TypeScript needs a dev server**: Any project with HTML + TypeScript MUST include Vite (or equivalent).
- **Trace ALL consumers before deleting** (ADR-2604050900): When deleting modules/crates/files, `grep -r` the ENTIRE workspace — not just the immediate directory. hex-agent was broken for a full session because the workplan only checked hex-nexus bindings, missing hex-agent's feature-gated imports.
- **Workplans need build gates between phases**: Every phase that deletes or restructures artifacts MUST end with `cargo check --workspace`. A workplan marked "done" with a broken build is worse than no workplan at all.
- **Parallelize by file boundary, serialize by file overlap**: Multiple worktree agents editing the same file produce conflicting diffs. Batch same-file edits into one agent or run sequentially.

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
mcp__hex__hex_inbox_notify     → hex inbox notify (ADR-060)
mcp__hex__hex_inbox_query      → hex inbox list (ADR-060)
mcp__hex__hex_inbox_ack        → hex inbox ack (ADR-060)
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

## Declarative Swarm Behavior (ADR-2603240130)

Agent and swarm behavior is defined declaratively in YAML, not hardcoded in Rust. The supervisor reads these YAMLs at startup and drives all behavior from them.

### Agent YAML Definitions (`hex-cli/assets/agents/hex/hex/`)

14 agent YAMLs define: model selection (tier/preferred/fallback/upgrade), context loading (L1 AST summary, L2 signatures, L3 full source), workflow phases (TDD: red→green→refactor), feedback loop gates (compile/lint/test with on_fail instructions), quality thresholds, and input/output schemas.

**Schema varies by role:**
- **Coders** (`hex-coder.yml`): `workflow.phases[]` with blocking gates + `feedback_loop` with compile/lint/test
- **Planners** (`planner.yml`): `workflow.steps[]` for decomposition + `escalation` conditions
- **Reviewers/Validators**: simpler workflows, stricter quality thresholds

### Swarm Behavior YAMLs (`hex-cli/assets/swarms/`)

Swarm YAMLs define which agents participate, their cardinality, parallelism, and objectives:

```yaml
# hex-cli/assets/swarms/dev-pipeline.yml
name: dev-pipeline
topology: hex-pipeline
agents:
  - role: hex-coder
    cardinality: per_workplan_step    # one agent per step
    inference:
      task_type: code_generation
      model: preferred               # reads from agent YAML
      upgrade: { after_iterations: 3, to: opus }
  - role: hex-reviewer
    cardinality: per_source_file
    parallel_with: hex-tester
objectives:
  - id: CodeCompiles
    evaluate: "cargo check / tsc --noEmit"
    required: true
  - id: TestsPass
    evaluate: "cargo test / bun test"
    required: true
iteration:
  max_per_tier: 5
  on_max_iterations: escalate
```

Available swarm behaviors: `dev-pipeline`, `quick-fix`, `code-review`, `refactor`, `test-suite`, `documentation`, `migration`.

### Embedding

All templates (agents, swarms, hooks, skills, helpers, MCP config) live in `hex-cli/assets/` and are baked into both hex-cli and hex-nexus via `rust-embed` at compile time. hex-nexus extracts templates to target projects during `hex init`.

## Security

- `FileSystemAdapter` has path traversal protection via `safePath()`
- API keys loaded only in `composition-root.ts` from env vars
- Never commit `.env` files — use `.env.example`
- Primary adapters MUST NOT use `innerHTML`/`outerHTML`/`insertAdjacentHTML` with any data that originates outside the domain layer. Use `textContent` or DOM APIs (`createElement`) instead.
