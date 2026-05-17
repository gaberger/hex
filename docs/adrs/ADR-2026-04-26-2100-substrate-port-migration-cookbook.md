# ADR-2026-04-26-2100: Substrate port-migration cookbook

**Status:** Accepted
**Implementation-Present:** 2026-05-12 by auto-scan — evidence: hex-core/src/composition.rs, hex-nexus/src/orchestration/secret_shadow_router.rs, hex-nexus/src/orchestration/shadow_decision.rs (+1 more)
**Date:** 2026-04-26
**Accepted:** 2026-04-26
**Drivers:** Substrate ADR-2026-04-26-1500 declared the substrate as a port-agnostic runtime; ADR-2026-04-26-1801 defined the strategy-level integration model for the first port (`IInferencePort`); the second port migration (`ISecretPort` via `SecretShadowRouter`) shipped today and exercised every assumption empirically. The pattern is now stable enough to document as a reusable cookbook so the third, fourth, and Nth port migrations don't re-litigate any of these decisions.
**Supersedes:** None — this is a how-to ADR. It does not change any prior decision; it codifies the implementation pattern that ADR-2026-04-26-1500 P10 anticipated.

## Context

Two substrate ports exist as of today:

| Port | Router | Wrapper | Tests |
|------|--------|---------|-------|
| `IInferencePort` | `hex-nexus/src/orchestration/shadow_router.rs::ShadowRouter` | `ShadowRouterInferenceAdapter` | 5 |
| `ISecretPort` | `hex-nexus/src/orchestration/secret_shadow_router.rs::SecretShadowRouter` | `ShadowRouterSecretAdapter` | 7 |

Both share:
- The substrate `RuntimeComposition` (`hex-core/src/composition.rs`) — port-agnostic by construction; multi-port isolation proven by 5 dedicated tests.
- The STDB `swap_ticket` + `shadow_sample` tables — `port_id` column discriminates; no schema work required for new ports.
- The 5 operational governance layers (L2 swarm + L3 judge + L4 shrinkage + L5 ADR conformance + L6 founding-goals ritual).
- The `shadow_decision` helper module (`shadow_decision.rs`) — RNG-vs-fraction, `ActiveTicket`, `AdapterRoutingTracker`.

Per-port code in both routers is roughly 50–80 lines of unique logic; the rest is reuse.

### What changes between ports

Five things, and only five:

1. **The trait and its dispatched method.** `IInferencePort::complete(req) -> response` vs `ISecretPort::resolve_secret(key) -> String`.
2. **The handle map's value type.** `Arc<dyn IInferencePort>` vs `Arc<dyn ISecretPort>`.
3. **The agreement model.** Inference: `agreed = (incumbent_ok && candidate_ok)` (semantic comparison deferred to L3 judge per `SuccessCriterion`). Secrets: `agreed = (incumbent_value == candidate_value)` strict.
4. **The metrics shape recorded into `shadow_sample`.** Inference records latency + tokens. Secrets record `{"ok": bool}` only — never the values themselves (privacy).
5. **The wrapper adapter's pass-through methods.** Inference passes `stream` / `health` / `capabilities` through; secrets pass `claim_secrets` / `grant_secret` / `revoke_secret` through.

Everything else is reuse.

---

## Decision

**Future port migrations follow this six-step recipe. The cookbook is normative — deviation requires either a new ADR justifying the deviation or a successor cookbook ADR.**

### Step 1 — Pick the dispatched method(s)

Most ports have several methods; usually one or two are shadow-meaningful (the read-call or the cost-bearing call). Pick those. The rest pass through to the fallback.

Heuristic: a method is shadow-meaningful when answering "would the candidate produce the same effect?" is operationally interesting. `IInferencePort.complete` (different model could produce different text). `ISecretPort.resolve_secret` (different vault could produce different value). `ICoordinationPort.swarm_status` (different backend could produce different listing). NOT: `grant_secret` (mutating, idempotent-by-design), `claim_secrets` (one-shot, must be exclusive).

### Step 2 — Define the agreement model

Two patterns observed:

- **Strict equality** (secrets, file contents, SQL query results): `agreed = (incumbent_ok && candidate_ok && incumbent_value == candidate_value)`. Rejects the candidate on any divergence — appropriate when correctness, not optimization, is the goal.
- **Both-Ok permissive** (inference, web search, recommendation): `agreed = (incumbent_ok && candidate_ok)`. Defers semantic comparison to L3 judge via `SuccessCriterion::ResponseEquivalence` — appropriate when the candidate is *expected* to produce different-but-equivalent output.

If unsure, default to strict — it's safer and the L3 criteria can relax it via tolerance later.

### Step 3 — Decide what to record into `shadow_sample`

The substrate's `shadow_sample.incumbent_metrics_json` / `candidate_metrics_json` columns are operator-readable via `/api/swaps/:id/samples` and the dashboard.

**Privacy hard rule**: never record values that are themselves secrets, PII, or otherwise sensitive. The secrets router records `{"ok": bool}` for exactly this reason. When in doubt, record metadata (latency, sizes, status codes) and leave the payload off.

For ports whose call result is naturally non-sensitive (e.g. inference responses are shown to the requester anyway), record richer telemetry — latency, token counts, error class — so the L3 judge has signal to evaluate `SuccessCriterion::LatencyP99BelowMs` etc.

### Step 4 — Build the per-port router

```rust
pub struct <PortName>ShadowRouter {
    comp: Arc<SpacetimeRuntimeComposition>,
    state: Arc<dyn ISwapTicketStatePort>,
    handles: RwLock<BTreeMap<AdapterId, Arc<dyn <PortName>>>>,
    active_tickets: RwLock<BTreeMap<PortId, ActiveTicket>>,
    call_seqs: Mutex<BTreeMap<String, u64>>,
    rng: SampleRng,
    tracker: AdapterRoutingTracker,
}
```

The `route` / `dispatch` method body is the same shape across ports:

1. Resolve `live_id` from `comp.binding_id(&port)`.
2. Resolve `live_handle` from the typed `handles` map.
3. Read `active_tickets[port]` → `ActiveTicket?`.
4. `shadow_decision(active.as_ref(), (self.rng)())` — gets a typed verdict.
5. On `LiveOnly`: `tracker.mark_routed(&live_id)`, dispatch to live, return.
6. On `Mirror { ticket_id, candidate_adapter_id }`:
   - Resolve candidate handle; on missing → log warn, fall through to live-only (caller-visible-behaviour invariant: missing candidate is never a caller-visible failure).
   - `tracker.mark_routed` for both adapter ids.
   - `tokio::join!` the two dispatched calls.
   - Compute `agreed` per Step 2; compute metrics JSON per Step 3.
   - `state.shadow_sample_record(...)`. Telemetry write failure is logged at warn but does not propagate (caller-visible-behaviour invariant).
   - Return the **incumbent's** result. The candidate's result is only telemetry input.

`register_handle` / `begin_shadow` / `end_shadow` follow the existing API shape unchanged.

### Step 5 — Build the wrapper adapter

```rust
pub struct ShadowRouter<PortName>Adapter {
    router: Arc<<PortName>ShadowRouter>,
    fallback: Arc<dyn <PortName>>,
    port: PortId,
}
```

Implements the port trait itself:
- Shadow-meaningful methods → delegate through `router`.
- Non-shadow methods → pass straight through to `fallback`.

This is the consumer-opt-in surface: hand a consumer this wrapper instead of a raw port handle and they're substrate-aware without any call-site changes.

### Step 6 — Wire into the orchestration stack

The L2–L6 governance layers, the `swap_ticket` STDB tables, and the `PromoteOrchestrator` tick are already port-agnostic. Wiring the new port involves only:

1. New AppState fields: `<port>_runtime_composition`, `<port>_shadow_router`. Mirror the inference-port pattern in `state.rs` and `lib.rs::build_app`.
2. The `PromoteOrchestrator`'s `promote_one` is already polymorphic over the registered handle type — but its `stage_handle` call is currently `IInferencePort`-specific because the handle Arc gets cast to `Arc<dyn Any>`. To support a second port at promote-time, **the promote orchestrator construction must pick the right router** based on the ticket's `port_id`. For day one of a new port, the operator can manually drive promotion via a CLI subcommand; full auto-promote across multiple ports is a follow-up workplan.
3. The L2 adversarial swarm's `KnownPortReviewer` allowlist — add the new port id, otherwise every proposal on the new port is rejected by L2.
4. The `propose_swap` REST endpoint hardcodes `PortId::new("inference")`. For multi-port operator-facing proposals, **add a `port_id` field to `ProposeSwapBody`** (defaults to "inference" for backward compat) and route to the right router based on that field. Day-one workaround: add a sibling `propose_secret_swap` endpoint per port; promote to a polymorphic endpoint when 3+ ports exist.

Steps 1–3 are mechanical. Step 4 is the architectural extension that should land in a dedicated workplan when the third port migration starts (avoid the YAGNI two-port tax).

---

## Consequences

**Positive:**
- Future port migrations are bounded work — ~200 lines of router code + ~50 lines of wrapper + tests. No new core infrastructure.
- The cookbook is the contract. New port migrations that deviate must explain why (forces real architectural conversation, not silent drift).
- L2–L6 governance applies to every new port automatically — adding `IFileSystemPort` doesn't require re-inventing shrinkage, ADR conformance, etc.

**Negative:**
- The "register the port at propose-swap-time" plumbing (Step 6.4) doesn't generalize today — handled by operator CLI for now. Real polymorphic operator surface lands when the third port arrives.
- The promote orchestrator's per-port handle staging is `IInferencePort`-specific; a second port's promote requires a sibling orchestrator until the polymorphism work lands.
- The `swap_ticket` schema's `port_id` column already discriminates, but no STDB query joins across ports for cross-port operator views (e.g. "show me all in-flight swaps everywhere"). Dashboard `/swaps` view filters per-port; cross-port view is a follow-up.

**Mitigations:**
- The "operator CLI per port until polymorphism lands" pattern is fine for the first 2–3 ports. The cost is one extra endpoint per port; the savings is not over-engineering generic dispatch before we know the shape.
- The promote-orchestrator-per-port limitation is documented; today only the inference port has an auto-promote tick wired in `sched_service.rs`. Adding a second port's auto-promote is ~30 lines of sibling code.
- Cross-port dashboard view: deferred to "third port arrives" — until then per-port is fine.

---

## Implementation

| Phase | Description | Status |
|-------|-------------|--------|
| P1 | Document the cookbook (this ADR) | **Done** |
| P2 | Backport `ShadowRouter` (inference) to use `shadow_decision` + `AdapterRoutingTracker` helpers | Pending — refactor risk; deferred until refactor pays off |
| P3 | Generalize `propose_swap` REST endpoint to accept `port_id` (Step 6.4) | Pending — gate is "third port arrives" |
| P4 | Generalize `PromoteOrchestrator` for multi-port promote (Step 6.2) | Pending — same gate |
| P5 | Cross-port dashboard view | Pending — same gate |

P1 is shipped this turn. P2–P5 are explicitly gated on the third-port migration arriving — the cookbook itself names the YAGNI threshold to avoid premature generalization.

---

## References

- ADR-2026-04-26-1500 — Substrate (defines `RuntimeComposition`)
- ADR-2026-04-26-1801 — Substrate ↔ inference integration (strategy-level pattern, first-port migration)
- `hex-nexus/src/orchestration/shadow_router.rs` — first port migration (`IInferencePort`)
- `hex-nexus/src/orchestration/secret_shadow_router.rs` — second port migration (`ISecretPort`)
- `hex-nexus/src/orchestration/shadow_decision.rs` — port-agnostic primitives the recipe assumes
- `hex-core/src/composition.rs` — port-agnostic `RuntimeComposition` (multi-port isolation tests at the bottom)
