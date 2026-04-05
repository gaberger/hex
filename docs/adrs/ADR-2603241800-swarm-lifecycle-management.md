# ADR-2603241800: Swarm Lifecycle Management (Complete / Fail / Cleanup)

**Status:** Accepted
**Date:** 2026-03-24
**Drivers:** 21 swarms stuck in "active" status with no way to archive them via CLI or MCP. SpacetimeDB has `swarm_complete` and `swarm_fail` reducers but they are unreachable from user-facing surfaces.

## Context

- SpacetimeDB's `hexflo-coordination` module defines `swarm_complete(id, timestamp)` and `swarm_fail(id, reason, timestamp)` reducers that transition swarm status from `active` → `completed` or `failed`.
- hex-nexus exposes `PATCH /api/swarms/:id` which calls `swarm_complete`, but there is **no REST route for `swarm_fail`**.
- hex-cli has **no `swarm complete` or `swarm fail` subcommand**.
- MCP tools have **no complete/fail tool**.
- Result: swarms accumulate forever as "active", polluting dashboards and task lists. The only cleanup path is direct SpacetimeDB SQL, which violates the single-authority principle (ADR-046).

### Alternatives Considered

1. **Add an "archived" status** — Rejected. The existing `completed`/`failed` statuses are semantically correct and already implemented in WASM. Adding a third status increases complexity with no benefit.
2. **Delete swarms** — Rejected. Destroys reporting history. SpacetimeDB has no soft-delete mechanism in this module.
3. **Expose existing reducers through all surfaces** — Chosen. Minimal new code, leverages existing WASM logic.

## Decision

We will expose swarm lifecycle transitions (`complete` and `fail`) through all three user-facing surfaces: REST API, CLI, and MCP.

### Design

1. **REST** — Add `PATCH /api/swarms/:id/fail` route in `hex-nexus/src/routes/swarms.rs` that calls `port.swarm_fail(id, reason)`.
2. **CLI** — Add `hex swarm complete <id>` and `hex swarm fail <id> [reason]` subcommands in `hex-cli/src/commands/`.
3. **MCP** — Add `hex_hexflo_swarm_complete` and `hex_hexflo_swarm_fail` tools that delegate to the REST endpoints.
4. **Batch cleanup** — Add `hex swarm cleanup` command that:
   - Marks all-tasks-completed swarms as `completed`
   - Marks all-tasks-pending swarms older than 24h as `failed` (with reason "stale — no tasks started")
   - Reports what was changed (dry-run by default, `--apply` to execute)

### Constraints

- Swarm status transitions are one-way: `active` → `completed` | `failed`. No re-activation.
- The `swarm_complete` and `swarm_fail` reducers are the single authority (SpacetimeDB, per ADR-046).
- CLI and MCP always go through hex-nexus REST, never call reducers directly.

## Consequences

**Positive:**
- Swarm lists stay clean — only truly active swarms show
- Dashboard and reporting retain full history (completed/failed swarms are queryable)
- `swarm cleanup` provides automated hygiene for CI and long-running sessions

**Negative:**
- One-way transitions mean accidentally completing a swarm requires re-creating it
- Batch cleanup heuristic (24h stale) may not fit all workflows

**Mitigations:**
- `cleanup` defaults to dry-run mode — must pass `--apply` to execute
- Stale threshold is configurable via `--stale-hours` flag (default 24)

## Implementation

| Phase | Description | Status |
|-------|------------|--------|
| P1 | Add `PATCH /api/swarms/:id/fail` REST route | Pending |
| P2 | Add `hex swarm complete` and `hex swarm fail` CLI commands | Pending |
| P3 | Add MCP tools `hex_hexflo_swarm_complete` and `hex_hexflo_swarm_fail` | Pending |
| P4 | Add `hex swarm cleanup` with dry-run/apply and stale-hours | Pending |
| P5 | Run cleanup on current 21 swarms | Pending |

## References

- ADR-046: SpacetimeDB single authority for state
- ADR-027: HexFlo native coordination
- `spacetime-modules/hexflo-coordination/src/lib.rs` lines 778–810 (swarm_complete, swarm_fail reducers)
- `hex-nexus/src/routes/swarms.rs` line 161 (existing complete_swarm route)
- `hex-nexus/src/ports/state.rs` line 580 (swarm_complete/swarm_fail port traits)
