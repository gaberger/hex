# hex — The Operating System for AI Agents

**Local-first. Self-improving. Architecturally enforced.**

---

## What is hex?

hex is an AI Operating System (AIOS) — a runtime layer that manages AI coding agents the way Linux manages processes. It doesn't replace your AI tools; it sits underneath them, providing the coordination, enforcement, and intelligence they lack.

Think of it this way: you wouldn't run 10 user processes on a machine without an OS to manage memory, scheduling, and permissions. Why run 10 AI agents without one?

## The Core Insight: Not All Tasks Need Frontier Models

Every agent framework today sends every task to the same model. A typo fix costs the same as a feature implementation. hex changes this with **tiered inference routing**:

```
Task Complexity        Model              Cost        Speed
─────────────────────────────────────────────────────────────
T1: Fix a typo         qwen3:4b (local)   $0.00       2.3s
T2: Write a function   qwen2.5-coder:32b  $0.00      10.5s
T2.5: Multi-file edit  qwen3.5:27b        $0.00     380.0s
T3: Architecture       Claude Opus        $0.03+      varies
```

**On the workplans we've exercised, roughly 70% of tasks classify as T1 or T2** and run entirely on local hardware — a consumer GPU or even a laptop CPU. That ratio is a property of a workplan's decomposition (how many scaffolding / transform / single-function steps it contains), not a universal constant. Inspect your own: `hex plan analyze <workplan.json>` groups steps by tier.

The system classifies tasks automatically and routes them to the right model. When a local model fails, it escalates to frontier. When it succeeds, the RL engine records the win and routes similar tasks the same way next time.

## Mathematical Foundation: Hexagonal Architecture as Algebra

hex is built on **hexagonal architecture** (Ports & Adapters), formalized as a stack of composable algebras:

**The Port Algebra.** Every boundary in the system is a typed interface (port). Domain logic depends only on port signatures, never on implementations. This isn't a convention — it's enforced at compile time via tree-sitter AST analysis across TypeScript, Go, and Rust codebases:

```
Domain  →  can only import  →  Domain
Ports   →  can only import  →  Domain
Adapters → can only import  →  Ports
Adapters → CANNOT import    →  Other Adapters
```

Violations are caught before code ships: `hex analyze` returns a non-zero exit code on cross-layer imports, and the pre-commit hook runs it on every commit.

**The Dispatch Algebra.** Task routing is a function composition:

```
classify: Task → Tier
route:    Tier → Model
scaffold: Model × Prompt → [Completion₁, ..., Completionₙ]
gate:     Completion → {Pass, Fail}
select:   [Completion] → Result
reward:   (Tier, Model, Outcome) → Q-table update
```

Each step is pure, testable, and composable. The scaffold layer generates N completions, the compile gate validates each, and the RL engine learns which model performs best per task type. The system converges on optimal routing through experience, not configuration.

**The Coordination Algebra.** Multi-agent work is modeled as a partially ordered set of tasks with dependency edges. Tasks at the same tier execute in parallel (isolated git worktrees). Quality gates enforce ordering: domain and ports must compile before adapters begin. The coordination layer (HexFlo) runs inside SpacetimeDB WASM modules — transactional, real-time, zero external dependencies.

## What Makes hex Different

### 1. Local AI is a First-Class Citizen

hex was built for Ollama, not bolted onto it. The inference adapter speaks Ollama's API natively. GBNF grammar constraints force models to emit only valid output — on the reference corpus (a one-line typo fix against `qwen3:4b` Q4 on Strix Halo) grammar-constrained decoding cut generation time from 88.6s to 31.2s (2.8×). The Best-of-N compile gate generates multiple completions and returns the first that passes `rustc` / `tsc` / `go build`.

On the 9-task reference corpus (3 Rust + 3 TypeScript + 3 Go) every task compiled on the first attempt against local 32B models — the retry scaffolding wasn't exercised. Larger or more novel tasks will fail candidates; the retry loop feeds compiler errors back into the next attempt. Reproducer: `examples/standalone-pipeline-test/run.sh --verbose`. See [EVIDENCE.md](EVIDENCE.md).

### 2. RL Self-Improvement

A Q-learning engine in SpacetimeDB records every dispatch outcome:

```
State: tier:T2|task_type:single_function
Action: model:qwen2.5-coder:32b
Q-value: +1.1 (and climbing)
Visits: 5
```

Local models get a success bonus. Models that fail see Q-values drop. The epsilon-greedy selector (90% exploit, 10% explore) occasionally tries alternatives so the router can discover better pairings as outcomes accumulate. Convergence behavior on real workloads has not been published; inspect your own with `hex inference escalation-report`.

### 3. Three-Path Dispatch

```
Workplan Executor
  ├── Path C: T1/T2/T2.5 → direct inference (no agent process)
  │   Headless. Grammar-constrained. 2.3 seconds for a typo fix.
  │
  ├── Path A: T3 → spawn hex-agent with full tooling
  │   For tasks that need filesystem access and multi-step reasoning.
  │
  └── Path B: Claude Code → queue for outer session
      Integrates with Claude Code when available.
```

Path C is the key innovation: for simple tasks, skip the agent entirely. Send the prompt to Ollama, get code back, validate it compiles. No process fork, no tool loading, no shell. This makes local inference practical for high-throughput workplan execution.

### 4. Remote Agent Fleet

Any machine with Ollama becomes a compute node:

```bash
# GPU workstation joins the fleet
hex agent connect http://coordinator:5555

# Coordinator sees it immediately
hex agent list
# → nexus-agent (Mac, online)
# → agent-e4fa (bazzite.lan, online)
```

Tasks route to the least-loaded server with the requested model. SSH tunnels handle connectivity — no exposed ports, no firewall changes. Each machine's RL engine learns independently.

### 5. Architecture That Can't Drift

107 Architecture Decision Records document every design choice. Tree-sitter parses every source file into an AST and validates import boundaries in milliseconds. The analysis runs on every commit:

```bash
$ hex analyze .
  422 source files scanned
  0 boundary violations
  Score: 100/100  (weights in hex-cli/src/commands/analyze.rs)
```

The analyzer exit code is wired into the pre-commit hook and CI. A new cross-layer import fails the analyzer and blocks the commit. Treat the numeric score as a regression signal — the weights are tunable, so the absolute number is meaningful only relative to the project's own baseline.

## The Numbers

| Metric | Value |
|:-------|:------|
| Languages supported | Rust, TypeScript, Go |
| Boundary violations in codebase | 0 |
| Architecture Decision Records | 131 |
| SpacetimeDB WASM modules | 7 |
| Local models tested | 10 (qwen, devstral, gemma, nemotron, codegeex) |
| Compile gate pass rate (local 32B) | 100% across 3 languages |
| GBNF token reduction | 2.8x |
| RL Q-table entries | 9 state-action pairs (and growing) |
| Path C dispatch latency | 2.3s for T1, 10.5s for T2 |
| Remote vs local speedup | 2x (no network round-trip) |

## Try It

```bash
# Build from source
cargo build -p hex-cli --release
cargo build -p hex-nexus --release

# Start (auto-launches SpacetimeDB)
hex nexus start

# Run the pipeline smoke test
cd examples/standalone-pipeline-test
./run.sh --tier T1 --verbose    # 10-second T1 test
./run.sh                         # Full suite: Rust + TypeScript + Go

# See the RL Q-table learn
./run.sh --tier T1               # Run it again — Q-values climb
```

No API keys required. No cloud account. Just Ollama and a model.

---

*hex is open source under MIT. Built in Rust. Tested across Rust, TypeScript, and Go. Runs on Mac, Linux, and any machine with Ollama.*

*GitHub: [github.com/gaberger/hex](https://github.com/gaberger/hex)*
