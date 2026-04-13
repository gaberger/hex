# Architecture

> Back to [README](../README.md) | See also: [Getting Started](GETTING-STARTED.md) | [Inference](INFERENCE.md) | [Developer Experience](DEVELOPER-EXPERIENCE.md) | [Comparison](COMPARISON.md)

---

## System Architecture

```
hex-cli/               Rust CLI — shell + MCP server (canonical entry point)
hex-nexus/             Daemon — REST API, dashboard, filesystem bridge
hex-core/              Domain types + 10 port traits (zero external deps)
hex-agent/             Agent runtime — skills, hooks, architecture enforcement
hex-desktop/           Desktop app (Tauri wrapper)
hex-parser/            Code parsing utilities (tree-sitter)
spacetime-modules/     7 WASM modules (SpacetimeDB microkernel)
```

| Crate | OS Analog | Role |
|:------|:----------|:-----|
| **hex-cli** | Shell | Every `hex` command + MCP tool server for IDE integration |
| **hex-nexus** | System services | Filesystem ops, inference routing, fleet management, dashboard at `:5555` |
| **hex-core** | Syscall interface | 10 port traits — the contracts agents code against (zero deps) |
| **hex-parser** | Compiler | Tree-sitter grammars for TypeScript, Go, and Rust — AST extraction without compilation |
| **hex-agent** | Userland | 18 agent definitions, 20 skills, hooks, architecture enforcement |
| **spacetime-modules** | Microkernel | 7 WASM modules with ~130 reducers for transactional state |

---

## Architecture Enforcement That Agents Can't Bypass

<p align="center">
  <img src="../.github/assets/architecture.svg" alt="Hexagonal Architecture Enforcement" width="800">
</p>

hex enforces [hexagonal architecture](https://alistair.cockburn.us/hexagonal-architecture/) at the **kernel level**. This isn't a linting suggestion — it's a privilege boundary baked into the runtime. Domain can't reach adapters. Adapters can't reach each other. Only the composition root wires them.

The enforcement engine uses **[tree-sitter](https://tree-sitter.github.io/) grammars** to parse source code into ASTs without compiling — extracting every import, export, type definition, and function signature across **TypeScript, Go, and Rust** in milliseconds. Tree-sitter classifies each file into its hexagonal layer (`domain/`, `ports/`, `adapters/primary/`, `adapters/secondary/`, `usecases/`) and then validates that imports respect the dependency direction:

| Rule | Enforced Via |
|:-----|:-------------|
| Domain imports only domain | Import path extraction → layer boundary check |
| Ports import only domain | Cross-layer import validation |
| Adapters never import other adapters | Cross-adapter coupling detection |
| No circular dependencies | Directed graph cycle detection |
| No dead exports | Export scan → consumer trace across all files |
| Composition root is the only wiring point | Adapter import source verification |

Each language maps to its own conventions — Go uses `internal/domain/`, `cmd/`, `pkg/`; TypeScript uses `domain/`, `ports/`, `adapters/`; Rust uses module paths. Tree-sitter extracts **L0 file lists, L1 exports, L2 signatures, and L3 full source** at increasing detail levels, so agents receive exactly the context they need — full adapter context fits in **~200 tokens** instead of pasting entire files.

Each ADR maps to **static analysis rules** that run automatically. `adr-001-domain-purity` checks that the domain layer has zero external imports. `adr-039-spacetimedb-first` flags REST handlers that read from local state instead of SpacetimeDB. Violations are caught at commit time — before agents waste tokens on architectural drift.

```bash
hex analyze .    # Boundary violations, dead code, cross-adapter coupling — blocks commits
```

**Current grade: A+ (100/100)** — 0 boundary violations across 438 source files. Every AI-generated commit maintains this score.

**What this means in practice**: Workplan boundaries map to adapter boundaries. An agent working on `adapters/secondary/database` physically cannot edit `adapters/primary/cli`. Every AI-generated PR maintains the same architectural integrity as hand-crafted code.

---

## Swarm Coordination Without Merge Conflicts

<p align="center">
  <img src="../.github/assets/swarm.svg?v=2" alt="HexFlo Swarm Coordination" width="800">
</p>

HexFlo is the native Rust coordination layer that replaced external Node.js dependencies. Coordination call latency dropped from **~200ms to <1ms**. Agents work in **isolated git worktrees** — one per adapter boundary — so a feature touching 4 boundaries gets 4 parallel agents that never conflict.

Quality gates block tier advancement: domain and ports (Tier 0) must compile before secondary adapters (Tier 1) begin. Tests must pass before integration. Every agent heartbeats every 15 seconds — stale after 45s, dead after 120s, with automatic task reclamation.

```bash
hex swarm init feature-auth    # Spawn parallel agents across boundaries
hex task list                  # Real-time progress via WebSocket
hex task complete <id>         # Mark done — all clients see it instantly
```

**What this means in practice**: No more "two agents claimed the same task." No more zombie agents blocking swarms. Compare-And-Swap task claims prevent double-assignment. Heartbeat timeouts auto-recover from agent crashes. What used to take serial agent passes now runs concurrently with transactional guarantees.

---

## Capability-Based Agent Security

Every agent receives an **HMAC-SHA256 signed capability token** at spawn, scoped to exactly what it needs. Secrets never enter persistent storage — the SpacetimeDB grant table stores only metadata (key names, TTLs). If the database is compromised, attackers see zero secret values.

| Capability | What It Grants |
|:-----------|:---------------|
| `FileSystem(path)` | Read/write within a specific directory only |
| `TaskWrite` | Create and complete swarm tasks |
| `SwarmRead` / `SwarmWrite` | View or modify swarm state |
| `Memory(scope)` | Access scoped key-value store |
| `Inference` | Make LLM API calls through the broker |
| `Notify` | Send agent-to-agent notifications |
| `Admin` | Full system access (daemon agents only) |

**What this means in practice**: A coder agent scoped to `adapters/secondary/` can't touch `adapters/primary/`. A reviewer agent can read everything but write nothing. Daemon agents get admin; worker agents get the minimum they need. Principle of least privilege, enforced at the OS level — not by convention.

---

## Specs-First Pipeline With Independent Oracles

<p align="center">
  <img src="../.github/assets/workflow.svg" alt="7-Phase Development Pipeline" width="800">
</p>

Features follow a **7-phase gated lifecycle**. Behavioral specs are written BEFORE code. Each phase has quality gates — `cargo check` / `tsc --noEmit` between every phase, `cargo test` / `bun test` before integration.

```
Specs --> Plan --> Worktrees --> Code (TDD) --> Validate --> Integrate --> Finalize
```

The validation judge runs behavioral specs as **independent oracles**. This matters because when the same LLM writes code AND tests, the tests can encode the LLM's misunderstanding. Property tests and behavioral specs catch bugs that unit tests miss.

```bash
hex dev start "add user authentication"   # Drives the full pipeline autonomously
```

---

## Real-Time State via SpacetimeDB Microkernel

All coordination state lives in **7 WASM modules** running on SpacetimeDB — not in REST endpoints, not in SQLite, not in memory. State transitions are **atomic reducers**. Every client (CLI, dashboard, MCP tools, remote agents) connects via WebSocket and sees changes in milliseconds.

| Module | Responsibility |
|:-------|:---------------|
| `hexflo-coordination` | Swarms, tasks, agents, memory, fleet, lifecycle, cleanup |
| `agent-registry` | Agent lifecycle, heartbeats, stale detection |
| `inference-gateway` | LLM request routing, procedure-based inference |
| `secret-grant` | TTL-based key distribution, audit log |
| `rl-engine` | Reinforcement learning model selection |
| `chat-relay` | Message routing between agents and users |
| `neural-lab` | Experimental neural patterns |

The nexus daemon is **stateless and horizontally scalable**. Multiple hex-nexus processes can run simultaneously — all coordinating through shared SpacetimeDB. Config syncs from repo files to SpacetimeDB tables on startup; dashboard subscribers get reactive updates.

```bash
hex nexus start              # Start the daemon (requires SpacetimeDB)
hex status                   # Project overview
open http://localhost:5555   # Live dashboard — agents, tasks, health scores
```

---

## Remote Agents in One Command

Deploy agents to remote machines without manual setup:

```bash
hex agent spawn-remote user@build-server.local
```

This handles SSH provisioning, binary transfer, tunnel setup, agent launch, and verification automatically. WebSocket over SSH for bidirectional streaming. Local agents start automatically with `hex nexus start` — zero config for solo developers.

---

## Agent Roles

hex ships with **18 specialized agent definitions** in YAML. Each defines: model selection tiers, context loading strategy (L1 AST summary → L2 signatures → L3 full source), workflow phases, feedback loop gates, and quality thresholds.

| Agent | What It Does |
|:------|:-------------|
| `hex-coder` | Codes within one adapter boundary with TDD loop (red → green → refactor) |
| `planner` | Decomposes requirements into adapter-bounded workplan steps |
| `integrator` | Merges worktrees in dependency order, runs integration tests |
| `swarm-coordinator` | Orchestrates full lifecycle via HexFlo |
| `validation-judge` | Post-build semantic validation — **blocking gate** |
| `behavioral-spec-writer` | Writes acceptance specs before code generation |
| `adversarial-reviewer` | Hunts dangling refs, stale config, build breakage |
| `rust-refactorer` | Rust-specific refactoring with cross-crate awareness |

---

## Example: HexFlo-Coordinated Task Tracker (16 tests, 4 layers)

The [`examples/hex-task-tracker/`](../examples/hex-task-tracker/) shows hex's architecture enforcement on a real app — built using HexFlo swarm coordination with Claude as the inference engine.

**How it was built:**

1. HexFlo swarm created with 4 tasks (one per hex layer)
2. Each task executed by Claude, code written, gate validated
3. Task marked complete in SpacetimeDB via HexFlo PATCH
4. Swarm completed — all 16 tests passing

```
$ rustc --edition 2021 --test src/main.rs -o task-tracker-test && ./task-tracker-test

running 16 tests
test domain::tests::new_task_is_todo ... ok
test domain::tests::todo_to_in_progress ... ok
test domain::tests::in_progress_to_done ... ok
test domain::tests::todo_to_done_invalid ... ok   <- domain enforces transitions
test domain::tests::done_is_terminal ... ok
test domain::tests::cancel_from_todo ... ok
test domain::tests::cancel_from_in_progress ... ok
test domain::tests::revert_to_todo ... ok
test domain::tests::priority_ordering ... ok
test domain::tests::display_includes_all_fields ... ok
test adapters::tests::save_and_find ... ok
test adapters::tests::duplicate_rejected ... ok    <- adapter validates uniqueness
test adapters::tests::find_mut_transition ... ok
test adapters::tests::list_sorted_by_priority ... ok
test adapters::tests::remove_works ... ok
test adapters::tests::not_found ... ok

test result: ok. 16 passed; 0 failed
```

**Architecture enforced by hex:**

```
src/
+-- domain/mod.rs    <- Pure types: Task, Status (with transition rules), Priority
|                      10 tests. ZERO external deps. Cannot import ports or adapters.
|
+-- ports/mod.rs     <- Trait contracts: TaskStore, Command, parse_args
|                      Imports ONLY from domain. Defines WHAT, not HOW.
|
+-- adapters/mod.rs  <- InMemoryTaskStore implementing TaskStore
|                      6 tests. Imports from domain + ports. NEVER imports other adapters.
|
+-- main.rs          <- Composition root -- the ONLY file that imports adapters.
                       Wires InMemoryTaskStore -> TaskStore trait -> CLI commands.
```

Every import boundary is validated by `hex analyze .`. An agent working on `adapters/` physically cannot import from another adapter — the architecture enforcement blocks it at commit time, not in code review.

**Distributed execution proven.** The same task-tracker was also built on a remote GPU box (bazzite) via `hex plan execute` → HexFlo swarm → bazzite worker with local Ollama (qwen2.5-coder:32b). The worker received hex architecture rules + GBNF grammar in every inference call, ran ADR-005 compile gates with error-feedback retry, and reported results back to the Mac coordinator via SSH tunnel. Zero cloud APIs, $0 cost.

See also: [`examples/hex-weather/`](../examples/hex-weather/) for a workplan-driven build with compile gates, [`examples/standalone-pipeline-test/`](../examples/standalone-pipeline-test/) for the inference routing smoke test, and [`docs/remote-agent-walkthrough.md`](remote-agent-walkthrough.md) for the full distributed agent guide.
