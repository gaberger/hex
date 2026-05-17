# ADR-2604261801: Substrate ↔ inference integration model

**Status:** Accepted
**Implementation-Present:** 2026-05-12 by auto-scan — evidence: hex-nexus/src/adapters/inference_router/mod.rs, hex-nexus/src/lib.rs
**Date:** 2026-04-26
**Accepted:** 2026-04-26
**Drivers:** ADR-2604261500 (substrate) defined `RuntimeComposition` as a port→adapter binding the substrate can swap. wp-substrate-shadow-promotion wired a `ShadowRouter` that intercepts calls between consumers and bound adapters. The remaining question — and this ADR's subject — is *what level* the substrate intercepts inference at: the per-server level (mirror to a different Ollama instance) or the strategy level (mirror to a different routing policy entirely). The answer determines what consumers do with the `ShadowRouter` handle now wired into `AppState`.
**Supersedes:** None — this is a clarifying ADR for ADR-2604261500's first port migration.

## Context

`hex-nexus` has two interacting abstractions today:

1. **`IInferencePort`** (hex-core) — the call-shape contract. `complete(req) -> response`.
2. **`InferenceRouterAdapter`** (`hex-nexus/src/adapters/inference_router/mod.rs`, ~750 lines including tier_config) — a strategy that picks a model + server per call, applies RL-driven scoring, records reward, talks to the chosen server via a per-call `IInferencePort` adapter built by the injected factory.

Substrate `RuntimeComposition` says: bind one adapter per port; swap the binding atomically; mirror traffic to a candidate during shadow.

These two collide on the "what gets bound" question:

- **Server-level binding** would put each Ollama / vLLM / Anthropic server endpoint behind its own `AdapterId` and have `ShadowRouter` choose between them per call. This conflicts with `InferenceRouterAdapter`'s RL-driven server selection — both abstractions want to make the per-call routing decision, with no clean way to compose.
- **Strategy-level binding** puts the *whole* `InferenceRouterAdapter` (or any alternative strategy — round-robin, hardcoded, even a different RL policy) behind a single `AdapterId`. The substrate then swaps *strategies*. Per-call routing within a strategy is the strategy's concern; substrate doesn't see it.

Strategy-level wins on hexagonal grounds: a port has one binding, the binding implements the call-shape, internal complexity stays inside the binding. It is also what the ShadowRouter design as-built supports (one live handle, one candidate handle, mirror calls between them).

### Where the substrate stands today

`AppState` now holds:
- `inference_runtime_composition: Option<Arc<SpacetimeRuntimeComposition>>` — port `inference` bound to adapter id `default-inference`.
- `inference_shadow_router: Option<Arc<ShadowRouter>>` — wraps the composition, with the live `IInferencePort` handle (currently `OllamaInferenceAdapter` in standalone mode) registered against `default-inference`.

The substrate is built but **bypassed**: every existing call site still calls `inference_port.complete(req)` directly. Until consumers opt in by routing through `shadow_router.route(PortId::new("inference"), req)`, the substrate is dormant.

### Forces

- **Backward compatibility.** Consumer rewires must be additive, opt-in per call site. Breaking the existing `inference_port.complete` contract or its perf characteristics would invalidate every dispatch path.
- **Hexagonal preservation.** Strategy-level binding aligns with "adapters never import other adapters" — `InferenceRouterAdapter` is one adapter implementing one port, and an alternative strategy is just another adapter.
- **Substrate semantic clarity.** Mirroring at the strategy level lets the substrate answer questions like "what fraction of inference traffic goes to RL-policy-v2 vs RL-policy-v1?" — meaningful at the dashboard level. Mirroring at the server level produces noise (RL was trying to load-balance; substrate is fighting it).

### Alternatives considered

| Alternative | Rejected because |
|-------------|------------------|
| Server-level binding, retire RL-driven router | Discards a year of RL infrastructure and the per-call learning loop |
| Server-level binding, RL only inside one adapter | Restricts each adapter to one server, breaking the multi-server fleet model |
| No substrate intercept of inference at all | Defeats the substrate's first-port pilot; blocks ADR-2604261500 P10 (second-port migration justification) |

---

## Decision

**The substrate intercepts inference at the strategy level.** A bound `IInferencePort` adapter is a *whole inference strategy* — `InferenceRouterAdapter`, an alternative `InferenceRouterAdapter` configured with different RL state, a hardcoded fallback, or a generated adapter (per ADR-2604261500 C4). The substrate swaps strategies; per-call routing within a strategy stays the strategy's responsibility.

Concretely:
1. **Default binding** is the existing inference dispatch path bound under `AdapterId("default-inference")`.
2. **Candidate strategies** for swap proposals are alternative `Arc<dyn IInferencePort>` instances with distinct adapter ids. Examples:
   - `InferenceRouterAdapter` with a different `tier_models` (e.g. `qwen3:4b` everywhere instead of tier-resolved).
   - `InferenceRouterAdapter` with RL state cleared (cold-start comparison).
   - A fixed-server adapter (no RL, single Ollama target) — useful for incident response.
3. **Consumers route via `ShadowRouter::route`** when `inference_shadow_router` is `Some`; otherwise fall back to direct `inference_port.complete`. Substrate is opt-in at the call site, never load-bearing.

### Consumer-rewire policy

A consumer is "substrate-aware" when it:
1. Holds `Option<Arc<ShadowRouter>>` from `AppState`.
2. Calls `shadow_router.route(PortId::new("inference"), req)` if present.
3. Falls back to `inference_port.complete(req)` if not.

Initial rewire targets, in priority order:
1. `InferenceRouterAdapter::route_request` — the direct `adapter.complete()` call site at `hex-nexus/src/adapters/inference_router/mod.rs:295`. This is the per-server call after RL selection. **Skip for the strategy-level model** — wrapping inside the router doesn't help; the router itself is the strategy.
2. `WorkplanExecutor` Path C dispatch — calls `inference_port.complete` for headless task execution. **Primary rewire candidate.**
3. `agent_manager` dispatch — secondary candidate.
4. Any future consumer added to `AppState.inference_port`.

### What remains explicitly out of scope

- **Per-call mirroring inside `InferenceRouterAdapter`.** That conflicts with RL routing.
- **STDB schema change to include strategy-id telemetry.** The existing `swap_ticket` schema is sufficient; the strategy id *is* the adapter id.
- **Auto-migration from `inference_port.complete` to `shadow_router.route`.** Per call site, opt-in, with explicit fallback.

---

## Consequences

**Positive:**
- Clean separation of concerns: substrate decides "which strategy is live"; strategy decides "which server within."
- The existing `InferenceRouterAdapter` becomes a swappable unit without modification.
- `IAdapterGenerator` (ADR-2604261500 C4) gets a clear target: generate alternative `IInferencePort` strategies, not server-level adapters.
- Dashboard `/swaps` view (P5.1) shows what it's supposed to show: which strategies are being compared.

**Negative:**
- Wholesale strategy comparison is more expensive per call than per-server comparison (full RL+routing overhead doubled during shadow).
- Strategy candidates have to be constructible at swap-propose time, which means their dependencies (state ports, factories) need to be resolvable from the substrate's swap context. Today `InferenceRouterAdapter::new` takes hand-picked args; we'll need a builder or factory that swap proposals can call.
- Consumer rewires are spread across multiple files; coordination is each consumer's call.

**Mitigations:**
- Default `shadow_traffic_fraction` stays at 5% per ADR-2604261500 — strategy-level shadow at 5% is roughly the cost of a single per-server miss-route in normal operation.
- Strategy builder/factory work is queued as part of the C4 (`IAdapterGenerator` first impl) workplan.
- Consumer rewires are tracked as individual workplan tasks in `wp-substrate-inference-consumer-rewires.json`.

---

## Implementation

| Phase | Description | Status |
|-------|-------------|--------|
| P1 | AppState fields + boot wiring (`inference_runtime_composition`, `inference_shadow_router`) | **Done** (this turn — see `lib.rs` substrate-boot block) |
| P2 | `WorkplanExecutor` opt-in to `shadow_router.route` with fallback | Pending |
| P3 | `agent_manager` opt-in to `shadow_router.route` with fallback | Pending |
| P4 | Strategy-builder helper so swap proposals can construct alternative `InferenceRouterAdapter` configurations | Pending |
| P5 | First real swap proposal — `default-inference` vs `cold-rl-inference` (RL state cleared) — flush through to `shadow_green` | Pending |

P1 is unblocked and shipped. P2–P5 are sized as a follow-up workplan: `wp-substrate-inference-consumer-rewires.json`.

---

## References

- ADR-2604261500 — Substrate (defines `RuntimeComposition`, the port-binding model)
- ADR-2604261303 — IModelProvider port + crate split (re-frames `IInferencePort` as the prototype substrate port)
- wp-substrate-shadow-promotion — built `SpacetimeRuntimeComposition`, `ShadowRouter`, `PromotionJudge`
- `hex-nexus/src/adapters/inference_router/mod.rs:225` — `route_request` (the strategy whose substitution this ADR governs)
- `hex-nexus/src/lib.rs` substrate-boot block — the `inference_runtime_composition` + `inference_shadow_router` wiring
