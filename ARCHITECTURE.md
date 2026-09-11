# hex — Architecture (current state)

> **This is the map, not the ledger.** It describes hex *as it is today* and is
> rewritten freely whenever the design shifts. For *why* a decision was made, read
> the Architecture Decision Records in [`docs/adrs/`](docs/adrs/) — those are an
> append-only history and are never edited to match the present. A generated index
> of every decision, grouped by epoch, lives at
> [`docs/adrs/INDEX.md`](docs/adrs/INDEX.md) (`hex adr reindex`).
>
> **Current epoch: `solo`** (since 2026-08-24, [ADR-2608241500](docs/adrs/ADR-2608241500-collapse-to-solo-software-engineering-agent.md))
> — hex is one binary that writes hexagonally-correct code against an inference
> server. No daemon, no database, no dashboard, no network peer. It closes
> `hybrid-inference`, whose thesis it keeps and whose apparatus it removes.

## What hex is

**A solo software-engineering agent.** You give it a task, a file, and a command
that must pass. It reads the code, edits it, runs your command, and commits only
if the command exits 0.

```bash
hex do run "make add() return a + b, not a - b" \
  --file src/lib.rs --evidence "cargo test --test add"
```

That is the whole product. Everything below is in service of it.

hex is still built on **hexagonal architecture** (ports and adapters), and still
*enforces* those rules on the code it writes — that is what `hex analyze` is for.
Scaffolded target projects keep the full rule set.

### What it is not, any more

It was an "AI Operating System": a 96k-line daemon, eleven SpacetimeDB WASM
modules, a Solid.js control plane, a second agent binary, a retired SOP pipeline,
and sixteen C-suite persona YAMLs. About 200k of 261k lines existed to serve a
fleet model that [ADR-2606061359](docs/adrs/ADR-2606061359-single-agent-loop-retire-org-sim.md)
had already retired in principle two and a half months earlier. The code simply
stayed.

The measured result of running that fleet was unbounded registry growth,
~100-instance spawn churn on restart, and an SOP path that routed and persisted
work without ever acting on it. Quality came from context and gates, not from
agent head-count.

## The execution model — a single ReAct loop, in process

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
- **Inference is a function call.** `hex-exec` calls `hex-infer` directly. It used
  to POST to `127.0.0.1:5555/api/inference/complete` — a localhost hop into a
  daemon whose handler had no state of its own.
- **Curated, guarded tools** — `hex-exec/src/tools/`. The loop exposes read and
  verify tools (`repo_read`, `repo_grep`, `cargo_check`, `typescript_check`,
  `dep_audit`, `secret_scan`) plus the terminal `propose_edit`. No arbitrary
  shell. Tools reject path traversal, block critical paths, and cap output.
- **Code-graph context** — `gather_context` loads `graph-out/graph.json` and calls
  `hex_graph::context::context_for(file)` + `rank_lessons`. The differentiator is
  the *quality of context* assembled for one loop, not the number of loops.
- **The evidence gate is the sole authority on what commits.** Vacuous passes are
  rejected; failed edits revert atomically.
- **Per-run worktree isolation** — autonomous runs execute in a `hex/auto/<id>`
  worktree off the operator's branch, with a hard guard against committing to the
  operator's tree. The interactive `hex do` path commits on its own branch.
- **Best-of-N across complementary models** — a run walks an ordered candidate
  list (`.hex/project.json → inference.react_models`) and commits the first
  candidate whose edit passes the gate. The gate, not a classifier, picks the
  winner, so a mis-route costs latency and nothing else.
- **Frontier fallback via `claude -p`** — a `claude-code` candidate delegates the
  whole task to the operator's logged-in `claude` CLI, inside the same worktree,
  gate and commit. No API key, no VRAM ceiling.

## The agentic harness

The single loop handles *bounded* work. For whole systems, two verbs run
in-process fan-out of inference calls — no registry, no heartbeat, no peer:

- **`hex build '<challenge>' --target <dir> --gate '<test>'`** — propose N
  divergent designs, red-team each, synthesize one spec, build until the gate
  passes.
- **`hex harden <path> --gate '<test>'`** — hunt for bugs by failure-class lens,
  verify each finding skeptically (default-refute), fix the confirmed ones under
  the gate.
- `hex build --harden` chains them.

They were `hex swarm build` / `hex swarm review`. Only the name changed: "swarm"
described a cluster that was never there.

What keeps this disciplined is that the **test gate is the only authority**, and
the verifier defaults to refuting, so plausible-but-wrong findings die before any
edit. From one-line specs it has built a concurrent durable job queue (WAL + crash
recovery, ~2,900 LOC), a thread-safe LRU+TTL cache, and a token-bucket rate
limiter — the adversarial pass finding 6, 1 and 0 real bugs the builds' own
passing tests missed.

## Workspace crates

| Crate | Role |
|---|---|
| **hex-core** | The contract surface: `IInferencePort` and its mock, the message/tool/validation domain types, the quantization and resource-governor value types. Zero runtime dependencies — that is founding goal G3's test, and it is the reason nothing below can bleed a runtime concern upward. |
| **hex-infer** | Every inference adapter (Ollama, OpenAI-compatible, Anthropic, `claude -p`), the endpoint registry, and tier resolution. **The single enforcement point for G1:** no file outside this crate names a provider or a model. |
| **hex-exec** | The agent loop — `direct_react`, `direct_exec`, per-run worktree isolation, the adversarial harness, transcript compression, the guarded tool library, and the file-backed local store. |
| **hex-graph** | The code knowledge graph: `context_for`, `rank_lessons`, community detection. Builds and reads `graph-out/graph.json`. |
| **hex-analysis** | Tree-sitter boundary checking, the layer classifier, dead-export and cycle detection, ADR conformance, the architecture fingerprint, and the six health detectors. Powers `hex analyze`. |
| **hex-git** | Git plumbing over libgit2. |
| **hex-parser** | Parsing utilities. |
| **hex-cli** | The binary. The only composition root — the one place that wires adapters together. |

There is one process and one binary. `hex-nexus`, `hex-agent`, `hex-desktop`,
`hex-state`, `hex-analyzer` and `spacetime-modules/` are gone.

## State

All of it is files on disk. There is no database.

| What | Where |
|---|---|
| Lessons, gaps, decisions | `~/.hex/memory.jsonl` (`hex memory`) |
| Agent run feed | `~/.hex/runs.jsonl` (`hex do runs`) |
| Token spend | `~/.hex/spend.jsonl` |
| Registered inference backends | `~/.hex/inference-servers.json` (`hex config inference`) |
| Code knowledge graph | `graph-out/graph.json` (`hex graph build`) |
| ADRs, specs, workplans | `docs/` |
| Project config | `.hex/project.json` |

The registry file was always the source of truth: the daemon preloaded
SpacetimeDB from it on every startup ([ADR-2026-04-08-0813](docs/adrs/)), so the
database was downstream of the file. Severing it cost no data and needed no
migration.

## Tiered inference routing

A task's tier selects a model from `.hex/project.json → inference.tier_models`.
Nothing in the source names a model.

| Tier | Use case |
|------|----------|
| T1 | scaffold / transform / script / classification |
| T2 | standard codegen |
| T2.5 | complex reasoning |
| T3 | frontier work, via `claude -p` |

The do-loop selects separately via `inference.react_models`, chosen empirically
with **`hex bench agentic`** — a worktree-isolated benchmark that runs fixtures
through the *real* loop and scores per-model pass rates
([ADR-2606071734](docs/adrs/); corpus in `docs/benchmarks/`). It exists because
external coding-leaderboard scores do not predict agentic-loop performance:
measured here, the top-leaderboard local model scored last on the grid.

## Hexagonal architecture rules (enforced)

Checked by `hex analyze`:

1. `domain/` imports only `domain/`.
2. `ports/` imports `domain/` only (value types).
3. `usecases/` imports `domain/` + `ports/` only.
4. `adapters/` import `ports/` only — never other adapters.
5. The composition root is the only place that imports adapters.

hex obeys them: `hex analyze .` is **A+, 100/100, 0 violations**. The predecessor
daemon scored **F (30/100)** against this same analyzer.

## Decision governance

- **ADRs are an append-only ledger.** A decision is never edited to match the
  present and never deleted; a changed decision gets a *new* ADR that supersedes
  the old one. Lifecycle: `Proposed → Accepted → Completed`, or
  `→ Rejected | Abandoned`, or `→ Superseded | Deprecated`.
- **Epochs** group ADRs by era. `hex adr reindex` regenerates
  [`docs/adrs/INDEX.md`](docs/adrs/INDEX.md); `hex graph consumers <module>` is
  the excision oracle every deletion runs through first.
- **`founding-goals.md` is the one artifact agents may not author or amend.**
  Editing it takes a human commit under CODEOWNERS, and retiring a goal needs a
  Retirement-ADR alongside.
- **This file is the living map.** Point a new contributor or an LLM here first.

**Invariant:** *ADRs are git history for decisions — you don't rebase published
history; `ARCHITECTURE.md` is the thing that always describes HEAD.*

## Epochs — what changed

| Epoch | Span | Identity |
|-------|------|----------|
| `foundation` | 2026-03 → 2026-04 | Hexagonal microkernel + SpacetimeDB state core + FS-bridge daemon |
| `org-sim` | 2026-04 → 2026-06-06 | **(retired)** Multi-agent organization simulation: C-suite personas, SOP state machine, autonomous spawn daemon, MAPE-K |
| `single-agent` | 2026-06-06 → 2026-06-07 | One gateway-mediated agent loop; context and memory as the differentiator; nexus decomposed into crates behind ports |
| `hybrid-inference` | 2026-06-07 → 2026-08-24 | **(matured)** The loop works and is hybrid: best-of-N across complementary local models plus a `claude -p` frontier path; benchmark-driven model choice; the cooperative+adversarial harness |
| **`solo`** *(current)* | 2026-08-24 → | One binary. The daemon, the database, the dashboard, the second agent binary and the SOP pipeline are deleted. ~261k LOC → ~59k; 3 processes → 1. G2 (multi-host scaleout) retired |

**Two epochs are retired, and both for the same reason.** `org-sim` assumed agent
head-count was the axis along which this gets better; `solo` concedes the same
about host count. Each was built, run and measured before it was retired. Their
ADRs stay in the ledger as `Superseded` history.

**If a component you find in the code or in older docs contradicts this file, this
file wins** — and the contradiction is worth an ADR.

## Build & test

```bash
cargo build -p hex-cli --release
cargo test --workspace
hex analyze .              # the architecture grade
hex bench agentic          # per-model pass rates through the real loop
```

Nothing needs to be running. There is no service to start.

---
*Operational rules (how to drive hex day-to-day) live in [`CLAUDE.md`](CLAUDE.md).
This file is the architectural map; `CLAUDE.md` is the operator's manual.*
