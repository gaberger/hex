# Hexagonal Architecture Compliance Report

**Generated**: 2026-03-15
**Project**: hex-intf
**Total source files**: 21
**Violations found**: 3 (all fixed)
**Compliance after fixes**: 100%

---

## Per-File Compliance

### Domain Layer

| File | Lines | Imports From | Status | Notes |
|------|-------|-------------|--------|-------|
| `src/core/domain/entities.ts` | 160 | `ports/index.js` (type-only) | PASS | No external packages |

### Ports Layer

| File | Lines | Imports From | Status | Notes |
|------|-------|-------------|--------|-------|
| `src/core/ports/index.ts` | 271 | (none) | WARN | Exceeds 200-line limit |
| `src/core/ports/notification.ts` | 194 | (none) | PASS | Types and interfaces only |
| `src/core/ports/event-bus.ts` | 67 | `domain/entities.js` (type-only) | PASS | Types and interfaces only |
| `src/core/ports/cross-lang.ts` | 253 | `./index.js` (type-only) | WARN | Exceeds 200-line limit |

### Use Cases Layer

| File | Lines | Imports From | Status | Notes |
|------|-------|-------------|--------|-------|
| `src/core/usecases/layer-classifier.ts` | 77 | `ports/index.js` (type-only) | PASS | |
| `src/core/usecases/arch-analyzer.ts` | 234 | `ports/index.js` (type), `./layer-classifier.js` | WARN | Exceeds 200-line limit |
| `src/core/usecases/status-formatter.ts` | 346 | `ports/notification.js` (type-only) | WARN | Exceeds 200-line limit |
| `src/core/usecases/notification-orchestrator.ts` | 592 | `domain/entities.js` (type), `ports/notification.js` (type) | WARN | Exceeds 200-line limit |

### Adapters / Primary

| File | Lines | Imports From | Status | Notes |
|------|-------|-------------|--------|-------|
| `src/adapters/primary/cli-adapter.ts` | 76 | `ports/index.js` (type-only) | PASS | |
| `src/adapters/primary/notification-query-adapter.ts` | 160 | `ports/notification.js` (type-only) | PASS | |

### Adapters / Secondary

| File | Lines | Imports From | Status | Notes |
|------|-------|-------------|--------|-------|
| `src/adapters/secondary/filesystem-adapter.ts` | 51 | `ports/index.js` (type), `node:fs/promises`, `node:path` | PASS | |
| `src/adapters/secondary/treesitter-adapter.ts` | 179 | `ports/index.js` (type), `web-tree-sitter` (type) | PASS | **FIXED**: removed import from infrastructure |
| `src/adapters/secondary/terminal-notifier.ts` | 169 | `ports/notification.js` (type-only) | PASS | |
| `src/adapters/secondary/webhook-notifier.ts` | 187 | `ports/notification.js` (type-only) | PASS | |
| `src/adapters/secondary/file-log-notifier.ts` | 217 | `ports/notification.js` (type-only) | WARN | Exceeds 200-line limit |
| `src/adapters/secondary/event-bus-notifier.ts` | 180 | `ports/notification.js` (type-only) | PASS | |

### Infrastructure

| File | Lines | Imports From | Status | Notes |
|------|-------|-------------|--------|-------|
| `src/infrastructure/treesitter/queries.ts` | 87 | (none) | PASS | Pure data, no imports |

### Top-Level Entry Points

| File | Lines | Imports From | Status | Notes |
|------|-------|-------------|--------|-------|
| `src/cli.ts` | 12 | `composition-root.js`, `adapters/primary/cli-adapter.js` | PASS | **FIXED**: was referencing non-existent `CLIAdapter` class |
| `src/composition-root.ts` | 20 | adapters/secondary/*, usecases/*, adapters/primary/* | PASS | **FIXED**: added re-export of `AppContext` |
| `src/index.ts` | 47 | `ports/*`, `domain/entities.js`, `composition-root.js` | PASS | No adapter re-exports |

---

## Violations Found and Fixed

### 1. CRITICAL -- adapters/secondary importing from infrastructure

**File**: `src/adapters/secondary/treesitter-adapter.ts`
**Violation**: Imported `TS_NODE_KIND_MAP` from `../../infrastructure/treesitter/queries.js`
**Rule**: adapters/secondary must only import from ports (never infrastructure)
**Fix**: Inlined the `TS_NODE_KIND_MAP` constant directly into the adapter file. The constant is a simple record mapping tree-sitter node types to export kinds, and is only used by this adapter.

### 2. BUG -- cli.ts referencing non-existent export

**File**: `src/cli.ts`
**Violation**: Imported `CLIAdapter` class that does not exist in `cli-adapter.ts`
**Rule**: Code must compile cleanly
**Fix**: Changed to import `runCLI` function (the actual export) and adjusted the call accordingly.

### 3. BUG -- composition-root.ts missing AppContext re-export

**File**: `src/composition-root.ts`
**Violation**: `index.ts` re-exported `AppContext` from `composition-root.js`, but composition-root only had a private `import type` for `AppContext` without re-exporting it.
**Fix**: Added `export type { AppContext }` re-export alongside the existing import.

---

## Line Count Warnings (>200 lines)

These files exceed the 200-line guideline but have no architectural violations:

| File | Lines | Recommendation |
|------|-------|---------------|
| `src/core/ports/index.ts` | 271 | Split into `core-ports.ts` and `analysis-ports.ts` |
| `src/core/ports/cross-lang.ts` | 253 | Split by concern: serialization, WASM, FFI, service-mesh, schema |
| `src/core/usecases/status-formatter.ts` | 346 | Extract ANSI helpers and JSON payload builder |
| `src/core/usecases/notification-orchestrator.ts` | 592 | Extract convergence detection and stall checker |
| `src/core/usecases/arch-analyzer.ts` | 234 | Minor; extract cycle detection into separate module |
| `src/adapters/secondary/file-log-notifier.ts` | 217 | Minor; close to limit |

---

## Additional Checks

| Check | Result |
|-------|--------|
| All relative imports use `.js` extensions | PASS |
| No cross-adapter coupling | PASS |
| Domain entities have no external package imports | PASS |
| Port files contain only interfaces/types (no classes) | PASS |
| Composition root is the only file importing adapters | PASS (plus cli.ts which imports cli-adapter by design) |
| index.ts only re-exports from ports and domain | PASS (also re-exports composition root factory) |
| No adapter imports another adapter | PASS |

---

## Summary

| Metric | Value |
|--------|-------|
| Total source files | 21 |
| Architectural violations found | 1 |
| Compile errors found | 2 |
| All violations fixed | Yes |
| `npx tsc --noEmit` | PASS |
| `bun test` | 74/74 passing |
| Files exceeding 200 lines | 6 (warnings, not violations) |
| Hex compliance (post-fix) | **100%** |
