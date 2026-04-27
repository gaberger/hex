<p align="center">
  <img src=".github/assets/banner.svg" alt="hex" width="900">
</p>

<p align="center">
  <a href="https://www.rust-lang.org/"><img src="https://img.shields.io/badge/Rust-1.75+-dea584?style=flat-square&logo=rust&logoColor=white" alt="Rust"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-MIT-3fb950?style=flat-square" alt="License"></a>
  <a href="docs/adrs/"><img src="https://img.shields.io/badge/ADRs-189-bc8cff?style=flat-square" alt="ADRs"></a>
  <a href="#status"><img src="https://img.shields.io/badge/Release-Alpha-bc8cff?style=flat-square" alt="Alpha"></a>
</p>

<p align="center">
  <strong>The substrate that hosts AI coding agents — hexagonal architecture, evidence-gated state, adversarial governance, and a self-improvement loop that interrogates its own design every tick.</strong>
</p>

---

## What hex is

Most AI coding tools are interactive assistants. hex is a **runtime substrate** that *hosts* those tools (Claude Code, Aider, Cursor, local Ollama agents) inside a governed execution model. The model — frontier API, local Ollama, or anything else with an inference port — is hot-swappable. The architecture, audit trail, trust loop, and self-improvement machinery stay constant.

Three claims define hex:

1. **Architecture is enforced, not encouraged.** Tree-sitter parses every commit; cross-layer imports fail the build. `hex-core` (zero external deps), `ports/`, `adapters/`, `usecases/` — the seams are named, machine-checked, and runtime-swappable through a substrate composition root.

2. **Completion is derived from evidence, not self-reported.** Agents claim done; hex doesn't believe them. Every task has a file-evidence gate (`hex-nexus/src/orchestration/workplan_executor.rs::check_evidence_gate`) and a workplan-scoped commit-subject reconciler (`hex plan reconcile --strict`). Lying agents get marked `failed`; a P1 inbox notification fires.

3. **The system improves itself within a bounded autonomy envelope.** The sched daemon ticks every 30s and runs reconcile, swarm-cleanup, ADR-doctor, and the improver. The improver discovers drift across three classes — workplan integrity, RL-loop health, and architectural design — generates 3 adversarial variants per hypothesis, scores them on a 5-axis rubric, and ships the winner. Tier-A actions auto-apply via shadow-promotion; Tier-C requires human review.

Work is tracked as **workplan JSON** (phases, tasks, adapter boundaries, gates). Architecture decisions are tracked as **ADRs** (189 in tree). State lives in **SpacetimeDB**. Every coupling has a name, every mutation has a record.

---

## What hex adds vs the field

Most alternatives sit somewhere between "smart autocomplete" and "black-box autonomy." hex covers the gap.

| Category | Examples | What they give you | What hex adds |
|---|---|---|---|
| Per-prompt assistant | Claude Code, Cursor, Copilot | one-shot suggestions, conversational repair | continuous tick loop; evidence-gated state; self-correcting reconciler |
| Git-aware single-shot agent | Aider | edits scoped to a commit | tier routing across model sizes; adversarial best-of-N; architectural import enforcement |
| Black-box autonomy | Devin, AutoGPT, Open Interpreter | "go figure it out" | append-only event log per state transition; shadow-promotion before any swap; explicit autonomy tier table (A/B/C) |
| Local IDE agent | OpenCode, Cline, Continue | local model integration | hexagonal substrate hosts them as adapters behind one port; cross-tool governance |
| Multi-agent orchestrator | AutoGen, CrewAI, LangGraph | task graph + role prompts | structurally enforced hexagonal layout; six-layer adversarial governance; shadow-promote for swaps |
| MCP server / tool surface | hundreds | tool calls into a model | hex *is* an MCP server **and** the runtime that gates which servers are allowed |

The positioning that makes everything click: **hex is a Linux kernel for coding agents, not a fancier user-space tool.** Models, CLIs, IDEs, and orchestrators are processes that run inside hex's address space. They get scheduled, audited, sandboxed, hot-swapped — and the design of their execution is itself rewritable from inside the runtime.

---

## The advantage stack (and what it costs to copy it)

| Capability | Owned mechanism | Why nothing else has it |
|---|---|---|
| Compile gate on agent output | best-of-N + `cargo check` / `tsc --noEmit`; failed candidates feed back into the next attempt | Most agents trust the model. hex doesn't. |
| Layer-boundary enforcement at commit | tree-sitter scan in `hex analyze`; pre-commit hook; CI gate | Hexagonal rules without enforcement aren't rules. |
| Evidence-gated `done` | files-exist + workplan-scoped commit-subject match (ADR-2604270800 P0) | Other systems store `status: "done"` and trust the writer. hex demotes any task without git evidence. |
| Adversarial governance for changes | adversarial-swarm proposes 3 variants; structured judge with 5-axis rubric; shadow-promote (ADR-2604261311) | Single-shot LLM proposals are biased. hex makes them compete. |
| Continuous self-improvement | sched-daemon `tick_improver` discovers → proposes → judges → enqueues; ~20 detectors across operational + architectural classes (ADR-2604271100, ADR-2604271200) | Most "agentic" systems run a loop on user prompts. hex runs a loop without one. |
| Architectural-health interrogation | god-types, port cohesion, adapter skew, latency drift, swap-starvation, composition drift — all become hypotheses | Linters check syntax. hex's improver checks whether the *design* is paying its way. |
| Tier-routed local-first inference | T1 4B / T2 32B / T2.5 24B / T3 frontier; `strategy_hint` selects; compile gate validates | Pricing-driven routing without a verifier produces worse code. hex pairs them. |
| Bounded-autonomy state mutation | Tier A: auto-apply via shadow-promote on a sched/auto-fix branch; Tier B: write fix, P2 inbox; Tier C: P1 inbox, no action | Agents that mutate without rollback envelopes wreck repos. hex's mutations are addressable for `git revert`. |
| Provider-agnostic inference | one `IInferencePort`, multiple adapters (Anthropic, OpenAI, Ollama, OpenRouter); secret-grant via STDB | Tool lock-in is real. hex sees every provider as an adapter. |
| Standalone (Claude-Code-free) operation | `AgentManager` + `OllamaInferenceAdapter` engaged when `CLAUDE_SESSION_ID` is unset; `hex doctor composition` reports active variant | Most agentic systems hard-depend on a frontier API. hex runs on a laptop. |

---

## How a task flows through hex

```
operator prompt or improver-emitted hypothesis
        │
        ▼
classify_work_intent ──► tier routing
        │
        ▼
spec → workplan (JSON, behavioral, machine-checked)
        │
        ▼
hex plan execute  ──► HexFlo swarm dispatches per adapter
        │
        ▼
agent in worktree feat/<wp>/<layer>
        │
        ▼
best-of-N inference  ──► cargo check / tsc --noEmit (blocking)
        │
        ▼
evidence gate: every task.file exists OR commit subject mentions task+wp
        │
        ▼
judge: behavioral spec passes; rubric scores ≥ confidence threshold
        │
        ▼
hex worktree merge (NEVER raw checkout — ADR-2604131930)
        │
        ▼
hex plan reconcile --strict  ──► append-only event log; status derived
        │
        ▼
sched tick loops: ADR-doctor, improver detectors, swarm-cleanup
        │
        ▼
improver discovers next hypothesis ──► back to top
```

Every arrow is an event row. Every state transition is recorded. Operator's role becomes kill-switch + judge-rubric tuning, not per-decision approval.

---

## Quick start

### Docker

```bash
docker run -d --name hex \
  -p 5555:5555 -p 3033:3033 \
  -v $(pwd):/workspace \
  ghcr.io/gaberger/hex-nexus:latest
```

### CLI

```bash
curl -L https://github.com/gaberger/hex/releases/latest/download/hex-darwin-arm64 -o /usr/local/bin/hex
chmod +x /usr/local/bin/hex
hex                           # status + next-step suggestions
hex sched daemon --background --interval 30
```

Dashboard: `http://localhost:5555`. Standalone (no Claude Code, local Ollama): see [Getting Started](docs/GETTING-STARTED.md).

---

## Core commands

```bash
# project state
hex                           # status + next steps
hex analyze .                 # boundary violations + dead code + (soon) architectural detectors
hex adr list                  # 189 decisions in tree
hex adr doctor                # registry health (ADR-2604270800)

# workplans
hex plan draft "<prompt>"     # auto-invoked on T3 prompts
hex plan execute <wp.json>
hex plan reconcile --strict   # workplan-scoped evidence verification

# autonomous loop
hex sched daemon --background --interval 30
hex sched enqueue workplan <wp.json>
hex sched queue list
hex sched scores              # RL routing leaderboard
hex sched improver discover --once   # preview what the loop would propose

# substrate
hex substrate composition     # active adapters behind each port
hex substrate swaps           # shadow-promotion ledger
```

Natural-language dispatch (`hex hey "rebuild and validate"`) routes through the same classifier.

---

## Repository layout

```
hex-cli/              CLI binary, MCP server, tier classifier, improver
hex-nexus/            Daemon (REST API, dashboard, filesystem bridge, orchestration, inference adapters)
hex-core/             Port traits + domain types (zero external deps)
hex-agent/            Architecture-enforcement runtime
hex-parser/           Tree-sitter wrappers
hex-analyzer/         Static-design detectors (orphan, cohesion, god-types, dead-layer)
spacetime-modules/    7 WASM modules: hexflo-coordination, agent-registry, inference-gateway,
                                      secret-grant, rl-engine, chat-relay, neural-lab
docs/adrs/            189 ADRs (the why behind every mechanism)
docs/specs/           Behavioral specs (written before code)
docs/workplans/       Active workplans (state derived from event log)
docs/algebra/         TLA+ specs of coordination, scheduling, feature pipeline (TLC-checked)
```

Two operating modes:

- **Claude-integrated**: `CLAUDE_SESSION_ID` set. Dispatches through Claude Code as one of many possible front-ends.
- **Standalone**: `CLAUDE_SESSION_ID` unset. Dispatches through `AgentManager` + `OllamaInferenceAdapter` (ADR-2604112000). Same workplan executes either way.

`hex doctor composition` reports which is active.

---

## Status

Alpha — but a different kind of alpha than most. Every mechanical claim above has a reproducer in [EVIDENCE.md](docs/EVIDENCE.md): exact command, prerequisites, expected output. The substrate (ADR-2604261500), six-layer governance (ADR-2604261311), evidence gate (ADR-2604270800), workplan state model (ADR-2604271000), self-improvement loop (ADR-2604271100), and architectural-health detectors (ADR-2604271200) are all named and most are partially landed; the chain that closes the operator-asks-nothing loop is the active development frontier. ADR drift, false-done propagation, and detector blind spots are themselves visible in the system as findings the improver will surface — not hidden.

Formal specs live in `docs/algebra/` (TLA+, TLC-model-checked). Benchmarks in [INFERENCE.md](docs/INFERENCE.md) measured on Strix Halo + Vulkan-Ollama; reproducer ships with the doc.

---

## Documentation

| Doc | Contents |
|---|---|
| [Evidence](docs/EVIDENCE.md) | Reproducer for every claim — commands, tests, expected output |
| [Architecture](docs/ARCHITECTURE.md) | Crates, layers, analyzer rules, SpacetimeDB modules |
| [Getting Started](docs/GETTING-STARTED.md) | Install, standalone mode, remote agents |
| [Inference](docs/INFERENCE.md) | Tier routing, GBNF grammar constraints, RL model selection |
| [Comparison](docs/COMPARISON.md) | hex vs SpecKit, BAML, Claude Agent SDK, LangChain, Aider |
| [Developer Experience](docs/DEVELOPER-EXPERIENCE.md) | Pulse / Brief / Console / Override layers |
| [Formal Verification](docs/FORMAL-VERIFICATION.md) | TLA+ models and TLC workflow |
| [Self-improvement](docs/SELF-IMPROVEMENT.md) | Improver loop, detectors, judge rubric, autonomy envelope |
| [ADRs](docs/adrs/) | 189 decision records — the `why` behind each mechanism |

---

## Credits

Builds on hexagonal architecture ([Alistair Cockburn, 2005](https://alistair.cockburn.us/hexagonal-architecture/)), tree-sitter ([Max Brunsfeld et al.](https://tree-sitter.github.io/)), and SpacetimeDB. HexFlo coordination was informed by [claude-flow](https://github.com/ruvnet/claude-flow) (Reuven Cohen). Architecture-fitness-functions inspiration from Ford & Parsons.

| Contributor | Role |
|---|---|
| Gary ([@gaberger](https://github.com/gaberger)) | Creator, architect |
| Claude (Anthropic) | Pair programmer; subject of, and surface of, the trust loop |

## License

[MIT](LICENSE)
