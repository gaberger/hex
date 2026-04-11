# ADR-2604101900: Steering/Interrupt API — Full Implementation

**Status:** Proposed
**Date:** 2026-04-10
**Drivers:** Gap analysis ADR-2604100000 identified steering/interrupt API but implementation is stubbed

## Context

ADR-2604100000 (Managed Agents Gap Analysis) identified the need for:
- P4a: `POST /sessions/{id}/events` — steering events (restart, pause, resume)
- P4c: `POST /sessions/{id}/interrupt` — interrupt with new instructions

However, during parity verification, we discovered both endpoints are **stubbed**:
- `session_events` returns success but does nothing
- `session_interrupt` returns "interrupted" but doesn't actually stop/interrupt anything

This is a systemic issue: we implement the API surface but don't wire up the actual functionality.

## Impact Analysis

### Consumer Dependency Map

| API Endpoint | Consumers | How Used |
|--------------|-----------|---------|
| `POST /api/sessions/{id}/events` | CLI: `hex agent event` | Send steering events to running agent |
| `POST /api/sessions/{id}/interrupt` | CLI: `hex agent interrupt` | Interrupt agent with new instructions |
| `hex agent worker --role` | Background agents | Persistent polling workers |
| SpacetimeDB session table | Workplan executor | Track session state |

### Cross-Crate Analysis

| Crate | Files Affected | Dependency |
|-------|---------------|------------|
| `hex-nexus` | `routes/orchestration.rs` | Main API handlers |
| `hex-cli` | `commands/agent.rs` | CLI wrappers |
| `hex-agent` | Polling worker | Reads interrupt state |

### Blast Radius

| Component | Consumers | Impact | Mitigation |
|-----------|-----------|--------|------------|
| Session state in SpacetimeDB | Workplan executor | HIGH | Add session table |
| Interrupt signal to worker | Running agents | CRITICAL | WebSocket pub/sub |
| Steering events queue | All agents | HIGH | Event queue table |

### Build Verification Gates

```bash
# After session table implementation
cargo check --package hex-nexus
cargo check --package hex-cli

# After worker integration
cargo check --workspace
hex agent list | grep -i worker
```

## Decision

Implement full steering/interrupt API with SpacetimeDB-backed session state:

1. **Session Table** in SpacetimeDB to track active sessions and their state
2. **Event Queue** for steering events (pause, resume, restart)
3. **Interrupt Signal** with instruction injection
4. **Worker Polling** to read session state on each poll cycle

## Implementation

| Phase | Description | Validation Gate | Status |
|-------|-------------|-----------------|--------|
| 1 | Add instruction store to AppState | `cargo check -p hex-nexus` | ✅ Complete |
| 2 | Wire `session_events` to persist events | `cargo check -p hex-nexus` | ✅ Complete |
| 3 | Wire `session_interrupt` to set instruction | `cargo check -p hex-nexus` | ✅ Complete |
| 4 | Add `GET /api/sessions/:id/instructions` | `cargo check -p hex-nexus` | ✅ Complete |
| 5 | Integrate test | `hex analyze .` | Pending |

## Consequences

**Positive:**
- Full feature parity with Anthropic Managed Agents
- Actually works when user sends interrupt

**Negative:**
- Requires SpacetimeDB session table
- Workers must poll session state (adds latency)

**Mitigations:**
- WebSocket subscription for instant interrupt notification
- Local SQLite fallback if SpacetimeDB unavailable

## References

- ADR-2604100000: Managed Agents Gap Analysis
- ADR-2604101600: SpacetimeDB Workplan Coordination
- Anthropic Managed Agents: Steering Events, Interrupt API