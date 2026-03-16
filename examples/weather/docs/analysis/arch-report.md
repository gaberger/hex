# Architecture Health Report — hex-f1

**Date:** 2025-07-24
**Health Score:** 95 / 100
**Project:** F1 Race Statistics (Go backend + HTMX frontend)

## Summary

The Go backend has a clean hexagonal architecture with proper layer separation. One soft boundary violation exists in the HTTP adapter importing domain types directly. The TypeScript frontend is scaffolded but unimplemented.

## Boundary Violations (1)

| File | Layer | Violation | Severity |
|------|-------|-----------|----------|
| `backend/src/adapters/primary/http_adapter.go` | Primary Adapter | Imports `domain` package directly (should only import `ports`) | Soft |

**Note:** Importing domain value objects (e.g., `Season`, `RoundNumber`) in adapters for type conversion is common Go hex practice. The adapter contains no business logic — only HTTP↔domain mapping.

## Cross-Adapter Coupling

None detected. ✅

## Circular Dependencies

None detected (Go compiler enforces this). ✅

## Dead Exports

None detected. All domain types and port interfaces are consumed. ✅

## Layer Inventory

### Backend (Go) — Fully Implemented

| Layer | Package | Types | Functions |
|-------|---------|-------|-----------|
| Domain | `core/domain` | 17 types (Season, Driver, Race, etc.) | 2 (FullName, DateTime) |
| Ports | `core/ports` | 4 interfaces (IF1DataPort, IF1QueryPort, IHTTPServerPort, ICachePort) | — |
| UseCases | `core/usecases` | 1 (F1Service) | 9 methods |
| Primary | `adapters/primary` | 3 (HTTPAdapter, templateData, bytesBuffer) | 11 functions |
| Secondary | `adapters/secondary` | 13 (JolpicaAdapter, CacheAdapter, JSON DTOs) | 11 functions |
| Composition | `composition-root.go` | — | 1 (main) |

### Frontend (TypeScript) — Scaffold Only

| Layer | File | Status |
|-------|------|--------|
| Domain | `core/domain/index.ts` | Empty scaffold |
| Ports | `core/ports/index.ts` | Empty scaffold |
| Composition | `composition-root.ts` | Minimal AppContext |

## Recommendations

1. **Frontend implementation needed** — Domain entities, ports, and adapters are all empty scaffolds
2. **Consider extracting domain DTOs** — The `http_adapter.go` could use port-level request/response types instead of importing `domain` directly, eliminating the soft violation
3. **Add tests** — The pipeline shows Tests layer is `[todo]`. Priority: unit tests for `F1Service` use case, integration tests for `JolpicaAdapter`
4. **Existing test files** — `index_test.go`, `cache_adapter_test.go`, and `f1_service_test.go` exist but are not tracked in the pipeline

## Score Breakdown

| Category | Deduction | Count |
|----------|-----------|-------|
| Boundary violations | -5 each | 1 |
| Circular dependencies | -3 each | 0 |
| Dead exports | -1 each | 0 |
| **Total deductions** | | **-5** |
| **Final score** | | **95/100** |
