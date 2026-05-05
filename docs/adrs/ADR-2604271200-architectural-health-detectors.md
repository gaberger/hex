# ADR-2604271200: Architectural-health detectors — the improver interrogates domain → port → adapter

**Status:** Accepted (2026-05-05)
**Date:** 2026-04-27
**Drivers:** ADR-2604271100 names a discovery loop but its detector vocabulary today is operational-drift (ADR drift, workplan drift, RL-loop health). It does not interrogate whether the software *itself* is well-designed and efficiently operating: are domain types well-factored, are ports cohesive, are adapters carrying their weight, is the composition root drifting from the ADRs that authored it. The substrate (ADR-2604261500) ships `PortTelemetry` (commit `5556a65f`) and a shadow-promotion ledger that already produce the runtime signal needed; `hex analyze` already produces the static signal. Neither feeds the improver. This ADR routes both into the discovery surface so the self-learning loop interrogates the hexagon end-to-end.
**Depends on:** ADR-2604271100 (sched improver), ADR-2604261500 (substrate), ADR-2604261311 (six-layer governance + PortTelemetry), ADR-001 (hexagonal architecture), ADR-2603283000 (Rust workspace boundary analysis)
**Relates to:** ADR-2604142243 (rules as data), ADR-008 (dogfooding), ADR-031 (RL-driven model selection)

## Context

### What "self-learning" means today vs. what it should mean

Today the improver (once it lands) checks: "is the JSON-of-record consistent with reality?" That's drift detection. It does not ask:

1. **Is the design itself coherent?** — A workplan can ship code that compiles and tests-pass but introduces a god-port, leaky abstraction, or orphan adapter. None of that surfaces.
2. **Is the system operating efficiently?** — Latency drift through a specific port, adapter skew (same port, different adapters performing differently), traffic concentration that argues for a port split: all observable, none surfaced.
3. **Is the substrate's promise being exercised?** — The substrate exists to enable hot-swap of adapters. If no swaps have happened in N weeks, either we built unused machinery or we're missing a swap opportunity. Same loop.

Hexagonal architecture is structurally an *audit-able* design — every coupling is named (port), every external dependency is named (adapter), every wiring is centralized (composition root). The improver should walk that structure on every tick.

### What primitives already exist

| Signal | Source | Status |
|---|---|---|
| Domain → port import discipline | `hex analyze .` (tree-sitter, ADR-2603283000) | shipped |
| Cross-adapter import (forbidden) | `hex analyze .` | shipped |
| Composition-root wiring map | `hex analyze .` AST | shipped |
| Per-port runtime metrics (call count, p99 latency) | `PortTelemetry` trait (commit `5556a65f`) | shipped (instrumentation hook) |
| Shadow-promotion history (swap diversity) | substrate `swap_ticket` STDB table | shipped |
| Adapter call edges | tree-sitter on adapter implementations | shipped |
| Dead-code analysis | `dead-code-analyzer` agent | shipped |

What's missing is a *detector* row per signal class in `hex-cli/assets/improver/detectors.toml`, and the formatter that turns each into a `Hypothesis`. No new analysis engines.

## Decision

Add ten architectural-health detectors to the improver. Each maps to existing CLI/MCP commands or STDB queries, emits JSON, and becomes a hypothesis the adversarial-swarm proposes against and the judge picks.

### Detector taxonomy

Three classes, each with concrete rows:

#### A. Static design detectors

Run cheaply on every improver tick. Inputs: source AST + composition-root parse.

| Detector | Signal | Existing primitive | Hypothesis kind |
|---|---|---|---|
| `domain_leak` | Domain types referenced from adapter code without going through a port | `hex analyze . --json` (already detects layer violations) | "leak from domain X via adapter Y → propose anti-corruption boundary or port surface" |
| `port_cohesion` | Port trait with >7 methods, or methods grouping into ≥2 unrelated concerns (heuristic: noun-cluster split) | `hex analyze . --port-cohesion` (new flag, AST-only) | "port P appears multi-concern → propose split into Pa + Pb" |
| `adapter_orphan` | Adapter implementing a port but not registered in the composition root | composition-root parse + adapter trait impls | "adapter A is unwired → propose register-or-delete" |
| `port_orphan` | Port with no adapter implementing it | trait + impl scan | "port P has no concrete adapter → propose stub adapter or remove port" |
| `adapter_duplication` | Two adapters implementing similar logic the use case could hoist | dead-code-analyzer + similarity heuristic | "adapters A and B share 60%+ logic → propose use-case-level extraction" |
| `god_domain_type` | Domain type with >300 LOC or >10 public methods | tree-sitter | "domain type T is a god-class → propose decomposition" |

#### B. Operational efficiency detectors

Run on hourly cadence. Inputs: `PortTelemetry` STDB rows + `swap_ticket` history.

| Detector | Signal | Existing primitive | Hypothesis kind |
|---|---|---|---|
| `port_latency_drift` | p99 latency through a port rose >25% week-over-week | `PortTelemetry` STDB rollup | "port P latency drift → propose adapter swap, caching layer, or substrate shadow-test" |
| `adapter_skew` | Same port, two adapters bound to different operators show >2x latency or success-rate variance | `PortTelemetry` per `(port_id, adapter_id)` | "adapter A2 outperforms A1 on port P → propose swap via shadow-promote" |
| `port_traffic_concentration` | One port handles >70% of total inference / request traffic | `PortTelemetry` aggregate | "port P is hot → propose split, batching layer, or dedicated adapter pool" |
| `adapter_unused` | Adapter wired but zero calls in 7 days | `PortTelemetry` + composition-root | "adapter A unused → propose remove or document the future-use case" |

#### C. Composition coherence detectors

Run on improver tick. Inputs: ADR list + composition-root + workplan history.

| Detector | Signal | Existing primitive | Hypothesis kind |
|---|---|---|---|
| `composition_drift` | Composition-root churn rate exceeds ADR acceptance rate over 30 days | `git log` + ADR list | "wiring is being changed faster than ADRs justify it → propose ADR-up or revert" |
| `substrate_swap_starvation` | Zero swap_ticket entries promoted in 30 days | `swap_ticket` STDB | "substrate's hot-swap value is unused → propose first-swap experiment or scope-down ADR-2604261500" |
| `dead_layer` | An entire layer dir (e.g. `usecases/`) with no inbound edges from primary adapters | tree-sitter call graph | "layer L is bypassed → propose layer collapse or restore the missing edge" |

### Detector authoring rule

Per ADR-2604142243, detectors are TOML rows in `hex-cli/assets/improver/detectors.toml`. Adding a new detector = adding a row + ensuring the referenced CLI command emits JSON. No Rust changes for new detectors.

The detectors above ship as a single batch in `wp-architectural-health-detectors`. After that, operators (and eventually the improver itself, Tier-C with human approval) propose new detector rows.

### What the improver does with these

The existing P2 + P3 pipeline (adversarial variants + judge) is unchanged. Each architectural finding becomes a hypothesis. The judge's rubric (alignment / blast-radius / dependency-satisfaction / reversibility / historical-reject-rate) already handles strategic variant scoring; no new judge axis is needed for v1.

For Tier-A actions: same envelope as the operational improver — write ADR + workplan to docs/, archive losers, enqueue. No code-mutating action without an explicit operator promote.

## Consequences

**Positive**
- The improver interrogates the hexagon end-to-end on every tick. Domain god-types, port skew, dead adapters, unused substrate machinery all become first-class findings.
- Reuses existing primitives — no new analysis engines, no parallel telemetry pipeline.
- The existence of the `substrate_swap_starvation` detector means the substrate's value is itself audited by the improver. That closes the meta-loop: hex's self-learning loop interrogates whether hex's own self-learning is justified.

**Negative / risks**
- False-positive risk on cohesion/duplication heuristics. Mitigations: each detector emits at most one hypothesis per scope per tick (deduplicated), and the judge's confidence threshold (0.6) already gates low-confidence propositions to P2 inbox rather than auto-action.
- Telemetry-cost: per-port metrics aggregation runs on every tick. Cheap (<10ms for the current ~30-port surface) but worth bounding when the system grows. Add a per-tick budget cap to the improver's tick handler if/when this becomes load-bearing.
- The "god type / cohesion" heuristics are subjective. We intentionally err on emitting a hypothesis (not auto-acting) — humans review the proposal, judge picks if confidence is high.

**Out of scope**
- Architecture-fitness-functions in the test suite (Ford/Parsons style). That's a different pattern — assertions in CI, not improver hypotheses. Future ADR.
- ML-driven design pattern detection (e.g. embeddings to find similar adapters across the workspace). Useful eventually; the symbolic detectors above are the cheap-and-correct first cut.
- Detector self-modification (Tier-C). The improver may not propose changes to its own detector table without operator approval.

## Implementation gates

Workplan implementing this ADR is `done` only when:

1. All 13 detector rows above exist in `hex-cli/assets/improver/detectors.toml`, each pointing at a CLI command that emits JSON.
2. Any CLI commands referenced (`hex analyze . --port-cohesion`, etc.) exist and emit JSON; new flags ship under `hex analyze`.
3. The `PortTelemetry` rollup (commit `5556a65f`) emits weekly p99 / call-count rows in STDB so detectors B can read them — not just the existing per-call counter.
4. End-to-end test: plant a fixture workspace with a god-type, an orphan adapter, and a stale swap_ticket; one improver tick must emit ≥3 hypotheses one of each class.
5. The improver-judge's rubric handles architectural-class hypotheses without new code (regression test against the e2e fixture).
