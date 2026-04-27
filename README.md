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
  <strong>A self-adaptive runtime substrate for AI coding agents — hexagonal architecture, evidence-gated state, adversarial governance, and a closed-loop control system that observes, plans, judges, and applies changes to itself within a bounded autonomy envelope.</strong>
</p>

---

## What hex is

Most AI coding tools are interactive assistants. hex is a **self-adaptive runtime substrate** that *hosts* those tools (Claude Code, Aider, Cursor, local Ollama agents) inside a governed execution model. The model — frontier API, local Ollama, or anything else with an inference port — is hot-swappable. The architecture, audit trail, trust loop, and self-improvement machinery stay constant. The runtime watches itself work, decides when its own design needs to change, and applies those changes through the same gates it uses for application code.

Three claims define hex:

1. **Architecture is enforced, not encouraged.** Tree-sitter parses every commit; cross-layer imports fail the build. `hex-core` (zero external deps), `ports/`, `adapters/`, `usecases/` — the seams are named, machine-checked, and runtime-swappable through a substrate composition root (ADR-2604261303, ADR-2604261500).

2. **Completion is derived from evidence, not self-reported.** Agents claim done; hex doesn't believe them. Every task has a file-evidence gate (`hex-nexus/src/orchestration/workplan_executor.rs::check_evidence_gate`) and a workplan-scoped commit-subject reconciler (`hex plan reconcile --strict`). Lying agents get marked `failed`; a P1 inbox notification fires (ADR-2604270800, ADR-2604271000).

3. **The system adapts itself within a bounded autonomy envelope.** A control loop ticks every 30s observing the running system, plans changes through adversarial competition, judges them against a structured rubric, and applies them via shadow-promotion. Tier-A changes auto-merge; Tier-C halts at the operator. The loop's targets include hex's own ADRs, workplans, port telemetry, and architectural design (ADR-2604261311, ADR-2604271100, ADR-2604271200).

Work is tracked as **workplan JSON** (phases, tasks, adapter boundaries, gates). Architecture decisions are tracked as **ADRs** (189 in tree). State lives in **SpacetimeDB** as an append-only event log. Every coupling has a name, every mutation has a record.

---

## The self-adaptive substrate (MAPE-K, made concrete)

The substrate ADRs (2604261500, 2604261311, 2604261800, 2604262100) describe hex as "the runtime substrate that hosts applications which rewrite themselves under LLM supervision." That isn't marketing — it's a working **MAPE-K** control loop with concrete code paths for each phase:

| MAPE-K phase | What it does | Where it lives | Status |
|---|---|---|---|
| **M**onitor | Read telemetry, ADR registry, workplan state, git, inbox, RL scores, port latency. ~20 detectors as TOML rules. | `hex-cli/assets/improver/detectors.toml`; `hex analyze`; `PortTelemetry` STDB rollups | detector vocabulary scaffolded, telemetry rollup in flight |
| **A**nalyze | Each detector emits `Hypothesis { id, source, scope, severity, evidence }`; deduped by `(source, scope)`. | `hex-cli/src/commands/sched/improver/discover.rs` | scaffolded; full detector wiring in `wp-architectural-health-detectors` |
| **P**lan | Adversarial-swarm spawns N=3 strategic variants per hypothesis at T2.5 (devstral-small-2): "conservative refactor," "aggressive redesign," "minimum viable patch." | `hex-nexus/src/orchestration/adversarial_swarm.rs::propose_strategic` | propose_strategic in `wp-sched-improver` P2 |
| **E**xecute | Structured judge scores variants on 5 axes (alignment, blast-radius, dependency-satisfaction, reversibility, historical-reject-rate); winner is applied via shadow-promotion on a `sched/improver/<id>` worktree branch; losers archived to `docs/workplans/rejected/`. | `hex-nexus/src/orchestration/improver_judge.rs` + `improver_act.rs` | `wp-sched-improver` P3+P4 |
| **K**nowledge | All transitions append to `improver_event` STDB rows: hypothesis text, variants, verdict, action taken, outcome. The judge consults this history (`historical_reject_rate` axis) so the system learns which variant patterns the operator tends to overrule. | STDB `improver_event` table | `wp-sched-improver` P5 |

The autonomy envelope (ADR-2604270800 §1a) is a three-tier table that names exactly which actions the loop may apply without operator consent:

| Tier | Examples | Auto? | Rollback envelope |
|---|---|---|---|
| **A** | Status-frontmatter regex rewrite; trailing whitespace; missing newline; new ADR/workplan in `docs/`; enqueue workplan | yes — shadow-promote → `hex worktree merge` | dedicated branch, single `git revert` |
| **B** | Mutate existing accepted ADR's status; restore broken cross-link | draft only — diff written, P2 inbox for human merge | branch persists until human acts |
| **C** | Modify code outside `docs/`; delete files; mutate two-or-more accepted ADRs at once | never auto — P1 inbox notification | manual review only |

This is the structural answer to "agents wreck repos when given autonomy." hex doesn't trust autonomy; it bounds it.

### What the loop's targets actually are

The improver doesn't only watch application code. It watches **itself**:

- **`hex adr doctor`** scans the ADR registry every tick. Unparseable status → Tier-A auto-fix. Duplicate ID → Tier-C operator review. Stale-Proposed ADR → Tier-B drafted demotion. (ADR-2604270800)
- **`hex plan reconcile --strict`** demotes any task whose stored `done` doesn't match the event log. Removes the multi-writer race that produces false-completes. (ADR-2604271000)
- **`hex substrate telemetry`** rollups detect port latency drift, adapter skew, traffic concentration, idle adapters, swap starvation — the substrate auditing whether its own hot-swap machinery is paying its way. (ADR-2604271200)
- **Architectural detectors** find god-domain-types, kitchen-sink ports, orphan adapters, dead layers, composition drift — design-quality findings, not just compile errors.

The composition root (ADR-2604261303 + cookbook ADR-2604262100) is itself a runtime artifact that the loop can rewrite — adapter swaps go through the same shadow-promotion pipeline application code does.

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
