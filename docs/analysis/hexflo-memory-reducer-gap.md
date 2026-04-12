# HexFlo Memory Reducer Gap Analysis

**Workplan**: wp-hex-standalone-dispatch
**Phase**: P5.1 (Reducer inventory + gap analysis)
**ADR**: ADR-2604112000 (Hex Self-Sufficient Dispatch)
**Author**: hex-coder agent (P5 dispatch)
**Date**: 2026-04-11

## Executive verdict

**GREEN** — all four `hexflo_memory_*` adapter operations have a usable
reducer or SQL path in `spacetime-modules/hexflo-coordination`. The WASM
module side of this phase is a no-op. P5.2 is **skipped**.

The real work left for P5.3/P5.4 is:

1. Fix a SQL-injection-shaped correctness bug in `hexflo_memory_retrieve`
   (unescaped single-quote concatenation).
2. Add hermetic unit tests that prove the adapter talks to the SpacetimeDB
   HTTP API with the expected URL / method / body shape. None exist today.
3. Note (not fix) a pre-existing port-surface bug in
   `HexFlo::memory_list(scope)` → `hexflo_memory_search(query)`: the port
   trait has no scope parameter, and the caller incorrectly passes the
   scope string as the search query. Fixing this requires a port-trait
   change (out of scope for P5, tracked as follow-up).

## 1. Reducer inventory

`spacetime-modules/hexflo-coordination/src/lib.rs`:

| Reducer | Line | Signature | Covers |
|---|---|---|---|
| `memory_store` | 1496 | `(ctx, key: String, value: String, scope: String, timestamp: String) -> Result<(), String>` | `hexflo_memory_store` — upsert |
| `memory_delete` | 1520 | `(ctx, key: String) -> Result<(), String>` | `hexflo_memory_delete` — returns `Err("Key 'X' not found")` if missing |
| `memory_clear_scope` | 1748 | `(ctx, scope: String) -> Result<(), String>` | bulk clear — unused by adapter |

`memory_retrieve` and `memory_search` have **no dedicated reducer**. This is
intentional and correct — SpacetimeDB exposes SQL over its HTTP
`/v1/database/<db>/sql` endpoint, so reads go through `query_table(...)`
against the `hexflo_memory` table directly.

## 2. Tables inventory

```rust
#[table(name = hexflo_memory, public)]
pub struct HexFloMemory {
    #[unique]
    pub key: String,       // primary lookup
    pub value: String,
    pub scope: String,     // "global" | "swarm:<id>" | "agent:<id>"
    pub updated_at: String, // RFC3339
}
```

- `key` is `#[unique]`, so lookups by key are O(log n) via `ctx.db.hexflo_memory().key().find(...)`.
- No secondary indexes; `scope` scans are O(n).
- SpacetimeDB does not support SQL `LIKE`, so `hexflo_memory_search` must
  fetch all rows and filter client-side (the real adapter impl already
  does this).

## 3. Gap summary

| Port method | Reducer / path available? | Currently wired? | Gap |
|---|---|---|---|
| `hexflo_memory_store` | `memory_store` reducer | Yes (`spacetime_state.rs:1070`) | **None** — arg order matches `[key, value, scope, timestamp]` |
| `hexflo_memory_retrieve` | SQL query over `hexflo_memory` | Yes (`spacetime_state.rs:1076`) | **Correctness**: uses `format!("... WHERE key = '{}'", key)` — breaks on keys containing `'` (SQL-injection-shaped). Harden before shipping. |
| `hexflo_memory_search` | SQL `SELECT key, value` + client-side substring match | Yes (`spacetime_state.rs:1082`) | **Pre-existing port-surface bug**: the port has no `scope` arg; `HexFlo::memory_list(scope)` passes scope string as query, which causes substring matching of the literal scope string against `key`/`value` rather than exact-matching on the `scope` column. Out of scope for P5 — requires trait change. Documented as follow-up below. |
| `hexflo_memory_delete` | `memory_delete` reducer | Yes (`spacetime_state.rs:1098`) | **Minor**: reducer returns `Err("Key 'X' not found")` for missing keys, but the port trait returns `Result<(), StateError>`. Today `HexFlo::memory_delete` guards with a `hexflo_memory_retrieve` existence check first, which masks this. The adapter's current wiring is correct for the happy path. |

## 4. Stub vs. real impl — the feature flag gate

`hex-nexus/src/adapters/spacetime_state.rs` has **two** `impl IHexFloMemoryStatePort for SpacetimeStateAdapter` blocks:

- **Line 1069** (real): `#[cfg(feature = "spacetimedb")]` — calls `self.call_reducer(...)` and `self.query_table(...)`. This is the build that ships by default (`[features] default = ["spacetimedb"]`).
- **Line 1710** (stub): `#[cfg(not(feature = "spacetimedb"))]` — every method returns `Err(Self::err())` ("SpacetimeDB not compiled"). This is what the ADR-2604112000 context section is describing; the audit says "`hexflo_memory_*` all return `Err(Self::err())`" which is only true for the no-feature build.

**Implication**: the ADR's framing is partially stale. The default build has been wired to the real reducers for a while (ADR-044 timeframe). What's missing is not the wiring but **test coverage** proving the wiring is correct. Until this phase, the only coverage was a single `#[ignore]` live-contract test that requires a running SpacetimeDB server.

This audit revises the characterization of the gap:

- Real impl exists and is plumbed end-to-end (`HexFlo::memory_*` → `IHexFloMemoryStatePort` → HTTP → SpacetimeDB `/v1/database/hex/{call,sql}/...`).
- Real impl has a SQL-escaping bug in `hexflo_memory_retrieve`.
- Real impl has **zero hermetic tests** — all error-path behavior and URL/body shapes are untested.
- Stub impl (no-feature build) is strictly a compile placeholder and an operator should never run it; `hex` ships with `default = ["spacetimedb"]`.

## 5. Bindings status

`hex-nexus/src/adapters/spacetime_bindings/` — **does not exist**. There is
no hand-maintained bindings layer. The adapter talks to SpacetimeDB over
its stable HTTP API directly (`POST /v1/database/<db>/call/<reducer>` and
`POST /v1/database/<db>/sql`). No codegen, no per-reducer Rust stubs.

This is why P5.2 is structurally a no-op regardless of reducer changes: the
adapter would not need bindings regeneration even if reducers had changed.
The only thing that could require a real rebuild is `spacetime publish`,
and that is deferred (no running SpacetimeDB in this dispatch).

## 6. Decision

**Path A-simplified (happy)**.

Reducers exist, the real adapter impl exists, no WASM work is needed, no
bindings regeneration is needed. P5.2 is a no-op. P5.3's scope is reduced
to a **correctness harden** (SQL escaping) plus adding tracing spans. P5.4
shifts from "live REST integration test" (which needs a running STDB) to
"hermetic httpmock-backed unit test of URL / body / method / response
parsing". `httpmock = "0.7"` is already in `hex-nexus`'s `dev-dependencies`
so no `Cargo.toml` work is needed.

### Justification

1. **The WASM republish rabbit hole is unnecessary.** The reducers we need
   are already published — `memory_store` and `memory_delete` are in the
   live module. Rebuilding would change nothing observable.
2. **The port trait has no scope parameter.** Fixing the `memory_search`
   scope-filter bug requires changing `IHexFloMemoryStatePort` which ripples
   into `HexFlo::memory_list`, the REST route handler, and the port's
   SQLite fallback path (which doesn't exist yet but would be implied).
   This is a cross-phase refactor and belongs in its own workplan.
3. **SQLite fallback (Path B) is not needed here.** The real impl is
   already correct on the SpacetimeDB path; we don't need to invent a
   fallback layer to prove the adapter works. ADR-011's fallback pattern
   would still be the right shape *if* we ever compile the no-feature
   build, but that's not what's blocking standalone dispatch today.

### Chosen path

**Path A-simplified**:

- P5.1: this doc (GREEN verdict).
- P5.2: **skipped** (no reducer changes needed, no bindings to regen).
- P5.3: harden SQL escaping in `hexflo_memory_retrieve`; add tracing::debug
  spans to all four methods; leave stub impl untouched (it's a no-feature
  compile placeholder and out of scope).
- P5.4: `hex-nexus/tests/hexflo_memory_adapter.rs` — httpmock-backed unit
  tests covering store (POST to `/v1/database/hex/call/memory_store` with
  `[key, value, scope, timestamp]` body), retrieve (POST SQL
  `SELECT value FROM hexflo_memory WHERE key = '...'`), search (SQL fetch-all
  + client-side filter), delete (POST to
  `/v1/database/hex/call/memory_delete` with `[key]` body), plus error
  mapping (HTTP 500 → `StateError::Storage`, connection refused →
  `StateError::Connection`), plus SQL escaping regression.

## 7. Follow-up tickets

These are surfaced for future workplans and **not** fixed in this dispatch:

1. **Port-trait scope arg.** `hexflo_memory_search(query)` should become
   `hexflo_memory_search(query, scope: Option<&str>)` so `HexFlo::memory_list`
   can stop abusing the query parameter. Pre-existing bug. Requires
   coordinated changes across port trait, all impls (including stub),
   `HexFlo::memory_*`, and the REST route handlers.
2. **SQLite fallback for `#[cfg(not(feature = "spacetimedb"))]` build.**
   Today the no-feature build returns `Err(Self::err())` for every method.
   ADR-011 implies this should fall back to `~/.hex/hub.db`. Worth doing
   if the no-feature build becomes a shipped configuration, but today it's
   a compile-time dead branch.
3. **WASM republish with `memory_clear_scope` exposure.** The reducer
   exists at lib.rs:1748 but isn't called anywhere. If future work needs
   bulk scope clear, wire it through `IHexFloMemoryStatePort` with a new
   method.

## 8. Verification evidence

Reducers confirmed by:

```
grep -n 'pub fn memory_' spacetime-modules/hexflo-coordination/src/lib.rs
1496:pub fn memory_store(
1520:pub fn memory_delete(
1748:pub fn memory_clear_scope(
```

Real adapter impl confirmed at `spacetime_state.rs:1069-1103`. Stub at
`spacetime_state.rs:1710-1715`. Feature flag `default = ["spacetimedb"]`
at `hex-nexus/Cargo.toml:[features]`. Bindings directory absence
confirmed: `ls hex-nexus/src/adapters/spacetime_bindings/` returns no
such file or directory.
