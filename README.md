# hex — AI-Assisted Integrated Development Environment (AAIDE)

**hex** enforces hexagonal architecture as a hard constraint on every AI agent, coordinates multi-agent swarms natively through a real-time SpacetimeDB backbone, and gates every commit behind automated compile, lint, test, and semantic validation checks.

It is not a deployable application. It is a framework and CLI toolchain installed into target projects. Architecture compliance is not advisory — it is enforced at the point of code generation, commit, and merge.

[![Build](https://img.shields.io/badge/build-passing-brightgreen)](https://github.com/hex-project/hex-intf)
[![License](https://img.shields.io/badge/license-MIT-blue)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.75%2B-orange)](https://www.rust-lang.org/)
[![SpacetimeDB](https://img.shields.io/badge/spacetimedb-required-red)](https://spacetimedb.com/)

---

## Table of Contents

1. [What is hex?](#1-what-is-hex)
2. [System Architecture](#2-system-architecture)
3. [Hexagonal Architecture Enforcement](#3-hexagonal-architecture-enforcement)
4. [Specs-First Development Pipeline](#4-specs-first-development-pipeline)
5. [HexFlo Swarm Coordination](#5-hexflo-swarm-coordination)
6. [Quick Start](#6-quick-start)
7. [CLI Reference](#7-cli-reference)
8. [Agent Fleet](#8-agent-fleet)
9. [Competitive Positioning](#9-competitive-positioning)
10. [Contributing](#10-contributing)

---

## 1. What is hex?

### The Problem

AI coding agents fail at scale in predictable ways:

- **Context explosion**: Without boundaries, an agent needs the entire codebase in context to make a safe change. At any non-trivial project size this is expensive, slow, and produces hallucinated cross-cutting changes.
- **Boundary violations**: Agents generate code that imports across layer boundaries — usecases calling adapters directly, adapters importing other adapters — destroying the architectural invariants that make a codebase maintainable.
- **Unvalidated output**: Generated code may compile but still violate behavioral specs, skip tests, or introduce regressions. Without a blocking validation gate, bad code reaches main.
- **No coordination**: Multiple agents working in parallel on the same working tree produce merge conflicts. There is no mechanism to assign work, track completion, or recover from a failed agent.
- **Architectural drift**: Projects lose their intended structure over time. Linters catch style; they do not enforce that a secondary adapter is not importing a primary adapter.

### The Solution

hex treats hexagonal architecture as a hard execution constraint, not a convention:

1. **Boundary enforcement at import level**: `hex analyze` uses tree-sitter to parse every import in the codebase and verify it does not cross a layer boundary. Violations fail the build.
2. **Worktree-per-adapter isolation**: Each parallel agent operates in its own git worktree, scoped to a single adapter boundary. Agents cannot conflict because they cannot touch each other's files.
3. **Mandatory specs-first pipeline**: No code is written without a machine-readable behavioral spec. No code is merged without passing a semantic validation gate.
4. **Native swarm coordination**: HexFlo (built into hex-nexus in Rust, backed by SpacetimeDB) tracks task state, monitors agent heartbeats, and reclaims work from dead agents — with no external dependencies.

### Key Insight

Architecture enforcement in hex is not a linter that runs after the fact. It is a pre-execution constraint that shapes what agents are allowed to generate in the first place. Agents receive a workplan scoped to one adapter boundary. They never see — and cannot import from — code outside that boundary. The boundary violation cannot happen because the agent never had the context to create it.

---

## 2. System Architecture

hex consists of five deployment units. SpacetimeDB must be running for any hex operation that involves state, coordination, or real-time visibility.

```
┌─────────────────────────────────────────────────────────────────────┐
│                         Developer Workflow                          │
│    hex CLI (hex-cli)  ←→  Claude Code (MCP: hex mcp)               │
└────────────────────────────┬────────────────────────────────────────┘
                             │ REST API (port 5555)
┌────────────────────────────▼────────────────────────────────────────┐
│                        hex-nexus daemon                             │
│   ┌──────────────┐  ┌──────────────┐  ┌──────────────────────────┐ │
│   │  HexFlo      │  │  arch        │  │  hex-dashboard           │ │
│   │  (swarm      │  │  analysis    │  │  (Solid.js + Tailwind,   │ │
│   │   coord)     │  │  tree-sitter │  │   served at :5555)       │ │
│   └──────┬───────┘  └──────────────┘  └──────────────────────────┘ │
│          │ WebSocket / REST                                          │
└──────────┼──────────────────────────────────────────────────────────┘
           │
┌──────────▼──────────────────────────────────────────────────────────┐
│                         SpacetimeDB (REQUIRED)                      │
│   18 WASM modules: hexflo-coordination, agent-registry,             │
│   inference-gateway, workplan-state, chat-relay, fleet-state,       │
│   architecture-enforcer, + 11 more                                  │
│   ─────────────────────────────────────────────────────             │
│   Fallback: SQLite (~/.hex/hub.db) when SpacetimeDB unavailable     │
└─────────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────────┐
│                      hex-agent (per developer machine)              │
│   Skills · Hooks · ADRs · Workplans · YAML agent definitions        │
│   Enforcement runtime for AI agents (local or remote)               │
└─────────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────────┐
│                      Inference Layer                                │
│   inference-gateway WASM + hex-nexus HTTP bridge                    │
│   Providers: Anthropic · OpenAI · Ollama · OpenRouter · vLLM        │
└─────────────────────────────────────────────────────────────────────┘
```

### SpacetimeDB — Coordination Backbone (REQUIRED)

SpacetimeDB is the transactional state backbone for hex. All clients — CLI, dashboard, agents — connect via WebSocket and receive real-time state updates without polling. 18 WASM modules provide reducers for swarm coordination, agent lifecycle, inference routing, workplan state, chat relay, and architecture enforcement.

**Critical limitation**: WASM modules cannot access the filesystem, spawn processes, or make network calls. This constraint is why hex-nexus exists as a separate daemon.

SpacetimeDB must be running. hex-nexus will not start without it (SQLite is available as a fallback for offline/single-node operation per ADR-025, but the full coordination surface requires SpacetimeDB).

### hex-nexus — Filesystem Bridge Daemon

hex-nexus is a Rust daemon (axum, port 5555) that bridges SpacetimeDB with the host OS. It is the component that actually reads and writes files, runs architecture analysis, manages git worktrees, syncs repository config into SpacetimeDB tables on startup (ADR-044), and serves the dashboard frontend (baked into the binary via `rust-embed`).

The REST API exposed by hex-nexus is the single backend used by both the CLI and the MCP server. Every `hex` command and every `mcp__hex__*` tool delegates to this API.

### hex-agent — Architecture Enforcement Runtime

hex-agent must be present on any machine running hex development agents. It provides the runtime environment: slash-command skills, pre/post operation hooks for boundary validation and pattern training, YAML-defined agent roles, HexFlo dispatchers, and quality gate feedback loops (compile → lint → test).

### hex-dashboard — Developer Control Plane

A Solid.js + TailwindCSS single-page application served at `http://localhost:5555`. It provides multi-project management with live freshness indicators, agent fleet control with heartbeat monitoring, architecture health scores with violation breakdown, command dispatch to any connected project, and inference monitoring (model requests, token consumption). All data arrives via SpacetimeDB WebSocket subscriptions — no polling.

### Inference Layer

The `inference-gateway` WASM module routes LLM requests; hex-nexus performs the actual HTTP calls. The layer is model-agnostic: Anthropic, OpenAI, Ollama, OpenRouter, vLLM, and any OpenAI-compatible endpoint are all supported. Model selection is per-swarm, with configurable tier (haiku/sonnet/opus), fallback, and upgrade-after-N-iterations rules.

---

## 3. Hexagonal Architecture Enforcement

### Layer Rules

hex enforces a strict dependency hierarchy. These rules are checked on every commit via hooks and on demand via `hex analyze`:

| Layer | May import from |
|---|---|
| `domain/` | `domain/` only |
| `ports/` | `domain/` only |
| `usecases/` | `domain/`, `ports/` |
| `adapters/primary/` | `ports/` only |
| `adapters/secondary/` | `ports/` only |
| `composition-root.ts` | All layers (single DI point) |

**Adapters must never import other adapters.** Cross-adapter coupling is the most common architectural failure in AI-generated code and is unconditionally rejected.

### What `hex analyze` Checks

```bash
hex analyze <path>              # Full analysis
hex analyze <path> --strict     # Warnings become errors
hex analyze <path> --json       # Structured output for CI
hex analyze <path> --adr-compliance-only
```

The analyzer runs tree-sitter over every source file and builds a dependency graph. It checks:

1. **Boundary validation**: Every import is classified by source layer and target layer. Any import that violates the table above is reported as a violation.
2. **Cycle detection**: Circular dependencies within and across layers are detected and reported with the full cycle path.
3. **Dead export detection**: Exported symbols that are never imported are reported as unused debt.
4. **ADR compliance** (ADR-054): Code patterns that violate specific ADRs (deprecated APIs, disallowed constructs) are flagged.

Output is color-coded PASS/FAIL in the CLI, structured JSON in `--json` mode, and a live score ring on the dashboard.

### Dependency Tier Table

Tiers determine agent execution order in a swarm. Lower tiers have no dependencies and can run immediately; higher tiers wait for their dependencies to complete.

| Tier | Layer | Depends on | Parallel |
|---|---|---|---|
| 0 | Domain + Ports | Nothing | No (serial) |
| 1 | Secondary adapters | Tier 0 | Yes |
| 2 | Primary adapters | Tier 0 | Yes |
| 3 | Use cases | Tiers 0–2 | Yes |
| 4 | Composition root | Tiers 0–3 | No (serial) |
| 5 | Integration tests | Everything | No (serial) |

### Why This Matters for AI Agents

The tier structure makes parallel agent execution safe. Two agents working on independent secondary adapters (Tier 1) cannot produce merge conflicts because they operate in separate git worktrees and their code has no shared imports. The ports layer (Tier 0) defines the compile-time contracts both adapters must satisfy — the boundary violation literally cannot compile even if an agent attempts it.

Token efficiency follows directly: an agent scoped to one adapter boundary needs to load only that adapter's port interface and its own implementation files. Tree-sitter AST summaries (L0–L3) reduce context further — a function signature costs ~20 tokens instead of 200+.

---

## 4. Specs-First Development Pipeline

Every feature follows a mandatory 7-phase pipeline. No phase may be skipped or reordered.

```
  ┌──────────┐    ┌──────────┐    ┌───────────┐    ┌──────────┐
  │  SPECS   │───▶│   PLAN   │───▶│ WORKTREES │───▶│   CODE   │
  │ Phase 1  │    │ Phase 2  │    │  Phase 3  │    │ Phase 4  │
  └──────────┘    └──────────┘    └───────────┘    └──┬───────┘
                                                       │
  ┌──────────┐    ┌──────────┐    ┌───────────┐       │
  │ FINALIZE │◀───│INTEGRATE │◀───│ VALIDATE  │◀──────┘
  │ Phase 7  │    │ Phase 6  │    │  Phase 5  │
  └──────────┘    └──────────┘    └───────────┘
                                  (BLOCKING GATE)
```

| Phase | Agent | Input | Output | Gate |
|---|---|---|---|---|
| 1. SPECS | `behavioral-spec-writer` | Requirements (text, screenshot, video) | `docs/specs/{feature}.json` | Readable by domain expert |
| 2. PLAN | `planner` | Specs JSON | `docs/workplans/feat-{feature}.json` | 2–4 adapter-bounded steps |
| 3. WORKTREES | `feature-workflow.sh` | Workplan JSON | One git worktree per adapter | All worktrees created and checked out |
| 4. CODE | `hex-coder` (parallel per tier) | Workplan step + port interfaces | Source + unit tests | Tests pass, no lint errors |
| 5. VALIDATE | `validation-judge` | Code + tests + specs | PASS/FAIL report | **BLOCKING — must pass to proceed** |
| 6. INTEGRATE | `integrator` | Code in worktrees | Merged to main in dependency order | Full test suite passes |
| 7. FINALIZE | Operator | Merged code | Worktrees removed, report generated | All artifacts documented |

### Worktree-Per-Adapter Pattern

Each adapter boundary gets its own git worktree, named `feat/<feature>/<adapter>`. Agents work inside their worktree in isolation. There are no shared working trees between parallel agents. Merge order follows the dependency tier table: domain → ports → secondary adapters → primary adapters → use cases → integration tests.

```bash
# Worktree lifecycle
./scripts/feature-workflow.sh setup <feature-name>    # Create worktrees from workplan
./scripts/feature-workflow.sh status <feature-name>   # Show progress
./scripts/feature-workflow.sh merge <feature-name>    # Merge in dependency order
./scripts/feature-workflow.sh cleanup <feature-name>  # Remove worktrees + branches
```

### Validation Judge: The Blocking Gate

Phase 5 runs the `validation-judge` agent (Opus-tier model required). It checks whether the generated code matches the behavioral spec from Phase 1, whether tests are semantically meaningful (not just coverage-padding), and whether architecture is sound. There is no bypass. A FAIL verdict stops the pipeline; the swarm re-enters Phase 4.

---

## 5. HexFlo Swarm Coordination

HexFlo is the native Rust coordination layer built into hex-nexus (ADR-027). It replaces any external coordination dependency. State is persisted in SpacetimeDB via the `hexflo-coordination` WASM module, with SQLite fallback for offline operation.

### Task State Machine

```
Task:  pending ──▶ in_progress ──▶ completed
                        │
                        └──▶ dead (reclaimed after agent timeout)

Agent: registered ──▶ active ──▶ stale (45s no heartbeat)
                                    │
                                    └──▶ dead (120s) → tasks reclaimed
```

### Heartbeat Protocol

Agents send a heartbeat on every user interaction via `hex hook route`. The hub marks agents stale after 45 seconds and dead after 120 seconds. Dead agent tasks are automatically returned to `pending` and reassigned. This means a crashed or disconnected agent never blocks a swarm indefinitely.

### API Surface

| Operation | CLI | REST | MCP Tool |
|---|---|---|---|
| Initialize swarm | `hex swarm init <name>` | `POST /api/swarms` | `mcp__hex__hex_hexflo_swarm_init` |
| Swarm status | `hex swarm status` | `GET /api/swarms` | `mcp__hex__hex_hexflo_swarm_status` |
| Create task | `hex task create <swarm-id> <title>` | `POST /api/swarms/:id/tasks` | `mcp__hex__hex_hexflo_task_create` |
| Complete task | `hex task complete <id>` | `PATCH /api/swarms/tasks/:id` | `mcp__hex__hex_hexflo_task_complete` |
| Store memory | `hex memory store <key> <value>` | `POST /api/hexflo/memory` | `mcp__hex__hex_hexflo_memory_store` |
| Retrieve memory | `hex memory get <key>` | `GET /api/hexflo/memory/:key` | `mcp__hex__hex_hexflo_memory_retrieve` |
| Search memory | `hex memory search <query>` | `GET /api/hexflo/memory/search` | `mcp__hex__hex_hexflo_memory_search` |

### Spawning Agents with Task Tracking

Include `HEXFLO_TASK:{task_id}` in the agent prompt. The `subagent-start` and `subagent-stop` hooks read this from stdin and automatically call the HexFlo API to transition task state.

```typescript
// Spawn a background coder agent with task tracking
Agent({
  prompt: `HEXFLO_TASK:88bb424c-591a-482e-ac4f-55969549b7cf
Implement the FileSystemAdapter for the IFileSystemPort interface.
Work in worktree: hex-worktrees-feat-example-p1.1/
Port interface: src/ports/IFileSystemPort.ts`,
  subagent_type: "general-purpose",
  mode: "bypassPermissions",   // REQUIRED for background file writes
  run_in_background: true
})
```

Background agents **must** use `mode: "bypassPermissions"`. Using `mode: "acceptEdits"` silently denies all file writes when no user is present to approve them.

### Swarm Topologies

| Topology | Strategy | Best For |
|---|---|---|
| `hierarchical` | Leader coordinates workers | Feature development across tiers |
| `mesh` | Peer-to-peer | Parallel independent tasks |
| `adaptive` | Starts mesh, promotes leader when needed | Unknown workload shape |

### Declarative Agent Behavior

Agent behavior is defined in YAML, not hardcoded in Rust. The 14 agent definitions in `hex-cli/assets/agents/hex/hex/` specify model selection (tier, preferred, fallback, upgrade threshold), context loading (L0–L3 AST summaries), workflow phases (TDD: red → green → refactor), feedback loop gates (compile → lint → test with `on_fail` instructions), and quality thresholds.

---

## 6. Quick Start

### Prerequisites

- **SpacetimeDB** — must be running before `hex nexus start`
- **Rust 1.75+** — for building hex-cli and hex-nexus
- **Bun** — for the TypeScript library and dashboard
- **tree-sitter-cli** — for grammar-based analysis

### Install hex CLI

```bash
# Build from source
git clone https://github.com/hex-project/hex-intf.git
cd hex-intf
cargo build -p hex-cli --release
# Add target/release/hex to your PATH

# Verify installation
hex --version
```

### Initialize a Project

```bash
# Start SpacetimeDB first
hex stdb start

# Start the hex-nexus daemon
hex nexus start

# Verify daemon health
hex nexus status

# Initialize hex in an existing project
hex init /path/to/your/project

# Check architecture health
hex analyze /path/to/your/project

# Open the dashboard
open http://localhost:5555
```

### Start the MCP Server (Claude Code integration)

Add to your Claude Code settings:

```json
{
  "mcpServers": {
    "hex": {
      "command": "hex",
      "args": ["mcp"]
    }
  }
}
```

All `mcp__hex__*` tools are now available in Claude Code.

---

## 7. CLI Reference

All commands are served by the `hex` Rust binary. MCP tools map 1:1 to CLI commands via the hex-nexus REST API.

### Daemon & SpacetimeDB

| Command | Description |
|---|---|
| `hex stdb start` | Start local SpacetimeDB instance |
| `hex stdb status` | Check SpacetimeDB health |
| `hex nexus start` | Start hex-nexus daemon (port 5555) |
| `hex nexus status` | Check daemon health and SpacetimeDB connectivity |
| `hex nexus logs` | Stream daemon logs |

### Architecture Analysis

| Command | Description |
|---|---|
| `hex analyze <path>` | Full architecture health check (boundaries, cycles, dead exports) |
| `hex analyze <path> --strict` | Promote warnings to errors |
| `hex analyze <path> --json` | Structured output for CI |
| `hex enforce mode` | Check current enforcement mode |
| `hex enforce list` | List all enforcement rules |

### Swarm Coordination

| Command | Description |
|---|---|
| `hex swarm init <name> [topology]` | Initialize a swarm |
| `hex swarm status` | Show active swarms with task/agent counts |
| `hex task create <swarm-id> <title>` | Create a task |
| `hex task list [--swarm <id>]` | List tasks with status |
| `hex task complete <id> [result]` | Mark task completed |
| `hex memory store <key> <value>` | Store key-value (persists across sessions) |
| `hex memory get <key>` | Retrieve value |
| `hex memory search <query>` | Search memory |

### Architecture Decision Records

| Command | Description |
|---|---|
| `hex adr list` | List all ADRs with status |
| `hex adr status <id>` | Show ADR detail |
| `hex adr search <query>` | Search ADRs by keyword |
| `hex adr abandoned` | Find stale ADRs |

### Agent Management

| Command | Description |
|---|---|
| `hex agent list` | List all registered agents |
| `hex agent info <id>` | Show agent details |
| `hex inbox list` | Check agent notification inbox |
| `hex inbox notify <agent-id> --kind <type>` | Send notification |
| `hex inbox ack <notification-id>` | Acknowledge notification |

### Workplans & Pipeline

| Command | Description |
|---|---|
| `hex plan list` | List all workplans |
| `hex plan status <file>` | Show workplan detail |
| `hex plan execute <file>` | Run workplan end-to-end |
| `hex spec write` | Start behavioral spec writer |
| `hex dev` | Interactive TUI development pipeline |

### Git & Worktrees

| Command | Description |
|---|---|
| `hex worktree create <name>` | Create git worktree |
| `hex worktree list` | List active worktrees |
| `hex worktree merge <name>` | Merge worktree to main |
| `hex worktree remove <name>` | Delete worktree |
| `hex git status` | Show git status for project |

### Inference

| Command | Description |
|---|---|
| `hex inference add <id> --url <url> --model <name>` | Register inference endpoint |
| `hex inference list` | Show registered endpoints |
| `hex inference test <id>` | Test endpoint connectivity |
| `hex inference discover` | Scan LAN for inference endpoints |

### Secrets

| Command | Description |
|---|---|
| `hex secrets status` | Show secrets backend status |
| `hex secrets vault set <key> <value>` | Store secret |
| `hex secrets grant <agent-id> <key>` | Grant agent access to secret |
| `hex secrets revoke <grant-id>` | Revoke access |

---

## 8. Agent Fleet

All 14 agents are defined in YAML (`hex-cli/assets/agents/hex/hex/`) and deployed to `.claude/agents/` on `hex init`. Each agent has a defined boundary — agents decline work outside that boundary.

| Agent | Role | When to Use | Model |
|---|---|---|---|
| `hex-coder` | Code generator | Writing production code within one adapter boundary (TDD, London school) | Sonnet / Haiku fallback / Opus upgrade |
| `planner` | Task decomposer | Breaking requirements into adapter-bounded workplan steps | Sonnet / Haiku fallback |
| `behavioral-spec-writer` | Spec generator | Writing acceptance specs before any code is generated | Sonnet / Haiku fallback |
| `validation-judge` | Quality gate keeper | Semantic validation after code generation — BLOCKING gate | Opus required |
| `feature-developer` | Lifecycle orchestrator | Running the full 7-phase pipeline for a feature | Opus coordinator |
| `swarm-coordinator` | Swarm driver | Spawning and monitoring parallel hex-coder agents via HexFlo | Sonnet |
| `integrator` | Merge coordinator | Merging worktrees in dependency order and running full test suite | Sonnet |
| `dead-code-analyzer` | Debt detector | Finding dead exports, unused ports, and boundary violations | Haiku |
| `dependency-analyst` | Tech recommender | Analyzing dependencies and recommending tech stack | Sonnet |
| `scaffold-validator` | Readiness checker | Verifying generated projects are actually runnable | Sonnet |
| `adr-reviewer` | ADR enforcer | Reviewing code changes for ADR compliance and deprecated API usage | Sonnet |
| `status-monitor` | Progress tracker | Real-time swarm monitoring, heartbeat tracking, duration estimates | Haiku |
| `rust-refactorer` | Code improver | Refactoring Rust code for readability, performance, and clippy compliance | Sonnet |
| `dev-tracker` | Session tracker | Maintaining audit trail of commits vs task completions | Haiku |

---

## 9. Competitive Positioning

### Where hex Fits vs. SPECkit and BAML

SPECkit and BAML both address real problems in AI-assisted development. They solve sub-problems that hex either incorporates or assumes solved. Understanding the division of scope clarifies when each tool applies.

**SPECkit** (GitHub's open-source SDD toolkit) imposes a documentation-first discipline through structured markdown templates and slash commands (`/speckit.specify`, `/speckit.plan`, `/speckit.tasks`). It works with any CLI-based agent and has minimal adoption friction. It covers what hex calls Phase 1 (behavioral specs) and part of Phase 2 (planning) — and stops there. SPECkit has no runtime, no architecture analysis, no coordination backend, no worktree isolation, and no validation gate. Compliance with the spec it produces is on the honor system.

**BAML** (BoundaryML) is a DSL for defining typed LLM functions. It provides compile-time guarantees on prompt inputs and outputs, resilient parsing of freeform LLM responses, and multi-provider retry/fallback routing. It solves the function-level reliability problem extremely well. It has no concept of multi-agent coordination, project architecture, task graphs, or the development lifecycle. It is analogous to hex's typed inference port and adapter — the piece of the stack that makes LLM calls reliable — without the surrounding architecture that connects inference to the rest of a codebase.

hex operates at a different level of abstraction than either. It incorporates a specs-first workflow (covering SPECkit's territory) and typed inference (covering BAML's territory), and adds the enforcement and coordination layer that neither competitor provides.

### Feature Comparison Matrix

| Feature | hex | SPECkit | BAML |
|---|---|---|---|
| Spec-first development | Yes — machine-readable JSON | Yes — markdown templates | No |
| Structured LLM output / type safety | Via typed port interfaces | No | Yes — core product |
| Multi-provider LLM routing | Yes (inference-gateway WASM) | Agent-dependent | Yes (retry/fallback/rotation) |
| Static architecture enforcement | Yes — tree-sitter boundary analysis | No | No |
| Hexagonal layer isolation at import level | Hard rule, checked on every commit | Spec document only | Not applicable |
| Multi-agent swarm coordination | Yes — HexFlo, SpacetimeDB, task graph | Prompt-based role switching | No |
| Git worktree isolation per agent | Yes (ADR-004) | No | No |
| Task state machine with dead-agent reclamation | Yes (45s stale / 120s dead) | No | No |
| Validation gate before merge | Yes (validation-judge, blocking) | No | No |
| Token-efficient AST code summaries | Yes (tree-sitter L0–L3) | No | No |
| Fleet management dashboard | Yes (real-time, multi-project) | No | No |
| MCP server integration | Yes (all commands) | Yes (AGENTS.md convention) | Yes (ActionRunner) |
| Persistent memory across sessions | Yes (HexFlo memory, SpacetimeDB/SQLite) | No | No |
| Spec → workplan → code → validate → merge as one workflow | Yes | Partial (no validate/merge) | No |
| YAML-declarative agent + swarm behavior | Yes (14 agent YAMLs, 7 swarm YAMLs) | Partial (AGENTS.md) | No |
| SpacetimeDB real-time coordination backbone | Yes (18 WASM modules) | No | No |
| Open source | Yes | Yes (MIT) | Yes (Apache 2.0) |

### Narrative Summary

SPECkit is what hex's `behavioral-spec-writer` agent produces as Phase 1 output — the beginning of the pipeline, not the whole pipeline. BAML is what hex's typed inference port and adapter provide — reliable structured LLM calls — without the surrounding architecture and coordination layer.

hex's unique territory is the **enforcement and coordination layer**: static architecture analysis that catches violations before merge, worktree-isolated parallel agents that cannot conflict, a real-time coordination backbone with dead-agent reclamation, and a blocking semantic validation gate. Neither SPECkit nor BAML has anything analogous to `hex analyze`, HexFlo task reclamation, or the SpacetimeDB coordination backend.

The trade-off is real: hex requires more infrastructure (SpacetimeDB, hex-nexus daemon) and more initial learning investment than either competitor. The payoff is the only guarantee in this comparison that architectural compliance is verified at the point of merge.

---

## 10. Contributing

hex follows the same specs-first pipeline for its own development that it enforces on consumer projects.

### Before Writing Code

1. **Check for an existing ADR**: `hex adr search <topic>`. Every architectural decision has an ADR in `docs/adrs/`. Read it before touching the affected code.
2. **Write a behavioral spec first**: `hex spec write`. The spec is the contract. Code that does not match the spec fails Phase 5.
3. **Create a workplan**: `hex plan` or use the `planner` agent. The workplan decomposes the feature into adapter-bounded steps with dependency tiers.

### Development Workflow

```bash
# Build Rust components (use debug builds during iteration)
cargo build -p hex-cli
cargo build -p hex-nexus

# Build TypeScript library
bun run build

# Run all tests
bun test

# Architecture check (must pass before committing)
hex analyze .
```

### Key Constraints for Contributors

- **Hexagonal architecture rules are enforced** — `hex analyze .` must pass. Cross-adapter imports will be rejected.
- **All relative imports in TypeScript must use `.js` extensions** (NodeNext module resolution).
- **Do not use `mock.module()` in tests** — use dependency injection via the Deps pattern (ADR-014).
- **Do not commit `.env` files** — use `.env.example` and load secrets via `hex secrets vault`.
- **Every new port, adapter, or external dependency requires an ADR** before implementation begins.

### Reference Documents

- `CLAUDE.md` — authoritative system design, behavioral rules, and file organization
- `docs/adrs/` — 37 Architecture Decision Records documenting every significant design choice
- `docs/specs/` — behavioral specifications for implemented features
- `docs/workplans/` — workplan artifacts for features in progress
- `docs/analysis/` — adversarial review reports with lessons learned
