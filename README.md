<p align="center">
  <img src=".github/assets/banner.svg" alt="hex" width="900">
</p>

<p align="center">
  <a href="https://www.rust-lang.org/"><img src="https://img.shields.io/badge/Rust-edition%202021-dea584?style=flat-square&logo=rust&logoColor=white" alt="Rust"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-MIT-3fb950?style=flat-square" alt="License"></a>
  <a href="docs/adrs/INDEX.md"><img src="https://img.shields.io/badge/ADRs-264-bc8cff?style=flat-square" alt="ADRs"></a>
  <img src="https://img.shields.io/badge/tests-977-3fb950?style=flat-square" alt="977 tests">
  <img src="https://img.shields.io/badge/self--grade-A%2B%20100%2F100-3fb950?style=flat-square" alt="A+ 100/100">
  <a href="#what-is-not-proven"><img src="https://img.shields.io/badge/Release-Alpha-bc8cff?style=flat-square" alt="Alpha"></a>
</p>

<p align="center">
  <strong>A scaffolding system for hexagonal projects, in one binary.</strong><br>
  It creates them, grows them, and stops them drifting — every step gated on a<br>
  command that must exit 0 and an architecture grade that must hold.<br>
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

Most scaffolding tools hand you a folder and wish you luck. The template is right on
day one and wrong by week three, because nothing checks it again.

hex scaffolds a project and then keeps checking. Three things, in order:

**1. A floor that runs.** Deterministic, byte-identical every time, from templates
embedded in the binary. A manifest, a correct ports-and-adapters layout, four passing
tests, and a gate command. No inference involved.

```bash
hex init . --scaffold --lang rust        # skeleton + .hex/ADR-rules.toml
```

**2. Your project, built onto that floor.** A frontier model writes it; two gates
decide whether it counts.

```bash
hex scaffold "A bookmark service: SQLite store, axum HTTP API, tag search" \
  --target ./linkstore --lang rust --grade A
```

```
✓ 8 files (rust) — gate: cargo test
✓ floor gate green: cargo test                      ← before spending a model call
✓ 2 designs → 2 critiques → spec 33505ch → build GREEN
✓ gate re-run: PASS — 27 test(s) ran                ← does it run?
✓ architecture grade: A+ — score 100/100 (floor A)  ← is it what you asked for?
```

**3. Rules that travel with it.** Every scaffolded project gets a
`.hex/ADR-rules.toml`, and `hex analyze` runs it. Each rule cites the incident that
produced it. The scaffold is not a starting point you leave behind — it is the
contract the project is measured against from then on.

### Then you grow it

| Verb | For |
|---|---|
| **`hex scaffold`** | Create or extend a project on the hexagonal floor |
| **`hex build`** | Add a whole subsystem from one description, built to a gate |
| **`hex do`** | One bounded change, one file, one gate |
| **`hex harden`** | Point it at working code; it hunts bugs the tests missed |

Each is the same bargain at a different size: **a command that must exit 0 decides
whether the work counts, and the architecture grade decides whether it belongs.**

```bash
hex do run "make add() return a + b, not a - b" \
  --file src/lib.rs --evidence "cargo test --test add"
```

hex edits, runs your command, and **commits only if it exits 0**. Otherwise the edit
is reverted. A model that wanders commits nothing.

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

Six projects scaffolded from one description each, three languages, every gate re-run
independently from a clean build:

| Project | Language | Tests | What it proves |
|---|---|---:|---|
| [`ratelimiter-proof`](examples/ratelimiter-proof) | Rust | 18 | `hex harden` found **3 real bugs** its own 14 passing tests missed |
| [`game-life-rs`](examples/game-life-rs) | Rust | 36 | Conway, from one sentence |
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

### And one codebase hex did not write

Six greenfield projects prove hex can *build*. They say nothing about whether it
can change code someone else wrote. So: [`weave`](docs/analysis/2609120300-brownfield-trial.md)
— 1,685 files, 495 test files, a different project entirely. Ten realistic
single-token bugs injected into ten files, each verified to break a real test first.

| | Result |
|---|---:|
| **Repair**, given the file and the failing test | **10/10** — every one restoring the original line exactly, **0** test files edited |
| **Localisation**, given only the failing test name | **1/10** — and 0/10 in the top three |

Read those two numbers separately. The first is the easy half: a frontier model
told which file and which test will find a `>` that should be `>=`. The second is
what "point it at a codebase" actually means, and **`hex do run` requires
`--file`** — no verb accepts "this test fails, find why."

## Why the grade is a gate and not a report

This is the part that makes it a scaffolding *system* rather than a generator.

A test suite tells you the code works. It tells you nothing about whether the shape
survived. A frontier model will happily hand you a working program whose use case
imports a database driver — and every test will pass.

So `hex scaffold --grade A` **fails the build** if the result does not earn the grade.
`hex analyze` is the grader, and hex scores **A+ / 100 / 0 violations** against its own
analyzer. The daemon it replaced scored **F (30/100)**.

That gate earned its keep on `linkstore-svc`. With a live SQLite file and a live HTTP
listener, the shortcut is a use case reaching for `rusqlite` or an axum type — and no
pure-function project ever puts that under pressure. It didn't:

```
src/usecases/  →  crate::domain, crate::ports, std::sync::Arc
```

No database, no HTTP, no runtime, in the layer that must not have them.

## What the scaffold carries: rules, not advice

A scaffolded `CLAUDE.md` full of good intentions rots. This repository proved it — the
template shipped into every project told the agent to run five verbs that had been
deleted months earlier, and nothing failed, because prose cannot fail.

So the scaffold ships `.hex/ADR-rules.toml` instead, and `hex analyze` runs it. Each rule
cites the incident that produced it. An unenforced rule is prose with extra steps, so each
has a test that plants a violation and proves it fires. The advice that genuinely cannot be
pattern-matched stays prose — and is labelled as unenforceable, with its reason.

hex was then handed its own rule file, and the rules found four live defects in hex:

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

The brownfield trial found the worst instance of it. In a fresh clone with no
`git config user.email` — the default state of any clone or CI box — hex made a
correct fix, watched its gate pass with 9 tests green, failed to `git commit`,
**reverted the fix**, and reported *"did not pass evidence"*. It destroyed correct
work and blamed the tests. All three commit paths had it; two reverted. Fixed, and
the change is now kept and unstaged with a message naming the half that failed.

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

Stated plainly, because the rest of this page is a list of things that are.

**Localisation.** hex can repair unfamiliar code — 10/10 on the brownfield trial — but only
when you tell it which file. Given just a failing test name it found the right file **once in
ten**, and `hex do run` does not even accept that input. Finding the bug is the half that
matters when you point a tool at a codebase, and it is the half that does not work. The next
thing to build is a verb that takes a failing command and returns ranked candidate files,
gated by actually repairing one.

**Harder brownfield changes.** The trial used single-token bugs with the failing test named
in the prompt. Multi-file changes, bugs that need intent understood across modules, and
anything where the test does not name the concept are all untested.

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
hex bootstrap                       # prerequisites, inference server, config
```

**Scaffold a project**

```bash
# just the floor — deterministic, runnable, carries its own rules
hex init ./myapp --scaffold --lang rust

# the floor plus what you described, gated on the build AND the grade
hex scaffold "<what to build>" --target ./myapp --lang rust --grade A
```

**Grow it**

```bash
hex build "<subsystem>" --target <dir> --gate "<cmd>" --harden
hex do run "<task>" --file <f> --evidence "<cmd>"
hex harden <path> --gate "<cmd>"
```

**Keep it honest**

```bash
hex analyze .                       # architecture grade + rule violations
hex graph consumers <path>          # trace before you delete
hex bench agentic                   # measure a model through the real loop
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
