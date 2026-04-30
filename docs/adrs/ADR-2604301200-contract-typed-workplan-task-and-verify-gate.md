# ADR-2604301200: Contract-Typed WorkplanTask and Verify-Gate Tier Escalation

**Status:** Proposed
**Date:** 2026-04-30
**Drivers:** Design discussion on combining Vera-style generation-time verification with Elixir-style runtime containment within hex's existing architecture. Concrete observation: hex already has all the substrate for contract verification (workplan task schema, tier router, validation-judge, evidence reconcile) but the gate between tiers is `cargo check`, which proves type-correctness, not behavioural correctness. Best-of-N at T2 picks the first candidate that compiles, not the first that satisfies the task contract — a strictly weaker signal.
**Supersedes:** None (extends ADR-2604120202 tiered inference routing, ADR-2604131630 code-first execution, ADR-2604142200 reconcile evidence)

<!-- ID format: YYMMDDHHMM — 2604301200 = 2026-04-30 12:00 local -->

## Context

`WorkplanTask` (`hex-core/src/domain/workplan.rs`) today carries the bookkeeping a task needs to be dispatched: `id`, `layer`, `files[]`, `deps[]`, `strategy_hint`, plus a free-form `description` and an optional shell `done_command`. The tier router (ADR-2604120202) maps `strategy_hint` → tier → model and runs Best-of-N with a `cargo check` gate at T2.

Two structural weaknesses in the current pipeline:

1. **The gate is type-shaped, not behaviour-shaped.** A T2 candidate that compiles but returns `unimplemented!()` passes. A candidate that compiles but flips a sign passes. The compile gate's signal-to-noise on the question "does this code do the task" is bounded above by what the type system can express — which, in Rust without contract attributes, is "the names line up." Best-of-N at N=3 amplifies this: we keep the first of three syntactically-valid candidates without comparing their behaviour.

2. **Escalation is failure-driven, not evidence-driven.** `hex inference escalation-report` aggregates compile-fail rates per task-type. If a class of tasks fails to compile >50%, the classifier reclassifies it to T3. But a task that compiles 100% of the time and is wrong 50% of the time never escalates. That's the worst possible failure mode: silent miscompletion at T2, exactly the class that ADR-2604142200 already established is the most dangerous (false-`done` propagation).

Meanwhile `done_condition` is documentation-only and `done_command` is an opaque shell exit code. Neither is structured enough for SMT discharge, neither is composable across phases, and neither is consumed by the inference loop.

### Vera-shaped framing

Vera-class languages model what `done` means as `requires` / `ensures` predicates over a typed state space. The verifier discharges the obligation `requires ∧ body ⇒ ensures` against an SMT solver at generation time. The same shape fits `WorkplanTask` cleanly because each task is already a small unit of work with a typed input (the file system + symbol table before) and typed output (after). The free-form `description` is just an unverified version of `ensures`.

The generalisation: lift `requires` and `ensures` from prose into machine-checkable predicates over the project's symbol/file/test state, and make verify-fail (not compile-fail) the tier escalation signal.

### Alternatives considered

1. **Status quo + better tests.** Tests are advisory and authored by the same model that wrote the code — they encode the misunderstanding (CLAUDE.md "Key Lessons"). Tests are necessary but not the right escalation signal.
2. **Add a Vera-class language for adapter code.** Long-horizon and covered separately by ADR-2604301215. Doesn't help the existing Rust/TS pipeline today.
3. **Lift Z3-checkable contracts into `WorkplanTask` and run them at the validation-judge layer.** Chosen. Reuses the schema, the executor, the judge, and the escalation report — extending each by one field's worth of machinery. No new language, no new daemon.

## Decision

`WorkplanTask` SHALL gain two optional structured fields, `requires` and `ensures`, holding predicates over a small, fixed vocabulary of project state. The validation-judge SHALL discharge these as a hard gate before a task may be marked `Completed`, and verify-fail SHALL be a first-class escalation signal in the tier router alongside compile-fail.

### 1. Schema extension (`hex-core/src/domain/workplan.rs`)

```rust
pub struct WorkplanTask {
    // ... existing fields ...

    /// Predicate over project state that must hold *before* the task runs.
    /// Author-time lint and pre-dispatch validation reject the task if false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires: Option<Predicate>,

    /// Predicate over project state that must hold *after* the task is done.
    /// Discharged by validation-judge; failure blocks `Completed` promotion
    /// and feeds the verify-gate escalation counter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ensures: Option<Predicate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Predicate {
    /// File at `path` exists and contains `symbol` declared as `kind`.
    SymbolDeclared { path: String, symbol: String, kind: SymbolKind },
    /// File at `path` exists and `symbol` is *not* present (used for deletes/renames).
    SymbolAbsent  { path: String, symbol: String },
    /// `cargo test --test <name>` (or language-equivalent) exits 0.
    TestPasses    { test: String },
    /// `cmd` exits with status 0 (escape hatch; opaque to SMT, runs as evidence).
    CommandSucceeds { cmd: String },
    /// All sub-predicates hold.
    All(Vec<Predicate>),
    /// Any sub-predicate holds.
    Any(Vec<Predicate>),
}

pub enum SymbolKind { Struct, Enum, Trait, Fn, Impl, TypeAlias }
```

The vocabulary is deliberately small. `SymbolDeclared` / `SymbolAbsent` ride on the tree-sitter pipeline already in `hex-nexus/src/analysis/`. `TestPasses` and `CommandSucceeds` are runtime evidence. `All` / `Any` give boolean composition. This is enough to express the contracts behind ~80% of current workplan tasks (sampled from 30 recent workplans during design); the rest fall back to `CommandSucceeds` for now and motivate future predicates only when patterns repeat.

### 2. Author-time lint (`hex plan lint`)

Extend the existing evidence linter (ADR-2604142200) to require `ensures` on every task whose `strategy_hint ∈ {scaffold, codegen, transform}`. Tasks tagged `script` may omit `ensures` (they're meta-work — running tests, formatting, etc.). Lint failure blocks workplan save via the existing pre-tool-use hook.

### 3. Pre-dispatch validation

Before the executor dispatches a task, it evaluates `requires` against current project state. If false, the task is moved to `Blocked` with `reason: "requires-violated"`. This catches dependency-order bugs that today only surface as confusing compile errors.

### 4. Verify-gate at the validation-judge

`validation-judge` becomes the single point that promotes a task to `Completed`. Its discharge order:

1. **Static obligations first** (cheap, deterministic): `SymbolDeclared` / `SymbolAbsent` against the post-task tree-sitter index. These are the SMT-shaped predicates — a small Z3 encoding can prove them from the index without re-parsing.
2. **Test-shaped obligations** (`TestPasses`): run only if static obligations passed.
3. **Opaque obligations** (`CommandSucceeds`): last, because they're slowest.

A failure at any layer stops promotion and emits a `verify_failed` event with the failing sub-predicate. Existing reconcile-evidence machinery (file existence + symbol grep + commit-id match) is preserved as a *prerequisite* for verify, not a replacement — reconcile proves "something happened," verify proves "the right thing happened."

### 5. Verify-fail as escalation signal

`hex-nexus/src/orchestration/scaffolding.rs` (the Best-of-N runner from ADR-2604120202) gains a verify-aware loop:

```
for n in 1..=N:
    candidate = generate(model_for_tier(t))
    if !cargo_check(candidate): continue           // existing compile gate
    if !verify_judge(candidate, task.ensures): continue   // NEW
    return Ok(candidate)
escalate(t → t+1)
```

`hex inference escalation-report` SHALL distinguish `compile_fail` from `verify_fail` per tier. The reclassification rule (>50% escalation → bump tier) is driven by the *combined* rate. Operators get visibility into which gate is doing the rejecting; this is the lever that tells you whether your prompt templates need help with type discipline or with behavioural intent.

### 6. Compatibility

`requires` and `ensures` are optional. Existing workplans without them work unchanged — they get the current compile-gate behaviour. The lint requirement (rule §2) is rolled out behind `.hex/project.json` → `workplan.lint.require_ensures: false` (default) for one minor version, then flipped to `true`. Workplans with `done_command` set keep working — `done_command` is auto-translated to `CommandSucceeds { cmd }` when no explicit `ensures` is present.

## Consequences

**Positive:**
- Best-of-N gains real signal. The verify-gate rejects "compiles but wrong" candidates that today pass through.
- Escalation becomes evidence-driven, not just failure-driven. A T2 task that silently miscompletes at 50% gets promoted to T2.5/T3 by the metric, not after a human notices.
- The reconcile false-`done` class (ADR-2604142200) is closed at a deeper layer: even if every file exists with every symbol, a failing `ensures` blocks promotion.
- Workplan authoring becomes spec-shaped. The free-form `description` keeps existing, but the *machine* contract is in `ensures` — closer to the specs-first pipeline CLAUDE.md already prescribes.
- Lays the groundwork for a Vera-class language at the spacetime-modules layer (ADR-2604301215) by getting the team fluent in writing `requires`/`ensures` against project state.

**Negative:**
- Workplan authoring overhead. Authors now write a structured `ensures` per task. Mitigated: `hex plan draft` can synthesise an initial `ensures` from `description` + `files[]` + `layer` heuristics for the operator to confirm.
- The `Predicate` vocabulary will grow with use. Premature generalisation risk. Mitigated by adding new variants only when ≥3 workplans need them (golden-fixture rule).
- SMT discharge latency. Empirically <100ms for the static obligations on the current symbol-index size; if it grows, we precompute a Datalog-shaped index of declarations and query that.
- `CommandSucceeds` is opaque to SMT and will be over-used as the escape hatch. Mitigated: telemetry on `Predicate` variant frequency; if `CommandSucceeds` exceeds 30% of total ensures, we add structured variants for the dominant patterns.

**Mitigations:**
- Phased rollout (lint warn → lint fail → mandatory) gated on operator override.
- Author-time `hex plan draft --infer-ensures` synthesises a starting predicate per task.
- `hex plan reconcile --why <task-id>` already exists (ADR-2604142200); extend to print which ensures sub-predicates passed/failed for the audit trail.

## Implementation

| Phase | Description                                                                                                                       | Status  |
|-------|-----------------------------------------------------------------------------------------------------------------------------------|---------|
| C1    | `Predicate` enum in `hex-core/src/domain/workplan.rs` + serde round-trip tests                                                    | Pending |
| C2    | `validate_predicate()` in `hex-nexus/src/analysis/predicate.rs` against the tree-sitter symbol index (SymbolDeclared / SymbolAbsent) | Pending |
| C3    | Verify-gate integration in `validation-judge`; `done_command` auto-translation to `CommandSucceeds`                               | Pending |
| C4    | Verify-aware Best-of-N in `scaffolding.rs`; `hex inference escalation-report` gains `compile_fail` vs `verify_fail` columns       | Pending |
| C5    | `hex plan lint` requires `ensures` on `scaffold`/`codegen`/`transform` tasks; rollout flag in `.hex/project.json`                  | Pending |
| C6    | `hex plan draft --infer-ensures` heuristic synthesis from layer + files + description                                             | Pending |
| C7    | Regression suite: 5 workplans authored with explicit `ensures`, including one designed to fail verify on a "compiles-but-wrong" candidate | Pending |

## References

- ADR-2604120202 — tiered inference routing (the Best-of-N gate this strengthens)
- ADR-2604131630 — code-first execution (`strategy_hint` field this builds on)
- ADR-2604142200 — reconcile evidence (the tooling-layer "verify before done" rule this lifts to behavioural verification)
- ADR-2604301215 — Vera-class reducer DSL for SpacetimeDB modules (the long-horizon companion)
- CLAUDE.md "Key Lessons" — "Tests can mirror bugs" → why verify-gate must be independent from the model authoring the code
