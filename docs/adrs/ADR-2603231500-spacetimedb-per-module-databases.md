# ADR-2603231500: SpacetimeDB Per-Module Databases

**Status:** Proposed
**Date:** 2026-03-23
**Drivers:** 17 of 18 WASM modules are dormant because SpacetimeDB's single-database publish model replaces the entire schema on each publish. Chat, inference, secrets, and workplan features are unavailable.
**Supersedes:** Implicit single-database design documented in ADR-2603231400 D5.

## Context

hex has 18 SpacetimeDB WASM modules in `spacetime-modules/`. The original design published all modules to a single database (`hex`). SpacetimeDB treats each `spacetime publish` as a full schema replacement — publishing module B drops module A's tables.

ADR-2603231400 mitigated this by publishing only `hexflo-coordination` (the mega-module with 15 core tables). This leaves 17 modules dormant:

- **chat-relay** (5 tables) — message routing, session history
- **inference-gateway** (5 tables) — LLM request routing
- **inference-bridge** (4 tables) — model integration
- **secret-grant** (4 tables) — secret distribution
- **workplan-state** (2 tables) — task status, phase tracking
- **agent-registry** (2 tables) — agent lifecycle, heartbeats
- **fleet-state** (1 table) — compute node registry
- **file-lock-manager** (1 table) — distributed file locking
- **architecture-enforcer** (2 tables) — server-side boundary validation
- **skill-registry** (2 tables) — skill definitions
- **hook-registry** (2 tables) — hook definitions
- **agent-definition-registry** (2 tables) — agent role definitions
- **rl-engine** (3 tables) — reinforcement learning
- **hexflo-lifecycle** (3 tables) — swarm lifecycle events
- **hexflo-cleanup** (4 tables) — stale agent cleanup
- **conflict-resolver** (1 table) — merge conflict resolution
- **test-results** (2 tables) — test session tracking

The nexus adapters (`spacetime_inference.rs`, `spacetime_chat.rs`, `spacetime_secrets.rs`, `spacetime_session.rs`) already connect to separate database names via env vars (`HEX_INFERENCE_STDB_DATABASE`, `HEX_CHAT_STDB_DATABASE`, etc.), but hydration never published those modules.

## Decision

### D1: Each module publishes to its own database

Every WASM module gets a database name matching its directory name:

| Module | Database Name |
|--------|--------------|
| hexflo-coordination | `hex` (unchanged, backward-compatible) |
| chat-relay | `chat-relay` |
| inference-gateway | `inference-gateway` |
| secret-grant | `secret-grant` |
| workplan-state | `workplan-state` |
| agent-registry | `agent-registry` |
| ... | ... (directory name = database name) |

### D2: Tiered hydration restored with per-module database names

`publish_modules_ordered()` passes each module's directory name as the database argument instead of the global `hex` database. The tiered dependency order is restored for documentation and future cross-module references.

### D3: Nexus adapters use database name constants

Each adapter's default database name is defined as a constant in `hex-core` alongside `SPACETIMEDB_PING_PATH`. Env var overrides remain for flexibility.

### D4: Binary priority order fix

`find_nexus_binary()` checks local build artifacts (`target/debug`, `target/release`) BEFORE PATH lookup, so stale-binary detection works during development.

### D5: Hydration status fix

`HydrationResult::status()` returns "hydrated" when all active tier modules publish successfully, regardless of dormant tiers.

### D6: Un-gitignore docs/adrs

Remove the `docs` gitignore pattern so ADRs can be added without `-f`.

## Consequences

**Positive:**
- All 18 modules can be published and used
- Chat, inference, secrets, workplan features restored
- Each module's schema is independent — publishing one never affects another
- Stale binary detection works during development

**Negative:**
- 18 separate databases instead of 1 — more connections, more state to manage
- SpacetimeDB must manage more databases (resource overhead)
- Migration from single-database to multi-database may lose existing data in `hex`

**Mitigations:**
- Connection pooling in nexus adapters
- `hex stdb hydrate` handles all databases in one command
- Existing `hex` database data preserved (hexflo-coordination still publishes there)

## Implementation

| Phase | Description | Status |
|-------|------------|--------|
| P1 | Fix `find_nexus_binary()` priority: local builds before PATH | Pending |
| P2 | Fix `HydrationResult::status()` to match active tiers only | Pending |
| P3 | Un-gitignore `docs/adrs/` | Pending |
| P4 | Add database name constants to `hex-core` | Pending |
| P5 | Update `publish_module()` to accept per-module database names | Pending |
| P6 | Restore `MODULE_TIERS` with all 18 modules, map each to its database | Pending |
| P7 | Update nexus adapters to use constants from `hex-core` | Pending |
| P8 | Full hydration test — all 18 modules publish successfully | Pending |
| P9 | Kill stuck `spacetimedb-update` processes in `ensure_spacetimedb()` | Pending |
| P10 | Install `wasm-opt` or suppress warning in hydration output | Pending |

## References

- ADR-2603231400: SpacetimeDB Operational Resilience (predecessor)
- ADR-039: SpacetimeDB is source of truth
- ADR-025: SQLite fallback
- `spacetime-modules/Cargo.toml`: workspace with 18 members
