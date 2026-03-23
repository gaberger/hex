# ADR-2603231400: SpacetimeDB Operational Resilience

**Status:** Accepted
**Date:** 2026-03-23
**Drivers:** hex-nexus reported "SpacetimeDB: unavailable" despite SpacetimeDB running. Root cause investigation revealed 4 layered failures in startup, health checking, and schema hydration.

## Context

On 2026-03-23, `hex nexus start` consistently showed `SpacetimeDB: unavailable` even though SpacetimeDB was running. Investigation revealed four compounding issues:

1. **Stale ping endpoint**: 15 call sites hardcoded `/v1/ping` (a dead SpacetimeDB endpoint). The correct endpoint is `/database/ping`. One call site (`SpacetimeLauncher::health_check`) had the correct path, but all others diverged — classic string-literal drift.

2. **Port collision**: SpacetimeDB's default port (3000) conflicts with Next.js, Rails, and other common dev servers. A Next.js server on port 3000 returned HTTP 200 for `/database/ping`, causing a false positive — hex believed SpacetimeDB was healthy when it was actually talking to Next.js.

3. **Stale daemon binary**: After rebuilding hex-nexus, `hex nexus start` detected the daemon was "already running" and returned early — using the old binary with the broken ping path. There was no mechanism to detect that the running daemon's binary had been rebuilt.

4. **Schema hydration failure**: All 18 WASM modules published to a single SpacetimeDB database (`hex`). SpacetimeDB treats each publish as a full schema replacement — publishing module B after module A would DROP module A's tables, blocked by migration safety. Only `hexflo-coordination` (the mega-module with 15 tables) could publish successfully.

## Decision

### D1: Canonical ping constant (prevents string drift)

All SpacetimeDB health-check URLs **must** use `hex_core::SPACETIMEDB_PING_PATH` — never hardcode the path string. ADR compliance rules `adr-039-no-stale-ping` (error) and `adr-039-use-ping-constant` (warning) enforce this via `hex analyze`.

### D2: Port 3033 default (prevents port collision)

SpacetimeDB default port moves from 3000 to 3033 (`hex_core::SPACETIMEDB_DEFAULT_HOST`). Port 3033 avoids conflicts with Next.js (3000), Rails (3000), and other common dev servers. Overridable via `HEX_SPACETIMEDB_HOST` env var.

### D3: Content-type validation (prevents false positive health checks)

The `is_spacetimedb_reachable()` function checks that the ping response does NOT have `content-type: text/html`. SpacetimeDB returns `text/plain` or no content-type — HTML indicates a non-SpacetimeDB server (e.g., Next.js catch-all route).

### D4: Stale binary auto-restart (prevents stale daemon)

`hex nexus start` compares the running daemon's build hash (via `GET /api/version → buildHash`) against the on-disk binary's hash (via `hex-nexus --build-hash`). If they differ, the daemon is automatically stopped and restarted. `hex nexus status` shows a `STALE` warning when hashes diverge.

### D5: Single-module hydration (prevents schema conflicts)

Only `hexflo-coordination` is published to the `hex` database. It contains all 15 core tables and 40+ reducers needed by hex-nexus. The other 17 modules are documented but dormant in `MODULE_TIERS` until they are migrated to their own databases.

## Consequences

**Positive:**
- SpacetimeDB connectivity is reliable across port configurations
- String drift in endpoint paths is prevented at compile time (constant) and lint time (ADR rule)
- Developers never run a stale daemon after rebuilding
- Hydration succeeds on first attempt with `hex stdb hydrate`

**Negative:**
- 17 WASM modules are dormant — their tables and reducers are unavailable
- Port 3033 is non-standard — existing `.hex/state.json` files may need updating
- Build-hash comparison adds ~200ms to `hex nexus start` when daemon is running

**Mitigations:**
- Dormant modules can be revived by giving each its own database name (future work)
- Port migration is handled by updating defaults in `hex-core`; env var override available
- Build-hash check only runs when the daemon is already running (not on cold start)

## Implementation

| Phase | Description | Status |
|-------|------------|--------|
| P1 | Consolidate 15 hardcoded ping paths into `SPACETIMEDB_PING_PATH` constant | Done |
| P2 | Move default port to 3033, add content-type validation | Done |
| P3 | Add build-hash stale binary detection to `hex nexus start/status` | Done |
| P4 | Reduce `MODULE_TIERS` to single hexflo-coordination module | Done |
| P5 | Add `adr-039-no-stale-ping` and `adr-039-use-ping-constant` lint rules | Done |
| P6 | Add 3 missing modules to spacetime-modules workspace | Done |
| P7 | Migrate dormant modules to per-module databases | Pending |

## References

- ADR-039: SpacetimeDB is source of truth (state management)
- ADR-025: SQLite fallback when SpacetimeDB unavailable
- Commits: `e4990ee`, `1f4bfb3`
- `.hex/adr-rules.toml`: rules `adr-039-no-stale-ping`, `adr-039-use-ping-constant`
