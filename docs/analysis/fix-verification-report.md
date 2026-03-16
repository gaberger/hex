# Architecture Review Fix Verification Report

Date: 2025-03-15

---

## Fix 1: Tree-sitter WASM Loading

**Verdict: VERIFIED**

- `composition-root.ts` lines 94-98: grammar dirs are relative strings (`'config/grammars'`, `'node_modules/tree-sitter-wasms/out'`, `'node_modules/web-tree-sitter'`).
- `TreeSitterAdapter.findGrammar()` (line 85-93): uses `fs.exists(relative)` for safe traversal checking, then `pathResolve(this.rootPath, relative)` to produce an absolute path for `Language.load()`.
- If `node_modules/tree-sitter-wasms/out/` does not exist, `findGrammar` returns null for each grammar, `langMap` stays empty, and `_isStub` becomes `true` (line 77). The composition root then sets `astIsStub = true` (line 100-101). No crash.

**Remaining concern:** None. Clean fallback path.

---

## Fix 2: Import Path Normalization

**Verdict: PARTIALLY FIXED**

What works:
- `path-normalizer.ts` correctly handles `.js` to `.ts`, `.jsx` to `.tsx`, leading `./` stripping, and extensionless paths.
- `arch-analyzer.ts` uses `normalizePath()` on `summary.filePath` (line 71) and `resolveImportPath()` on import targets (line 75). Both sides of edge comparison are normalized.

**Issues found:**

1. **Barrel imports without `/index`**: If code imports `'../ports'` (no `/index.js`), `normalizePath` appends `.ts` producing `src/core/ports.ts` instead of `src/core/ports/index.ts`. This will cause dead-export false positives for anything exported from barrel files via directory imports. **NOT FIXED.**

2. **Bare specifier pollution**: `normalizePath('node:path')` returns `node:path.ts` (confirmed by test on line 37). This is harmless for dead-export detection since bare specifiers won't match any `filePath`, but it is semantically wrong and the test explicitly asserts this incorrect behavior as correct.

3. **Query strings / hash fragments**: Not handled. Unlikely in TypeScript but not guarded against.

---

## Fix 3: Token Estimates Vary by Level

**Verdict: VERIFIED**

- **L0** (line 108): `Math.ceil((filePath.length + 20) / 4)` -- tiny estimate based on filename only.
- **L1/L2** (lines 138-140): estimate based on serialized summary text (export signatures + import lines), not raw source. Proportional to structural content extracted.
- **L3** (line 116): `Math.ceil(source.length / 4)` -- full raw source token estimate.
- **L1 vs L2**: Both use the same formula. L2 includes signatures (`withSigs=true` on line 133), so its serialized summary text will be longer, producing a naturally larger `tokenEstimate`. Meaningful differentiation.
- **Parse failure fallback** (line 129): falls back to `fullTokenEstimate` -- reasonable since we have no structural data.

---

## Fix 4: unusedPorts Detection

**Verdict: PARTIALLY FIXED**

What works:
- `detectUnusedPorts()` (lines 251-296) correctly filters for interfaces matching `I*Port` naming convention from `/ports/` files.
- Checks whether any `/adapters/` file imports that port name.

**Issues found:**

1. **IEventBusPort in event-bus.ts**: This port is defined in `src/core/ports/event-bus.ts`. The method checks `normalized.includes('/ports/')` (line 259), so it would be found. However, `IEventBusPort` is imported by `composition-root.ts` (line 21) which is NOT in `/adapters/`. It is imported via `type` import for the `AppContext` interface, and the `NULL_EVENT_BUS` stub implements it inline. No adapter file imports `IEventBusPort`, so it would be flagged as an unused port. This is a **false positive** -- the port is used, just not by a file in `/adapters/`.

2. **Use-case imports ignored**: If a port is imported by a use case (e.g., `ArchAnalyzer` imports `IASTPort`), but no adapter imports it by name (adapter implements it but uses `implements` keyword without importing the interface type), the port would be flagged unused. The detection only checks adapter import names, not `implements` clauses.

3. **No test coverage**: The arch-analyzer tests do not include any test for `detectUnusedPorts` or `unusedAdapters`. The `analyzeArchitecture` tests check `healthScore` and `violationCount` but never assert on `unusedPorts` or `unusedAdapters` arrays.

---

## Fix 5: Health Score Formula

**Verdict: VERIFIED (with concern)**

Formula (lines 219-224):
- Start at 100
- `-10` per boundary violation
- `-15` per circular dependency
- `-1` per dead export, **capped at 20**
- `-1` per unused port, **capped at 10**
- Clamped to `[0, 100]`

**Concern:** The dead export cap of 20 means a project with 100 dead exports scores the same as one with 20. For a large codebase with significant technical debt, this might be too lenient. However, the rationale is likely that dead exports are minor issues compared to violations/cycles, and the cap prevents them from dominating the score. Reasonable design choice.

---

## Fix 6: Silent Fallback Now Warns

**Verdict: NOT FIXED**

- `composition-root.ts` lines 104-116: The `catch` block creates a stub AST silently. There is **no `console.warn()` or `console.error()` call anywhere** in the catch block or the `isStub()` code path.
- The `isStub()` boolean is set and `astIsStub` is exposed on `AppContext`, but no warning is emitted to stderr (or stdout).
- The composition-root integration test (`tests/integration/composition-root.test.ts`) does not test for any warning output.
- A user whose tree-sitter fails to load will get silently degraded analysis with no diagnostic feedback.

---

## Summary Table

| # | Fix Claimed | Verdict | Severity of Gap |
|---|-------------|---------|-----------------|
| 1 | Tree-sitter WASM loading | **VERIFIED** | -- |
| 2 | Import path normalization | **PARTIALLY FIXED** | Medium (barrel imports break dead-export detection) |
| 3 | Token estimates vary by level | **VERIFIED** | -- |
| 4 | unusedPorts detection | **PARTIALLY FIXED** | Medium (false positives, no tests) |
| 5 | Health score formula | **VERIFIED** | Low (cap debatable but reasonable) |
| 6 | Silent fallback now warns | **NOT FIXED** | Medium (no warning emitted at all) |

## Recommended Actions

1. Add barrel/directory import resolution to `path-normalizer.ts` (check if path is a directory and append `/index`).
2. Expand `detectUnusedPorts` to also check use-case and composition-root imports, not only adapter imports.
3. Add `console.warn('[hex-intf] tree-sitter failed to initialize, using stub AST')` to the catch block in `composition-root.ts`, writing to stderr.
4. Add unit tests for `detectUnusedPorts` and `unusedAdapters` in `arch-analyzer.test.ts`.
5. Fix the bare-specifier test to not assert `node:path.ts` as correct behavior -- bare specifiers should be returned unchanged.
