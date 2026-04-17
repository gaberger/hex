<p align="center">
  <img src=".github/assets/banner.svg" alt="hex" width="900">
</p>

<p align="center">
  <a href="https://www.rust-lang.org/"><img src="https://img.shields.io/badge/Rust-1.75+-dea584?style=flat-square&logo=rust&logoColor=white" alt="Rust"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-MIT-3fb950?style=flat-square" alt="License"></a>
  <a href="docs/adrs/"><img src="https://img.shields.io/badge/ADRs-182-bc8cff?style=flat-square" alt="ADRs"></a>
  <a href="#status"><img src="https://img.shields.io/badge/Release-Alpha-bc8cff?style=flat-square" alt="Alpha"></a>
</p>

<p align="center">
  <strong>A Rust runtime that routes coding-agent tasks to the cheapest model that can pass a compile gate,
  enforces hexagonal boundaries at commit time, and reconciles agent work against git evidence.</strong>
</p>

---

## What hex is

hex is a local-first runtime for AI coding agents. It sits between an agent (Claude Code, or a local Ollama model in standalone mode) and your codebase, and does three concrete things:

1. **Classifies every task into a tier and routes it to a model of matching size.** A prompt's `strategy_hint` (`scaffold` / `codegen` / `inference`) and structural heuristics pick T1 (4B), T2 (32B), T2.5 (24B), or T3 (frontier). See `hex-cli/src/commands/hook.rs::classify_work_intent`.

2. **Wraps model output in a compile gate.** For T1/T2/T2.5 it generates best-of-N completions and only accepts a candidate that passes `cargo check` or `tsc --noEmit`. Compiler errors from failed candidates are fed back into the next attempt. See `hex-nexus/src/remote/transport.rs`.

3. **Validates hexagonal architecture on every commit.** `hex analyze` parses TypeScript and Rust with tree-sitter, classifies each file into a layer (`domain` / `ports` / `adapters` / `usecases`), and fails if a cross-layer import violates the dependency direction. See `hex-cli/src/commands/analyze.rs`.

Work is tracked as a **workplan JSON** (phases, tasks, adapter boundaries, gates). Completion is not self-reported — the reconciler walks `git log` and requires non-empty `evidence.commits[]` before a task is considered done (`hex plan reconcile`).

---

## Why it exists

AI agents write code that compiles locally and fails at integration. They violate layering boundaries that aren't enforced by the build. They report "done" on work they didn't do. And cloud inference pricing doesn't scale when you run many agents.

hex addresses these with mechanical checks, not promises:

| Failure mode | hex response |
|---|---|
| Agent writes non-compiling code | Best-of-N + compile gate (cannot be accepted without passing `cargo check`/`tsc --noEmit`) |
| Agent violates layer boundaries | Tree-sitter import scan blocks commits with cross-adapter imports |
| Agent self-reports false completion | Reconciler requires git commits touching the task's files |
| Frontier API cost per task | Tier classifier routes boilerplate to local 4B/32B models; escalates only on failure |
| Two agents edit the same file | HexFlo worktree-per-adapter + CAS task claims |

---

## Quick start

### Docker (recommended)

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
```

Dashboard: `http://localhost:5555`. Full install + standalone (Ollama-only) setup: [Getting Started](docs/GETTING-STARTED.md).

---

## How a feature flows through hex

```
user prompt ──► classify_work_intent ──► tier
                                          │
                      T1: answered in-session (TodoWrite)
                      T2: one-line suggestion in hook output
                      T3: auto-drafts docs/workplans/drafts/draft-*.json
                                          │
                                /hex-feature-dev
                                          │
                                          ▼
                behavioral-spec-writer ──► docs/specs/<feature>.json
                                          │
                            planner ──► docs/workplans/feat-<feature>.json
                                          │
                        hex plan execute <workplan>
                                          │
                    HexFlo swarm dispatches tasks per adapter
                                          │
                          worktree: feat/<feature>/<layer>
                                          │
                 best-of-N ──► cargo check / tsc --noEmit (blocking gate)
                                          │
                     validation-judge ──► PASS / FAIL (blocking)
                                          │
                        integrator merges worktrees in dependency order
                                          │
                     hex plan reconcile ──► verify evidence.commits[]
```

Each artifact is auditable: the spec is a JSON file, the workplan is a JSON file, every task carries the commits it produced, and every merge runs the analyzer.

---

## Commands

```bash
# project status
hex                           # next-step suggestions
hex status                    # overview
hex analyze .                 # architecture violations + dead code

# workplans
hex plan draft <prompt>       # create a workplan stub (auto-invoked on T3 prompts)
hex plan execute <wp.json>    # dispatch to HexFlo
hex plan reconcile --update   # sync task status with git evidence

# daemon (autonomous tick loop)
hex sched daemon --background --interval 30
hex sched enqueue workplan <wp.json>
hex sched queue list

# swarm
hex swarm init <name>
hex task list
```

Natural-language dispatch (`hex hey "rebuild nexus and validate"`) routes through the classifier; explicit commands are equivalent.

---

## Repository layout

```
hex-cli/              CLI binary, MCP server, tier classifier
hex-nexus/            Daemon (REST API, dashboard, filesystem bridge, inference adapters)
hex-core/             Port traits + domain types (zero external deps)
hex-agent/            Agent runtime (skills, hooks, boundary enforcement)
hex-parser/           Tree-sitter wrappers
spacetime-modules/    7 WASM modules (coordination state, only in AIOS-linked mode)
docs/adrs/            182 Architecture Decision Records
docs/specs/           Behavioral specs (written before code)
docs/workplans/       Active and archived workplans
```

hex runs in two modes:
- **Claude-integrated**: `CLAUDE_SESSION_ID` set. Dispatches through Claude Code.
- **Standalone**: `CLAUDE_SESSION_ID` unset. Dispatches through an Ollama adapter (ADR-2604112000). The same workplan executes either way. Run `hex doctor composition` to see which is active.

---

## Status

Alpha. 182 ADRs document the design trail; core paths (classifier, compile gate, reconciler, HexFlo, tree-sitter analyzer) are in place and exercised by tests and examples. Every mechanical claim above has a reproducer — see [EVIDENCE.md](docs/EVIDENCE.md) for the exact command per claim, prerequisites, and expected output. Benchmark numbers in [INFERENCE.md](docs/INFERENCE.md) were measured on a single Strix Halo + Vulkan-Ollama box and will differ on other hardware; the evidence page includes a script you can run to get numbers for your own environment.

Formal specs of the coordination, scheduling, and feature-pipeline state machines live in `docs/algebra/` (TLA+, model-checked with TLC).

---

## Documentation

| Doc | Contents |
|---|---|
| [Evidence](docs/EVIDENCE.md) | Reproducer for every claim in this README — commands, tests, expected output |
| [Architecture](docs/ARCHITECTURE.md) | Crates, layers, analyzer rules, SpacetimeDB modules |
| [Getting Started](docs/GETTING-STARTED.md) | Install, standalone mode, remote agents |
| [Inference](docs/INFERENCE.md) | Tier routing, GBNF grammar constraints, RL model selection |
| [Comparison](docs/COMPARISON.md) | hex vs. SpecKit, BAML, Claude Agent SDK, LangChain |
| [Developer Experience](docs/DEVELOPER-EXPERIENCE.md) | Pulse / Brief / Console / Override layers |
| [Formal Verification](docs/FORMAL-VERIFICATION.md) | TLA+ models and TLC workflow |
| [ADRs](docs/adrs/) | 182 decision records — the `why` behind each mechanism |

---

## Credits

hex builds on hexagonal architecture ([Alistair Cockburn, 2005](https://alistair.cockburn.us/hexagonal-architecture/)), tree-sitter ([Max Brunsfeld et al.](https://tree-sitter.github.io/)), and SpacetimeDB. HexFlo was informed by [claude-flow](https://github.com/ruvnet/claude-flow) (Reuven Cohen).

| Contributor | Role |
|---|---|
| Gary ([@gaberger](https://github.com/gaberger)) | Creator, architect |
| Claude (Anthropic) | Pair programmer |

## License

[MIT](LICENSE)
