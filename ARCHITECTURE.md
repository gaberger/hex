# hex — Architecture (current state)

> **This is the map, not the ledger.** It describes hex *as it is today* and is
> rewritten freely whenever the design shifts. For *why* a decision was made, read
> the Architecture Decision Records in [`docs/adrs/`](docs/adrs/) — those are an
> append-only history and are never edited to match the present. A generated index
> of every decision, grouped by epoch, lives at
> [`docs/adrs/INDEX.md`](docs/adrs/INDEX.md) (`hex adr reindex`).
>
> **Current epoch: `single-agent`** (since 2026-06-06). Governing decisions:
> [ADR-2606061359](docs/adrs/ADR-2606061359-single-agent-loop-retire-org-sim.md)
> (collapse the org-sim to one agent loop) and
> [ADR-2606071500](docs/adrs/ADR-2606071500-react-tool-use-loop.md) (the ReAct
> execution model). See [Epochs](#epochs--what-changed) for what this replaced.

## What hex is

hex is a microkernel-based **AI Operating System (AIOS)** built on **hexagonal
architecture** (Ports & Adapters). It installs *into* a target project to orchestrate
AI-driven development: agents are the users, developers are the sysadmins. Everything in
this repo — hooks, skills, agents, statuslines, settings — is instantiated into the
target project. `examples/` holds sample targets that consume hex as a dependency.

The current design is **one strong agent loop fed by tools, code-graph context, and
memory** — not a simulated organization of many agents. The differentiator is the
*quality of context* assembled for that single loop (code-graph relevance + ranked
lessons), not agent head-count.

## The execution model — a single ReAct loop

The canonical path is `hex do` → an evidence-gated **ReAct loop** (reason → act →
observe → repeat) over a curated, guarded toolset.

```
task + graph context + ranked lessons + windowed file
        │
        ▼
  ┌──────────────────────────────────────────────┐
  │  ReAct loop  (hex-nexus/src/direct_react.rs)   │
  │  • compress transcript (bounded context)       │
  │  • inference call with curated tool schema     │
  │  • dispatch read/verify tools, append observ.  │
  │  • repeat until terminal action or max_steps   │
  └──────────────────────────────────────────────┘
        │  terminal: propose_edit
        ▼
  apply edit → run evidence command → commit iff exit 0
                                    └ else revert + return error to agent
```

- **Loop & parsing** — `hex-nexus/src/orchestration/simple_agent.rs` provides the
  tool-use protocol: native function-calling with a text-mode JSON fallback
  (`extract_tool_uses` tries Anthropic blocks → OpenAI `tool_calls` → fast-path →
  fenced JSON), dispatch via `ToolRegistry::execute`, and iteration/token budgets.
- **Curated tools (safeguard)** — the loop exposes only read/verify tools (`repo_read`,
  `repo_grep`, `cargo_check`, `typescript_check`, `dep_audit`, `secret_scan`) plus the
  terminal `propose_edit`. No arbitrary shell, no side-effecting/persona tools. Tools in
  `hex-nexus/src/tools/` are already guarded: path-traversal rejection, repo-root
  canonicalization, `is_critical_path` blocks, subprocess timeouts, output caps.
- **Evidence gate** — `direct_exec.rs` owns the apply → evidence → commit guarantee
  (`run_evidence` under `set -o pipefail`, pathspec-scoped commit). The evidence command
  is the *sole* authority on what commits; vacuous passes are rejected. (ADR-2026-06-04-1740,
  ADR-2026-05-19-0720 evidence gate.)
- **Context bound** — a multi-step loop accumulates observations; they are compressed
  each turn (cap + summarize) to stay within the window. Recursive decomposition is noted
  as a future frontier (ADR-2606071500).

### Standalone vs. Claude-mediated

When `CLAUDE_SESSION_ID` is unset, hex-nexus drives the loop itself via `AgentManager` +
an Ollama/OpenAI-compatible inference adapter — no Claude CLI needed (ADR-2026-04-11-2000).
`hex doctor composition` diagnoses the active variant; `hex ci --standalone-gate` validates
the path.

## System components

| Component | Role |
|---|---|
| **SpacetimeDB** (required) | Coordination & state core. WASM modules in `spacetime-modules/`. Clients connect via WebSocket. WASM can't touch the FS / spawn procs / make network calls — that's why hex-nexus exists. (ADR-025) |
| **hex-nexus** (`hex-nexus/`) | Filesystem-bridge daemon. Runs the agent loop, reads/writes files, tree-sitter analysis, git, syncs config → SpacetimeDB on startup (ADR-044), serves the dashboard at `:5555`, exposes the REST API. Editing `assets/` requires a release rebuild. |
| **hex-cli** (`hex-cli/`) | The canonical user entry point. Every runtime capability is a `hex` verb that calls nexus. |
| **hex-agent** (`hex-agent/`) | Architecture-enforcement runtime — enforces hex rules via skills/hooks/ADRs/workplans on any host running dev agents. |
| **hex-dashboard** (`hex-nexus/assets/`) | Solid.js + Tailwind control plane; real-time via SpacetimeDB subscriptions. Redesigned around the single-agent workflow (ADR-2606061359). |
| **hex-core / hex-parser / hex-desktop** | Shared domain types & port traits (zero deps) · code parsing · Tauri wrapper for the dashboard. |
| **Inference** | `inference-gateway` + `inference-bridge` WASM modules route requests; hex-nexus makes the actual HTTP calls. Model-agnostic (Anthropic, OpenAI, Ollama, OpenAI-compatible). |

## Tiered inference routing

Tier is driven by a task's `strategy_hint`. T1/T2/T2.5 use best-of-N with a compile gate;
T3 is single-shot and bypasses scaffolded dispatch. Override per-tier in
`.hex/project.json → inference.tier_models`. (ADR-2026-04-12-0202, ADR-2026-04-13-1630;
budget-aware routing ADR-2026-07-10-1000.)

| Tier | Default model | Use case |
|------|---------------|----------|
| T1 | `qwen3:4b` | scaffold / transform / script |
| T2 | `qwen2.5-coder:32b` | standard codegen (adapters, tests) |
| T2.5 | `devstral-small-2:24b` | complex reasoning (cross-adapter, architecture) |
| T3 | Claude (frontier) | frontier tasks |

## Coordination & memory

State lives in SpacetimeDB (SQLite fallback at `~/.hex/hub.db` for offline). HexFlo
(`hex-nexus/src/coordination/`) still provides swarm/task primitives, but the **default
mode is single-agent**, not a persona organization (ADR-027 is retired — see Epochs).
Memory is graph-relevant: retrieval ranks lessons by code-graph proximity to the task
(Level 1, commit `38fc9e3f`); lessons feed the loop's seed context.

## Hexagonal architecture rules (enforced)

Checked by `hex analyze .` + the dead-code analyzer:

1. `domain/` imports only `domain/`.
2. `ports/` imports `domain/` only (value types).
3. `usecases/` imports `domain/` + `ports/` only.
4. `adapters/primary|secondary/` import `ports/` only.
5. Adapters never import other adapters.
6. `composition-root.ts` is the only file importing adapters.
7. All relative imports use `.js` extensions (NodeNext).

## Decision governance

- **ADRs are an append-only ledger.** Decisions are never edited to match the present and
  never deleted; a changed decision gets a *new* ADR that supersedes the old one (carrying
  a `Superseded-By:` backlink). Lifecycle: `Proposed → Accepted → Completed`, or
  `→ Rejected | Abandoned`, or `Accepted/Completed → Superseded | Deprecated`. Status is
  changed only via `hex adr accept|complete|supersede` / the adr-steward — never free-hand.
- **Epochs** group ADRs by era of design philosophy (`**Epoch:**` field, else derived from
  the decision date). `hex adr reindex` regenerates [`docs/adrs/INDEX.md`](docs/adrs/INDEX.md);
  `hex adr doctor` keeps the ledger self-consistent. (ADR-2606071243.)
- **This file (`ARCHITECTURE.md`) is the living map** — point an LLM or new contributor
  here first. Keeping it current is part of the definition-of-done for any epoch-ending
  capstone ADR.

## Epochs — what changed

| Epoch | Span | Identity |
|-------|------|----------|
| `foundation` | 2026-03 → 2026-04 | Hexagonal microkernel + SpacetimeDB state core + FS-bridge daemon |
| `org-sim` | 2026-04 → 2026-06-06 | Multi-agent organization simulation: C-suite personas + SOP state machine + autonomous spawn daemon + MAPE-K |
| **`single-agent`** *(current)* | 2026-06-06 → | One gateway-mediated agent loop; code-graph context + memory as the differentiator |

**The org-sim epoch is retired** (ADR-2606061359). The multi-agent "factory" — ~33 persona
agent types, the SOP dispatch state machine, the autonomous spawn daemon, declarative-swarm
YAMLs, and the proposed MAPE-K self-improvement loop — proved operationally fragile
(unbounded agent-registry growth, spawn churn, persona dispatch that routed but never
engaged). It was collapsed to the single ReAct loop above. ADRs from that epoch remain in
the ledger as `Superseded` history; do **not** read them as current guidance. If a
component you find in the code or older docs (personas, brain/sched autonomous spawn, SOP)
contradicts this file, **this file wins** — and the contradiction is worth an ADR.

## Build & test

```bash
# Rust (primary)
cargo build -p hex-cli --release
cargo build -p hex-nexus --release
hex dev validate            # chains build + test + analyze + specs

# TypeScript library (secondary)
bun run build               # bundle to dist/
bun test                    # unit + property + smoke
bun run check               # tsc --noEmit
```

Editing `hex-nexus/assets/*` → rebuild the binary → restart the daemon (`hex nexus stop`
then `hex nexus start`) → hard-refresh the browser.

---
*Operational rules (how to drive hex day-to-day) live in [`CLAUDE.md`](CLAUDE.md). This file
is the architectural map; `CLAUDE.md` is the operator's manual.*
