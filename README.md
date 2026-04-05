<p align="center">
  <img src=".github/assets/banner.svg" alt="hex — AI Operating System" width="900">
</p>

<p align="center">
  <a href="https://www.rust-lang.org/"><img src="https://img.shields.io/badge/Rust-1.75+-dea584?style=flat-square&logo=rust&logoColor=white" alt="Rust"></a>
  <a href="https://spacetimedb.com/"><img src="https://img.shields.io/badge/SpacetimeDB-WASM-58a6ff?style=flat-square" alt="SpacetimeDB"></a>
  <a href="https://github.com/gaberger/hex/blob/main/LICENSE"><img src="https://img.shields.io/badge/License-MIT-3fb950?style=flat-square" alt="License"></a>
  <a href="https://github.com/gaberger/hex/actions"><img src="https://img.shields.io/badge/CI-passing-3fb950?style=flat-square&logo=github-actions&logoColor=white" alt="CI"></a>
</p>

<p align="center">
  <strong>The operating system for AI agents.</strong><br>
  <em>Manage processes. Enforce architecture. Coordinate swarms. Route inference.</em>
</p>

---

## The Problem

AI coding agents produce working code — until they don't. Without guardrails, agents drift: they violate architecture boundaries, create circular dependencies, duplicate logic across layers, and make decisions no human reviewed. Scale to multiple agents and the problem compounds — conflicting edits, race conditions, no coordination.

**Existing tools solve parts of this.** None solve the whole thing.

<p align="center">
  <img src=".github/assets/comparison.svg" alt="hex vs BAML, SpecKit, HUD" width="800">
</p>

**BAML** gives you typed LLM functions — great for structured output, but no agent lifecycle, no orchestration, no architecture rules. It's a calling convention, not a runtime.

**SpecKit** adds spec-driven workflow gates — but it's a process layer. It doesn't execute code, enforce boundaries at runtime, or manage agent processes. When the agent ignores the spec, SpecKit can't stop it.

**HUD** benchmarks and evaluates agents — essential for training, but it doesn't ship with your code. It tells you how good your agents are, not how to keep them good in production.

**hex is the runtime that sits underneath all of them.** It manages agent processes like an OS manages user processes — with lifecycle tracking, capability-based permissions, enforced boundaries, and coordinated resource access.

---

## What You Get

### Agents That Can't Break Architecture

<p align="center">
  <img src=".github/assets/architecture.svg" alt="Hexagonal Architecture Enforcement" width="800">
</p>

hex enforces **hexagonal architecture at the kernel level**. Domain can't reach adapters. Adapters can't reach each other. Only the composition root wires them. This isn't a linting rule — it's a privilege boundary. Agents physically cannot import across boundaries they don't have access to.

```bash
hex analyze .    # Real-time architecture health check — violations block commits
```

> **Why it matters**: Every AI-generated PR maintains the same architectural integrity as hand-crafted code. No more "the agent added a database call inside the domain layer" surprises.

### Multi-Agent Swarms That Don't Collide

<p align="center">
  <img src=".github/assets/swarm.svg" alt="HexFlo Swarm Coordination" width="800">
</p>

HexFlo coordinates parallel agents across **isolated git worktrees** — one per adapter boundary. Agents work simultaneously on domain, ports, and adapters without merge conflicts. Quality gates block tier advancement until compile + test + architecture analysis pass.

```bash
hex swarm init feature-auth    # Spawn parallel agents across boundaries
hex task list                  # Track progress in real-time
```

> **Why it matters**: A feature that touches 4 adapter boundaries gets 4 parallel agents, each in an isolated worktree, each blocked from producing code that breaks the build. What used to take serial agent passes now runs concurrently.

### Capability-Based Agent Security

Every agent receives an **HMAC-signed capability token** at spawn, scoped to exactly what it needs:

| Capability | What It Grants |
|:-----------|:---------------|
| `FileSystem(path)` | Read/write within a specific directory |
| `TaskWrite` | Create and complete swarm tasks |
| `SwarmRead` / `SwarmWrite` | View or modify swarm state |
| `Memory(scope)` | Access scoped key-value store |
| `Inference` | Make LLM API calls |
| `Notify` | Send agent-to-agent notifications |
| `Admin` | Full system access (daemon only) |

> **Why it matters**: A coder agent scoped to `adapters/secondary/` can't touch `adapters/primary/`. A reviewer agent can read everything but write nothing. Principle of least privilege, enforced at the OS level.

### RL-Driven Inference That Learns

The reinforcement learning engine tracks which `(model, context_strategy)` pair performs best per task type, then routes requests to the optimal provider:

| Tier | Bits | Typical Tasks |
|:-----|:-----|:-------------|
| **Q2** | 2-bit | Scaffolding, docstrings, formatting |
| **Q4** | 4-bit | General coding, test generation |
| **Q8** | 8-bit | Complex reasoning, security review |
| **Fp16** | 16-bit | Cross-file planning, architecture |
| **Cloud** | — | Frontier APIs (Anthropic, OpenAI, OpenRouter) |

> **Why it matters**: Stop paying cloud API prices for boilerplate generation. The RL engine automatically downgrades simple tasks to local models and upgrades complex reasoning to frontier models — adapting as it learns your codebase.

### Specs-First Development Pipeline

<p align="center">
  <img src=".github/assets/workflow.svg" alt="7-Phase Development Pipeline" width="800">
</p>

Features follow a **7-phase gated lifecycle**. Behavioral specs are written BEFORE code. Each phase has quality gates that block advancement until compile + test + architecture checks pass.

```
Specs → Plan → Worktrees → Code (TDD) → Validate → Integrate → Finalize
```

> **Why it matters**: "It compiles" is not "it works." The validation judge runs behavioral specs as independent oracles — catching bugs that unit tests miss because they were written by the same agent that wrote the code.

### Real-Time Coordination via SpacetimeDB

All state lives in **7 WASM modules** running on SpacetimeDB. Every client — CLI, dashboard, MCP tools, remote agents — connects via WebSocket and sees changes instantly. When one agent completes a task, every other agent knows immediately.

```bash
hex nexus start              # Start the daemon
hex status                   # Project overview
open http://localhost:5555   # Live dashboard
```

> **Why it matters**: No polling. No stale state. No "two agents claimed the same task." SpacetimeDB provides transactional guarantees with real-time subscriptions — the same foundation that powers multiplayer games, applied to multi-agent development.

---

## Quick Start

```bash
# Build
cargo build -p hex-cli --release
cargo build -p hex-nexus --release

# Start (requires SpacetimeDB)
hex nexus start
hex status

# Open the dashboard
open http://localhost:5555
```

### Core Commands

```bash
# Architecture
hex analyze .                   # Health check — boundary violations, dead code
hex adr list                    # Architecture Decision Records

# Swarm coordination
hex swarm init <name>           # Initialize a swarm
hex task create <sid> <title>   # Create a task
hex task list                   # Track all tasks
hex task complete <id>          # Mark done

# Memory & IPC
hex memory store <key> <value>  # Persistent key-value store
hex inbox list                  # Agent notification inbox
hex inbox notify                # Send priority notification

# Inference
hex inference list              # Available models
hex inference discover          # Scan for local models
```

---

## System Architecture

```
hex-cli/               Rust CLI — shell + MCP server
hex-nexus/             Daemon — REST API, dashboard, filesystem bridge
hex-core/              Domain types + 9 port traits (zero deps)
hex-agent/             Agent runtime — skills, hooks, enforcement
hex-desktop/           Desktop app (Tauri)
spacetime-modules/     7 WASM modules (SpacetimeDB microkernel)
```

| Crate | OS Analog | What It Does |
|:------|:----------|:-------------|
| **hex-cli** | Shell | Every `hex` command + MCP tool server |
| **hex-nexus** | System services | Filesystem ops, inference routing, fleet management, dashboard |
| **hex-core** | Syscall interface | Port traits — the contracts agents code against |
| **hex-agent** | Userland | Skills, hooks, agent YAML definitions, architecture enforcement |
| **spacetime-modules** | Microkernel | Transactional state: swarms, agents, inference, secrets, RL |

---

## Who Is This For?

**Teams running AI coding agents at scale.** If you're using Claude Code, Copilot, Cursor, or any LLM-powered coding tool and want:

- Architectural boundaries that agents can't violate
- Multi-agent coordination without merge conflicts
- Per-agent security scoping (not just "the AI has access to everything")
- Inference cost optimization with automatic model selection
- A specs-first pipeline that catches semantic bugs, not just syntax errors

hex installs into your project and acts as the operating system layer between your agents and your codebase.

---

## Documentation

| Resource | Description |
|:---------|:------------|
| [Architecture Decision Records](docs/adrs/) | 70+ design decisions with rationale and status |
| [Behavioral Specs](docs/specs/) | Feature specifications and acceptance criteria |
| [Workplans](docs/workplans/) | Structured task decomposition for features |
| [Examples](examples/) | Reference apps built with hex (Flappy Bird, weather, todo, etc.) |

---

## License

[MIT](LICENSE)
