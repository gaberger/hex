# hex Glossary — Canonical Terminology

> **This is the single source of truth for hex terminology.** All documentation, ADRs, code comments, agent prompts, and UI text must use these terms consistently. When in doubt, use the term from this glossary.

---

## System Components

| Term | Definition | Avoid |
|:-----|:-----------|:------|
| **hex** | AI-Assisted Integrated Development Environment (AAIDE) — an opinionated framework + toolchain for AI-driven development using hexagonal architecture. Installed into target projects. | "harness" alone, "framework" alone, "tool" alone |
| **SpacetimeDB** | Coordination & state core — the required backbone service. All clients connect via WebSocket for real-time state synchronization. Embeds application logic as WASM modules with transactional reducers. | "database" alone (undersells its role), "backend" alone |
| **hex-nexus** | Filesystem bridge daemon — bridges SpacetimeDB's sandboxed WASM execution and the local operating system. Provides REST API, architecture analysis, git operations, config sync, and serves the dashboard. Runs on port 5555. | "hub", "orchestration nexus", "daemon" alone, "server" alone |
| **hex-agent** | Architecture enforcement runtime — the component that must be present (locally or remotely) on any system running hex development agents. Enforces hexagonal architecture through skills, hooks, ADRs, workplans, HexFlo dispatchers, and agent definitions. Named "agent" because it is the runtime environment for hex's AI agents. | Do not confuse with the hexagonal architecture concept of "adapter" |
| **hex-dashboard** | Developer control plane — a single interface for managing AI-driven development across multiple projects and systems. Provides agent fleet control, architecture health monitoring, command dispatch, and inference monitoring. Served by hex-nexus as a Solid.js SPA. | "dashboard" alone (ambiguous), "UI" alone |
| **hex-cli** | The canonical CLI binary (Rust). All hex commands go through this binary. Also serves MCP tools via `hex mcp`. | "the CLI" without specifying which one |
| **hex-desktop** | Desktop application — a Tauri wrapper around the hex-dashboard web UI. | |
| **hex-core** | Shared Rust library — domain types and port traits used across all Rust crates. Zero external dependencies beyond serde. | |
| **hex-parser** | Code parsing utilities Rust crate. | |

## Architecture Concepts

| Term | Definition | Avoid |
|:-----|:-----------|:------|
| **hexagonal architecture** | Ports & Adapters pattern (Alistair Cockburn, 2005). Application logic depends only on port interfaces; adapters implement ports for specific technologies. Also called "Ports and Adapters." | "clean architecture" (related but distinct), "layered architecture" |
| **port** | A typed interface contract between architecture layers. Ports define what the application needs (input ports) or provides (output ports) without specifying how. | "API", "service", "interface" alone |
| **adapter** | An implementation of a port for a specific technology. Primary adapters drive the application (CLI, MCP, dashboard). Secondary adapters are driven by it (filesystem, git, LLM). | "plugin", "driver", "module" (in the adapter context) |
| **primary adapter** | A driving adapter — receives external input and translates it into port calls. Examples: CLI adapter, MCP adapter, dashboard adapter. | "input adapter", "inbound adapter" |
| **secondary adapter** | A driven adapter — implements a port by calling an external system. Examples: filesystem adapter, git adapter, LLM adapter. | "output adapter", "outbound adapter" |
| **composition root** | The single file (`composition-root.ts`) that wires adapters to ports. The ONLY file allowed to import from adapter modules. This is the dependency injection point. | "config", "bootstrap", "main", "entry point" |
| **domain** | Pure business logic layer. Zero external dependencies. Contains value objects, entities, and domain events. May only import from itself. | "model", "core" alone (ambiguous with hex-core crate) |
| **use case** | Application logic that composes ports to implement a business operation. May import from domain and ports only. | "service", "controller", "handler" |
| **boundary** | The import rule between architecture layers. `hex analyze` validates that boundaries are not violated (e.g., adapters must never import other adapters). | "layer boundary", "module boundary" |
| **boundary violation** | An import that crosses an architecture boundary illegally. Detected by `hex analyze`. | "coupling", "dependency violation" |

## SpacetimeDB Concepts

| Term | Definition | Avoid |
|:-----|:-----------|:------|
| **WASM module** | A SpacetimeDB server-side logic unit compiled from Rust to `wasm32-unknown-unknown`. Contains table definitions and reducer functions. Lives in `spacetime-modules/`. | "plugin", "extension", "service" |
| **reducer** | A transactional stored procedure inside a WASM module. Executes atomically — either fully succeeds or fully rolls back. Called by clients or scheduled by the runtime. | "endpoint", "handler", "mutation", "function" alone |
| **table** | A SpacetimeDB table defined with `#[table]`. Can be public (clients can subscribe) or private (server-side only). Automatically replicated to subscribed clients via WebSocket. | "collection", "model" |
| **subscription** | A SQL query that a client registers with SpacetimeDB. The server pushes matching row changes in real-time via WebSocket. Replaces polling. | "listener", "watcher", "observer" |
| **scheduled reducer** | A reducer that runs on a timer (e.g., `run_cleanup` every 30s in hexflo-cleanup). Defined with `#[spacetimedb::reducer(init)]` or `schedule!()`. | "cron job", "timer", "background task" |

## Coordination Concepts

| Term | Definition | Avoid |
|:-----|:-----------|:------|
| **HexFlo** | Native Rust swarm coordination layer built into hex-nexus (ADR-027). Manages swarm lifecycle, task assignment, agent heartbeats, and persistent memory. State persisted in SpacetimeDB. | "ruflo" (predecessor, deprecated), "claude-flow" (upstream project) |
| **swarm** | A coordinated group of AI agents working on a feature. Has a topology (hierarchical, mesh, hierarchical-mesh) and tracks tasks, agents, and memory. | "cluster", "team", "group" |
| **topology** | The communication pattern between agents in a swarm: hierarchical (tree), mesh (peer-to-peer), or hierarchical-mesh (hybrid). | "architecture" (overloaded term) |
| **heartbeat** | Periodic signal (every 15s) from an agent to confirm it is alive. Agents without heartbeat are marked stale (45s), then dead (120s), and their tasks are reclaimed. | "ping", "keepalive" (WebSocket keepalive is different) |
| **worktree** | A git worktree — an isolated working copy of the repository. Each agent gets its own worktree to prevent merge conflicts during parallel development. | "branch" (worktrees are more isolated than branches) |
| **workplan** | A structured task decomposition — breaks a feature into adapter-bounded steps organized by dependency tier. Generated by the planner agent. | "plan" alone, "task list" |
| **tier** | A dependency level in a workplan. Tier 0 (domain + ports) has no dependencies; each subsequent tier depends on lower tiers. Tiers execute sequentially; tasks within a tier execute in parallel. | "phase" (used for feature lifecycle phases, not workplan tiers) |

## Development Workflow Concepts

| Term | Definition | Avoid |
|:-----|:-----------|:------|
| **behavioral spec** | An acceptance specification written before code. Defines what "correct" looks like as testable assertions. Generated by the behavioral-spec-writer agent. | "spec" alone (ambiguous), "test" (specs are not tests) |
| **validation judge** | A post-build verification gate. Checks behavioral spec assertions, property test invariants, smoke scenarios, and hex boundary analysis. **Blocking** — code cannot ship without passing. | "validator" alone, "linter" |
| **feature lifecycle** | 7-phase process: SPECS → PLAN → WORKTREES → CODE → VALIDATE → INTEGRATE → FINALIZE. | |
| **composition** | The act of wiring adapters to ports in the composition root. NOT code composition/inheritance. | |

## Inference Concepts

| Term | Definition | Avoid |
|:-----|:-----------|:------|
| **inference** | An LLM request — sending a prompt and receiving a completion. Model-agnostic. | "completion" alone, "chat" (too specific) |
| **inference gateway** | SpacetimeDB WASM module that routes inference requests to providers. Handles budgets, rate limiting, and health checks. | "API gateway", "proxy" |
| **inference bridge** | SpacetimeDB WASM module that handles model integration and response processing. Works with the gateway for request lifecycle. | |
| **provider** | An LLM service (Anthropic, OpenAI, Ollama, etc.) registered with the inference gateway. hex is model-agnostic. | "model" (a provider offers many models) |

## Documentation Concepts

| Term | Definition | Avoid |
|:-----|:-----------|:------|
| **ADR** | Architecture Decision Record — a document capturing a design decision, its context, and consequences. Stored in `docs/adrs/`. Follows a lifecycle: proposed → accepted → deprecated/superseded/rejected. | "RFC", "design doc" |
| **reference doc** | Canonical component documentation in `docs/reference/`. The authoritative source for how a component works. | "wiki", "guide" alone |
| **skill** | A Claude Code slash command (e.g., `/hex-scaffold`). Defined as Markdown files. Guides AI agents to produce architecture-compliant output. | "command" alone, "plugin" |
| **agent definition** | A YAML file defining an AI agent's role, constraints, allowed tools, and model. Shipped in the `agents/` directory. | "agent config", "agent template" |
| **hook** | A pre/post operation trigger. Validates boundaries, auto-formats, trains patterns. Configured in `.claude/settings.json`. | "callback", "middleware" (different pattern) |

## Summary Level Concepts

| Term | Definition |
|:-----|:-----------|
| **L0** | File list only (~2% tokens) |
| **L1** | Exports + function signatures (~6% tokens) — **ideal for AI context** |
| **L2** | L1 + function bodies (~40% tokens) |
| **L3** | Full source code (100% tokens) |

---

## Usage Rules

1. **Always use the full term on first reference** in a document (e.g., "hex-nexus (filesystem bridge daemon)"), then the short form thereafter.
2. **Never use deprecated terms** — if you find "ruflo", "hex-hub", or "hex-agent" in existing docs, update them.
3. **When adding a new term**, add it to this glossary FIRST, then use it in documentation.
4. **Component names are hyphenated** — `hex-nexus`, not `HexNexus` or `hex nexus` (except in Rust identifiers where `HexNexus` is appropriate).
5. **SpacetimeDB is one word** — not "Spacetime DB" or "spacetimedb" (except in Cargo.toml dependency names).
