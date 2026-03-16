# Contract Test Report: Mock Assumptions vs Real Adapter Behavior

**Date**: 2025-03-15
**Scope**: All unit test mocks in `tests/unit/` vs real adapters in `src/adapters/`

---

## Summary

| # | Contract Test | Result | Severity |
|---|---------------|--------|----------|
| 1 | fs.glob returns relative paths without `./` or `/` prefix | PASS | - |
| 2 | ast.extractSummary relative imports end in `.js` | PASS | - |
| 3 | resolveImportPath output matches glob format | PASS | - |
| 4 | L1 exports include interface declarations | PASS | - |
| 5 | Dead export detection on real codebase | INFO | - |
| 6 | Unused ports detection (ICodeGenerationPort etc.) | PASS | - |
| 7 | Mock import format matches tree-sitter output | PASS | - |
| 8 | normalizePath appends `.ts` to bare specifiers | PASS (cosmetic) | Low |
| 9 | Type-only imports (`import type {}`) parsed with names | PASS | - |
| 10 | `export type { X } from` re-exports NOT parsed as exports | **FAIL** | **High** |
| 11 | False positive dead exports from re-export bug | **FAIL** | **High** |
| 12 | Mock hides re-export bug | **FAIL** | **High** |
| 13 | hasReExports on ports/index.ts | PASS (mitigated by isEntryPoint) | - |
| 14 | composition-root imports vs real ports | PASS (false alarm -- source was refactored) | - |
| 15 | Root cause: composition-root only imports 4 from ports/index.js | PASS | - |
| 16 | Mock AppContext shape vs real AppContext | RISK | Medium |

---

## Bug 1 (HIGH): Tree-sitter misses `export type { X } from` re-exports

**Location**: `src/adapters/secondary/treesitter-adapter.ts`, `extractExports()` method (line 175)

**Root Cause**: The `extractExports()` method only looks for `export_statement` nodes that contain a declaration child (function, class, interface, etc. via `TS_NODE_KIND_MAP`). It does NOT handle `export_statement` nodes with `named_exports` (i.e., `export type { X, Y } from './foo.js'`).

**Impact**:
- `src/core/ports/index.ts` has `export type { Language, ASTSummary, ExportEntry, ImportEntry, ... }` re-exporting ~22 types from `value-objects.ts`
- Tree-sitter reports 0 of these as exports (only finds the 10 `interface` declarations)
- `findDeadExports()` then reports 3 false positives in `value-objects.ts`: `ExportEntry`, `ImportEntry`, `TestFailure`
- The remaining ~19 re-exported types avoid false-positive status only because they happen to be imported directly by other files

**Why mocks hide it**: The `arch-analyzer.test.ts` mock factory `mockAST()` manually populates `exports` arrays with whatever names the test wants. It never tests with real `export type {}` re-export patterns, so the tree-sitter parsing gap is invisible to unit tests.

**Fix**: In `treesitter-adapter.ts` `extractExports()`, add handling for `named_exports` nodes inside `export_statement`. These contain `export_specifier` children with `name` fields.

---

## Bug 2 (COSMETIC): normalizePath appends `.ts` to bare specifiers

**Location**: `src/core/usecases/path-normalizer.ts`, `normalizePath()` (line 43)

**Example**: `resolveImportPath('src/foo.ts', 'node:path')` returns `node:path.ts`

**Impact**: None on dead export detection (no file matches `node:path.ts`). The `dependencies` array in `ASTSummary` will contain `node:path.ts` instead of `node:path`, which is cosmetically wrong but functionally harmless since dependency analysis only uses file paths from `fs.glob()` for matching.

**Why mocks hide it**: The `path-normalizer.test.ts` explicitly tests this and documents the behavior. The test passes because it asserts the (wrong) output. This is "tested but incorrect" behavior.

---

## Risk 1 (MEDIUM): Mock AppContext missing 8 fields

**Location**: `tests/unit/cli-adapter.test.ts`, `mockContext()` function

**Issue**: The mock `AppContext` is missing: `notificationOrchestrator`, `llm`, `git`, `worktree`, `build`, `eventBus`, `notifier`, `swarm`

**Impact**: If CLI commands are added that reference these fields, the mock will silently return `undefined` instead of failing at compile time. Currently safe because the CLI adapter only uses `archAnalyzer`, `ast`, `fs`, `summaryService`.

**Mitigation**: The `AppContext` type in `cli-adapter.ts` may define its own narrower interface. If so, this is by design. If it imports the full `AppContext` from `composition-root.ts`, this is a type safety gap.

---

## Verified Mock Assumptions (PASS)

1. **Glob format**: Real `fs.glob()` returns `src/core/domain/entities.ts` format (no `./` prefix, no absolute path). Mocks use the same format. CORRECT.

2. **Import path format**: Real tree-sitter produces `{ from: '../ports/index.js' }` with `.js` extension. Mocks use the same format. CORRECT. (This was the previous normalization bug -- now fixed.)

3. **resolveImportPath alignment**: `resolveImportPath('src/adapters/secondary/git.ts', '../../core/ports/index.js')` returns `src/core/ports/index.ts`, which matches glob output. CORRECT.

4. **Type import parsing**: `import type { X }` statements correctly produce import entries with names populated. Mocks assume this. CORRECT.

5. **Layer classification**: `classifyLayer()` uses `/domain/`, `/ports/`, etc. substring matching. Glob paths contain these substrings. CORRECT.

---

## Recommendations

1. **Fix Bug 1**: Add `named_exports` / `export_specifier` handling to `extractExports()` in tree-sitter adapter. This will eliminate false-positive dead exports from re-export barrel files.

2. **Add integration contract test**: Create a test that runs `extractSummary` on a known file and asserts the export count matches expected. This catches tree-sitter parsing regressions without mocking.

3. **Fix Bug 2 (optional)**: Add early return in `normalizePath()` for bare specifiers (`!path.startsWith('.')` and `!path.startsWith('src/')`) to avoid appending `.ts`.

4. **Tighten AppContext mock**: Use `Partial<AppContext>` with `as AppContext` cast, or define a minimal interface that the CLI actually needs.
