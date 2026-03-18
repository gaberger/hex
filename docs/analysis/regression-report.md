# Regression Report: Post-Refactor Analysis

Date: 2025-03-15
Scope: TreeSitter downgrade (0.26->0.25), ArchAnalyzer additions, composition root changes, dashboard/CLI additions

## Test Baseline

All 172 tests pass (0 failures). This report covers regressions not caught by existing tests.

---

## Findings

### 1. PERFORMANCE REGRESSION: `analyzeArchitecture` calls `collectSummaries` 4x

**Rating: REGRESSION (confirmed)**

In `src/core/usecases/arch-analyzer.ts` line 193-201, `analyzeArchitecture()` does:

```
const summaries = await this.collectSummaries();          // call 1
const edges = await this.buildDependencyGraph('');         // call 2 (collectSummaries inside)
const [deadExports, violations, circularDeps] = await Promise.all([
  this.findDeadExports(''),                                // call 3 (collectSummaries inside)
  this.validateHexBoundaries(''),                          // call 4 (buildDependencyGraph -> collectSummaries)
  this.detectCircularDeps(''),                             // call 5 (buildDependencyGraph -> collectSummaries)
]);
```

`collectSummaries()` is called **5 times** total per `analyzeArchitecture()` invocation. Each call does `fs.glob('**/*.ts')` + `ast.extractSummary()` for every file. This is an O(5N) parse where O(N) would suffice. The summaries collected at line 194 are never even used by the subsequent calls -- each sub-method re-collects independently.

**Impact**: 5x slower than necessary on every analysis run. For a 50-file project, this means ~250 parse operations instead of ~50.

**Fix**: Pass the pre-collected summaries and edges to the sub-methods, or cache the result of `collectSummaries()` within a single `analyzeArchitecture()` call.

---

### 2. RISK: E2E test `CLIContext` is missing required fields

**Rating: RISK (potential break on type changes)**

In `tests/e2e/self-analysis.test.ts` lines 118-123, the `cliCtx` object is constructed with only `{ rootPath, archAnalyzer, ast, fs }` but the CLI adapter's `AppContext` type (defined via `Pick<>` in `cli-adapter.ts` line 28-31) requires 7 fields: `rootPath`, `archAnalyzer`, `ast`, `astIsStub`, `fs`, `codeGenerator`, `workplanExecutor`, `summaryService`.

This currently compiles because TypeScript structural typing allows extra properties and `CLIContext` is typed as the imported type. However, the missing fields (`astIsStub`, `codeGenerator`, `workplanExecutor`, `summaryService`) mean:
- The `analyze` command won't show the tree-sitter warning even if the stub is active
- The `generate` and `plan` commands would crash with property access on undefined
- The `summarize` command path through `summaryService` would fail

The test passes only because the specific test cases (`analyze`, `summarize`) don't exercise code paths that read those fields. But this is fragile -- any future test case for `generate` or `plan` will get a runtime crash, not a compile error.

---

### 3. RISK: Dashboard HTML served relative to `import.meta.url` breaks under bundling

**Rating: RISK (potential break)**

In `src/adapters/primary/dashboard-adapter.ts` line 155:
```typescript
const dir = dirname(fileURLToPath(import.meta.url));
const htmlPath = resolve(dir, 'dashboard', 'index.html');
```

The `package.json` build script uses `bun build src/cli.ts --outfile dist/cli.js` which bundles everything into a single file. After bundling, `import.meta.url` points to `dist/cli.js`, so `htmlPath` resolves to `dist/dashboard/index.html`. But `index.html` is at `src/adapters/primary/dashboard/index.html` and is never copied to `dist/dashboard/`.

The dashboard command will return 500 with "Dashboard HTML not found" when run from the built package.

**Fix**: Either copy `dashboard/index.html` to `dist/dashboard/` in the build script, or inline the HTML as a template literal, or add it to the `files` array in package.json with a copy step.

---

### 4. RISK: Dashboard port conflict has no EADDRINUSE handling

**Rating: RISK (poor error UX)**

In `dashboard-adapter.ts` line 87, `server.listen(this.port)` has no error handler for `EADDRINUSE`. If port 3847 is already in use, the process will throw an unhandled error that propagates as a generic "Error:" message through the CLI catch block rather than a helpful "Port 3847 already in use, try --port 3848".

---

### 5. OK: web-tree-sitter 0.25 module exports

**Rating: OK (verified working)**

The adapter does `const mod = await import('web-tree-sitter'); const ParserClass = mod.Parser;` and `mod.Language.load(wasmPath)`. Verified at runtime that web-tree-sitter 0.25.10 exports `{ Parser, Language, ... }` as named ESM exports. No `default` export exists, and the code correctly does not use one.

---

### 6. OK: TreeSitterAdapter.create() signature

**Rating: OK (backward compatible)**

The new signature is `create(grammarDirs: string | string[], fs: IFileSystemPort, rootPath?: string)`. The `rootPath` parameter is optional with a default of `process.cwd()`. Old 2-arg callers (`create(dirs, fs)`) will still work. The composition root was updated to pass 3 args.

---

### 7. OK: ArchAnalysisResult type has unusedPorts/unusedAdapters

**Rating: OK (verified)**

The `ArchAnalysisResult` interface in `value-objects.ts` already includes `unusedPorts: string[]` and `unusedAdapters: string[]`. The CLI test mock at `tests/unit/cli-adapter.test.ts` line 16 correctly includes both fields. No consumer breakage.

---

### 8. RISK: No tests for `detectUnusedPorts()`

**Rating: RISK (untested new feature)**

The `detectUnusedPorts()` method (arch-analyzer.ts lines 251-296) has zero test coverage. It also contributes to the health score formula (line 223: `healthScore -= Math.min(10, unusedPorts.length * 1)`). The health score change is also untested -- existing tests only assert `healthScore === 100` or `healthScore < 100`, never the specific penalty formula.

---

### 9. RISK: No tests for dashboard adapter

**Rating: RISK (untested new adapter)**

`dashboard-adapter.ts` (346 lines) has no unit or integration tests. This includes untested HTTP routing, SSE streaming, JSON parsing in POST /api/decisions, CORS headers, caching logic, and the swarm status fallback.

---

### 10. OK: No circular dependency from CLI -> dashboard dynamic import

**Rating: OK (verified)**

`cli-adapter.ts` does `await import('./dashboard-adapter.js')` dynamically. `dashboard-adapter.ts` imports `AppContext` from `../../composition-root.js` as a type-only import. No circular dependency exists because: (a) the import is dynamic/lazy, and (b) the type import is erased at runtime.

---

### 11. RISK: Three separate `AppContext` definitions risk drift

**Rating: RISK (maintainability)**

There are three `AppContext` definitions:
1. `src/composition-root.ts` line 42 -- canonical full interface
2. `src/core/ports/app-context.ts` line 14 -- port-layer duplicate
3. `src/adapters/primary/cli-adapter.ts` line 28 -- `Pick<>` subset
4. `src/adapters/primary/dashboard-adapter.ts` line 27 -- different `Pick<>` subset

The port-layer `AppContext` (item 2) is a separate interface that could drift from the composition root version. If a field is added to one but not the other, consumers importing from different locations will get different contracts.

---

### 12. RISK: `setup` command uses `import.meta.dir` (Bun-only API)

**Rating: RISK (Node.js incompatibility)**

In `cli-adapter.ts` line 449:
```typescript
const cliDir = typeof import.meta.dir === 'string' ? import.meta.dir : dirname(import.meta.url.replace('file://', ''));
```

The `import.meta.dir` property is Bun-specific and does not exist in Node.js. The fallback (`dirname(import.meta.url.replace('file://', ''))`) works but the URL-to-path conversion via string replacement is fragile -- it fails on Windows paths and doesn't handle URL-encoded characters. The `fileURLToPath` from `node:url` (which is used correctly in dashboard-adapter.ts line 155) should be used here instead.

---

## Summary Table

| # | Finding | Rating | Severity |
|---|---------|--------|----------|
| 1 | `collectSummaries` called 5x in `analyzeArchitecture` | REGRESSION | High (perf) |
| 2 | E2E test CLIContext missing 4 required fields | RISK | Medium |
| 3 | Dashboard HTML not bundled to dist/ | RISK | High (broken feature) |
| 4 | No EADDRINUSE error handling for dashboard port | RISK | Low |
| 5 | web-tree-sitter 0.25 module exports compatible | OK | -- |
| 6 | TreeSitterAdapter.create() backward compatible | OK | -- |
| 7 | ArchAnalysisResult type includes new fields | OK | -- |
| 8 | No tests for detectUnusedPorts | RISK | Medium |
| 9 | No tests for dashboard adapter | RISK | Medium |
| 10 | No circular dependency in dashboard import | OK | -- |
| 11 | Three AppContext definitions risk drift | RISK | Medium |
| 12 | `import.meta.dir` Bun-only, fragile fallback | RISK | Low |

## Priority Action Items

1. **[HIGH]** Refactor `analyzeArchitecture()` to call `collectSummaries()` once and pass results to sub-methods
2. **[HIGH]** Add build step to copy `dashboard/index.html` to `dist/dashboard/` or inline it
3. **[MEDIUM]** Add unit tests for `detectUnusedPorts()` and the new health score formula
4. **[MEDIUM]** Fix E2E test CLIContext to include all required fields
5. **[MEDIUM]** Add basic tests for dashboard adapter HTTP routes
6. **[LOW]** Use `fileURLToPath` in setup command instead of string replacement
7. **[LOW]** Add EADDRINUSE handling with suggested alternative port
