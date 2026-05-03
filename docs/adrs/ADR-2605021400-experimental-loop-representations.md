# ADR-2605021400 — Experimental Loop Representations: Persona, Workload, Trial, Verdict

**Status:** Proposed
**Date:** 2026-05-02
**Drivers:** The vision plan in `docs/guides/SELF-REORGANIZING-HEXAGON.md` describes target apps reorganizing themselves under demand, with synthetic users placing challenges and an objective function to maximize. The existing ADR / spec / workplan / test taxonomy covers *deciding, accepting, planning, verifying*, but cannot express *who stresses the app, what to maximize, what we predict, what variants we're running, or did the change win*. Without those, the loop is open and the "experiment" is hand-waved.
**Related:** ADR-2604261430 (stash consolidation memory port — provides Hypothesis, Goal, Failure types we lift here), ADR-2604142243 (classifier rules as data — the data-fy precedent), ADR-2603240130 (declarative agent/swarm YAMLs — same "behavior is data" pattern), ADR-2604120202 (tiered inference routing — feeds RL signal that Verdicts archive)

## Context

The four current representations and what each captures vs. omits:

| Concept | Captures | Omits |
|---|---|---|
| **ADR** | decision to add/swap a port or adapter | predicted effect on a measurable outcome |
| **Spec** | Given/When/Then behavioral acceptance | quantitative goals — "p95 < 100 ms", "cost/req < $0.001" |
| **Workplan** | task DAG, tier mapping, gates | which architecture variant this run instantiates |
| **Test** | does the code match the spec | did the change move the objective the right direction |

The vision plan calls for a self-reorganizing target app where:

1. **Synthetic users** place workloads against the app.
2. **Telemetry** is gathered against an **objective function**.
3. **Hex** drafts ADRs proposing reorganizations, each carrying a **hypothesis** about expected impact.
4. **Variants** of the architecture run concurrently as **trials**.
5. **Verdicts** decide whether a trial graduates to the canonical app or rolls back.
6. **RL** archives the verdict so future ADR proposals are informed by past wins/losses.

None of those nouns exist as first-class domain types in hex today. Adding them ad-hoc per workplan would re-create the drift the four-concept taxonomy was meant to prevent.

Stash (per ADR-2604261430) already has three of the seven concepts we need: **Hypothesis**, **Goal**, and **Failure**, with a working Postgres + pgvector pipeline behind them. Lifting them as kernel domain types (rather than only-via-stash adapter types) means hex itself can reason about hypotheses and goals even when no stash sidecar is running.

## Decision

Introduce seven domain types in `hex-core/src/domain/experiment/` — three lifted from stash via the `IConsolidationMemoryPort` it already requires, and four new. All seven follow ADR-001 rule 1 (domain only depends on std + chrono + serde + thiserror).

### Adopted from stash (lift to kernel domain)

- **Goal** → renamed **Objective** when used at app level. Same shape: hierarchical, prioritized, optionally parented. App-level objectives are top-level (no parent); per-feature sub-objectives nest. Distinguished from `hex-cli/src/pipeline/objectives.rs` which is *dev-process* objectives ("compile success"); the **Objective** here is *target-app* ("p95 latency < 100 ms"). The two coexist; same shape, different scope.
- **Hypothesis** — same shape as stash. Adds one field: `target_objective: ObjectiveId` (which objective this hypothesis predicts movement on).
- **Failure** — same shape as stash. Becomes the post-mortem record attached to a rejected Trial.

### New domain types

- **Persona** — a synthetic user. Fields: `id`, `name`, `goals: Vec<String>` (what the persona is trying to do), `frustration_triggers: Vec<String>` (what makes them retry/abandon), `request_distribution` (Markov chain of action sequences), `tolerance: Tolerance` struct (max latency, max retry count, error patience), `volume_profile: VolumeProfile` enum (constant / spiky / diurnal / scheduled). Personas are *target-app-specific*; hex carries the schema, the target carries the values.
- **Workload** — an aggregate demand pattern. Fields: `id`, `personas: Vec<(PersonaId, f32)>` (mix weights), `duration: Duration`, `concurrency: ConcurrencyProfile`, `seed: u64` (reproducibility). A Workload is what gets *run* against the app; a Persona is a *kind* of user that participates.
- **Trial** — a specific architecture variant under test. Fields: `id`, `parent_app_version: AppVersionId`, `workplan_id: WorkplanId` (the workplan that instantiated this variant), `hypothesis_id: HypothesisId`, `workload_id: WorkloadId`, `started_at`, `ended_at: Option<DateTime>`, `status: TrialStatus` (queued / running / completed / aborted). Multiple trials can run concurrently against the same workload; the verdict picks a winner.
- **Verdict** — the quantified outcome record. Fields: `id`, `trial_id: TrialId`, `objective_id: ObjectiveId`, `baseline_score: f64`, `trial_score: f64`, `delta: f64`, `confidence: f64` (statistical), `decision: VerdictDecision` enum (graduate / hold / rollback / inconclusive), `archived_at`, `notes: String`. A Verdict is what actually closes the experimental loop. Stash's binary `confirm_hypothesis` / `reject_hypothesis` becomes a *projection* of this richer type — `delta > 0 && confidence > threshold ⇒ confirm`, etc.

### The experimental loop, wired

```
   target-app developer       hex                              target app
   ────────────────────       ───                              ──────────
1. defines Personas + Objective                              ─→ workload runs
                              records demand telemetry  ←─────  emits signals
2.                            measures Objective from telemetry
                              proposes ADR (with Hypothesis
                                predicting Δ on Objective)
3. approves ADR (gate)
                              instantiates Trial → Workplan
                              executes against Workload
                              compares trial_score to baseline_score
                              writes Verdict
                              feeds Verdict.delta to RL
4.                            graduate / hold / rollback / inconclusive
                              loop with the next ADR proposal informed by Verdict history
```

The four existing concepts ride steps 3 (ADR, Workplan, Spec) and the right edge (Tests). The seven new concepts ride steps 1, 2, 3 (Hypothesis attaches to ADR), and 4. Together they close the loop.

### Storage

Per the data-fy precedent in ADR-2604142243, all seven types get SpacetimeDB tables in a new module `spacetime-modules/experiment/` (or extension of `hexflo-coordination` if mass is small):

| Table | Indexes |
|---|---|
| `objectives` | `(project_id, status)`, parent fan-out |
| `hypotheses` | `(target_objective_id)`, `(adr_id)` |
| `failures` | `(trial_id)`, `(objective_id)` |
| `personas` | `(project_id)` |
| `workloads` | `(project_id, status)` |
| `trials` | `(workload_id, status)`, `(hypothesis_id)` |
| `verdicts` | `(trial_id)`, `(objective_id, decision)` |

Reducers: `objective_create`, `hypothesis_create`, `trial_start`, `trial_complete`, `verdict_record`, `workload_run`, etc. — full enumeration in P3 below.

### CLI surface

Additive:

```
hex objective create|list|score
hex persona create|list|describe
hex workload run <persona-mix> --duration --concurrency
hex trial start <hypothesis-id> --workload <id>
hex trial list [--running|--completed]
hex verdict <trial-id>          # forces evaluation, writes verdict
hex experiment status            # full loop snapshot for current project
```

`hex adr create` gains a `--hypothesis "<text>"` flag and a `--target-objective <id>` flag; absence of either on a target-app-modifying ADR yields a `hex doctor` warning.

### MCP surface

`mcp__hex__hex_experiment_*` tools mirror the CLI 1:1.

## Consequences

**Positive:**
- The experimental loop the vision plan describes becomes expressible. Today it is hand-waved; after this ADR it has nouns, schemas, and reducers.
- Three of the seven concepts are *adopted from stash*, not invented — less divergence from the upstream consolidation pipeline, more reuse of stash's pattern detection / contradiction handling.
- Verdicts archive into the same RL pipeline (`spacetime-modules/rl-engine`) the vision plan's Stage 1 wires up — this ADR is what makes Stage 1's "RL signal" a typed, queryable record instead of an opaque float.
- Synthetic-user representation (Persona + Workload) is genuinely missing from the AI-OS landscape; hex shipping a typed schema for it is a differentiator, not a parity feature.

**Negative:**
- Seven new domain types is a large surface. Mitigation: ship in two phases — Objective + Hypothesis + Verdict first (close the loop), Persona + Workload + Trial + Failure second (richen it). Phase 1 alone is enough for the first-step Stage-1 advisory experiment in the vision plan.
- Persona and Workload are easy to over-engineer; the temptation is to build a full traffic-simulation framework. Strict scope: target apps own the *values*, hex owns the *schema*. No simulation engine in this ADR.
- Verdict introduces a statistical-confidence concept hex hasn't carried before. Pinning the math (Welch's t-test? bootstrap? Bayesian?) is a P2 decision, not P1.

**Mitigations:**
- Two-phase ship (above).
- Persona schema deliberately under-specified — `request_distribution` is `serde_json::Value` until target apps prove a typed shape is needed.
- Verdict math defaults to a simple Welch's t-test; pluggable behind a `VerdictPolicy` trait so projects can swap in their preferred approach.

## Implementation

| Phase | Description | Status |
|-------|------------|--------|
| P1 | Domain types in `hex-core/src/domain/experiment/{objective,hypothesis,verdict}.rs` (the loop-closing three). Re-export under `hex-core::domain::experiment`. ADR-001 rule 1 — domain only | Pending |
| P2 | SpacetimeDB tables + reducers for Objective / Hypothesis / Verdict in `spacetime-modules/experiment/` (or `hexflo-coordination` if size is modest). `IExperimentPort` trait in `hex-core/src/ports/experiment.rs` | Pending |
| P3 | Adapter implementing `IExperimentPort` against the SpacetimeDB module. `StashExperimentAdapter` *projection* — when a Hypothesis is created in hex it mirrors to stash via `IConsolidationMemoryPort.create_hypothesis`; same for Goal→Objective and Failure | Pending |
| P4 | CLI: `hex objective`, `hex hypothesis`, `hex verdict`, `hex experiment status` subcommands. `hex adr create --hypothesis --target-objective` flags. `hex doctor` warning for ADRs missing them | Pending |
| P5 | Domain types in `hex-core/src/domain/experiment/{persona,workload,trial,failure}.rs` (the richening four). Reuse failure shape from stash | Pending |
| P6 | SpacetimeDB tables + reducers for Persona / Workload / Trial / Failure. `hex persona`, `hex workload`, `hex trial` CLI subcommands | Pending |
| P7 | Verdict policy plug-in: `VerdictPolicy` trait in `hex-core/src/ports/verdict.rs` with default Welch's t-test impl. Pluggable via `.hex/project.json` → `experiment.verdict_policy` | Pending |
| P8 | First end-to-end pilot: a tiny example app under `examples/`, one Objective, two Personas, a Workload, a baseline Trial, a hypothesis-driven Trial, a Verdict. Documents the loop in `docs/guides/EXPERIMENT-PILOT.md` | Pending |

## Citation

This ADR adopts three domain types from the Stash project (per ADR-2604261430):

> **Stash — Persistent memory for AI agents**
> Repository: https://github.com/alash3al/stash · License: Apache-2.0
> Pinned commit: `d1122a699cf2f0022409fbdf97871298273c20a6` (2026-04-25)
> Adopted concepts: `hypothesis`, `goal` (renamed to `Objective` at app scope), `failure`
> Schema reference: `internal/db/migrations/00010_*` (tables: `hypotheses`, `goals`, `failures`)
> MCP surface reference: `cmd/cli/mcp.go` — `create_hypothesis`, `create_goal`, `create_failure`, `confirm_hypothesis`, `reject_hypothesis`

The four new types — `Persona`, `Workload`, `Trial`, `Verdict` — are original to hex.

## References

- `docs/guides/SELF-REORGANIZING-HEXAGON.md` — the vision plan that surfaced this gap (Stages 4 & 5)
- ADR-2604261430 — stash consolidation memory port (provides the Hypothesis/Goal/Failure adapter route)
- ADR-2604142243 — classifier rules as data tables (the data-fy precedent for moving Rust constants into SpacetimeDB)
- ADR-2603240130 — declarative agent/swarm YAMLs (same "behavior is data" pattern this ADR extends)
- ADR-2604120202, ADR-2604131630 — tiered inference routing (consumes Verdict telemetry via RL)
- `hex-cli/src/pipeline/objectives.rs` — *dev-process* objectives, distinct from app-level Objective introduced here
- `spacetime-modules/neural-lab/` — Experiment/ResearchFrontier shape that Trial loosely follows
- `hex-core/src/domain/workplan.rs` — the existing Workplan domain that Trial references via `workplan_id`
