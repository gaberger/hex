<p align="center">
  <img src=".github/assets/banner.svg" alt="hex — AI Operating System" width="900">
</p>

<p align="center">
  <a href="https://www.rust-lang.org/"><img src="https://img.shields.io/badge/Rust-1.75+-dea584?style=flat-square&logo=rust&logoColor=white" alt="Rust"></a>
  <a href="https://spacetimedb.com/"><img src="https://img.shields.io/badge/SpacetimeDB-WASM-58a6ff?style=flat-square" alt="SpacetimeDB"></a>
  <a href="https://github.com/gaberger/hex/blob/main/LICENSE"><img src="https://img.shields.io/badge/License-MIT-3fb950?style=flat-square" alt="License"></a>
  <a href="docs/adrs/"><img src="https://img.shields.io/badge/ADRs-151_Accepted-bc8cff?style=flat-square" alt="ADRs"></a>
</p>

<p align="center">
  <strong>The operating system for AI agents — local-first, self-improving, zero cloud required.</strong><br>
  <em>Run coding agents on your own hardware. Enforce architecture. Coordinate swarms. Get smarter every run.</em>
</p>

---

## The Problem

AI coding agents are powerful — but they're expensive, uncontrolled, and cloud-dependent. Every agent call hits a frontier API. Every task pays the same price regardless of complexity. A typo fix costs as much as a feature implementation. And when you scale to multiple agents, you get conflicting edits, architecture violations, and no coordination.

**What if 70% of your agent tasks could run on a $0/month local model — with the same quality as frontier?** That's what hex does. It classifies tasks by complexity, routes simple work to fast local models, and only escalates to cloud when the task genuinely needs it. The system learns from every dispatch and gets better over time.

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

All major frameworks are Python-first, polling-based, cloud-dependent, and architecturally ad-hoc. hex is different — it runs on local models out of the box and self-improves over time.

| Framework | Language | Architecture | Local Models | Self-Improving |
|:----------|:---------|:--------------|:-------------|:---------------|
| **LangChain/LangGraph** | Python | Graph-based | Manual setup | No |
| **CrewAI** | Python | Role-based | Ollama only | No |
| **AutoGen/AG2** | Python/.NET | Conversation | Limited | No |
| **Claude Agent SDK** | TypeScript | Tool-first | No | No |
| **OpenHands** | Python | Agent loop | Ollama only | No |
| **hex** | **Rust** | **AIOS** | **Tiered routing + scaffolding** | **RL Q-learning** |

**Why hex is the best local AI agent system:**
- **Runs anywhere without cloud API keys** — Ollama + any GGUF model. T1/T2 tasks (70% of workplan steps) execute entirely on local hardware. Frontier models are optional, not required.
- **Tiered inference routing** — automatically classifies tasks by complexity and routes to the right model: 4B for typo fixes (68 tok/s), 32B for code generation (11 tok/s), frontier only for multi-file features. Not one-size-fits-all.
- **GBNF grammar constraints** — hard token-level masks force models to emit only valid output. A typo fix that takes 89 seconds without grammar takes 31 seconds with it. Same quality, 2.8x faster. No other framework does this.
- **Best-of-N + compile gate** — generates N completions, returns the first that passes `rustc`/`tsc`/`go build`. Observed 100% first-attempt compile rate across Rust, TypeScript, and Go on local 32B models.
- **RL self-improvement** — Q-learning engine in SpacetimeDB records every dispatch outcome and learns optimal model selection per task type. The system gets better the more you use it.
- **Native Rust** — not Python-dependent. Sub-100ms coordination, single binary, no runtime dependencies.
- **SpacetimeDB microkernel** — real-time WebSocket push, not polling. 7 WASM modules with atomic reducers.
- **Hexagonal enforcement** — compile-time boundary checking, not linting suggestions. Agents physically cannot violate architecture rules.

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

### Tiered Inference Routing with RL Self-Improvement

hex classifies every task by complexity and routes it to the right model — local 4B for typo fixes, local 32B for code generation, frontier for multi-file features. The tier→model mapping starts static and **self-optimizes via reinforcement learning** as the system accumulates dispatch outcomes ([ADR-2604120202](docs/adrs/ADR-2604120202-tiered-inference-routing.md)).

| Tier | Model | Task Type | tok/s | Pass Rate | Best-of-N |
|:-----|:------|:----------|------:|----------:|----------:|
| **T1** | qwen3:4b (Q4) | Trivial edits, renames, typo fixes | ~68 | 100% | 1 |
| **T2** | qwen2.5-coder:32b (Q4) | Single function + tests | ~11 | 100% | 3 |
| **T2.5** | qwen3.5:27b (Q4) | Multi-function, agentic | ~11 | 100% | 5 |
| **T3** | Frontier (Claude) | Multi-file features | — | — | 1 |

*Benchmarked on Strix Halo (Vulkan GPU, 32 tok/s peak) with 12 Ollama models over 4 pipeline runs.*

**Best-of-N + Compile Gate**: For T2/T2.5 tasks, hex generates N completions and returns the first that passes `rustc`/`cargo check`. Across 4 full pipeline runs, every task compiled on the first attempt — the scaffolding wasn't even needed. The ADR predicted ~85% one-shot; observed was 100%.

**RL Q-Learning closes the loop.** The SpacetimeDB `rl-engine` module records rewards after every dispatch and updates Q-values via the Bellman equation. After 3 pipeline runs, the learned Q-table:

```
tier:T1|rename_variable    model:qwen3:4b            Q=+1.308  visits=3
tier:T1|fix_typo           model:qwen3:4b            Q=+1.308  visits=3
tier:T2|single_function    model:qwen2.5-coder:32b   Q=+0.110  visits=2
tier:T2|function_w_tests   model:qwen2.5-coder:32b   Q=+0.110  visits=2
tier:T2.5|multi_fn_cli     model:qwen3.5:27b         Q=+0.110  visits=2
```

Local models get a `LOCAL_SUCCESS_BONUS` (+0.1) per successful dispatch, and Q-values compound with each run. The `select_action` reducer uses epsilon-greedy (90% exploit, 10% explore) to occasionally try alternative models — discovering better pairings automatically. When a model fails, its Q-value drops and the router shifts traffic to alternatives.

| Scenario | Frontier-Only | With Tiered Routing | Savings |
|:---------|:-------------|:-------------------|:--------|
| 10-agent swarm (code + analysis) | $22.50 | $2.10 | **91%** |
| Bulk summarization (50 files) | $15.00 | $1.50 | **90%** |
| Mixed interactive + analysis | $8.00 | $3.00 | **63%** |

```bash
hex inference list                              # Available providers + tiers
hex inference discover                          # Scan for local/remote models
hex inference add ollama http://host:11434 --model qwen2.5-coder:32b
```

#### Phase 2: Scaffolding Layer

Phase 1 (tier routing + RL) is live. Phase 2 adds three techniques that close the remaining quality and latency gaps:

**GBNF Grammar Constraints (live).** Local models generate verbose output — a 4B model produces ~5000 tokens of chain-of-thought reasoning for a one-line typo fix (89 seconds). GBNF (GGML BNF) grammars apply a **hard mask on token logits** at decode time, constraining output to only grammar-valid tokens. This isn't a prompt instruction the model can ignore — it's a physical constraint on the decoder.

A/B test results on T1 typo fix (qwen3:4b):

| Metric | Without Grammar | With Grammar | Improvement |
|:-------|:---------------|:-------------|:------------|
| Tokens | 5,096 | 1,968 | **2.6x reduction** |
| Time | 88.6s | 31.2s | **2.8x faster** |
| tok/s | 58.2 | 63.8 | 10% throughput gain |
| Correct | YES | YES | Same quality |

Four built-in grammars ship in `hex-nexus/src/orchestration/grammars.rs`:

| Agent Role | Grammar | Effect |
|:-----------|:--------|:-------|
| `hex-coder` | `CODE_ONLY_RUST` | Pure Rust code block, no prose |
| `planner` | `ANALYSIS` | Structured markdown with required section headings |
| General | `CODE_AND_COMMIT` | JSON: `{"code": "...", "commit_msg": "..."}` |

The grammar field flows through `InferenceRequest.grammar` → `OllamaInferenceAdapter` → Ollama's `/api/generate` `grammar` parameter → llama.cpp GBNF decoder. Other backends ignore the field gracefully.

**Error-Feedback Retry Loop.** When all N compilation attempts fail, the best compiler error is fed back to the model for up to 2 retries. Demonstrated in practice: the weather-cli example's mock provider had a mismatched brace — the `rustc` gate caught it, the error was fed back, and the model fixed it on the next pass. Implemented in `ScaffoldedDispatch::dispatch()` (`hex-nexus/src/orchestration/scaffolding.rs`).

**Cascading Escalation.** When a T2 task exhausts all attempts + retries, the scaffolding layer automatically escalates to frontier via `ScaffoldedDispatch::with_frontier()`. Escalation rates are tracked per task-type in the RL engine — if a task-type escalates >50% of the time, the tier classifier reclassifies it as T3.

### Three-Path Workplan Dispatch — Local, Remote, and Cloud

The workplan executor classifies every task and routes it through the optimal dispatch path:

```
hex plan execute workplan.json
  │
  ├─ Path C (T1/T2/T2.5) ─── headless inference ──→ Ollama (local or remote)
  │   No agent process spawned. Direct inference + GBNF grammar + compile gate.
  │   Fastest path: typo fix in 2.3s, function generation in 10s.
  │
  ├─ Path A (T3 fallback) ─── spawn hex-agent ────→ local process with full tooling
  │   For multi-file features that need filesystem access, git, and tool use.
  │
  └─ Path B (Claude Code) ─── inference queue ────→ Claude session dispatches
      When running inside Claude Code, tasks queue for the outer session.
```

**Path C is the breakthrough.** It eliminates the agent spawning overhead for 70% of workplan tasks. Instead of forking a process, loading tools, and waiting for a shell — the executor sends the prompt directly to Ollama with a GBNF grammar constraint and gets compilable code back in seconds. The inference router picks the best available server automatically, whether it's localhost or a GPU box on your LAN.

**Remote agents work over SSH tunnels.** Connect any machine with Ollama as a compute node:

```bash
# On the remote machine (e.g. a GPU workstation called "bazzite"):
hex agent connect http://nexus-host:5555

# On the coordinator:
hex agent list     # See all agents across your fleet
hex plan execute   # Tasks auto-route to the best available model
```

Tested with a two-node fleet (Mac coordinator + Linux GPU box):

| Where | Model | Task | Time |
|:------|:------|:-----|:-----|
| Bazzite (local Ollama) | qwen3:4b | Rename variable | **2.3s** |
| Mac → Bazzite (network) | qwen3:4b | Rename variable | 4.9s |
| Bazzite (local Ollama) | qwen2.5-coder:32b | Generate function | **10.5s** |
| Mac → Bazzite (network) | qwen2.5-coder:32b | Generate function | 17.3s |

Running the agent directly on the GPU box is **2x faster** — no network round-trip per token. hex supports both topologies: centralized (Mac dispatches to remote Ollama) and distributed (each machine runs its own hex-nexus with local Ollama). The RL engine on each machine learns its own optimal model selection independently.

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
hex adr list                    # 151 Architecture Decision Records
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

## Running hex Standalone — Zero Cloud Dependencies

hex is designed to run as a fully self-sufficient AIOS on local hardware. No API keys, no cloud accounts, no usage-based billing. When `CLAUDE_SESSION_ID` is unset, hex-nexus automatically selects the standalone composition path with Ollama as the default inference adapter ([ADR-2604112000](docs/adrs/ADR-2604112000-hex-standalone-dispatch.md)).

**Minimum hardware**: Any machine that can run Ollama (Mac/Linux/Windows). A 4B model (qwen3:4b) handles T1 tasks on 8GB RAM. A 32B model (qwen2.5-coder:32b) needs 24GB — consumer GPUs like RTX 4090 or Strix Halo APUs. No datacenter required.

```bash
# 1. Install and start Ollama (https://ollama.com)
ollama serve && ollama pull qwen2.5-coder:32b

# 2. Start hex (auto-starts SpacetimeDB + publishes WASM modules)
hex nexus start

# 3. Run the standalone pipeline smoke test
cd examples/standalone-pipeline-test && ./run.sh

# 4. Execute a workplan — entirely on local models
hex plan execute docs/workplans/wp-my-feature.json
```

The pipeline test exercises all tiers end-to-end with real compile gates and RL reward recording:

| Tier | Model | Task | Result | Speed |
|:-----|:------|:-----|:-------|:------|
| T1 | qwen3:4b | Rename variable (Rust) | PASS | 4.9s, 69 tok/s |
| T1 | qwen3:4b | Fix typo (Go) | PASS | 3.5s with GBNF |
| T2 | qwen2.5-coder:32b | Fibonacci (Rust) | PASS, attempt 1/3 | 10.5s |
| T2 | qwen2.5-coder:32b | Palindrome (TypeScript) | PASS, attempt 1/3 | 8.3s |
| T2.5 | qwen3.5:27b | CLI arg parser (Rust) | PASS, attempt 1/5 | 380s |

*9/9 tasks passed across Rust, TypeScript, and Go. All compiled on the first attempt. Tested on Strix Halo with Vulkan GPU.*

Use `hex doctor composition` to diagnose which composition variant is active. Use `--tier T1` for a 10-second smoke test, or `--no-grammar` to compare with/without GBNF constraints.

### Example: HexFlo-Coordinated Task Tracker (16 tests, 4 layers)

The [`examples/hex-task-tracker/`](examples/hex-task-tracker/) shows hex's architecture enforcement on a real app — built using HexFlo swarm coordination with Claude as the inference engine.

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
test domain::tests::todo_to_done_invalid ... ok   ← domain enforces transitions
test domain::tests::done_is_terminal ... ok
test domain::tests::cancel_from_todo ... ok
test domain::tests::cancel_from_in_progress ... ok
test domain::tests::revert_to_todo ... ok
test domain::tests::priority_ordering ... ok
test domain::tests::display_includes_all_fields ... ok
test adapters::tests::save_and_find ... ok
test adapters::tests::duplicate_rejected ... ok    ← adapter validates uniqueness
test adapters::tests::find_mut_transition ... ok
test adapters::tests::list_sorted_by_priority ... ok
test adapters::tests::remove_works ... ok
test adapters::tests::not_found ... ok

test result: ok. 16 passed; 0 failed
```

**Architecture enforced by hex:**

```
src/
├── domain/mod.rs    ← Pure types: Task, Status (with transition rules), Priority
│                      10 tests. ZERO external deps. Cannot import ports or adapters.
│
├── ports/mod.rs     ← Trait contracts: TaskStore, Command, parse_args
│                      Imports ONLY from domain. Defines WHAT, not HOW.
│
├── adapters/mod.rs  ← InMemoryTaskStore implementing TaskStore
│                      6 tests. Imports from domain + ports. NEVER imports other adapters.
│
└── main.rs          ← Composition root — the ONLY file that imports adapters.
                       Wires InMemoryTaskStore → TaskStore trait → CLI commands.
```

Every import boundary is validated by `hex analyze .`. An agent working on `adapters/` physically cannot import from another adapter — the architecture enforcement blocks it at commit time, not in code review.

**Distributed execution proven.** The same task-tracker was also built on a remote GPU box (bazzite) via `hex plan execute` → HexFlo swarm → bazzite worker with local Ollama (qwen2.5-coder:32b). The worker received hex architecture rules + GBNF grammar in every inference call, ran ADR-005 compile gates with error-feedback retry, and reported results back to the Mac coordinator via SSH tunnel. Zero cloud APIs, $0 cost.

See also: [`examples/hex-weather/`](examples/hex-weather/) for a workplan-driven build with compile gates, [`examples/standalone-pipeline-test/`](examples/standalone-pipeline-test/) for the inference routing smoke test, and [`docs/remote-agent-walkthrough.md`](docs/remote-agent-walkthrough.md) for the full distributed agent guide.

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

**Teams running AI coding agents at scale — especially those who want to own their inference.** If you're using Claude Code, Copilot, Cursor, or any LLM-powered coding tool and you've hit these problems:

- Agents violate architecture boundaries and you find out in code review
- Multi-agent runs produce merge conflicts or race conditions
- Every agent has full access to everything — no scoping, no least privilege
- You're paying frontier API prices for tasks a local model could handle
- "It compiles" keeps passing but the app doesn't actually work
- You want to run agents on airgapped networks, on-prem hardware, or without cloud accounts

hex installs into your project as the operating system layer between your agents and your codebase. It's model-agnostic, provider-agnostic, and runs on local models by default — cloud APIs are an optional upgrade for complex tasks, not a requirement. Any machine with Ollama and a 4B+ model can run hex agents autonomously.

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
