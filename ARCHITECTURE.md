# hex — Architecture (current state)

> **This is the map, not the ledger.** It describes hex *as it is today* and is
> rewritten freely whenever the design shifts. For *why* a decision was made, read
> the Architecture Decision Records in [`docs/adrs/`](docs/adrs/) — those are an
> append-only history and are never edited to match the present. A generated index
> of every decision, grouped by epoch, lives at
> [`docs/adrs/INDEX.md`](docs/adrs/INDEX.md) (`hex adr reindex`).
>
> **Current epoch: `single-agent`** (since 2026-06-06). Governing decisions (all
> Accepted): [ADR-2606061359](docs/adrs/ADR-2606061359-single-agent-loop-retire-org-sim.md)
> (collapse the org-sim to one agent loop), [ADR-2606071500](docs/adrs/ADR-2606071500-react-tool-use-loop.md)
> (the ReAct execution model), [ADR-2606071340](docs/adrs/ADR-2606071340-nexus-hexagonal-compliance-crate-split.md)
> (nexus hexagonal compliance + crate split), [ADR-2606071323](docs/adrs/ADR-2606071323-autonomous-execution-worktree-isolation.md)
> (autonomous worktree isolation), [ADR-2606071243](docs/adrs/ADR-2606071243-adr-epochs-living-architecture-doc-and-generated-index.md)
> (ADR epochs + this living doc). See [Epochs](#epochs--what-changed) for what this replaced.

## What hex is

hex is a microkernel-based **AI Operating System (AIOS)** built on **hexagonal
architecture** (Ports & Adapters). It installs *into* a target project to orchestrate
AI-driven development: agents are the users, developers are the sysadmins. Everything in
this repo — hooks, skills, agents, statuslines, settings — is instantiated into the
target project. `examples/` holds sample targets that consume hex as a dependency.

The current design is **one strong agent loop fed by tools, code-graph context, and
memory** — *not* a simulated organization of many agents. The differentiator is the
*quality of context* assembled for that single loop (code-graph relevance + ranked
lessons), not agent head-count.

## The execution model — a single ReAct loop

The canonical path is `hex do` → an evidence-gated **ReAct loop** (reason → act →
observe → repeat) over a curated, guarded toolset. The whole loop now lives in the
**`hex-exec`** crate.

```
task + graph context + ranked lessons + windowed file
   → [ compress transcript → inference call w/ curated tools
        → dispatch read/verify tools → append observations → repeat ]
   → terminal: propose_edit → apply → run evidence cmd
        → commit iff exit 0  (else revert + return error to agent)
```

- **Loop & tool protocol** — `hex-exec/src/direct_react.rs` (the ReAct loop) and
  `simple_agent.rs` (native function-calling with a text-mode JSON fallback). The
  single-shot path is `direct_exec.rs`.
- **Curated, guarded tools** — `hex-exec/src/tools/` (20 tools). The ReAct loop exposes
  only read/verify tools (`repo_read`, `repo_grep`, `cargo_check`, `typescript_check`,
  `dep_audit`, `secret_scan`) + the terminal `propose_edit`. No arbitrary shell. Tools
  enforce path-traversal rejection, critical-path blocks, timeouts, output caps.
- **Code-graph context** — `direct_exec.rs::gather_context` loads `graph-out/graph.json`
  and calls `hex_graph::context::context_for(file)` + `rank_lessons` to assemble
  structural context + ranked memory for every run. (`hex-graph` crate; ADR-2606061359.)
- **The evidence gate is the sole authority on what commits** — vacuous passes rejected,
  failed edits reverted atomically. (ADR-2026-06-04-1740, evidence gate ADR-2026-05-19-0720.)
- **Per-run worktree isolation** — `direct_workspace.rs` (ADR-2606071323): autonomous runs
  execute in a dedicated `hex/auto/<id>` worktree off the operator's branch, authored by a
  distinct `hex-factory` identity, with a hard guard against ever committing to the
  operator's tree. Results merge back via `hex worktree merge`. The interactive operator
  path (`hex do`, `isolate:false`) commits on its own branch.
- **Evidence-gated best-of-N across complementary models** — `react_execute_best_of_n`
  (ADR-2606072044): a single attempt is one ReAct loop, but the run iterates an *ordered
  candidate list* (`.hex/project.json → inference.react_models`, default
  `[devstral-small-2:24b, claude-code]`) and commits the **first candidate whose edit passes
  the evidence gate**. The gate — not a classifier — selects the winner, so a mis-route only
  costs latency. Local models have anti-correlated blind spots (measured: `hex bench
  agentic`), so a complementary pair covers what neither does alone.
- **Frontier fallback via `claude -p`** — a `claude-code` candidate delegates the *whole task*
  to the operator's logged-in `claude` CLI (`claude_execute` in `direct_react.rs` — an agent,
  not a per-step completion), inside the same worktree + evidence gate + commit. No API key,
  no VRAM ceiling: local runs free/fast, escalating to Claude only when the locals fail.

### Standalone vs. Claude-mediated

With `CLAUDE_SESSION_ID` unset, nexus drives the loop itself via an Ollama/OpenAI-compatible
adapter — no Claude CLI needed (ADR-2026-04-11-2000). `hex doctor composition` diagnoses the
active variant.

## Workspace crates (post-decomposition — ADR-2606071340)

nexus was a 117k-LOC god-daemon that *failed its own architecture analyzer* (F, 30/100).
It is now decomposed: each reusable bounded context is a crate behind ports, and
**`hex-nexus` is the composition root** — `hex analyze hex-nexus` is **A+ / 0 violations**.

| Crate | Role |
|---|---|
| **hex-core** | The gravity center — shared domain types + **all port traits** (`IStatePort` + sub-traits, `IInferencePort`, …) + the inference-task bus. Zero runtime deps. Every crate depends on it. |
| **hex-graph** | Code-knowledge-graph engine (`KnowledgeGraph`, `context_for`, `rank_lessons`, community/semantic). Builds `graph-out/graph.json`. graphify-influenced (see README). |
| **hex-analysis** | Tree-sitter hexagonal boundary checking, layer classifier, dead-export/cycle detection, ADR conformance. Powers `hex analyze`. |
| **hex-git** | Pure git plumbing (status/log/diff/blame/worktree/correlation) over libgit2. |
| **hex-state** | The SpacetimeDB state adapter — implements `hex_core::ports::state` over STDB's HTTP/reducer surface (`spacetimedb` feature; stub otherwise). |
| **hex-exec** | The single-agent loop: `direct_exec`/`direct_react`/`direct_workspace` + transcript compression + the guarded `tools` library. Depends only on hex-core/graph/git. |
| **hex-nexus** | **Composition root + daemon.** axum/HTTP (primary adapter, `:5555`), the dashboard host (rust-embed), DI wiring, and the daemon-coupled adapters/orchestration (HexFlo coordination, the git poller, routes). The only place that wires adapters together. |
| **hex-cli** | The canonical user entry point — every `hex` verb calls nexus (or runs standalone, e.g. `hex graph consumers`). |
| **hex-agent / hex-parser / hex-desktop** | Architecture-enforcement runtime · parsing · Tauri wrapper for the dashboard. |

> The daemon stays a **single process** (ADR-2606071340 Phase 2) — internally decomposed,
> not split into many services. That preserves the single-agent thesis (fewer moving parts).
> `hex-coordination` (HexFlo) intentionally stays *in* nexus: it broadcasts websocket events
> and owns the agent manager, so it's a daemon adapter, not a reusable library.

### State & coordination core

**SpacetimeDB** (required) is the coordination & state core — WASM modules in
`spacetime-modules/`; clients connect via WebSocket. WASM can't touch the FS / spawn procs /
make network calls — that's why **hex-nexus** exists as the FS-bridge daemon (ADR-025).
Memory is graph-relevant: retrieval ranks lessons by code-graph proximity (commit `38fc9e3f`).

## Tiered inference routing

Tier is driven by a task's `strategy_hint`; T1/T2/T2.5 use best-of-N with a compile gate;
T3 is single-shot. Override per-tier in `.hex/project.json → inference.tier_models`.
(ADR-2026-04-12-0202, ADR-2026-04-13-1630; budget-aware routing ADR-2026-07-10-1000.)

| Tier | Default model | Use case |
|------|---------------|----------|
| T1 | `qwen3:4b` | scaffold / transform / script |
| T2 | `qwen2.5-coder:32b` | standard codegen |
| T2.5 | `devstral-small-2:24b` | complex reasoning |
| T3 | Claude (frontier) | frontier tasks |

The tier table governs the **workplan/SOP path**. The **do-loop** (`hex do`) selects its
model(s) separately via `inference.react_models` (best-of-N + frontier fallback, above),
chosen empirically with **`hex bench agentic`** — a worktree-isolated benchmark that runs
fixtures through the *real* loop and scores per-model pass-rates (ADR-2606071734; corpus in
`docs/benchmarks/`). It exists because external coding-leaderboard scores do **not** predict
agentic-loop performance (measured: the top-leaderboard local model scored last on the grid).

## Hexagonal architecture rules (enforced — and now obeyed)

Checked by `hex analyze` (the `hex-analysis` engine) over both the TypeScript library and
the Rust workspace:

1. `domain/` imports only `domain/`.
2. `ports/` imports `domain/` only (value types).
3. `usecases/` imports `domain/` + `ports/` only.
4. `adapters/` import `ports/` only — never other adapters.
5. The composition root is the only place that imports adapters.

nexus itself passes these now (the `ports → remote/transport` violations were fixed by
relocating the transport contract to `domain/`, and the state-port contract to hex-core).

## Decision governance

- **ADRs are an append-only ledger.** Decisions are never edited to match the present and
  never deleted; a changed decision gets a *new* ADR that supersedes the old one (carrying a
  `Superseded-By:` backlink). Lifecycle: `Proposed → Accepted → Completed`, or
  `→ Rejected | Abandoned`, or `Accepted/Completed → Superseded | Deprecated`. Status changes
  only via `hex adr accept|complete|supersede` / the adr-steward — never free-hand.
- **Epochs** group ADRs by era of design philosophy (`**Epoch:**` field, else derived from
  the decision date). `hex adr reindex` regenerates [`docs/adrs/INDEX.md`](docs/adrs/INDEX.md);
  `hex adr doctor` keeps the ledger self-consistent; `hex graph consumers <module>` is the
  graph-driven dead-code/excision oracle. (ADR-2606071243.)
- **This file (`ARCHITECTURE.md`) is the living map** — point an LLM or new contributor here
  first. Keeping it current is part of the definition-of-done for any epoch-ending capstone ADR.

**Invariant:** *ADRs are git history for decisions — you don't rebase published history;
`ARCHITECTURE.md` is the thing that always describes HEAD.*

## Epochs — what changed

| Epoch | Span | Identity |
|-------|------|----------|
| `foundation` | 2026-03 → 2026-04 | Hexagonal microkernel + SpacetimeDB state core + FS-bridge daemon |
| `org-sim` | 2026-04 → 2026-06-06 | **(retired)** Multi-agent organization simulation: C-suite personas + SOP state machine + autonomous spawn daemon + MAPE-K |
| **`single-agent`** *(current)* | 2026-06-06 → | One gateway-mediated agent loop; code-graph context + memory as the differentiator; nexus decomposed into crates behind ports |

**The org-sim epoch is retired** (ADR-2606061359). The multi-agent "factory" — ~33 persona
agent types, the SOP dispatch state machine, the autonomous spawn daemon, declarative-swarm
YAMLs, and the proposed MAPE-K loop — proved operationally fragile and was collapsed to the
single ReAct loop above. Much of its code has since been excised from nexus (ADR-2606071340
Phase 0); the remaining org-sim ADRs are kept in the ledger as `Superseded` history. **If a
component you find in the code or older docs (personas, the brain/sched spawn daemon, SOP)
contradicts this file, this file wins** — and the contradiction is worth an ADR.

## Build & test

```bash
# Rust workspace (primary)
cargo check -p hex-nexus        # the daemon (the relevant gate; --workspace needs libdbus-1-dev for hex-desktop)
cargo build -p hex-cli --release
hex dev validate                # chains build + test + analyze + specs
hex dev deploy                  # one-command build + install hex → ~/.local/bin + restart daemon (ADR-2606071702)
hex analyze hex-nexus           # the architecture grade (A+)

# TypeScript library (secondary)
bun run build  ·  bun test  ·  bun run check
```

Editing `hex-nexus/assets/*` → rebuild the binary → restart the daemon (`hex nexus stop`
then `hex nexus start`) → hard-refresh the browser.

---
*Operational rules (how to drive hex day-to-day) live in [`CLAUDE.md`](CLAUDE.md). This file
is the architectural map; `CLAUDE.md` is the operator's manual.*
