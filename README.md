<div align="center">

<img src=".github/assets/banner.svg" alt="hex — Hexagonal Architecture Harness for AI-Driven Development" width="900"/>

<br/>

**Hexagonal architecture enforcement · Native swarm coordination · Specs-first pipeline**

[![Build](https://img.shields.io/badge/build-passing-brightgreen?style=flat-square)](https://github.com/gaberger/hex)
[![Status](https://img.shields.io/badge/status-alpha-yellow?style=flat-square)](https://github.com/gaberger/hex/releases)
[![Rust](https://img.shields.io/badge/rust-1.75%2B-orange?style=flat-square&logo=rust)](https://www.rust-lang.org/)
[![SpacetimeDB](https://img.shields.io/badge/spacetimedb-required-red?style=flat-square)](https://spacetimedb.com/)
[![License](https://img.shields.io/badge/license-MIT-blue?style=flat-square)](LICENSE)
[![Agents](https://img.shields.io/badge/agents-14-purple?style=flat-square)](hex-cli/assets/agents/hex/hex/)

<br/>

**hex** is an AI-Assisted Integrated Development Environment (AAIDE) for hexagonal architecture.
It enforces architectural boundaries at code generation time, coordinates parallel AI agents
via isolated git worktrees, and validates output against behavioral specs before any merge.

</div>

---

> [!IMPORTANT]
> hex requires **SpacetimeDB** (running) and **hex-nexus** (daemon) before any coordination or analysis features work. It also requires more setup than a linter or prompt framework. If you need a fast start with minimal infrastructure, see the [trade-off note](#the-infrastructure-trade-off) before investing time.

---

## Table of Contents

- [Why hex Exists](#why-hex-exists)
- [Quick Start](#quick-start)
- [System Architecture](#system-architecture)
- [Hexagonal Architecture Enforcement](#hexagonal-architecture-enforcement)
- [Specs-First Development Pipeline](#specs-first-development-pipeline)
- [HexFlo: Native Swarm Coordination](#hexflo-native-swarm-coordination)
- [CLI Reference](#cli-reference)
- [Agent Fleet](#agent-fleet)
- [Advanced Capabilities](#advanced-capabilities)
- [Competitive Positioning](#competitive-positioning)
- [Contributing](#contributing)

---

## Why hex Exists

<table>
<tr>
<td width="50%">

**The problem with AI agents at scale**

- Context explosion: agents need the whole codebase to make safe changes
- Boundary drift: agents find the shortest path, not the correct path
- No coordination: parallel agents on a shared working tree produce conflicts
- No validation: generated code compiles but violates behavioral specs
- No memory: each session starts blind — architectural decisions get re-litigated

</td>
<td width="50%">

**What hex provides**

- Adapter-scoped context: agents see only what they need to change
- Hard boundary enforcement: `hex analyze` rejects violations at commit time
- Worktree isolation: one git worktree per agent, merge in dependency order
- Blocking validation gate: `validation-judge` (Opus) must pass before merge
- SpacetimeDB memory: swarm state, task history, and agent memory persist across sessions

</td>
</tr>
</table>

> [!IMPORTANT]
> hex enforces architecture at the point of **code generation**, not after. An agent scoped to a secondary adapter boundary cannot import from another adapter because it never had that code in context. The violation cannot be written because the context for writing it was never provided.

---

## Quick Start

### Prerequisites

| Requirement | Version | Notes |
|---|---|---|
| Rust | 1.75+ | For building hex-cli and hex-nexus |
| SpacetimeDB | 1.x | Must be running before `hex nexus start` |
| Bun | 1.x | For the TypeScript library and tests |
| Claude Code | any | Optional — for MCP integration only |
| Platform | macOS, Linux | Windows: not tested |

### The Infrastructure Trade-Off

hex requires SpacetimeDB (coordination backbone) and hex-nexus (daemon) running alongside your development workflow. This is more infrastructure than a linter or prompt framework. The payoff: architectural compliance verified at every commit, dead-agent task reclamation, and real-time fleet visibility across projects. If your team doesn't need multi-agent coordination or architecture enforcement at the commit level, SPECkit or BAML may be a faster starting point.

### Install and Run

```bash
# 1. Build hex CLI from source
git clone https://github.com/gaberger/hex.git
cd hex
cargo build -p hex-cli --release
export PATH="$PWD/target/release:$PATH"

# 2. Start SpacetimeDB
hex stdb start

# 3. Start hex-nexus daemon
hex nexus start

# 4. Initialize hex in your project
hex init /path/to/your/project

# 5. Smoke test — run architecture analysis
hex analyze /path/to/your/project

# 6. Open the dashboard
open http://localhost:5555
```

### What `hex init` adds to your project

```
your-project/
├── .claude/
│   ├── agents/            # 14 YAML agent definitions (hex-coder, planner, etc.)
│   ├── skills/            # Slash commands (/hex-generate, /hex-scaffold, etc.)
│   └── settings.json      # MCP server config (hex mcp)
├── docs/
│   ├── adrs/              # Architecture Decision Records directory
│   ├── specs/             # Behavioral specs (written before code)
│   └── workplans/         # Feature workplans
├── scripts/
│   └── feature-workflow.sh  # Worktree lifecycle management
└── CLAUDE.md              # Project-specific hex rules and architecture guide
```

### Claude Code Integration

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

All `mcp__hex__*` tools are now available in Claude Code and map 1:1 to CLI commands.

---

## System Architecture

hex is five components working in concert. SpacetimeDB is the backbone — everything else is a client.

```
  ╔══════════════════════════════════════════════════════════╗
  ║            Developer Interface                           ║
  ║   hex CLI  ──────────────────────  Claude Code (MCP)     ║
  ╚══════════════════════════╤═══════════════════════════════╝
                             │  REST (port 5555)
  ╔══════════════════════════▼═══════════════════════════════╗
  ║                   hex-nexus daemon                       ║
  ║                                                          ║
  ║   ┌────────────┐  ┌──────────────┐  ┌────────────────┐  ║
  ║   │  HexFlo    │  │  tree-sitter │  │  hex-dashboard │  ║
  ║   │  (swarm    │  │  arch        │  │  Solid.js      │  ║
  ║   │   coord)   │  │  analysis)   │  │  :5555         │  ║
  ║   └─────┬──────┘  └──────────────┘  └────────────────┘  ║
  ╚═════════╪════════════════════════════════════════════════╝
            │  WebSocket  ▲  real-time subscriptions
  ╔═════════▼═════════════╪════════════════════════════════╗
  ║              SpacetimeDB  ★ REQUIRED ★                 ║
  ║                                                        ║
  ║  18 WASM modules — transactional, zero-copy state      ║
  ║  hexflo-coordination · agent-registry · workplan-state ║
  ║  inference-gateway · chat-relay · fleet-state          ║
  ║  architecture-enforcer · + 11 more                     ║
  ║                                                        ║
  ║  Fallback: SQLite (~/.hex/hub.db) when offline         ║
  ╚════════════════════════════════════════════════════════╝

  ╔═════════════════════════╗   ╔══════════════════════════╗
  ║  hex-agent              ║   ║  Inference Layer         ║
  ║  (per developer machine)║   ║                          ║
  ║  Skills · Hooks · ADRs  ║   ║  inference-gateway WASM  ║
  ║  Workplans · 14 agents  ║   ║  Anthropic · OpenAI      ║
  ║  YAML-declarative       ║   ║  Ollama · OpenRouter     ║
  ╚═════════════════════════╝   ╚══════════════════════════╝
```

| Unit | Language | Where it runs | Required |
|---|---|---|---|
| **SpacetimeDB** | WASM (Rust) | Local or remote server | Yes — coordination backbone |
| **hex-nexus** | Rust (axum) | Local daemon, port 5555 | Yes — filesystem bridge + dashboard |
| **hex-agent** | YAML + Rust hooks | Developer's machine | Yes — enforcement runtime |
| **hex-dashboard** | Solid.js + Tailwind | Served by nexus at :5555 | No — but recommended |
| **Inference** | WASM + Rust bridge | Via nexus HTTP | No — needed for AI agent features |

![hex dashboard](assets/hex-dashboard.png)

*Real-time architecture health, agent fleet, swarm task graph, and hexagonal dependency visualization*

---

## Hexagonal Architecture Enforcement

### The Layer Contract

```
        ┌─────────────────────────────────────┐
        │           composition-root.ts        │  ← only file that sees everything
        └──────────────────┬──────────────────┘
              ┌────────────┼────────────┐
              ▼            ▼            ▼
        ┌──────────┐ ┌──────────┐ ┌──────────┐
        │ adapters │ │ adapters │ │ usecases │
        │ primary/ │ │secondary/│ │          │
        └────┬─────┘ └────┬─────┘ └────┬─────┘
             │             │            │
             └──────┬──────┘            │
                    ▼                   ▼
               ┌─────────┐        ┌─────────┐
               │  ports/ │◀───────│  ports/ │
               └────┬────┘        └─────────┘
                    ▼
               ┌─────────┐
               │ domain/ │  ← zero external deps
               └─────────┘
```

| Layer | May import from |
|---|---|
| `domain/` | `domain/` only |
| `ports/` | `domain/` only |
| `usecases/` | `domain/`, `ports/` |
| `adapters/primary/` | `ports/` only |
| `adapters/secondary/` | `ports/` only |

> [!WARNING]
> Adapters may **never** import other adapters. Cross-adapter coupling is the most common architectural failure in AI-generated code and is unconditionally rejected by `hex analyze`.

`hex analyze` runs tree-sitter over every source file, builds the full import graph, and rejects any edge that crosses a layer boundary. This runs on every commit via pre-commit hook. There is no warning mode — violations fail the build.

<details>
<summary>What hex analyze checks</summary>

```bash
hex analyze <path>              # full boundary + cycle + dead-export check
hex analyze <path> --strict     # warnings become errors
hex analyze <path> --json       # structured output for CI
```

- **Boundary violations** — every import classified by source/target layer
- **Cycle detection** — circular deps within and across layers, full path reported
- **Dead exports** — exported symbols never imported anywhere
- **ADR compliance** — deprecated API patterns flagged per recorded ADRs

</details>

---

## Specs-First Development Pipeline

No code is written without a behavioral spec. No code merges without passing semantic validation. These are not conventions — they are enforced pipeline gates.

```
  Phase 1         Phase 2         Phase 3         Phase 4
 ┌─────────┐     ┌─────────┐     ┌──────────┐     ┌─────────┐
 │  SPECS  │────▶│  PLAN   │────▶│WORKTREES │────▶│  CODE   │
 │         │     │         │     │          │     │(parallel│
 │spec-    │     │planner  │     │one per   │     │ per tier│
 │writer   │     │agent    │     │adapter   │     │ agent)  │
 └─────────┘     └─────────┘     └──────────┘     └────┬────┘
                                                        │
  Phase 7         Phase 6         Phase 5               │
 ┌─────────┐     ┌─────────┐     ┌──────────┐          │
 │FINALIZE │◀────│INTEGRATE│◀────│ VALIDATE │◀─────────┘
 │         │     │         │     │          │
 │cleanup  │     │merge in │     │BLOCKING  │
 │worktrees│     │dep order│     │GATE      │
 └─────────┘     └─────────┘     └──────────┘
                                 validation-judge
                                 (Opus required)
```

### The Worktree Pattern

Each adapter boundary gets an isolated git worktree. Agents cannot see — or conflict with — other agents' work. Merge order follows the dependency tier table.

```bash
./scripts/feature-workflow.sh setup my-feature   # creates worktrees from workplan
./scripts/feature-workflow.sh status my-feature  # live progress
./scripts/feature-workflow.sh merge my-feature   # merge in tier order
./scripts/feature-workflow.sh cleanup my-feature # remove worktrees + branches
```

| Tier | Layer | Parallel? |
|---|---|---|
| 0 | Domain + Ports | No — everything depends on these |
| 1 | Secondary adapters | Yes — isolated per worktree |
| 2 | Primary adapters | Yes — isolated per worktree |
| 3 | Use cases | Yes — after tiers 0–2 |
| 4 | Composition root | No — wires everything |
| 5 | Integration tests | No — full suite |

---

## HexFlo: Native Swarm Coordination

HexFlo is the native Rust coordination layer built into hex-nexus, backed by SpacetimeDB for real-time multi-client state sync. It evolved from ruflo (an external claude-flow-based registry) into a first-class infrastructure component with zero external dependencies.

### Task State Machine

```
Task:   pending ──▶ in_progress ──▶ completed
                         │
                         └──▶ reclaimed (agent dead after 120s)

Agent:  registered ──▶ active ──▶ stale (45s) ──▶ dead (120s)
                                                      │
                                                tasks returned to pending ◀─┘
```

### Spawning an Agent with Task Tracking

```typescript
// Include HEXFLO_TASK:{id} — hooks auto-transition task state via the HexFlo API
Agent({
  prompt: `HEXFLO_TASK:88bb424c-591a-482e-ac4f-55969549b7cf
Implement IFileSystemPort secondary adapter.
Worktree: hex-worktrees-feat-example-p1.1/
Port: src/ports/IFileSystemPort.ts`,
  subagent_type: "general-purpose",
  mode: "bypassPermissions",  // REQUIRED — acceptEdits silently blocks background writes
  run_in_background: true
})
```

> [!WARNING]
> Always use `mode: "bypassPermissions"` for background agents. `acceptEdits` requires a human present to approve each write — background agents will silently produce no output.

### Memory Across Sessions

HexFlo provides scoped persistent memory backed by SpacetimeDB (SQLite fallback). State survives process restarts, reconnects, and session boundaries.

```bash
hex memory store "feature/auth/decision" "Using JWT, not sessions — see ADR-031"
hex memory get "feature/auth/decision"
hex memory search "auth"
```

---

## CLI Reference

<details>
<summary>Daemon & infrastructure</summary>

| Command | Description |
|---|---|
| `hex stdb start` | Start local SpacetimeDB instance |
| `hex stdb status` | Check SpacetimeDB connectivity |
| `hex nexus start` | Start hex-nexus daemon (port 5555) |
| `hex nexus status` | Daemon health + SpacetimeDB connectivity |
| `hex status` | Project status overview |

</details>

<details>
<summary>Architecture analysis</summary>

| Command | Description |
|---|---|
| `hex analyze <path>` | Full boundary, cycle, dead-export check |
| `hex analyze <path> --strict` | Warnings become errors |
| `hex analyze <path> --json` | Structured CI output |
| `hex enforce list` | Show all enforcement rules |
| `hex enforce mode` | Show current mode |

</details>

<details>
<summary>Swarm coordination</summary>

| Command | Description |
|---|---|
| `hex swarm init <name> [topology]` | Initialize swarm |
| `hex swarm status` | Active swarms + task/agent counts |
| `hex task create <swarm-id> <title>` | Create task |
| `hex task list` | All tasks with status |
| `hex task complete <id>` | Mark completed |
| `hex memory store <key> <value>` | Persist memory |
| `hex memory get <key>` | Retrieve memory |
| `hex memory search <query>` | Search memory |

</details>

<details>
<summary>Pipeline & workplans</summary>

| Command | Description |
|---|---|
| `hex spec write` | Start behavioral spec writer |
| `hex plan list` | List workplans |
| `hex plan execute <file>` | Run workplan end-to-end |
| `hex plan status <file>` | Show workplan detail |
| `hex dev` | Interactive TUI development pipeline |

</details>

<details>
<summary>ADRs, agents, inference, secrets</summary>

| Command | Description |
|---|---|
| `hex adr list` | All ADRs with status |
| `hex adr search <query>` | Search by keyword |
| `hex adr abandoned` | Find stale ADRs |
| `hex agent list` | Registered agents |
| `hex inbox list` | Notification inbox |
| `hex inference add` | Register LLM endpoint |
| `hex inference discover` | Scan LAN for endpoints |
| `hex secrets vault set <key> <val>` | Store secret |
| `hex secrets grant <agent> <key>` | Grant agent access |

</details>

---

## Agent Fleet

All 14 agents are defined in YAML (`hex-cli/assets/agents/hex/hex/`), deployed to `.claude/agents/` on `hex init`. Each agent is boundary-scoped and will decline work outside its defined role.

| Agent | Role | Model |
|---|---|---|
| `hex-coder` | Production code within one adapter boundary, TDD loop | Sonnet → Opus (after 3 iterations) |
| `planner` | Decompose requirements into adapter-bounded workplan steps | Sonnet |
| `behavioral-spec-writer` | Machine-readable acceptance specs before any code | Sonnet |
| `validation-judge` | Semantic validation gate — BLOCKING, no bypass | **Opus required** |
| `feature-developer` | Orchestrate the full 7-phase pipeline | Opus |
| `swarm-coordinator` | Spawn and monitor parallel hex-coder agents via HexFlo | Sonnet |
| `integrator` | Merge worktrees in dependency order, run full suite | Sonnet |
| `dead-code-analyzer` | Dead exports, unused ports, boundary violations | Haiku |
| `dependency-analyst` | Tech stack analysis and runtime recommendations | Sonnet |
| `scaffold-validator` | Verify generated projects are actually runnable | Sonnet |
| `adr-reviewer` | Review code for ADR compliance and deprecated APIs | Sonnet |
| `rust-refactorer` | Rust code quality, clippy compliance, performance | Sonnet |
| `status-monitor` | Real-time swarm progress, heartbeat tracking | Haiku |
| `dev-tracker` | Audit trail — commits vs task completions | Haiku |

---

## Advanced Capabilities

The following capabilities are implemented and active in hex's core pipeline.

### Context Pipeline Compression

Long multi-phase pipeline runs accumulate context: prior phase outputs, file reads, tool results, error recovery loops. hex implements a full compression pipeline so agents never silently degrade when context fills.

```
70% pressure → hex inbox warning
80% pressure → PromptCompressor activates
90% pressure → inference blocked, relief strategy executes
```

| Mechanism | Effect |
|---|---|
| `ContextPressureTracker` — running token estimate per session | Agents know how full their context window is |
| Tiered context loading (`load: always / on_demand / active_edit`) | ~60% reduction in initial context load for hex-coder |
| `PromptCompressor` — 3:1 prose compression, code blocks preserved verbatim | Extends effective budget 2–3× on validate/integrate phases |
| Anthropic `cache_control: ephemeral` on static context sections | Up to 90% token savings on repeated TDD feedback loop calls |

Configurable per agent in YAML:
```yaml
token_budget:
  max: 100000
  pressure:
    warn_at_pct: 70
    compress_at_pct: 80
    block_at_pct: 90
    relief: summarize_history   # or: drop_oldest | escalate
```

### Encrypted Secrets Vault

Secrets are encrypted at rest (AES-256-GCM), zeroed from heap on drop (`ZeroizeOnDrop`), and never stored as raw values in SpacetimeDB.

```
SpacetimeDB stores:  vault:ANTHROPIC_API_KEY   ← reference only
SQLite stores:       AES-256-GCM ciphertext    ← key from ~/.hex/vault.key (mode 0600)
In-process:          Zeroizing<String>         ← zeroed after use
```

Every agent access is scoped, time-limited, and single-use. Grant claims appear in real-time on every connected dashboard client:

```bash
hex secrets grant <agent-id> ANTHROPIC_API_KEY   # creates TTL-scoped grant
hex secrets revoke <grant-id>                    # invalidates immediately

# Register frontier providers with key references, never raw values
hex inference add anthropic --model claude-sonnet-4-6 --key-ref ANTHROPIC_API_KEY
hex inference add openai    --model gpt-4o           --key-ref OPENAI_API_KEY --fallback anthropic
```

### Grade A Quality Loop

`hex dev` iterates through a quality gate until output is provably correct — it does not stop at generated code.

```
Code generated
  │
  ├── 1. Compile check  (tsc --noEmit / cargo check)
  │       Fail? → inference fixes compile errors → retry
  │
  ├── 2. Tests          (bun test / cargo test)
  │       Fail? → inference fixes test failures → retry
  │
  ├── 3. hex analyze    (boundary check, cycle detection, dead exports)
  │       Score < 90? → inference fixes violations → retry
  │
  └── Grade A (≥90/100, zero violations) → advance to commit
```

Each gate retries up to 3 times with the actual error output as context. The loop reports cost per iteration:

```
Phase 5: Quality Gate — 3 iterations, $0.003 fix cost
  Compile:    PASS
  Tests:      5/5 passing
  Analyze:    94/100 Grade A  (2 violations fixed automatically)
```

### Haiku Preflight & Automatic Context Compaction

hex implements multi-model orchestration within a single conversation turn — a cheap Haiku classifier gates whether the expensive reasoning model runs and with what context.

- **Startup quota check** — ~50-token Haiku request verifies API connectivity before building any context. Fail-fast in <500ms instead of after a 15k-token context build that then hits a 429.
- **Topic change detection** — Haiku classifies each user input as continuation or new topic. New topic → automatic compaction before the reasoning model sees the input.
- **Automatic compaction at 85%** — no manual `/compact` required.

### OpenRouter: 300+ Models via One API Key

OpenRouter is a provider-of-providers: one API key, one billing dashboard, automatic upstream failover. hex treats it as a first-class inference provider.

```bash
hex inference discover --provider openrouter   # sync available models into SpacetimeDB
hex secrets set OPENROUTER_API_KEY sk-or-...
```

| Model | Context | $/M input | Best for |
|---|---|---|---|
| `meta-llama/llama-4-maverick` | 1M | $0.25 | General coding, large context |
| `meta-llama/llama-4-scout` | 512K | $0.15 | Summarization, batch analysis |
| `deepseek/deepseek-r1` | 128K | $0.55 | Complex reasoning, security review |
| `qwen/qwen3-235b` | 128K | $0.20 | Multilingual, structured output |
| `google/gemini-2.5-pro` | 1M | $1.25 | Long-context analysis |

OpenRouter reports actual cost per request — used directly for budget tracking instead of estimating from token counts. Extended fallback chains:

```
Complex reasoning:   Opus → OpenRouter(deepseek-r1) → Sonnet
Code generation:     Sonnet → OpenRouter(llama-4-maverick) → MiniMax
Budget-constrained:  OpenRouter(llama-4-scout) → Local → Haiku
```

---

### Experimental Capabilities

The following capabilities are accepted in design (ADRs written) but not yet fully implemented. They represent the active development roadmap.

#### RL-Driven Model Selection

hex-nexus runs a Q-learning engine that learns optimal model and context strategy per task type across sessions — no manual tuning.

```
State:  task_type + codebase_size + agent_count + token_usage + rate_limited
Action: model (Haiku / Sonnet / MiniMax / Opus / Local) + context strategy
Reward: success(+1.0) + fast_bonus − rate_limit_penalty − token_cost
```

Fallback chain (triggered on 429, with RL penalty applied):
```
Opus → Sonnet → MiniMax → MiniMaxFast → Haiku → Local → error
```

#### Quantization-Aware Inference Routing

Routes each request to the cheapest local model that meets the quality floor for that task's complexity. A 2-bit local model handles scaffolding; cloud handles cross-file reasoning.

| Tier | Bits | Memory (7B) | Typical use |
|---|---|---|---|
| Q2 | 2 | ~2 GB | Scaffolding, formatting, docstrings |
| Q4 | 4 | ~4.5 GB | General coding, test generation |
| Q8 | 8 | ~8 GB | Complex reasoning, security review |
| FP16 | 16 | ~14 GB | Cross-file planning, novel architecture |
| Cloud | — | — | Frontier tasks (Anthropic / OpenAI) |

#### Goal-Driven Supervisor Loop

The supervisor defines objectives and loops until all are met, re-evaluating everything after every agent action.

```
Objectives: CodeGenerated · CodeCompiles · TestsPass · ReviewPasses · ArchitectureGradeA · UxReviewPasses · DocsGenerated

Iteration 1: CodeCompiles ✗ (3 errors)        → hex-fixer
Iteration 2: TestsPass ✗ (no tests)           → hex-tester
Iteration 3: TestsPass ✗ (2 fail)             → hex-fixer
Iteration 4: CodeCompiles ✗ (fix broke import) → hex-fixer
Iteration 5: All ✓ → advance to next tier
```

#### Neural Lab: Autonomous Architecture Research

Neural network architecture encoded as transactional SpacetimeDB state. Autonomous experiment loop runs via scheduled WASM reducers — no external orchestrator required.

```bash
hex neural-lab experiment create --hypothesis "increase n_embd 512→768"
hex neural-lab frontier          # best config + experiment history
```

Multi-agent research swarms run N mutations in parallel. RL-engine Q-values drive mutation strategy selection. Experiment results update the frontier atomically via SpacetimeDB reducers.

---

## Competitive Positioning

SPECkit and BAML address real sub-problems in AI-assisted development. hex incorporates those sub-problems and adds the enforcement and coordination layer that neither provides.

| Capability | hex | SPECkit | BAML |
|---|:---:|:---:|:---:|
| Specs-first workflow | ✅ | ✅ | ❌ |
| Typed structured LLM output | ✅ | ❌ | ✅ |
| Multi-provider LLM routing + fallback | ✅ | ❌ | ✅ |
| Static architecture boundary enforcement | ✅ | ❌ | ❌ |
| Hexagonal layer isolation at import level | ✅ | ❌ | ❌ |
| Multi-agent swarm coordination | ✅ | ❌ | ❌ |
| Git worktree isolation per agent | ✅ | ❌ | ❌ |
| Dead-agent task reclamation | ✅ | ❌ | ❌ |
| Semantic validation gate before merge | ✅ | ❌ | ❌ |
| Token-efficient AST summaries | ✅ | ❌ | ❌ |
| Fleet management dashboard | ✅ | ❌ | ❌ |
| Persistent session memory | ✅ | ❌ | ❌ |
| MCP server integration | ✅ | ✅ | ✅ |
| YAML-declarative agent behavior | ✅ | ❌ | ❌ |
| Open source | ✅ | ✅ | ✅ |

**The honest summary:**

- **SPECkit** covers Phase 1 of the hex pipeline — specs and planning — with minimal friction and no infrastructure. hex's `behavioral-spec-writer` produces the same artifacts. SPECkit stops at the spec; hex treats the spec as the beginning.
- **BAML** solves the function-level reliability problem for LLM calls extremely well. hex's typed inference port and adapter cover the same ground. BAML has no concept of project architecture, agent coordination, or the development lifecycle.
- **hex's unique territory** — static boundary enforcement, worktree-per-agent isolation, dead-agent task reclamation, and the blocking semantic validation gate — is not addressed by either.

hex requires more infrastructure than either competitor. That infrastructure is the source of its guarantees.

---

## Contributing

hex follows the same specs-first pipeline for its own development.

```bash
# Before writing code
hex adr search <topic>          # check for existing ADR
hex spec write                  # write behavioral spec first
hex plan                        # decompose into workplan

# Build (use debug during iteration)
cargo build -p hex-cli
cargo build -p hex-nexus
bun run build                   # TypeScript library
bun test                        # all tests

# Must pass before commit
hex analyze .
```

**Key constraints:**
- Hexagonal rules are enforced — `hex analyze .` must pass
- TypeScript: all relative imports use `.js` extensions (NodeNext resolution)
- Tests: no `mock.module()` — use dependency injection via the Deps pattern (ADR-014)
- No `.env` commits — use `hex secrets vault` for secrets
- Every new port, adapter, or external dependency requires an ADR first

**Reference:**
- [`CLAUDE.md`](CLAUDE.md) — authoritative system design and behavioral rules
- [`docs/adrs/`](docs/adrs/) — Architecture Decision Records (80+ and growing)
- [`docs/specs/`](docs/specs/) — behavioral specifications
- [`docs/workplans/`](docs/workplans/) — active feature workplans

---

<div align="center">

Built on [Alistair Cockburn's Ports & Adapters pattern](https://alistair.cockburn.us/hexagonal-architecture/) (2005) · Coordination lineage: [ruflo](https://github.com/ruvnet/claude-flow) → HexFlo

</div>
