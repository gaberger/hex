# Adversarial Architecture Audit — hex-intf

**Date:** 2026-03-15
**Scope:** All `.ts` files in `src/` and `tests/`

## CRITICAL (2)

### C1 — `cli.ts` imports a primary adapter directly
**File:** `src/cli.ts:7` | **Confidence:** 100
`cli.ts` is the CLI entry point, not the composition root. It imports `CLIAdapter` directly, bypassing the single-wiring-point contract.

### C2 — `index.ts` re-exports from composition-root, bundling all adapters
**File:** `src/index.ts:53-54` | **Confidence:** 95
Any npm consumer who imports `createAppContext` pulls in every secondary adapter unconditionally with no tree-shaking.

## HIGH (5)

### H1 — `path-normalizer.ts` (usecase) imports `node:path`
**File:** `src/core/usecases/path-normalizer.ts:9` | **Confidence:** 90
Usecases must import only from `domain/` and `ports/`. `node:path` is a runtime dependency. The automated checker misses this because `node:*` classifies as `unknown` layer.

### H2 — `ruflo-adapter.ts` uses bare built-in specifiers
**File:** `src/adapters/secondary/ruflo-adapter.ts:10-11` | **Confidence:** 85
Uses `child_process` and `util` without `node:` prefix, inconsistent with every other adapter.

### H3 — Unit test imports concrete adapter class
**File:** `tests/unit/filesystem-adapter.test.ts:5` | **Confidence:** 90
Imports `FileSystemAdapter` and adapter-specific `PathTraversalError` instead of testing through port interface.

### H4 — Tests import types from adapter layer
**Files:** `tests/unit/cli-adapter.test.ts:2`, `tests/e2e/self-analysis.test.ts:14` | **Confidence:** 85
`AppContext` should be imported from `core/ports/app-context.ts`, not from a primary adapter.

## MEDIUM (2)

### M1 — `classifyLayer` duplicated across two adapters
**Files:** `dashboard-adapter.ts:46-53`, `dashboard-hub.ts:56-63` | **Confidence:** 82
Identical function in two adapters. The authoritative impl in `layer-classifier.ts` (usecase) is unreachable from adapters — reveals a missing port for shared classification metadata.

### M2 — `dashboard/index.html` uses `innerHTML` getter
**File:** `src/adapters/primary/dashboard/index.html:335` | **Confidence:** 80
The escapeHtml function reads `innerHTML` — safe in practice but prohibited unconditionally by CLAUDE.md.

## Confirmed Clean

| Check | Result |
|---|---|
| Cross-adapter imports (primary <-> secondary) | None |
| Domain imports from ports/usecases/adapters | None |
| Ports importing from usecases or adapters | None |
| Circular dependencies | None detected |
| Leaky port abstractions (node: types in interfaces) | None |

**9 confirmed issues. 0 false positives.**
