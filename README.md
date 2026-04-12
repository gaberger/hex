<p align="center">
  <img src=".github/assets/banner.svg" alt="hex — AI Operating System" width="900">
</p>

<p align="center">
  <a href="https://www.rust-lang.org/"><img src="https://img.shields.io/badge/Rust-1.75+-dea584?style=flat-square&logo=rust&logoColor=white" alt="Rust"></a>
  <a href="https://spacetimedb.com/"><img src="https://img.shields.io/badge/SpacetimeDB-WASM-58a6ff?style=flat-square" alt="SpacetimeDB"></a>
  <a href="https://github.com/gaberger/hex/blob/main/LICENSE"><img src="https://img.shields.io/badge/License-MIT-3fb950?style=flat-square" alt="License"></a>
  <a href="docs/adrs/"><img src="https://img.shields.io/badge/ADRs-145_Accepted-bc8cff?style=flat-square" alt="ADRs"></a>
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

| Tool | What It Does | What It Doesn't |
|:-----|:-------------|:----------------|
| **BAML** | Typed LLM functions, schema validation | No agent lifecycle, no orchestration, no architecture rules |
| **SpecKit** | Spec-driven workflow gates | No runtime enforcement, no code execution, can't stop agents that ignore specs |
| **HUD** | Agent benchmarks, RL evaluation, A/B testing | No code generation, no architecture enforcement, doesn't ship with your code |
| **hex** | **Full AIOS** — process lifecycle, enforced boundaries, swarm coordination, RL inference, capability auth | — |

hex is the **runtime that sits underneath all of them**. It manages agent processes like an OS manages user processes — with lifecycle tracking, capability-based permissions, enforced boundaries, and coordinated resource access.

---

### Agent Framework Comparison (2026)

All major frameworks are Python-first, polling-based, with ad-hoc architecture. hex is different:

| Framework | Language | Architecture | State | hex Advantage |
|:----------|:---------|:--------------|:------|:-------------|
| **LangChain/LangGraph** | Python | Graph-based | Polling + RAG | Rust + WASM, <1ms coordination |
| **CrewAI** | Python | Role-based | Polling + memory | SpacetimeDB WebSocket push |
| **AutoGen/AG2** | Python/.NET | Conversation | Message passing | Hexagonal compile-time enforcement |
| **Claude Agent SDK** | TypeScript | Tool-first | Polling | Self-improving model selection (Brain) |
| **hex** | **Rust + TypeScript** | **AIOS** | **SpacetimeDB** | All of the above + native |

**Why hex wins:**
- **Native Rust** — Not Python-dependent, sub-100ms response times, embedded binary
- **SpacetimeDB** — Real-time WebSocket push vs polling
- **Hexagonal** — Compile-time boundary enforcement (not linting suggestions)
- **Brain** — RL-based self-improving model selection
- **HexFlo** — Zero external dependencies for swarm coordination

---

## What You Get

### Architecture Enforcement That Agents Can't Bypass

<p align="center">
  <img src=".github/assets/architecture.svg" alt="Hexagonal Architecture Enforcement" width="800">
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

**What this means in practice**: Workplan boundaries map to adapter boundaries. An agent working on `adapters/secondary/database` physically cannot edit `adapters/primary/cli`. Every AI-generated PR maintains the same architectural integrity as hand-crafted code.

### Swarm Coordination Without Merge Conflicts

<p align="center">
  <img src=".github/assets/swarm.svg?v=2" alt="HexFlo Swarm Coordination" width="800">
</p>

HexFlo is the native Rust coordination layer that replaced external Node.js dependencies. Coordination call latency dropped from **~200ms to <1ms**. Agents work in **isolated git worktrees** — one per adapter boundary — so a feature touching 4 boundaries gets 4 parallel agents that never conflict.

Quality gates block tier advancement: domain and ports (Tier 0) must compile before secondary adapters (Tier 1) begin. Tests must pass before integration. Every agent heartbeats every 15 seconds — stale after 45s, dead after 120s, with automatic task reclamation.

```bash
hex swarm init feature-auth    # Spawn parallel agents across boundaries
hex task list                  # Real-time progress via WebSocket
hex task complete <id>         # Mark done — all clients see it instantly
```

**What this means in practice**: No more "two agents claimed the same task." No more zombie agents blocking swarms. Compare-And-Swap task claims prevent double-assignment. Heartbeat timeouts auto-recover from agent crashes. What used to take serial agent passes now runs concurrently with transactional guarantees.

### 90% Inference Cost Reduction via RL

The reinforcement learning engine learns which `(model, context_strategy)` pair performs best per task type, then routes automatically:

| Tier | Bits | Typical Tasks | Examples |
|:-----|:-----|:-------------|:---------|
| **Q2** | 2-bit | Scaffolding, docstrings, formatting | Local Llama 3.2 |
| **Q4** | 4-bit | General coding, test generation | Ollama, vLLM |
| **Q8** | 8-bit | Complex reasoning, security review | MiniMax M2.5 |
| **Fp16** | 16-bit | Cross-file planning, architecture | Cloud Sonnet |
| **Cloud** | — | Frontier reasoning | Opus, GPT-4o |

The RL state space encodes: task type, codebase size, agent count, token usage, rate limit status, and retry count. The reward function penalizes cost, rewards speed and quality. It **self-optimizes over time**.

| Scenario | Frontier-Only | With RL Routing | Savings |
|:---------|:-------------|:---------------|:--------|
| 10-agent swarm (code analysis) | $22.50 | $2.10 | **91%** |
| Bulk summarization (50 files) | $15.00 | $1.50 | **90%** |
| Mixed interactive + analysis | $8.00 | $3.00 | **63%** |

On top of model selection: **prompt caching saves 90% on input tokens** for repeated system prompts (~15k tokens) and tool definitions (~8k tokens). Haiku preflight checks detect quota/key issues in **<500ms** before building full context — costing ~$0.000013 per check.

```bash
hex inference discover --provider openrouter   # Scan 300+ models
hex inference list                              # Available providers + tiers
hex inference test <provider-id>                # Verify connectivity
```

### Capability-Based Agent Security

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

### Specs-First Pipeline With Independent Oracles

<p align="center">
  <img src=".github/assets/workflow.svg" alt="7-Phase Development Pipeline" width="800">
</p>

Features follow a **7-phase gated lifecycle**. Behavioral specs are written BEFORE code. Each phase has quality gates — `cargo check` / `tsc --noEmit` between every phase, `cargo test` / `bun test` before integration.

```
Specs → Plan → Worktrees → Code (TDD) → Validate → Integrate → Finalize
```

The validation judge runs behavioral specs as **independent oracles**. This matters because when the same LLM writes code AND tests, the tests can encode the LLM's misunderstanding. Property tests and behavioral specs catch bugs that unit tests miss.

```bash
hex dev start "add user authentication"   # Drives the full pipeline autonomously
```

### Real-Time State via SpacetimeDB Microkernel

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

### Remote Agents in One Command

Deploy agents to remote machines without manual setup:

```bash
hex agent spawn-remote user@build-server.local
```

This handles SSH provisioning, binary transfer, tunnel setup, agent launch, and verification automatically. WebSocket over SSH for bidirectional streaming. Local agents start automatically with `hex nexus start` — zero config for solo developers.

---

## Quick Start

```bash
# Build from source
cargo build -p hex-cli --release
cargo build -p hex-nexus --release

# Start (requires SpacetimeDB running)
hex nexus start
hex status

# Open the live dashboard
open http://localhost:5555

# Install into a target project
cd your-project && hex init
```

### Essential Commands

```bash
# Architecture enforcement
hex analyze .                   # Boundary check, dead code, coupling violations
hex adr list                    # 145 Architecture Decision Records
hex adr search "inference"      # Find relevant decisions

# Autonomous development
hex dev start "<description>"   # Full 7-phase pipeline
hex swarm init <name>           # Manual swarm initialization
hex task list                   # Track all tasks in real-time

# Inference management
hex inference discover          # Scan for local/remote models
hex inference list              # Available providers + tiers
hex inference add ollama http://localhost:11434 llama3.2:3b-q4_k_m

# Memory & coordination
hex memory store <key> <value>  # Persistent scoped key-value
hex inbox list                  # Priority notification inbox
hex secrets status              # Vault health check
```

---

## Running hex Standalone (without Claude Code)

hex can run as a fully self-sufficient AIOS without Claude Code installed. When `CLAUDE_SESSION_ID` is unset, hex-nexus automatically selects the standalone composition path with Ollama as the default inference adapter (ADR-2604112000).

```bash
# 1. Install and start Ollama (https://ollama.com)
ollama serve && ollama pull llama3.2:3b-q4_k_m

# 2. Start hex
hex nexus start

# 3. Execute a workplan
hex plan execute wp-my-feature.json
```

Use `hex doctor composition` to diagnose which composition variant is active and verify prerequisites. See [ADR-2604112000](docs/adrs/ADR-2604112000-hex-standalone-dispatch.md) for the full design decision.

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

### Agent Roles

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

## Who Is This For?

**Teams running AI coding agents at scale.** If you're using Claude Code, Copilot, Cursor, or any LLM-powered coding tool and you've hit these problems:

- Agents violate architecture boundaries and you find out in code review
- Multi-agent runs produce merge conflicts or race conditions
- Every agent has full access to everything — no scoping, no least privilege
- You're paying frontier API prices for tasks a local model could handle
- "It compiles" keeps passing but the app doesn't actually work

hex installs into your project as the operating system layer between your agents and your codebase. It's model-agnostic, provider-agnostic, and works with any AI coding tool that supports MCP.

---

## Documentation

| Resource | What You'll Find |
|:---------|:-----------------|
| [Architecture Decision Records](docs/adrs/) | 107 decisions with rationale — the "why" behind every design choice |
| [Development Guides](docs/guides/) | Workflow walkthrough, OpenRouter setup, feature UX integration |
| [Behavioral Specs](docs/specs/) | Feature specifications written before code |
| [Workplans](docs/workplans/) | Structured task decomposition driving HexFlo swarm execution |
| [Gap Analysis](docs/analysis/) | Honest assessment of what's built vs. what's planned |

---

## Credits & References

### Foundational Work

hex builds on the **Hexagonal Architecture** pattern (Ports & Adapters), originally conceived by [Alistair Cockburn](https://alistair.cockburn.us/hexagonal-architecture/) in 2005:

> *"Allow an application to equally be driven by users, programs, automated test or batch scripts, and to be developed and tested in isolation from its eventual run-time devices and databases."*

- **[Hexagonal Architecture](https://alistair.cockburn.us/hexagonal-architecture/)** — Alistair Cockburn
- **[Growing Object-Oriented Software, Guided by Tests](http://www.growing-object-oriented-software.com/)** — Steve Freeman & Nat Pryce (London-school TDD)
- **[Clean Architecture](https://blog.cleancoder.com/uncle-bob/2012/08/13/the-clean-architecture.html)** — Robert C. Martin

### Key Technologies

- **[tree-sitter](https://tree-sitter.github.io/)** — Max Brunsfeld et al. (AST-based architecture enforcement — parses TypeScript, Go, and Rust without compiling to extract imports, exports, and layer boundaries)
- **[SpacetimeDB](https://spacetimedb.com/)** — real-time database with WASM module execution
- **[claude-flow](https://github.com/ruvnet/claude-flow)** — Reuven Cohen (@ruvnet), multi-agent swarm coordination (predecessor to HexFlo)

### Authors

| Contributor | Role |
|:------------|:-----|
| **Gary** ([@gaberger](https://github.com/gaberger)) | Creator, architect, primary developer |
| **Claude** (Anthropic) | AI pair programmer — code generation, testing, documentation |

---

## License

[MIT](LICENSE)
