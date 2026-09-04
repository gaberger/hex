# ADR-2608241500: Collapse hex to a solo software-engineering agent — retire the daemon, the coordination core, and founding goal G2

**Status:** Proposed
**Date:** 2026-08-24
**Epoch:** solo (opens a new epoch; matures and closes `hybrid-inference`)
**Drivers:** Operator directive to make hex "a purely software-engineering agent, no clustering, no coordination, just leverage an inference server to write quality hex-based architectural code." Backed by a workspace survey (`docs/analysis/2026-08-24-hex-solo-refactor-plan.md`) showing the canonical execution path already requires none of the daemon tier, and that ~200k of ~261k LOC exists to serve a fleet model that ADR-2606061359 already retired.
**Relates-To:** ADR-2606061359 (collapse org-sim to single agent loop), ADR-2606071500 (ReAct tool-use loop), ADR-2606071340 (nexus hexagonal compliance + crate split), ADR-2606072243 (hybrid-inference epoch), ADR-025 (SpacetimeDB required), ADR-027 (HexFlo swarm coordination), ADR-2026-04-26-1303 (IModelProvider + crate split)
**Retires:** founding goal **G2 — Multi-host scaleout** (`founding-goals.md`). This ADR is the Retirement-ADR that file requires.

<!-- LIFECYCLE: Proposed → Accepted → Completed. Change Status only via adr_status_set / the adr-steward. -->

## Context

### What was decided before, and what was actually built

ADR-2606061359 (2026-06-06) collapsed the multi-agent organization simulation to a single
gateway-mediated agent loop. ARCHITECTURE.md records the result as the standing design:

> The current design is **one strong agent loop fed by tools, code-graph context, and memory** —
> *not* a simulated organization of many agents. The differentiator is the *quality of context*
> assembled for that single loop, not agent head-count.

**The decision was made. The code was never removed.** Two and a half months later the workspace
still carries the entire fleet apparatus: a 96,843-LOC daemon, 11 SpacetimeDB WASM modules, a
35,096-LOC Solid.js control plane, a 23,383-LOC second agent binary, a 16,850-LOC retired SOP
pipeline, and 16 C-suite persona YAMLs whose own `DEPRECATED.md` says *"Phase 4 of the workplan
removes them"* — a phase that never ran.

### What the survey measured (2026-08-24)

**The build is not broken.** Every crate passes `cargo check` with zero errors. The sole
workspace failure is `hex-desktop`, and it fails on missing *system* libraries (`gdk-3.0`,
`dbus-1`), not on code. `cargo check -p hex-nexus --no-default-features` — SpacetimeDB off —
returns **0 errors**. The seam this ADR needs already exists and already compiles.

**The canonical loop needs no daemon state.** `hex-nexus/src/routes/mod.rs:75`:

```rust
async fn direct_execute(
    Json(task): Json<crate::direct_exec::DirectTask>,
) -> Json<crate::direct_exec::DirectResult> {
    Json(crate::direct_exec::execute_direct(task).await)
}
```

No `State(...)` extractor. The `hex do` path — the execution model ARCHITECTURE.md calls
canonical — is a pure pass-through. The daemon contributes a localhost HTTP hop and nothing else.

**The dependency graph already points the right way.** `hex-cli` depends on `hex-exec` directly
and does **not** depend on `hex-nexus`. Nothing in the workspace depends on `hex-agent`.

**One real coupling remains.** `hex-exec` calls back into the daemon for inference
(`direct_exec.rs:771`, `direct_react.rs:131` → `/api/inference/complete`). The provider adapters,
tier routing, best-of-N, and the compile gate live inside nexus. This is the only part of the
collapse that is engineering rather than deletion.

**The mass is disproportionate to the thesis.** ~261k LOC of Rust, 584 transitive crates, three
processes to run, and a 31 GB working tree of which 20 MB is actually tracked.

### Forces

- **For collapse.** The fleet was built, run, and measured. ADR-2606061359 documented its
  operational cost directly: unbounded agent-registry growth to 378 rows, ~100-instance spawn
  churn on restart, and an SOP dispatch path that routed and persisted a board ask without ever
  acting on it. Quality came from context and gates, not head-count. Every line kept to serve the
  fleet is a line that must compile, be tested, and be reasoned about on every change.
- **Against collapse.** Founding goal G2 states the opposite in as many words (below). Multi-host
  placement is a real capability, and deleting it is expensive to reverse — the WASM modules and
  coordination schemas are not trivially reconstructible from memory.
- **Constraint.** `founding-goals.md` is the one artifact agents may not author or amend. Editing
  it requires a human commit with CODEOWNERS approval.

### Alternatives considered

1. **Feature-gate the daemon tier instead of deleting it.** Rejected. The `spacetimedb` feature
   flag already proves the seam works, and the code still rots behind it: it must compile in CI,
   it still appears in `hex --help`, and it still shapes `hex-core`'s port surface. A flag defers
   the decision without collecting the benefit.
2. **Fork to a clean repository.** Rejected. `.git` is 93 MB over 2,978 commits — history is not
   the weight. A clean fork discards the 264-ADR ledger and the `docs/benchmarks/` corpus, which
   is the only empirical basis for model selection in this project. The work is ~75% deletion, and
   git is good at deletion.
3. **Keep coordination, drop only SpacetimeDB.** Rejected. Coordination *is* the reason STDB
   exists (ADR-025: WASM cannot touch the filesystem, spawn processes, or make network calls —
   "that's why hex-nexus exists"). Removing the store while keeping the fleet leaves the harder
   half of the complexity and none of the durability.

## Decision

**hex becomes a single binary that writes hexagonally-correct code against an inference server,
and nothing else.**

### 1. The execution model

```
hex do "<task>" --file <f> --evidence "<cmd>"
   ├─ hex-graph      → code-graph context + ranked lessons (local graph-out/graph.json)
   ├─ hex-exec       → ReAct loop over guarded tools, in-process
   ├─ hex-infer      → provider adapters + tier routing + best-of-N + compile gate   ← NEW
   ├─ hex-analysis   → hexagonal boundary enforcement (the quality bar)
   └─ hex-git        → apply → evidence gate → commit iff exit 0
```

All state is files on disk (`.hex/`, `graph-out/`, `docs/`). There is no database, no daemon, no
dashboard, and no network peer.

### 2. Extract `hex-infer`

A new crate implementing `hex_core::ports::inference::IInferencePort`, assembled from
`hex-nexus/src/adapters/inference/{ollama,claude_code}.rs`,
`hex-agent/src/adapters/secondary/{openai_compat,anthropic}.rs`, and
`routes/inference.rs::inference_complete` (lines 66–1099) lifted from an axum handler to a library
function. Configuration comes from `.hex/project.json`, where it is already declared. `hex-exec`
depends on the port, not on HTTP.

**Retained:** tier routing, best-of-N with compile gate, `claude -p` frontier fallback.
**Dropped:** LoRA augmentation (ADR-2606161300 is Proposed, not built), STDB inference logging,
rate limiting, the `/v1` OpenAI and Anthropic proxy shims, calibration endpoints.

`hex-infer` is the single enforcement point for **G1**: no consumer may name a provider.

### 3. Delete the daemon tier

`hex-nexus`, `hex-agent`, `hex-desktop`, `hex-state`, `spacetime-modules/` (all 11 WASM modules),
`hex-nexus/assets/` (the dashboard), `hex-cli/src/pipeline/` (the retired SOP pipeline),
`hex-cli/src/tui/`, `hex-nexus/src/orchestration/agent_loop/` (superseded by hex-exec), the seven
committed `.wasm` blobs embedded in the `hex` binary, the 16 org-sim persona YAMLs their own
`DEPRECATED.md` already condemns, the 8 swarm-behavior YAMLs, and the ~34,500 LOC of
daemon-mediated CLI verbs.

Every deletion is preceded by a `hex graph consumers` trace across the whole workspace
(ADR-2606071713) and followed by a blocking `cargo check --workspace`. This is not ceremony:
ADR-2026-04-05-0900 records hex-agent being broken for a session because one workplan missed a
feature-gated import.

### 4. `hex swarm build` / `hex swarm review` survive, renamed

The cooperative+adversarial harness (`hex-exec/src/adversarial.rs`, 475 LOC) is **in-process
fan-out of inference calls**, not distribution. It has no registry, no heartbeats, no shared
state, and no peers. It is also the measured source of quality: per ARCHITECTURE.md it built a
~2,900-LOC durable job queue from a one-line spec, and its adversarial pass found 6 real bugs that
the build's own passing tests missed.

It is retained and renamed **`hex build`** / **`hex harden`**, so that "swarm" stops implying a
cluster that no longer exists.

### 5. Retire founding goal G2

`founding-goals.md` currently states:

> **G2 — Multi-host scaleout.** "A self-modifying substrate that runs only on the developer's
> laptop is a toy. The substrate must be able to run as a fleet — agents on one host, inference on
> another, coordination state shared — with placement decided by the substrate rather than
> configured by the user. Without this, hex remains a single-tenant build helper instead of an
> operating system for AI work."

**G2 is retired.** The goal assumed head-count and placement were the axis along which an AI
development substrate gets better. hex built that, ran it, and measured it: the fleet produced
registry growth, spawn churn, and silent dispatch failures, while quality came from context
assembly and evidence gates. ADR-2606061359 conceded this for agents; this ADR concedes it for
hosts. "Single-tenant build helper" is not the failure mode G2 feared — it is the product.

G1 (model tiering and independence) and G3 (hexagonal rigor at the workspace level) are
**unaffected and strengthened**: G1 collapses to one port with one implementation crate, and G3
becomes enforceable across 8 crates instead of 13.

**This ADR does not edit `founding-goals.md`.** That file forbids agent authorship. Landing the
edit is a human commit under CODEOWNERS, and it MUST land before any deletion phase begins —
otherwise the repository's governance layer contradicts its own head commit.

### 6. Scope boundary

This ADR covers the hex substrate only. Scaffolded target projects keep the full hexagonal rule
set; nothing here changes what hex *generates*, only what hex *is*.

## Consequences

**Positive:**
- ~261k → ~49k LOC; 584 → ~180 transitive crates; 3 processes → 1; 4 binaries → 1.
- Working tree drops from 31 GB to ~200 MB (23 GB `target/`, a 5.4 GB vendored llama.cpp checkout,
  and ~1.2 GB of example build output are evicted).
- `hex analyze .` becomes honest. nexus scored **F (30/100)** against hex's own analyzer before its
  split; a 49k-LOC single binary has no excuse for failing the rules it enforces on others.
- The validation gate starts working. `hex dev validate` chains `bun test`, whose script globs
  `tests/unit tests/property tests/smoke` — directories deleted when the TypeScript library was
  excised. The gate has been running a command that cannot pass.
- Every agent session gets a cleaner context: 64 `.claude/skills/` drop to ~12 once 31 foreign
  claude-flow / AgentDB / GitHub skills are removed.
- One process means one failure mode. No daemon liveness, no STDB health, no reconciliation
  between what the daemon believes and what the repository contains.

**Negative:**
- **Multi-host is gone and expensive to rebuild.** The WASM modules and coordination schemas are
  not reconstructible from memory. Recovering them means `git revert` against this ADR's commits,
  and the further hex evolves the harder that gets.
- **The dashboard is gone.** Observability becomes stdout and the local run log. Anyone who wants
  a live view of long-running work loses it.
- **A founding goal is being retired 4 months after it was stated**, on the strength of one
  operator's judgment and one prior ADR's evidence. If the fleet thesis was right and simply
  under-built, this ADR is the mistake.
- ~1,000 nexus tests and ~683 CLI tests are deleted. Coverage that exercises `hex-exec` or
  `hex-analysis` behavior must be ported forward first, or real regressions become invisible.
- `hex-core` loses ~5,400 LOC of ports and domain types. Anything depending on those shapes
  outside this workspace breaks.

**Neutral:**
- All 264 ADRs are retained. The ledger is append-only; the daemon-era decisions are grouped under
  the `hybrid-inference` epoch by `hex adr reindex`, not deleted.
- `docs/benchmarks/` is retained in full — it is the regression oracle for phase 8.

## Verification

The decision is falsifiable at two gates, both blocking:

1. **P1 gate — `hex do` completes an evidence-gated task with the nexus daemon stopped.** If this
   fails, the claim that the daemon is a pass-through is wrong and the entire plan is void.
2. **P8 gate — `hex bench agentic` shows no pass-rate regression against the baseline recorded in
   P0**, and `hex analyze .` scores A or better. External coding-leaderboard scores are known not
   to predict agentic-loop performance in this project (ADR-2606071734: the top-leaderboard local
   model scored last on the grid), so the local corpus is the only admissible evidence.

Implemented by workplan `wp-2608241500-hex-solo`, specs in `docs/specs/hex-solo.json`.
