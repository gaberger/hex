# ADR-2026-04-26-1500: Hex as a continuously self-modifying application substrate

**Status:** Accepted
**Implementation-Present:** 2026-05-12 by auto-scan — evidence: hex-core/src/composition.rs, hex-core/src/ports/adapter_generator.rs, hex-core/src/ports/inference.rs (+3 more)
**Date:** 2026-04-26
**Accepted:** 2026-04-26
**Drivers:** ADR-2026-04-26-1303 (IModelProvider + crate split) and ADR-2026-04-26-1311 (six-layer governance) each solve a real problem, but neither states what hex *is*. Read alone they look like cleanup. Read together they imply something the codebase has never named: hex is no longer a build tool that produces applications — it is the runtime substrate that *hosts* applications which rewrite themselves under LLM supervision. Without that frame, every future ADR re-litigates "is this scope creep?" instead of "does this serve the substrate's contract?" This ADR makes the frame explicit and binds the prior two to it as instantiations.
**Supersedes:** None — this is the foundational ADR for the substrate. ADR-2026-04-26-1303 and ADR-2026-04-26-1311 are the first two implementations of the contracts defined here.

## Context

### What hex is becoming, whether we name it or not

The artifacts shipped over the last six months are not the artifacts of a build tool:

- A workplan executor that dispatches LLM agents to write code (`hex-nexus/src/orchestration/workplan_executor.rs`).
- A tier-aware inference router that picks models per-task (`hex-nexus/src/adapters/inference_router/mod.rs`).
- A swarm coordination layer with heartbeats, task reclaim, and reactive subscriptions (`spacetime-modules/hexflo-coordination/`).
- A behavioral-spec writer + judge loop that gates merges on semantic correctness (`hex-cli/assets/agents/hex/hex/`).
- An ADR-driven decision substrate the agents themselves consult before acting (`hex adr search`, `hex adr list`).
- A six-layer adversarial governance proposal that would extend gating from commit-time to run-time (ADR-2026-04-26-1311).

A build tool produces a frozen artifact. None of the above produces a frozen artifact. The system is continuously rewriting its own composition — adapters get added, swapped, deleted; ports gain capabilities; agents are retired; tier policies shift. Every commit is a runtime mutation of the same long-running system. The user is not the runtime's user — the *agents* are.

We have been writing build-tool ADRs for a substrate-shaped system. That mismatch is why every cleanup feels Sisyphean: the cleanup ADRs assume the system stops moving long enough to be tidied, but the system never stops moving.

### Why this matters now (and not as a future-vision doc)

Three concrete pressures force the reframe today:

1. **The IModelProvider port (ADR-2026-04-26-1303) is the first port in the system whose adapter set is expected to grow indefinitely under LLM authorship.** Anthropic, OpenAI, Ollama, OpenRouter, vLLM, llama.cpp today; tomorrow whatever the agent decides it needs. If we treat each new provider as a hand-written adapter, we lose. We need a generator port whose output is itself a hex-conformant adapter.

2. **The six-layer governance proposal (ADR-2026-04-26-1311) is over-scoped for commit-time review and under-scoped for what it actually has to do.** Adversarial swarms reviewing a PR is theatre if the same swarm cannot review *the next runtime composition swap* with the same rigor. Governance has to gate runtime rewrites, not just merges.

3. **The accretion metrics from ADR-2026-04-26-1311 (57 commands, 188 ADRs, 7 STDB modules, all 55 workplans `Failed`) are the symptom of a system without a North Star file.** Every locally-correct decision drifts the global shape because there is no document the agents are required to re-read before deciding "should this exist." Founding goals exist in CLAUDE.md prose and in user memory. They are not a contract; they are not enforced; nothing reads them on a schedule.

These three pressures cannot be solved by another adapter, another agent, or another hook. They require declaring what hex *is* and binding the runtime to that declaration.

### Forces

- **The founding goals must be the only thing in the system that does not get rewritten.** In a self-modifying system, every other artifact is candidate for replacement by the next agent loop. If the goals are also rewritable by agents, the system has no fixed point and drifts unboundedly. Goals must be human-authored, version-controlled, and read by a layer no agent can bypass.
- **Backward compatibility is non-negotiable.** 200+ ADRs, 55 workplans, downstream targets in `examples/`. The substrate frame must be additive — existing ports, adapters, and the composition-root file keep working unchanged.
- **"Self-modifying" must not mean "autonomous-against-the-user."** The user remains the principal. The substrate's job is to make the agent's modifications *legible, gated, and reversible*, not to remove the user from the loop.
- **Hexagonal architecture is load-bearing here, not aesthetic.** Ports are the only construct in the codebase that gives a stable contract while the implementation churns. They are exactly the abstraction a self-modifying system needs. Reframing hex around the substrate idea is what finally explains *why* the hexagonal rules deserve to be enforced as hard rules rather than style preferences.

### Alternatives considered

| Alternative | Rejected because |
|-------------|------------------|
| Don't write this ADR; let 1303 + 1311 stand alone | They would be re-interpreted as "yet another cleanup pass" by the next agent and the substrate framing would be lost |
| Frame hex as an "agent framework" (à la LangChain/CrewAI) | Agent frameworks are libraries the user wires up. Hex is a long-running substrate the user installs into. The framing matters for what we build next |
| Frame hex as a microkernel only | Microkernel describes the *shape* (small core + pluggable services) but not the *behavior* (continuous rewriting under supervision). Substrate is the behavior; microkernel is one of its properties |
| Wait until ADR-2026-04-26-1303 is implemented before writing this | The implementation choices for 1303 (e.g. how `IModelProvider` adapters get registered, how telemetry flows, how shadow-promotion works) are *determined* by this ADR. Writing it after is writing it too late |

---

## Decision

**Hex is a runtime substrate for applications that continuously rewrite themselves under LLM supervision. Hexagonal ports are the contract that makes the rewriting safe. The substrate provides six concrete mechanisms; everything else in the codebase is an implementation of one of them or is governed by all of them.**

### The six substrate contracts

#### C1 — Application = (composition root + ports + adapters + founding goals)

A "hex application" is now formally defined as a 4-tuple: a composition root file, a set of port traits, a set of adapter implementations, and a `founding-goals.md` file at the project root. The first three are rewritable by agents. The fourth is human-authored and writable only by a human commit (enforced by a pre-commit hook + CODEOWNERS).

Any artifact in a hex project that is not one of these four is a derived artifact and may be regenerated. This is the test for "does this thing belong in the repo": if removing it would not change the application as defined by the 4-tuple, it does not belong in the repo and the shrinkage daemon (ADR-2026-04-26-1311 Layer 4) is permitted to delete it.

#### C2 — Hot-swappable composition root API

The composition root becomes a runtime object, not just a file that's read once at boot:

```rust
// hex-core/src/composition.rs
pub trait RuntimeComposition: Send + Sync {
    fn ports(&self) -> &PortRegistry;
    fn snapshot(&self) -> CompositionSnapshot;
    fn propose_swap(&self, swap: CompositionSwap) -> Result<SwapTicket, SwapError>;
    fn promote(&self, ticket: SwapTicket) -> Result<(), SwapError>;
    fn rollback(&self, ticket: SwapTicket) -> Result<(), SwapError>;
}
```

A `CompositionSwap` is a typed proposal: "replace the adapter bound to port `P` with adapter `A'`, leaving all other bindings unchanged." `propose_swap` enters the candidate into shadow mode (C5). `promote` makes it live. `rollback` reverts. The implementation is single-writer (one swap in flight per port) and uses the standard atomic-pointer-swap pattern under the hood.

The current `composition-root.ts` / `hex-nexus/src/composition.rs` files become the *initial* composition. They keep working. They just become editable at runtime, by agents, through a typed interface, gated by C5 + C6 instead of by a fresh build.

#### C3 — Per-port telemetry as a first-class concern

Every port trait declares its telemetry contract alongside its method signatures, in the same crate, owned by the port. Telemetry is not a logging afterthought sprinkled inside adapters:

```rust
// hex-core/src/ports/inference.rs
pub trait IModelProvider: Send + Sync {
    fn complete(&self, req: CompletionRequest) -> BoxFuture<Result<CompletionResponse, ProviderError>>;
    fn capabilities(&self) -> ProviderCapabilities;
}

pub struct InferenceTelemetry {
    pub p50_latency_ms: Histogram,
    pub p99_latency_ms: Histogram,
    pub error_rate: Counter,
    pub tokens_in: Counter,
    pub tokens_out: Counter,
    pub cost_usd: Counter,
}

impl PortTelemetry for IModelProvider {
    type Metrics = InferenceTelemetry;
    fn emit(adapter_id: AdapterId, sample: Self::Metrics);
}
```

A `PortTelemetry` derive macro auto-instruments adapters at registration time so adapter authors (human or LLM) cannot forget. The substrate refuses to register an adapter that does not satisfy its port's `PortTelemetry`. The shadow-promotion protocol (C5) reads telemetry to decide promotion; the governance layers (C6) read telemetry to decide rejection. Telemetry is therefore the substrate's only feedback signal — making it first-class is the load-bearing decision, not the implementation detail.

#### C4 — `IAdapterGenerator` port + first implementation

A port whose adapters are LLM agents that produce other adapters:

```rust
// hex-core/src/ports/adapter_generator.rs
pub trait IAdapterGenerator: Send + Sync {
    fn target_port(&self) -> PortId;
    fn generate(&self, spec: AdapterSpec) -> BoxFuture<Result<GeneratedAdapter, GenError>>;
    fn capabilities(&self) -> GeneratorCapabilities;
}

pub struct GeneratedAdapter {
    pub source: SourceTree,
    pub manifest: AdapterManifest,
    pub shadow_test_plan: ShadowTestPlan,
}
```

The first implementation generates `IModelProvider` adapters from natural-language descriptions of a provider's API. Output is a hex-conformant Rust crate that compiles, registers via `PortTelemetry`, and ships a shadow-test plan the substrate can execute (C5). This is the first point in the codebase where agents author *components of the substrate itself*, not just files inside a repo. The port boundary is what makes that safe: a generated adapter is just another binding behind `IModelProvider`, indistinguishable to consumers.

#### C5 — Shadow-test promotion protocol

A candidate adapter (whether human-written or generator-produced) does not enter the live composition by being merged. Merging adds it to the *registry*. Promotion to live is a separate protocol:

1. **Candidate.** Adapter compiles, passes its own unit tests, satisfies `PortTelemetry`. Registered via `RuntimeComposition::propose_swap` → `SwapTicket(state=Candidate)`.
2. **Shadow.** Substrate routes a configurable fraction of live traffic to *both* the incumbent and the candidate, returning the incumbent's response to the caller and recording the candidate's. Promotion-judge agent (a Layer 3 reviewer from ADR-2026-04-26-1311) compares responses and telemetry against the candidate's `ShadowTestPlan` over a configurable window. State transitions to `ShadowGreen` or `ShadowRed`.
3. **Promote.** Only `ShadowGreen` tickets are eligible. `RuntimeComposition::promote` performs the atomic swap. Old adapter is retained for rollback for a configurable window.
4. **Rollback.** Any port whose post-swap telemetry crosses a configured regression threshold triggers automatic rollback. Manual rollback is also available via CLI.

The whole protocol is the substrate's answer to "how do agents change the running system without breaking it." Every step is a typed transition, recorded in SpacetimeDB, observable from the dashboard, and gated by Layer 3.

#### C6 — Governance gates runtime rewrites, not just commits

ADR-2026-04-26-1311's six layers are re-bound to swap-ticket transitions, not to PR review:

| Layer | ADR-1311 role | Substrate role (this ADR) |
|-------|---------------|----------------------------|
| L1 Sentinel | Lints commits | Lints `AdapterSpec` before generation; rejects spec that would violate hexagonal rules |
| L2 Adversarial swarm | Reviews PRs | Adversarial review of a `Candidate` ticket before it enters Shadow |
| L3 Promotion judge | Judges merge | Judges `ShadowGreen` eligibility from C5 telemetry |
| L4 Shrinkage daemon | Deletes dead code | Deletes adapters the substrate has not routed traffic to within a configured idle window |
| L5 ADR conformance | Validates ADR adherence | Blocks promotion of a swap that violates an Accepted ADR; flags swaps that necessitate a new ADR |
| L6 Founding-goals ritual | Quarterly human review | Reads `founding-goals.md`; blocks promotion of swaps that drift the system from a stated goal; only layer with authority to retire a goal — and only via human commit |

Governance therefore runs continuously against the swap stream, not periodically against commits. Commit review remains as a fast pre-filter. The deeper gating is at swap time.

### Founding goals file

`founding-goals.md` lives at the project root. Format:

```markdown
# Founding Goals

## G1 — <name>
**Stated:** <YYYY-MM-DD by <human>
**Why:** <one paragraph>
**Test:** <one sentence — how Layer 6 decides whether the system still serves this goal>
**Retirement:** <only by human commit removing the goal, with a Retirement-ADR linked>
```

For hex itself, the initial founding goals are the three the existing ADRs already imply but never wrote down:

- **G1 — Model tiering and independence.** The substrate must remain swappable across providers without re-architecting consumers.
- **G2 — Multi-host scaleout.** The substrate must be able to run as a fleet, with placement as a property of the substrate, not a property of the user.
- **G3 — Hexagonal rigor at the workspace level.** Every crate must conform to the layering rules, not just the TS source tree.

Layer 6 reads this file quarterly. Every Accepted ADR must cite which goal(s) it serves. Any swap whose telemetry shows drift away from a goal's `Test` is grounds for L6 rejection.

### Where the prior ADRs land

| Prior ADR | Was framed as | Re-frames as |
|-----------|---------------|--------------|
| ADR-2026-04-26-1303 — `IModelProvider` + crate split | Refactor / cleanup | First implementation of C2 (composition root API) and C3 (per-port telemetry); the crate split is the unit of swap for C5 |
| ADR-2026-04-26-1311 — six-layer governance | Anti-accretion process | Safety harness for C5 + C6; layers re-bound from commit-time to swap-time |
| ADR-2026-04-12-0202 — tier-aware routing | Inference policy | Becomes a *policy adapter* behind `IModelProvider`, swappable like any other |
| ADR-2026-04-24-1820 — RL-aware model selection | Optimization | Becomes a generator candidate for routing-policy adapters |
| Shrinkage daemon | Dead-code cleanup | C6/L4: entropy sink that keeps the search space tractable for the generator |

Without this ADR, the above are five disconnected initiatives. With it, they are five components of one coherent program.

---

## Consequences

**Positive:**

- Future ADRs have a single test for relevance: "does this implement, gate, or extend one of the six contracts?" If no, the ADR probably belongs in a downstream project, not in hex.
- The hexagonal rules become *necessary* rather than stylistic. Ports are the only construct that holds a stable contract while implementations swap at runtime; reframing hex as a substrate makes that load-bearing.
- LLM agents now have a typed surface (`RuntimeComposition`, `IAdapterGenerator`, `PortTelemetry`) for modifying the substrate. They no longer modify by editing files at the workspace level — they modify by submitting swap tickets. That is enforceable in a way that "please follow the architecture rules" is not.
- The accretion failure mode from ADR-2026-04-26-1311 gets a structural answer: the only artifacts that survive are those tied to a port + adapter binding the substrate is actively routing to. Everything else gets deleted by L4 because nothing is calling it.
- Multi-host scaleout (G2) becomes a substrate property, not a separate roadmap item: a swap on host A propagates as a `CompositionSwap` event the other hosts subscribe to.

**Negative:**

- Heavy conceptual lift. The codebase, agents, and downstream consumers must learn to think in swaps and tickets, not in commits and PRs. This will take months and several broken assumptions.
- The substrate model adds a layer of indirection: every port call now goes through `RuntimeComposition::ports().resolve(P)` rather than a direct field read on a composition struct. Latency cost is small but non-zero, and stack traces become harder to read.
- Shadow-test traffic doubling for `IModelProvider` is *expensive* in inference cost. The promotion protocol must default to a small shadow fraction (1–5%) and require an opt-in for higher.
- Founding goals are an irrevocable governance artifact. A bad initial set will haunt the project for as long as the project exists. The three proposed above (G1–G3) are themselves a decision worth its own scrutiny.
- Implementing C2 and C5 correctly is research-grade work. The first six months will be uncomfortable as we discover edge cases (port-version skew during swap, cross-port invariants, ordering of dependent swaps).

**Mitigations:**

- The substrate model is *additive*: existing composition roots keep working as the "initial composition." Nothing in the codebase has to change on day one. Migration is per-port, opt-in, gated by C5.
- The indirection cost is paid only at port resolution, not per-call; resolved adapter handles are cached per consumer.
- Default shadow fraction is 1%; raising it requires an ADR citing a specific goal.
- Founding goals are *editable* via human commit + Retirement-ADR. The constraint is that no agent may rewrite them, not that they are eternal.
- The first implementation of C2/C5 is scoped to one port (`IModelProvider`) and uses a deliberately minimal `RuntimeComposition` — proof of concept before generalization. ADR-2026-04-26-1303's crate split is the carrier for this.

---

## Implementation

| Phase | Description | Depends on | Status |
|-------|-------------|------------|--------|
| P1 | Land `founding-goals.md` at repo root with G1–G3; add CODEOWNERS rule restricting edits to humans; add pre-commit hook rejecting agent-authored edits | — | Pending |
| P2 | Define `hex-core/src/composition.rs` — `RuntimeComposition`, `CompositionSwap`, `SwapTicket`, `PortRegistry`. No behavior beyond an in-memory implementation that wraps the existing static composition | — | Pending |
| P3 | Define `hex-core/src/telemetry.rs` — `PortTelemetry` trait, derive macro, `AdapterId`. Wire to existing tracing/metrics | P2 | Pending |
| P4 | Define `hex-core/src/ports/adapter_generator.rs` — `IAdapterGenerator` trait, `AdapterSpec`, `GeneratedAdapter`, `ShadowTestPlan`. No implementation yet | P2, P3 | Pending |
| P5 | Implement `IModelProvider` per ADR-2026-04-26-1303 *as the first port behind `RuntimeComposition`* — registered via the new APIs, telemetry-instrumented, ready for shadow swaps | P2, P3, ADR-1303 | Pending |
| P6 | Implement shadow-promotion protocol (C5) for `IModelProvider` only. SpacetimeDB tables for `swap_ticket`, `shadow_sample`. Dashboard view | P5 | Pending |
| P7 | Implement first `IAdapterGenerator` adapter — generates `IModelProvider` adapters from a JSON `AdapterSpec`. Output passes through C5 | P4, P6 | Pending |
| P8 | Re-bind ADR-2026-04-26-1311 layers L1–L6 to swap-ticket transitions per the C6 table. L4 (shrinkage) given authority to delete adapters not routed to within an idle window | P5–P7, ADR-1311 | Pending |
| P9 | First Layer 6 quarterly ritual against `founding-goals.md`. Output is either "no drift," a swap-rejection list, or a Retirement-ADR proposal | P1, P8 | Pending |
| P10 | Migrate a second port (`ICoordinationPort` is the natural candidate) to validate the abstractions generalize | P5–P8 | Pending |

P1, P2, P3 are unblocked today and have no dependencies on the other ADRs. They establish the contracts the rest of the program will fill in. P4 is small and unlocks P7. P5–P10 sequence behind ADR-2026-04-26-1303 and ADR-2026-04-26-1311.

---

## References

- ADR-2026-04-26-1303 — IModelProvider port + crate split (first implementation of C2 + C3)
- ADR-2026-04-26-1311 — Six-layer governance (re-bound to swap-time per C6)
- ADR-2026-04-12-0202 — Tier-aware routing (becomes a policy adapter)
- ADR-2026-04-24-1820 — RL-aware model selection (becomes a generator candidate)
- ADR-2026-04-11-0227 — Task tier routing (consumer of the new `IModelProvider`)
- ADR-2026-04-05-0900 — Trace-all-consumers rule (load-bearing for shrinkage daemon under L4)
- `docs/analysis/classifier-inventory.md` — empirical input to the founding-goals decision
