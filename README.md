<div align="center">

<img src=".github/assets/banner.svg" alt="hex — Hexagonal Architecture Harness for AI-Driven Development" width="900"/>

<br/>

**Hexagonal architecture enforcement · Native swarm coordination · Specs-first pipeline**

[![Build](https://img.shields.io/badge/build-passing-brightgreen?style=flat-square)](https://github.com/gaberger/hex)
[![Rust](https://img.shields.io/badge/rust-1.75%2B-orange?style=flat-square&logo=rust)](https://www.rust-lang.org/)
[![SpacetimeDB](https://img.shields.io/badge/spacetimedb-required-red?style=flat-square)](https://spacetimedb.com/)
[![License](https://img.shields.io/badge/license-MIT-blue?style=flat-square)](LICENSE)
[![Agents](https://img.shields.io/badge/agents-14-purple?style=flat-square)](hex-cli/assets/agents/hex/hex/)

<br/>

*hex is not a linter. It is not a prompt framework. It is an enforcement runtime for AI agents —
one that makes architectural violations impossible rather than merely detectable.*

<br/>

![hex dashboard](assets/hex-dashboard.png)

*The hex dashboard: real-time architecture health, agent fleet, swarm task graph, and dependency visualization*

</div>

---

## The Origin: Hexagonal Architecture

In 2005, Alistair Cockburn published the **Ports & Adapters** pattern — what he called "hexagonal architecture." The insight was simple and profound: an application should be equally driveable by users, programs, tests, or batch scripts, and should work in isolation from its eventual runtime devices and databases. You achieve this by separating the *application core* (domain + ports) from the *infrastructure* (adapters) through explicit typed interfaces.

For human developers, this is a design discipline enforced by code review. For AI agents, it is catastrophically easy to violate. An agent given a large codebase in context will find the shortest path to a working change — and that path almost always crosses a boundary.

**hex takes Cockburn's pattern and makes it a hard execution constraint.** Not a linter. Not a convention. A constraint: agents are scoped to one adapter boundary, given only the port interfaces they must satisfy, and their output is verified before it can reach main.

---

## From Ruflo to HexFlo

hex's multi-agent coordination has a lineage worth knowing.

**Ruflo** (built on `claude-flow`) was the first coordination layer: an external registry and tracking system that assigned tasks to agents, persisted state between sessions, and provided the swarm primitives (`swarm_init`, `task_create`, `task_complete`) that proved the model worked. It was the proof of concept.

**HexFlo** is the native evolution. Built in Rust, embedded directly in `hex-nexus`, backed by SpacetimeDB's WASM transactional runtime. Same mental model as ruflo — swarms, tasks, agents, memory — but with zero external dependencies, real-time WebSocket state sync across every connected client, dead-agent reclamation at the heartbeat level, and a fleet dashboard that shows you everything in one place.

The shift from ruflo to HexFlo was the shift from *tracking coordination externally* to *making coordination a first-class infrastructure concern*.

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

### The Five Units

| Unit | Language | Role |
|---|---|---|
| **SpacetimeDB** | WASM (Rust) | Real-time coordination backbone. 18 modules. All state lives here. |
| **hex-nexus** | Rust (axum) | Filesystem bridge. Runs analysis, manages git, serves dashboard. Port 5555. |
| **hex-agent** | YAML + Rust hooks | Architecture enforcement runtime. Lives on the developer's machine. |
| **hex-dashboard** | Solid.js + Tailwind | Control plane. Multi-project, real-time, zero polling. |
| **Inference** | WASM + Rust bridge | Model-agnostic LLM routing. WASM for logic, nexus for HTTP. |

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

| Layer | May import |
|---|---|
| `domain/` | `domain/` only |
| `ports/` | `domain/` only |
| `usecases/` | `domain/`, `ports/` |
| `adapters/primary/` | `ports/` only |
| `adapters/secondary/` | `ports/` only |
| **Adapters may NEVER import other adapters** | |

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

HexFlo is the direct successor to ruflo — the same coordination model (swarms, tasks, agents, memory, heartbeats) rebuilt as a first-class Rust component inside hex-nexus, backed by SpacetimeDB for real-time multi-client state sync.

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

## Quick Start

> [!NOTE]
> SpacetimeDB must be running before `hex nexus start`. hex-nexus will not start without it.

```bash
# 1. Build hex CLI
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

# 5. Check architecture health
hex analyze /path/to/your/project

# 6. Open the dashboard
open http://localhost:5555
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

## Competitive Positioning

SPECkit and BAML address real sub-problems in AI-assisted development. hex either incorporates those sub-problems or assumes them solved, and addresses the layer above.

| Capability | hex | SPECkit | BAML |
|---|:---:|:---:|:---:|
| Specs-first workflow | ✅ JSON behavioral specs | ✅ Markdown templates | ❌ |
| Typed structured LLM output | ✅ via port interfaces | ❌ | ✅ core product |
| Multi-provider LLM routing + fallback | ✅ | agent-dependent | ✅ |
| Static architecture boundary enforcement | ✅ tree-sitter, hard fail | ❌ | ❌ |
| Hexagonal layer isolation at import level | ✅ every commit | ❌ | ❌ |
| Multi-agent swarm coordination | ✅ HexFlo + SpacetimeDB | ❌ | ❌ |
| Git worktree isolation per agent | ✅ ADR-004 | ❌ | ❌ |
| Dead-agent task reclamation | ✅ 45s stale / 120s dead | ❌ | ❌ |
| Semantic validation gate before merge | ✅ validation-judge blocking | ❌ | ❌ |
| Token-efficient AST summaries | ✅ tree-sitter L0–L3 | ❌ | ❌ |
| Fleet management dashboard | ✅ real-time, multi-project | ❌ | ❌ |
| Persistent session memory | ✅ SpacetimeDB / SQLite | ❌ | ❌ |
| MCP server integration | ✅ | ✅ AGENTS.md | ✅ ActionRunner |
| YAML-declarative agent behavior | ✅ 14 agents, 7 swarms | partial | ❌ |
| Open source | ✅ | ✅ MIT | ✅ Apache 2.0 |

**The honest summary:**

- **SPECkit** covers Phase 1 of the hex pipeline — specs and planning — with minimal friction. hex's `behavioral-spec-writer` agent produces the same artifacts. SPECkit stops at the spec; hex treats the spec as the beginning.
- **BAML** solves the function-level reliability problem for LLM calls extremely well. hex's typed inference port and adapter cover the same ground. BAML has no concept of project architecture, agent coordination, or the development lifecycle.
- **hex's unique territory** — static boundary enforcement, worktree-per-agent isolation, dead-agent task reclamation, and the blocking semantic validation gate — is not addressed by either.

The trade-off is real: hex requires more infrastructure (SpacetimeDB running, hex-nexus daemon) than either competitor. The payoff is the only guarantee in this comparison that architectural compliance is verified at the point of merge.

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
- [`docs/adrs/`](docs/adrs/) — all Architecture Decision Records
- [`docs/specs/`](docs/specs/) — behavioral specifications
- [`docs/workplans/`](docs/workplans/) — active feature workplans

---

<div align="center">

Built on [Alistair Cockburn's Ports & Adapters pattern](https://alistair.cockburn.us/hexagonal-architecture/) · Coordination lineage from [ruflo](https://github.com/ruvnet/claude-flow) → HexFlo

</div>
