# Architecture Health Report — hex-f1

**Score: 97/100**
**Date: 2026-03-16**

## Summary

The hex-f1 Go backend exhibits excellent hexagonal architecture compliance. All import boundaries are respected across every layer, all adapter-to-port bindings have compile-time interface assertions, and there are no circular dependencies. Three minor dead exports were detected.

## Hex Boundary Validation

| File | Layer | Project Imports | Status |
|------|-------|-----------------|--------|
| `src/core/domain/index.go` | Domain | _(none — stdlib only)_ | OK |
| `src/core/ports/index.go` | Ports | `hex-f1/src/core/domain` | OK |
| `src/core/usecases/f1_service.go` | Usecases | `hex-f1/src/core/domain`, `hex-f1/src/core/ports` | OK |
| `src/adapters/primary/http_adapter.go` | Primary Adapter | `hex-f1/src/core/domain`, `hex-f1/src/core/ports` | OK |
| `src/adapters/secondary/jolpica_adapter.go` | Secondary Adapter | `hex-f1/src/core/domain`, `hex-f1/src/core/ports` | OK |
| `src/adapters/secondary/cache_adapter.go` | Secondary Adapter | `hex-f1/src/core/ports` | OK |
| `src/composition-root.go` | Composition Root | `hex-f1/src/adapters/primary`, `hex-f1/src/adapters/secondary`, `hex-f1/src/core/usecases` | OK |

**Result: 0 boundary violations.**

**Note on domain imports in adapters:** The primary and secondary adapters import `domain` for value-object types (`Season`, `Position`, etc.) used in type conversions. This is standard Go hex practice — the adapters contain no business logic, only mapping between external formats and domain types.

## Interface Compliance

| Adapter | Port | Compile-Time Assert | Status |
|---------|------|---------------------|--------|
| `JolpicaAdapter` | `IF1DataPort` | `var _ ports.IF1DataPort = (*JolpicaAdapter)(nil)` | OK |
| `CacheAdapter` | `ICachePort` | `var _ ports.ICachePort = (*CacheAdapter)(nil)` | OK |
| `HTTPAdapter` | `IHTTPServerPort` | `var _ ports.IHTTPServerPort = (*HTTPAdapter)(nil)` | OK |
| `F1Service` | `IF1QueryPort` | _(none)_ | MISSING |

`F1Service` correctly implements all 5 methods of `IF1QueryPort` (`GetCurrentSchedule`, `GetRaceResult`, `GetLatestResult`, `GetDriverStandings`, `GetConstructorStandings`), but lacks a compile-time interface assertion. Not a boundary violation, but a best-practice gap.

## Dead Exports

| Export | File | Reason |
|--------|------|--------|
| `Race.DateTime()` | `domain/index.go` | Only referenced in `domain/index_test.go`; no production caller |
| `F1Service.GetFullStandings()` | `usecases/f1_service.go` | Only referenced in `usecases/f1_service_test.go`; no adapter or composition-root calls it |
| `StandingsResponse` | `domain/index.go` | Only used by `GetFullStandings`; not referenced by any port or adapter |

3 dead exports detected (-1 point each).

## Circular Dependencies

None found. The dependency graph is strictly acyclic:

```
domain (leaf — no project imports)
  ^
  |
ports (imports domain only)
  ^
  |
usecases (imports domain + ports)

adapters/primary   (imports domain + ports)
adapters/secondary (imports domain + ports)

composition-root (imports adapters + usecases — the only DI point)
```

## Cross-Adapter Coupling

None detected. No adapter imports any other adapter package.

## Score Breakdown

| Category | Deduction | Count | Total |
|----------|-----------|-------|-------|
| Boundary violations | -5 each | 0 | 0 |
| Circular dependencies | -3 each | 0 | 0 |
| Dead exports | -1 each | 3 | -3 |
| **Final score** | | | **97/100** |

## Recommendations

1. **Add compile-time interface assertion for F1Service.** Add `var _ ports.IF1QueryPort = (*F1Service)(nil)` in `f1_service.go` to catch method signature drift at compile time.

2. **Decide on `GetFullStandings` / `StandingsResponse`.** This method exists in the usecase layer but is not exposed through any port interface and no adapter calls it. Either:
   - Add it to `IF1QueryPort` and wire it into an HTTP handler (e.g., a `/standings` endpoint), or
   - Remove it and `StandingsResponse` to reduce dead code.

3. **Decide on `Race.DateTime()`.** This domain method is tested but unused in production. If it will be needed for future features (e.g., countdown timers, timezone display), keep it. Otherwise, remove it to keep the domain surface minimal.
