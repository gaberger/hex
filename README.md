<p align="center">
  <img src=".github/assets/banner.svg" alt="hex" width="900">
</p>

<p align="center">
  <a href="https://www.rust-lang.org/"><img src="https://img.shields.io/badge/Rust-edition%202021-dea584?style=flat-square&logo=rust&logoColor=white" alt="Rust"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-MIT-3fb950?style=flat-square" alt="License"></a>
  <a href="docs/adrs/INDEX.md"><img src="https://img.shields.io/badge/ADRs-264-bc8cff?style=flat-square" alt="ADRs"></a>
  <img src="https://img.shields.io/badge/tests-973-3fb950?style=flat-square" alt="973 tests">
  <img src="https://img.shields.io/badge/self--grade-A%2B%20100%2F100-3fb950?style=flat-square" alt="A+ 100/100">
  <a href="#what-is-not-proven"><img src="https://img.shields.io/badge/Release-Alpha-bc8cff?style=flat-square" alt="Alpha"></a>
</p>

<p align="center">
  <strong>A solo software-engineering agent, in one binary.</strong><br>
  It writes code, runs your test command, and commits only if that command exits 0.<br>
  No daemon. No database. Nothing to start.
</p>

<p align="center">
  <a href="#quick-start">Quick start</a> ·
  <a href="#the-receipts">Receipts</a> ·
  <a href="ARCHITECTURE.md">Architecture</a> ·
  <a href="docs/adrs/INDEX.md">ADR ledger</a>
</p>

---

## The whole idea

A test command is the only thing that decides whether work is finished.

```bash
hex do run "make add() return a + b, not a - b" \
  --file src/lib.rs --evidence "cargo test --test add"
```

hex reads the code, edits it, runs `cargo test --test add`, and **commits only if it
exits 0**. If it fails, the edit is reverted. A model that wanders commits nothing.

That is the product. Everything else is a bigger version of it.

## Three verbs

| Verb | For |
|---|---|
| **`hex do`** | One bounded change, one file, one gate |
| **`hex build`** | A whole system from a one-line description, built to a gate |
| **`hex scaffold`** | A described project on a deterministic hexagonal floor, gated on the build **and** the architecture grade |
| **`hex harden`** | Point it at working code; it hunts for bugs the tests missed |

```bash
hex scaffold "A bookmark service: SQLite store, axum HTTP API, tag search" \
  --target ./linkstore --lang rust --grade A
```

```
✓ 8 files (rust) — gate: cargo test
✓ floor gate green: cargo test
✓ 2 designs → 2 critiques → spec 33505ch → build GREEN
✓ gate re-run: PASS — 27 test(s) ran
✓ architecture grade: A+ — score 100/100 (floor A)
```

## Why a gate instead of a spec

hex used to be specs-first: write behavioural specs, then code. That was measured and
retired ([ADR-2026-09-11-1900](docs/adrs/ADR-2026-09-11-1900-gate-first-development.md)).

Of 110 specs in this repository, **44 described features that had been deleted months
earlier and nothing ever failed.** Of the 17 that survived an audit, **one named a command
you could run.** A document that cannot fail cannot be trusted, because nothing separates
it from a document that is wrong.

So the pipeline is now:

```
Decide (ADR) → Gate (the command that must exit 0, written first)
            → Diverge (N designs, each red-teamed) → Build to the gate
            → Harden (adversarial hunt, every fix gated) → Ship
```

Three rules:

- **A spec that cannot be run does not exist.** It becomes a gate, or it becomes ADR prose
  — history, which is allowed to be unexecutable because it never claims to describe the present.
- **The gate is written before the code and is not derived from it.** A gate generated from
  the implementation tests the implementation against itself.
- **A vacuous gate is a failed gate.** A suite that exits 0 having run zero tests is not a pass.

## The receipts

Six projects, three languages, each from one description, each gate re-run independently
from a clean build:

| Project | Language | Tests | What it proves |
|---|---|---:|---|
| [`ratelimiter-proof`](examples/ratelimiter-proof) | Rust | 18 | `hex harden` found **3 real bugs** its own 14 passing tests missed |
| [`game-life-rs`](examples/game-life-rs) | Rust | 36 | |
| [`game-ttt-go`](examples/game-ttt-go) | Go | ✓ | Perfect minimax |
| [`game-2048-ts`](examples/game-2048-ts) | TypeScript | 88 | Gate falsified 5 ways before being believed |
| [`url-shortener-rs`](examples/url-shortener-rs) | Rust | 39 | A+ 100/100 |
| [`linkstore-svc`](examples/linkstore-svc) | Rust | 27 | **Real I/O** — axum, SQLite, migration, 12 end-to-end tests |

### The bug `hex harden` is proudest of

The rate limiter's 14 tests all passed. The adversarial pass read a comment that said:

> `u128` cannot overflow on a product of two `u64` values, so this does the job of `checked_mul`.

`Duration::as_nanos` returns a `u128`, not a `u64`. The product overflows, an `as` cast
truncates it, and the result is a rate limiter that **silently limits nothing**.

No spec would have caught that — a spec describes intent, and the intent here was correct.
Only an adversary reading the code finds it.

### Proving the tests aren't decoration

A passing test proves nothing until it can fail. For `linkstore-svc`:

| Sabotage | Result |
|---|---|
| HTTP `201` → `200` in the primary adapter | **4 tests failed** |
| Domain stops stripping tracking parameters | **3 tests failed** |
| Restored | **27 passed, 0 failed** |

## Hexagonal architecture, enforced

hex writes ports-and-adapters code and grades it. `hex analyze` is the grader, and **hex
scores A+ / 100 / 0 violations against its own analyzer.** The daemon it replaced scored
**F (30/100)**.

The grade is a gate, not a report. `hex scaffold --grade A` fails the build if the result
doesn't earn it — because a frontier model will happily hand you a working program whose
use case imports a database driver, and the test suite will pass it.

That gate earned its keep on `linkstore-svc`. With a live SQLite file and a live HTTP
listener, the shortcut is a use case reaching for `rusqlite` or an axum type. It didn't:

```
src/usecases/  →  crate::domain, crate::ports, std::sync::Arc
```

No database, no HTTP, no runtime, in the layer that must not have them.

## Lessons ship as rules, not as advice

Every project `hex init` creates gets a `.hex/ADR-rules.toml` of rules that `hex analyze`
runs. Each one cites the incident that produced it. An unenforced rule is prose with extra
steps, so each has a test that plants a violation and proves it fires.

Then hex was handed its own rule file, and they found four live defects in hex:

- **The health score wrapped.** A `usize` penalty narrowed with `as u8`: 26 boundary
  violations is a penalty of 260, which is `4` in a `u8`. The worst code in the repository
  scored **96/100**, and the score *rose* as violations were added.
- **`hex bootstrap` validated the wrong models** — it checked three model ids the project
  did not configure, while `ready` ignored model checks entirely. An install with no models
  at all reported ready.
- **`hex bootstrap` hung on Linux** — it awaited a foreground server process that never exits.
- **`hex analyze` printed a green tick for a check it had skipped** — "no rules file found"
  and "✓ All ADR rules satisfied", on the same run.

That last one is the lesson committed by the code that reports the lesson. It is the failure
mode this project takes most seriously: **a gate that fails — or passes — for a reason
unrelated to what it gates is indistinguishable from the truth.**

## Local models: the honest picture

hex is model-agnostic (Ollama, vLLM, OpenAI-compatible, Claude). But the *agentic loop* —
multi-turn tool use, not single-shot codegen — is demanding, and we measured it.

**No single local model wins.** A benchmark across the reachable models reordered the best
model on *every* fixture. The top-of-the-leaderboard local model (`gpt-oss:20b`) scored
**last** on our grid. Leaderboard scores do not predict agentic-loop performance.

**The language matters as much as the model.** The same CSV-parser task, three languages,
per-model pass rate:

| Model | Rust | TS | Go |
|---|---|---|---|
| qwen2.5-coder:14b | 0/5 | **2/3** | 0/3 |
| gpt-oss:20b | 0/5 | **1/3** | 0/3 |
| devstral-small-2:24b | 5/5 | 3/3 | 2/3 |
| gemma3:12b | 4/5 | 2/3 | 1/3 |

The lesson isn't "static typing is hard". **TypeScript is uniquely forgiving; Rust *and* Go
are strict.** The two models that recover in TS crash back to 0/3 in Go, where unused
imports are compile errors. So the local ceiling depends heavily on your language — lowest
for TS, highest for Rust and Go.

**So hex doesn't bet on one model.** It runs best-of-N across a complementary pair and falls
back to `claude -p` — your logged-in Claude CLI, no API key, no VRAM ceiling — for tasks the
local models can't finish. The gate, not a classifier, picks the winner, so a mis-route only
costs latency.

If you have a frontier API or a logged-in `claude`, hex is strong. If you're strictly local
on commodity hardware, hex works and inherits the local models' ceiling — and
`hex bench agentic` tells you exactly where that is.

## What is not proven

Two things, stated plainly because the rest of this page is a list of things that are.

**Brownfield.** All six projects above are greenfield. hex has never been pointed at a large
codebase someone else wrote and asked to change it safely. The evidence gate protects a
change; nothing has tested whether hex can find the *right* change in code it did not write.

**`hex analyze` is blind to third-party imports in `domain/`.** Rule 1, which hex writes into
every project it scaffolds, says *"domain imports only domain"*. The analyzer checks
layer-to-layer imports and does not check that — so a project can `use tokio` inside its
domain and still score A+. hex's headline rule is stricter than what hex enforces. Recorded
in [`docs/analysis/2609120100-real-io-proof.md`](docs/analysis/2609120100-real-io-proof.md),
unfixed.

**The code generation is `claude -p`.** hex's contribution is the deterministic floor, the
evidence gate, the architecture grade and the adversarial pass — not the writing. "hex built
a URL shortener" is more precisely "Claude built it and hex proved it was hexagonal and
green." That is not a weakness, but it changes what the claim is.

## Architecture

Eight crates, one binary, ~53k lines. Full detail in
**[ARCHITECTURE.md](ARCHITECTURE.md)** — the living map, always describing HEAD.

| Crate | Role |
|---|---|
| **hex-core** | The contract surface. Zero runtime dependencies |
| **hex-infer** | Every inference adapter, the endpoint registry, tier resolution. The only place a provider or model may be named |
| **hex-exec** | The agent loop, best-of-N, the `claude -p` delegate, the adversarial harness, guarded tools, the local store |
| **hex-graph** | Code knowledge graph → `graph-out/graph.json` |
| **hex-analysis** | Tree-sitter boundary checking + health detectors; powers `hex analyze` |
| **hex-git** · **hex-parser** | git plumbing (libgit2) · parsing |
| **hex-cli** | The binary, and the only composition root |

All state is files: `~/.hex/*.jsonl`, `~/.hex/inference-servers.json`,
`graph-out/graph.json`, `.hex/project.json`, `docs/`.

**2026-08-24 — the `solo` epoch.**
[ADR-2608241500](docs/adrs/ADR-2608241500-collapse-to-solo-software-engineering-agent.md)
deleted the daemon, the SpacetimeDB coordination core, the dashboard, the second agent
binary and the retired SOP pipeline: **~261k lines to ~53k, three processes to one.** What
made the loop good — context assembly and evidence gates — was never in any of it.

## Quick start

```bash
cargo build -p hex-cli --release

hex bootstrap                              # prerequisites, inference server, config
hex init . --scaffold --lang rust          # deterministic hexagonal skeleton + rules

hex do run "<task>" --file <f> --evidence "<cmd>"     # one gated change
hex scaffold "<what to build>" --target <dir> --lang rust --grade A
hex build "<challenge>" --target <dir> --gate "<cmd>" --harden
hex harden <path> --gate "<cmd>"

hex analyze .                              # architecture grade + violations
hex graph consumers <path>                 # trace before you delete
hex bench agentic                          # measure a model through the real loop
```

`hex --help` lists all 26 verbs. `hex go` suggests the next action.
`hex hey <intent>` routes natural language to a verb.

## Governance

- **ADRs are an append-only ledger.** Decisions are never edited or deleted; a changed
  decision gets a new ADR that supersedes the old one. Status changes only via
  `hex adr accept|complete|supersede`.
- **Epochs** group ADRs by design era: `foundation` → `org-sim` *(retired)* →
  `single-agent` → `hybrid-inference` → **`solo`** *(current)*.
- **[ARCHITECTURE.md](ARCHITECTURE.md) is the living map**; the ledger is its history. If
  code or older docs contradict the map, the map wins — and the contradiction is worth an ADR.
- **`founding-goals.md` is the one file agents may not touch.** Editing it needs a human
  commit under CODEOWNERS; retiring a goal needs a Retirement-ADR as well. That rule is the
  only thing separating "the agent decided the goal was obsolete" from "a human did".

## Influences

**`hex-graph`** is graphify-influenced — a GraphRAG-style code graph (typed nodes and edges,
community detection, `EXTRACTED`/`INFERRED`/`AMBIGUOUS` confidence) reimplemented natively in
Rust. The single-agent execution model converges with ideas from **OpenClaw** and **Hermes
Agent** (Nous Research). These shaped the design; the implementation is hex's own.

---

*Day-to-day operating rules live in [CLAUDE.md](CLAUDE.md). Every claim on this page is
checkable against the source, the [ADR ledger](docs/adrs/INDEX.md), `docs/analysis/`, or
`docs/benchmarks/`.*
