# ADR-2026-05-09-1100: Multi-host substrate composition

**Status:** Accepted

> Status flipped 2026-05-23 (operator). Workplan derivation will land via `workplan_auto_emitter` on next tick.
**Date:** 2026-05-09
**Drivers:** ADR-2026-04-26-1500 (self-modifying substrate) defines G2 — "Multi-host scaleout. The substrate must be able to run as a fleet, with placement as a property of the substrate, not a property of the user." The C2 contract (`RuntimeComposition`) currently lives in per-process nexus memory. A fleet of hex hosts cannot coordinate adapter swaps, telemetry-based promotion, or rollback decisions without shared composition state. This ADR realizes G2 by moving composition into SpacetimeDB and defining the placement algorithm.

## Context

### The single-host bottleneck

ADR-2026-04-26-1500 C2 defines `RuntimeComposition` as the hot-swappable composition root API:

```rust
pub trait RuntimeComposition: Send + Sync {
    fn ports(&self) -> &PortRegistry;
    fn snapshot(&self) -> CompositionSnapshot;
    fn propose_swap(&self, swap: CompositionSwap) -> Result<SwapTicket, SwapError>;
    fn promote(&self, ticket: SwapTicket) -> Result<(), SwapError>;
    fn rollback(&self, ticket: SwapTicket) -> Result<(), SwapError>;
}
```

The initial implementation (`hex-nexus/src/composition.rs`) stores `PortRegistry` in a `RwLock<HashMap<PortId, AdapterId>>` inside the nexus process. This works for a single host but breaks G2:

1. **Swap coordination.** Host A proposes a swap; host B continues routing to the old adapter until it restarts or polls for changes. No atomic fleet-wide promotion.
2. **Telemetry aggregation.** C3 `PortTelemetry` samples are per-host. The C5 shadow-promotion judge needs fleet-wide p50/p99/error-rate to decide `ShadowGreen`. Per-host samples cannot answer "is the candidate better than the incumbent across the entire fleet?"
3. **Placement blind spot.** A new `IModelProvider` adapter lands. Which hosts should bind it? The substrate has no placement algorithm; the user must manually configure each host or accept "all hosts run all adapters."

### Why this is G2, not an optimization

G2 states placement is a *property of the substrate*. If the user must configure which host runs which adapter, placement is a property of the user. The substrate has failed its contract. Multi-host composition is therefore architectural, not operational.

### Forces

- **C5 shadow promotion must remain deterministic.** The promotion judge reads telemetry and decides `ShadowGreen` or `ShadowRed`. If half the fleet has promoted and half has not, telemetry is a mix of two compositions and the judge cannot attribute a regression to the candidate or the incumbent.
- **Backward compatibility: single-host mode must not regress.** A developer running `hex nexus` locally must not pay fleet-coordination costs. Multi-host is an opt-in capability, not a mandatory dependency.
- **SpacetimeDB is the only shared state primitive hex trusts for coordination.** The substrate already uses `hexflo-coordination` for swarm heartbeats and task reclaim. Introducing a second coordination backend (etcd, Consul, etc.) violates the "one coordination story" principle.

## Decision

**Move `RuntimeComposition` from per-process memory to a SpacetimeDB table. Define `CompositionSwap` as a reducer emitting subscription events. Extend `PortTelemetry` rows with `host_id` tags. Implement a tier-aware placement algorithm that assigns adapters to hosts based on declared capabilities.**

### D1 — `runtime_composition` table

New table in `spacetime-modules/hexflo-coordination/src/lib.rs`:

```rust
#[spacetimedb(table, public)]
pub struct RuntimeComposition {
    #[primarykey]
    pub port_id: String,
    pub adapter_id: String,
    pub swap_ticket_id: Option<u64>,
    pub state: CompositionState,  // Live | ShadowCandidate | ShadowGreen | ShadowRed
    pub shadow_adapter_id: Option<String>,
    pub promoted_at_ms: u64,
    pub promoted_by_host: String,
}

#[spacetimedb(table, public)]
pub struct PortTelemetrySample {
    #[primarykey]
    #[autoinc]
    pub id: u64,
    pub port_id: String,
    pub adapter_id: String,
    pub host_id: String,  // NEW: per-host tag
    pub timestamp_ms: u64,
    pub p50_latency_ms: f64,
    pub p99_latency_ms: f64,
    pub error_count: u64,
    pub request_count: u64,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub cost_usd: f64,
}
```

`RuntimeComposition` is the single source of truth for which adapter is bound to which port. One row per port. `state=Live` means the binding is active fleet-wide. `state=ShadowCandidate` means `shadow_adapter_id` is receiving mirrored traffic on hosts that have registered it.

### D2 — `composition_swap_propose` reducer

```rust
#[spacetimedb(reducer)]
pub fn composition_swap_propose(
    ctx: &ReducerContext,
    port_id: String,
    candidate_adapter_id: String,
    shadow_test_plan_json: String,
) -> Result<u64, String> {
    // 1. Validate port_id exists in PortRegistry (not shown; assume a ports table)
    // 2. Check no in-flight swap for this port (state != ShadowCandidate)
    let existing = ctx.db.runtime_composition()
        .port_id().filter(|p| p == &port_id)
        .next();
    
    if let Some(row) = existing {
        if row.state == CompositionState::ShadowCandidate {
            return Err(format!("Port {} already has swap in flight", port_id));
        }
    }

    let ticket_id = ctx.db.swap_ticket().count() + 1;
    
    // 3. Insert SwapTicket row (not shown)
    // 4. Update RuntimeComposition to ShadowCandidate
    ctx.db.runtime_composition().port_id().update(RuntimeComposition {
        port_id: port_id.clone(),
        state: CompositionState::ShadowCandidate,
        shadow_adapter_id: Some(candidate_adapter_id.clone()),
        swap_ticket_id: Some(ticket_id),
        ..existing.unwrap()
    });

    // 5. Emit event so subscribed hosts learn of the candidate
    Ok(ticket_id)
}
```

Hosts subscribe to `runtime_composition` table updates. When a row transitions to `ShadowCandidate`, subscribing hosts that have the candidate adapter registered begin mirroring traffic (C5 step 2).

### D3 — Per-host telemetry aggregation

The C5 promotion judge (Layer 3 agent, `hex-cli/assets/agents/promotion-judge/`) queries `PortTelemetrySample` grouped by `(port_id, adapter_id, timestamp_window)` across all `host_id`s. Fleet-wide p50 is computed as the median of per-host p50s weighted by `request_count`. Fleet-wide error rate is `SUM(error_count) / SUM(request_count)`.

Promotion rule: candidate is `ShadowGreen` if, over a 10-minute window with >100 requests fleet-wide:
- p50 latency ≤ incumbent p50 × 1.1
- p99 latency ≤ incumbent p99 × 1.2
- error rate ≤ incumbent error rate × 1.05
- cost per request ≤ incumbent cost × 1.0 (no cost regression)

Otherwise `ShadowRed`.

### D4 — `composition_swap_promote` reducer

```rust
#[spacetimedb(reducer)]
pub fn composition_swap_promote(
    ctx: &ReducerContext,
    ticket_id: u64,
) -> Result<(), String> {
    // 1. Validate ticket exists, state=ShadowGreen
    // 2. Atomic swap: set adapter_id = shadow_adapter_id, state = Live, clear shadow fields
    let row = ctx.db.runtime_composition()
        .swap_ticket_id().filter(|t| t == &Some(ticket_id))
        .next()
        .ok_or("Ticket not found")?;

    if row.state != CompositionState::ShadowGreen {
        return Err(format!("Ticket {} not ShadowGreen", ticket_id));
    }

    ctx.db.runtime_composition().port_id().update(RuntimeComposition {
        adapter_id: row.shadow_adapter_id.clone().unwrap(),
        state: CompositionState::Live,
        shadow_adapter_id: None,
        swap_ticket_id: None,
        promoted_at_ms: ctx.timestamp.elapsed().as_millis() as u64,
        promoted_by_host: ctx.sender.to_string(),
        ..row
    });

    // Subscribed hosts receive the update and atomically swap their in-memory PortRegistry
    Ok(())
}
```

Promotion is a single transactional write. All hosts observe the new binding within one subscription round-trip (~100ms).

### D5 — Placement algorithm (tier-aware)

New table:

```rust
#[spacetimedb(table, public)]
pub struct HostCapability {
    #[primarykey]
    pub host_id: String,
    #[primarykey]
    pub capability: String,  // e.g. "gpu", "tier_T3", "region_us_west"
}

#[spacetimedb(table, public)]
pub struct AdapterPlacementRule {
    #[primarykey]
    pub adapter_id: String,
    pub required_capabilities: Vec<String>,  // must have ALL
    pub preferred_capabilities: Vec<String>, // score +1 per match
}
```

Placement algorithm runs when:
1. A new adapter is registered (`adapter_register` reducer).
2. A new host joins the fleet (`host_capability_register` reducer).

Algorithm:
```
for each adapter A:
  candidates = hosts where host.capabilities ⊇ A.required_capabilities
  if candidates.empty():
    log warning "No host can run adapter A"
    continue
  scored = candidates.map(h => (h, count(h.capabilities ∩ A.preferred_capabilities)))
  selected = top 2 scored hosts (redundancy)
  emit host_adapter_assignment(host_id, adapter_id) rows
```

Hosts subscribe to `host_adapter_assignment`. When a new row appears with `host_id = self`, the host loads the adapter crate (via dynamic linking or a registry lookup, implementation deferred) and registers it with its local `PortRegistry`.

### D6 — Rollback

`RuntimeComposition` retains the previous `adapter_id` in a new `rollback_adapter_id` field for 1 hour post-promotion. `composition_swap_rollback(ticket_id)` is the inverse of promote: swap `adapter_id ↔ rollback_adapter_id`, emit update, hosts revert.

Automatic rollback: a Layer 4 "regression detector" agent polls `PortTelemetrySample` every 60s. If a port's error rate crosses 2× the pre-promotion baseline within 10 minutes of promotion, it calls `composition_swap_rollback`.

## Consequences

**Positive:**

- **G2 realized.** Placement is now a substrate property. The user declares host capabilities (`hex host capability-add gpu`) and adapter requirements (`hex adapter require gpu`). The substrate computes the assignment.
- **C5 shadow promotion works fleet-wide.** The promotion judge reads aggregated telemetry across all hosts. A candidate that looks good on host A but bad on host B is rejected.
- **Single-host mode unchanged.** A lone `hex nexus` process writes to a local SpacetimeDB instance (or an in-memory stub). No fleet-coordination cost.
- **Adapters become portable.** An adapter compiled on host A can be dynamically loaded on host B if both hosts share the same Rust toolchain version. (Full solution deferred to a future ADR on adapter packaging.)

**Negative:**

- **SpacetimeDB is now a hard runtime dependency for multi-host mode.** If SpacetimeDB is unavailable, the fleet cannot coordinate swaps. Mitigation: C6/L4 shrinkage daemon keeps the `runtime_composition` table small (one row per port, ~10-50 rows total), so STDB is unlikely to be a bottleneck.
- **Telemetry table grows unbounded.** `PortTelemetrySample` will accumulate millions of rows over weeks. A TTL reducer (delete rows older than 7 days) is required but not part of this ADR.
- **Dynamic adapter loading is assumed but not implemented.** This ADR assumes `host_adapter_assignment` triggers the host to load the adapter. The actual loading mechanism (dynamic lib, WASM, or pre-compiled registry) is deferred. Without it, placement is advisory only.

**Risks:**

- **Clock skew between hosts.** `timestamp_ms` fields assume hosts have synchronized clocks. A 10-second skew could cause the promotion judge to compare telemetry from different time windows. Mitigation: require NTP in the deployment guide; flag in the judge if timestamp spread exceeds 5 seconds.
- **Transactional semantics of STDB subscription updates.** If a host misses a subscription event (network partition, host restart), it may continue routing to a rolled-back adapter. Mitigation: hosts poll `runtime_composition` every 10 seconds as a backup to subscriptions.

**Compliance:**

- **Founding goal:** G2 (multi-host scaleout).
- **Substrate contracts:** C2 (composition root API extended to STDB), C3 (telemetry per-host tagging), C5 (shadow promotion fleet-aware).
- **Layering:** Infrastructure layer (SpacetimeDB module). Does not touch domain, ports, or usecases.

**Next steps:**

1. Implement `runtime_composition` and `PortTelemetrySample` tables in `spacetime-modules/hexflo-coordination/src/lib.rs`.
2. Add `composition_swap_propose`, `composition_swap_promote`, `composition_swap_rollback` reducers.
3. Extend `hex-nexus/src/composition.rs` to subscribe to `runtime_composition` and update in-memory `PortRegistry` on events.
4. Implement placement algorithm as a `hex-cli/src/commands/placement.rs` subcommand or a background agent.
5. Write promotion-judge agent that queries `PortTelemetrySample` and calls `composition_swap_promote` when `ShadowGreen` criteria are met.
6. Update `hex host` and `hex adapter` CLI commands to support capability registration.
