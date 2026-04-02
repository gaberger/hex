# ADR-2604020900: Eliminate SQLite Fallback — SpacetimeDB as Single State Authority

**Status:** Accepted
**Date:** 2026-04-02
**Drivers:** Dual-store divergence: projects registered via CLI/MCP land in SQLite but never reach SpacetimeDB, making them invisible to the portal. This split creates silent inconsistency and user confusion.
**Supersedes:** ADR-025 (SQLite fallback for offline operation)

## Context

hex-nexus currently maintains **two parallel state stores**:

1. **SpacetimeDB** — the intended single source of truth, subscribed to by the dashboard
2. **SQLite** (`~/.hex/hub.db`) — a fallback introduced in ADR-025 for "offline/single-node operation"

The fallback has created concrete harm:
- `hex_project_list` (REST → SQLite) reports a project as registered
- The portal (SpacetimeDB subscription) shows "No projects registered"
- The user must re-register via the portal to sync both stores
- Any write that bypasses the SpacetimeDB reducer silently diverges

The `sqlite-session` feature is in the `default` feature set, meaning SQLite is always compiled in and always used for session state, creating a persistent second store that grows stale.

SpacetimeDB is already required for all operations per CLAUDE.md: "SpacetimeDB must always be running to use hex." The offline scenario that motivated ADR-025 is not a supported configuration.

## Decision

**We will remove SQLite as a state store from hex-nexus entirely.**

- Delete `hex-nexus/src/adapters/sqlite_session.rs`
- Remove the `sqlite-session` feature flag and `rusqlite` dependency from `hex-nexus/Cargo.toml`
- Remove SQLite initialization from `hex-nexus/src/lib.rs` and `state.rs`
- Remove SQLite references from `adapters/mod.rs`, `cleanup.rs`
- Any SQLite usage in `adapters/events.rs` / `routes/events.rs` / `ports/events.rs` must be replaced with SpacetimeDB-backed storage or in-memory state
- If SpacetimeDB is unavailable, hex-nexus MUST return a clear error — never silently fall back to a stale local store
- The `tool_events` table (ADR-2604012137) uses its own SQLite file for the event log; this is separate from the state fallback and is evaluated independently

## Consequences

**Positive:**
- Single authoritative state store — portal and CLI always agree
- No more silent divergence between REST responses and WebSocket subscriptions
- Simpler codebase — removes ~1 adapter, feature flag, and dependency
- Eliminates `hub.db` stale state accumulation

**Negative:**
- hex-nexus requires SpacetimeDB to be running (already required per CLAUDE.md; this just enforces it)

**Mitigations:**
- Startup health check already exists (`hex nexus status`); will surface SpacetimeDB unavailability clearly
- `hex nexus start` handles SpacetimeDB startup

## Implementation

| Phase | Description | Status |
|-------|-------------|--------|
| P1 | Remove `sqlite_session.rs`, feature flag, `rusqlite` dep | Done |
| P2 | Remove SQLite init from `lib.rs`, `state.rs`, `adapters/mod.rs`, `cleanup.rs` | Done |
| P3 | Replace `SqliteEventAdapter` with `InMemoryEventAdapter` (1000-event ring buffer) | Done |
| P4 | `cargo build -p hex-nexus` — confirm clean compile | Done ✓ |
| P5 | Smoke test: register project via CLI → appears in portal immediately | Pending |

## References

- ADR-025: SQLite fallback (superseded by this ADR)
- ADR-2604012137: tool-call observability WebSocket (events SQLite is separate scope)
- CLAUDE.md: "SpacetimeDB must always be running to use hex"
