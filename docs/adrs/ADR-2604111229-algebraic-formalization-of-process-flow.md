# ADR-2604111229: Algebraic Formalization of hex Process Flow

**Status:** Proposed
**Date:** 2026-04-11
**Drivers:** User question "Is there an algebraic way to represent hex process flow?" during session review. The codebase has grown enough that boundary violations, swarm coordination bugs, and lifecycle ordering mistakes are no longer catchable by reading the code — they need a formal model.
**Supersedes:** None (complements ADR-2603221959 enforcement, ADR-027 HexFlo, ADR-2603240130 declarative swarm behaviors)

<!-- ID format: YYMMDDHHMM — 2604111229 = 2026-04-11 12:29 UTC -->

## Context

hex has four layers that each behave as independent concurrent systems, and
today none of them have a formal specification:

1. **Ports layer** (`hex-core/src/ports/`, 10 traits, ~40 operations total) —
   the effect surface. Every adapter implements a subset. The hexagonal
   boundary rules (`domain → ports → usecases → adapters`) are enforced by
   a tree-sitter scan in `hex analyze`, which checks *import edges* but has
   no notion of *operation signatures*. A use case that imports only
   `IInferencePort` but calls methods with effects outside the domain
   envelope would still pass `hex analyze`.

2. **Tier routing** (`hex-cli/src/commands/hook.rs::classify_work_intent`) —
   pure function today, already tested as one. But the dispatch pipeline
   that follows it (`classify → dispatch → heal → persist`) is stitched
   together imperatively.

3. **Swarm coordination** (HexFlo, `hex-nexus/src/coordination/`) — agents,
   heartbeats, timeouts, task reclamation, CAS task claims. The protocol is
   described in prose in CLAUDE.md ("heartbeat every 15 s, stale after 45 s,
   dead after 120 s, task reclaimed after timeout") but never written as a
   formal state machine. Subtle bugs like "task claimed by A, A dies, B
   claims, A recovers, both emit complete" can only be caught by running
   HexFlo under load and waiting.

4. **7-phase feature lifecycle** (`hex-cli/src/pipeline/supervisor.rs`) —
   Specs → Plan → Worktrees → Code → Validate → Integrate → Finalize, with
   fork/join parallelism inside the Code phase (tiered adapter dispatch).
   The phase ordering, tier barriers, and BLOCKING gates are encoded as
   Rust control flow in a 3,646-line supervisor file.

### What's broken without a formal model

- **`hex analyze` proves the wrong thing.** It verifies the import graph
  respects the layering, but it does *not* verify that a program in a given
  layer only *invokes* operations from the permitted signature. A
  domain-layer function that imports `IInferencePort` but never calls it
  looks identical to one that does.
- **Swarm coordination bugs surface only under load.** The agent lifecycle
  (register → heartbeat → claim → work → complete) is a classic
  concurrent-system spec that benefits enormously from model checking, but
  hex has no such model.
- **Lifecycle phase-ordering errors are easy to introduce.** The supervisor
  code has enough ad-hoc if/else that a refactor could accidentally let a
  Tier-3 step start before Tier-1 completes. A reachability analysis on a
  workflow net would catch this; inspection by hand will not.
- **Capability grants are runtime-enforced, not type-enforced.** The
  `secret-grant` module distributes `FileSystem(path)`, `TaskWrite`,
  `Memory(scope)` tokens to agents, but the agent code that receives them
  isn't typed against an effect row — the check runs at the moment of
  invocation, after the agent is already executing.

### Why algebra?

Algebraic methods (process calculi, free monads, workflow nets, effect row
types) exist *because* these are exactly the classes of problems they solve
for. hex's four layers each map cleanly to a well-studied formalism:

| hex layer | Natural algebra | What it proves |
|---|---|---|
| Ports | Free Σ-algebra over an effect signature | Layer L programs only use operations in Σ_L |
| Tier routing | Kleisli composition over Option/Result | Pipeline shape, short-circuit correctness |
| Swarm coordination | π-calculus (or CCS/CSP) | Deadlock freedom, liveness, task-loss safety |
| 7-phase lifecycle | 1-safe workflow Petri net | Reachability, phase ordering, BLOCKING gates |
| Capabilities | Effect row types / linear logic | Compile-time capability subsumption |

These aren't new ideas — they're 30-50 year old formalisms with mature
tooling (TLA+, ProB, Koka, OCaml 5 effects, CADP, mCRL2). The point of
this ADR is to *pick* which ones we commit to, in what order, and with
what scope.

## Decision

We will formalize hex's process flow as a **stack of algebras, one per
layer**, not a single monolithic calculus. The stack is:

```
┌───────────────────────────────────────────────────────────────┐
│  7-phase lifecycle           │ 1-safe workflow Petri net       │
├──────────────────────────────┼─────────────────────────────────┤
│  Swarm coordination          │ π-calculus / TLA+                │
├──────────────────────────────┼─────────────────────────────────┤
│  Tier routing                │ Kleisli composition (pure)       │
├──────────────────────────────┼─────────────────────────────────┤
│  Ports + capabilities        │ Free Σ-algebra + effect rows     │
└──────────────────────────────┴─────────────────────────────────┘
```

Each layer's algebra is specified in `docs/algebra/<layer>.md` and
cross-referenced from the ADR for the corresponding subsystem. The
specifications are **descriptive, not prescriptive**: they document the
algebra the existing code already implements, and point out discrepancies
where the code drifts from the intended model. Fixing those discrepancies
is a follow-up ADR, not this one.

### What we are NOT committing to (non-goals)

- **No rewrite of hex-core in Haskell / OCaml / Lean.** The algebra is a
  model, not an implementation language. hex stays in Rust.
- **No mandatory theorem proving.** Proofs are welcome where they're cheap
  and catch real bugs. We do not require every commit to ship a Coq proof.
- **No runtime overhead.** The Σ-algebra is a compile-time model. The
  workflow Petri net lives in documentation, not as a library dependency.
  The π-calculus spec is a TLA+ file checked in CI.
- **No blocking of in-flight feature work.** The algebra documents the
  system *as it is today* plus the small invariants we want to hold. It is
  not a rewrite gate.

### Specifying the layers (phased)

The formalization is rolled out in **four core phases (P1–P4)** plus two
optional follow-up phases (P5–P6) that the Implementation table lists
separately. Each deliverable is a single markdown file + optional TLA+
spec; phases are independently useful so the project stays shippable
between them.

**Phase 1 — Ports Σ-algebra** (immediate, in this ADR's first commit):

`docs/algebra/ports-signature.md` enumerates the 10 port traits in
`hex-core/src/ports/` as an algebraic signature Σ. For each port:

- The set of operations (trait methods)
- The type signature of each operation (arguments → result)
- The error type (bottom element of the Result codomain)
- The layer stratum it belongs to (domain / ports / usecases / adapters)
- Any preconditions or side-effect classes the operation implies

The document includes a short "Free algebra view" section explaining that
a program in layer L is a term in `T(Σ_L)` and the composition root is the
unique Σ-algebra morphism into `IO`. The hexagonal boundary rules become
theorems about which sub-signatures are visible to which layer.

**Phase 2 — Workflow net for the 7-phase lifecycle** (follow-up ADR):

`docs/algebra/lifecycle-net.md` + optional `docs/algebra/lifecycle.tla`.
The 7 phases are encoded as a 1-safe Petri net with fork/join inside the
Code phase. Reachability properties are machine-checkable via TLA+ TLC or
ProB. This is the cheapest win after phase 1 because the supervisor's
state machine is already sequential — encoding it as a net is mostly
transcription.

**Phase 3 — π-calculus / TLA+ spec for HexFlo** (follow-up ADR):

`docs/algebra/hexflo-swarm.md` + `docs/algebra/hexflo.tla`. Models agents
as processes, SpacetimeDB reducers as channels, heartbeat timeouts as
timed transitions. Checks deadlock freedom, no-task-loss under crash,
CAS correctness of task claims. This is the highest-payoff phase (bugs
here are the worst to diagnose in production) but also the most work.

**Phase 4 — Effect rows for capabilities** (follow-up ADR, optional):

`docs/algebra/capabilities-rows.md`. Formalizes the `secret-grant`
capability set as an effect row type. If we ever add a type-level
capability check (e.g., via a `Capability` marker trait or a compile-time
ACL check), this document is the spec it implements against. This phase
is optional because it's the highest-effort and the runtime check already
works for most threat models.

### Validation gates

Each phase must satisfy:

1. **Grounded in real code.** The signature is extracted from or verified
   against the actual source files. No aspirational signatures.
2. **Cross-referenced from the relevant ADR.** Each algebra file links
   back to the ADR that owns the subsystem it formalizes, and vice versa.
3. **Discrepancies flagged explicitly.** If the existing code diverges
   from the algebra, the divergence is listed in a "Known gaps" section,
   not silently papered over.
4. **Stays under 1000 lines.** A document nobody will read is worse than
   no document. Aim for executive-level brevity with links to deeper
   references.

## Consequences

**Positive:**

- **Catches classes of bugs that prose can't.** Deadlock freedom,
  phase-ordering errors, capability leaks, cross-layer effect escape — all
  become mechanically checkable or proof-obligation-generating.
- **Gives `hex analyze` a stronger story.** Today it checks import edges;
  with a Σ-algebra it can also check operation signatures, which means it
  can catch "domain function imports inference port but never calls it"
  as a dead import, and "adapter calls operation outside its capability
  row" as a violation.
- **Makes HexFlo reviewable by people who didn't write it.** A TLA+ spec
  is orders of magnitude easier for a new reviewer to understand than a
  2,000-line Rust coordination module with subtle concurrency.
- **Locks in the existing architecture's strength.** The hexagonal
  layering *already is* a stratified algebra. Making the stratification
  explicit prevents future refactors from accidentally flattening it.
- **Generative test oracles for free.** Given Σ, you can generate random
  terms in `T(Σ_L)`, evaluate them under the real interpreter and a mock,
  and catch divergences. This is more powerful than example-based tests
  because it exhaustively covers the signature.

**Negative:**

- **Documents drift.** If nobody updates the algebra when the code
  changes, the spec becomes a lie. Mitigation: tie the Σ-algebra file to
  the existing `hex readme validate` pattern — add a check that every
  `pub trait I*Port` in `hex-core/src/ports/` appears in the doc, and
  vice versa.
- **Some readers will be intimidated.** "Free Σ-algebra morphism" is a
  non-zero barrier to entry. Mitigation: lead with prose, put the
  notation in a "Formal statement" subsection, and include a worked
  example per port.
- **Opportunity cost.** Time spent writing TLA+ is time not spent
  shipping features. Mitigation: the phases are independent and
  deliverable separately. Phase 1 costs one afternoon; phases 2-4 are
  opt-in when their subsystem next needs a deep change.
- **Formal methods != zero bugs.** Model-checking TLA+ proves properties
  of the *model*, not of the Rust implementation. A spec-implementation
  mismatch is still a bug. Mitigation: use the spec as a *guide for
  testing*, not a replacement for testing.

**Mitigations:**

- **`hex readme validate` extension.** Add a check in phase 1 that every
  `pub trait I*Port` in `hex-core/src/ports/` appears as a section in
  `docs/algebra/ports-signature.md` — mirroring the agent/module/crate
  checks already in place for the README.
- **Every algebra file has a "Known gaps" section.** Drift is
  acknowledged rather than hidden.
- **One formalism per layer, not a grand unified theory.** Nobody has to
  learn category theory to read the workflow net.
- **The four phases are independent.** If phase 2 never gets written,
  phase 1 is still useful standalone.

## Implementation

| Phase | Description | Validation Gate | Status |
|-------|-------------|-----------------|--------|
| P1 | Write `docs/algebra/ports-signature.md` enumerating all 10 port traits as a Σ-algebra with operations, type signatures, errors, and layer stratum. Include a "Free algebra view" section explaining the interpreter as a morphism into IO. Cross-reference from CLAUDE.md's "Hexagonal Architecture Rules" section. | Document exists; every `pub trait I*Port` in `hex-core/src/ports/` has a corresponding section; file length <1000 lines | **In progress (this commit)** |
| P2 | Extend `hex readme validate` with a `ports-signature-matches-source` check: every trait in `hex-core/src/ports/*.rs` must appear in `docs/algebra/ports-signature.md`. Fails CI if a new port is added without updating the algebra doc. | New validator check reports 0 drift; CI passes | Pending |
| P3 | Write `docs/algebra/lifecycle-net.md` — 1-safe workflow Petri net for the 7-phase feature lifecycle. Places, transitions, tokens, fork/join semantics. Include an ASCII art diagram. Optional TLA+ encoding in `docs/algebra/lifecycle.tla`. | Document covers all 7 phases + tier barriers; reachability argument for the accepting state | Pending |
| P4 | Write `docs/algebra/hexflo-swarm.md` — π-calculus (or CCS) spec for HexFlo coordination. Agent lifecycle, heartbeat timeout, task claim CAS, reclamation. Optional TLA+ in `docs/algebra/hexflo.tla` with TLC-checkable invariants (no task loss, progress, deadlock freedom). | Document covers the full agent protocol; TLA+ (if shipped) passes TLC on the base spec | Pending |
| P5 | Write `docs/algebra/capabilities-rows.md` — effect row types for the `secret-grant` capability set. Subsumption rules, grant/revoke semantics, TTL invariants. | Document enumerates all grantable capabilities + subsumption lattice | Pending |
| P6 | Adversarial review: pick the three nastiest concurrency / boundary bugs ever found in hex, write them up, and confirm the algebra would have caught each one. | 3 case studies in `docs/algebra/postmortems.md` | Pending |

Only **P1 is in scope for this commit**. P2-P6 are follow-up ADRs or
follow-up commits under this ADR once the pattern is proven useful.

## References

- **Existing code:**
  - `hex-core/src/ports/*.rs` (10 trait files, the signature to formalize)
  - `hex-core/src/composition_root.rs` (future: the interpreter morphism)
  - `hex-nexus/src/coordination/mod.rs` (HexFlo — phase 4 subject)
  - `hex-cli/src/pipeline/supervisor.rs` (7-phase lifecycle — phase 3 subject)
- **Related ADRs:**
  - ADR-2603221959 — enforcement port (prerequisite for the Σ-algebra view)
  - ADR-027 — HexFlo native Rust coordination (phase 4 subject)
  - ADR-2603240130 — declarative swarm behaviors (phase 4 supporting material)
  - ADR-2603291900 — hexagonal architecture rules (the theorem the Σ-algebra proves)
  - ADR-2604110227 — auto-invoke planner on T3 (example of tier routing as Kleisli)
- **Foundational literature:**
  - Plotkin & Power, "Adequacy for Algebraic Effects" (2001) — the free-algebra view of effects
  - Milner, "A Calculus of Communicating Systems" (1980) — CCS
  - Milner, Parrow & Walker, "A Calculus of Mobile Processes" (1992) — π-calculus
  - van der Aalst, "The Application of Petri Nets to Workflow Management" (1998) — workflow nets
  - Leijen, "Koka: Programming with Row-Typed Effects" (2014) — effect row types
  - Lamport, "Specifying Systems" (2002) — TLA+
- **Tooling we'd use:**
  - TLA+ / TLC — https://lamport.azurewebsites.net/tla/tla.html
  - ProB — https://prob.hhu.de/
  - Koka (reference for effect rows) — https://koka-lang.github.io/
