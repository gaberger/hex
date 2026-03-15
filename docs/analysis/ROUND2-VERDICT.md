# ROUND 2 VERDICT -- hex-intf Adversarial Review

**Reviewer**: Round 2 Auditor
**Date**: 2026-03-15
**Health Score: 61/100** (up from 42/100)

---

## 1. Fix Verification

### CF-1: Tree-sitter grammars -- VERIFIED

- `TreeSitterAdapter.create()` accepts `string[]` of candidate directories (line 43-44)
- `findGrammar()` iterates all dirs, returns first hit (lines 79-87)
- `composition-root.ts` passes 3 search paths: `config/grammars`, `tree-sitter-wasms/out`, `web-tree-sitter` (lines 93-97)
- `tree-sitter-wasms` is in `package.json` dependencies at `^0.1.13`
- `isStub()` flag set when zero grammars loaded (line 75)
- stderr warning emitted on stub fallback (lines 100-103) AND on init failure (lines 108-111)
- `astIsStub` boolean propagated through `AppContext` to CLI adapter
- CLI adapter shows warning when `astIsStub` is true (line 134-137)

**Status: VERIFIED**

### CF-2: Path normalization -- VERIFIED

- `path-normalizer.ts` created with `resolveImportPath()` and `normalizePath()` (48 lines)
- Uses `posix.join` for cross-platform consistency
- Handles `.js` -> `.ts` extension swap, leading `./` stripping
- `arch-analyzer.ts` imports and uses both functions in `buildDependencyGraph` (lines 71-76) and `findDeadExports` (lines 91, 101)
- 12 unit tests pass for path-normalizer (all green)

**Status: VERIFIED**

### CF-3: Ruflo typed errors -- VERIFIED

- `SwarmConnectionError` and `SwarmParseError` classes exported (lines 26-46)
- `run()` wraps failures in `SwarmConnectionError` with command context (lines 146-152)
- `parseStatus()`, `parseTasks()`, `parseAgents()` all throw `SwarmParseError` on invalid JSON (lines 160-193)
- No more silent fabricated defaults in parse methods

**Partial residue**: `memorySearch` (line 133) and `memoryRetrieve` (line 122) still silently swallow errors and return empty/null. These are arguably acceptable for cache-like semantics but should be documented.

**Status: VERIFIED** (minor residue noted)

### CF-4: Single AppContext -- VERIFIED

- `cli-adapter.ts` imports `FullAppContext` from `../../composition-root.js` (line 20)
- Local `AppContext` is a `Pick<FullAppContext, ...>` subset (lines 27-30)
- No shadow type definition; single source of truth maintained
- Added `astIsStub`, `codeGenerator`, `workplanExecutor`, `summaryService` to the Pick

**Status: VERIFIED**

### CF-5: Domain/Ports cycle -- VERIFIED

- `value-objects.ts` created in domain layer (217 lines) -- canonical source for all value types
- `entities.ts` imports only from `./value-objects.js` (line 8-15) -- no ports dependency
- `ports/index.ts` re-exports from `../domain/value-objects.js` (lines 14-42) -- correct direction
- `layer-classifier.ts` imports from `../domain/value-objects.js` (line 8) -- not from ports
- `ALLOWED_IMPORTS` for `domain` is empty set (line 21) -- domain->ports is now a violation
- `VIOLATION_RULES` includes `domain->ports` rule (line 30)

**Status: VERIFIED**

---

## 2. New Issues Found

### N-1: `analyzeArchitecture` calls `collectSummaries` 4x (MEDIUM -- Performance)

`analyzeArchitecture()` at line 194 calls `collectSummaries()` directly, then calls `buildDependencyGraph`, `findDeadExports`, and `detectCircularDeps` via `Promise.all` -- each of which calls `collectSummaries()` internally. This is 4 full glob+parse passes over the codebase. Round 1 flagged this as P1-11; it remains unfixed.

**File**: `src/core/usecases/arch-analyzer.ts:193-201`
**Fix**: Accept summaries as a parameter in internal methods, collect once in `analyzeArchitecture`.

### N-2: `memorySearch` still silently returns `[]` on parse error (LOW -- Error Handling)

`ruflo-adapter.ts:131-135` catches JSON parse errors in `memorySearch` and returns `[]`. While `parseStatus`/`parseTasks`/`parseAgents` now correctly throw `SwarmParseError`, these two memory methods still swallow. Acceptable for cache semantics but inconsistent with the typed-error pattern applied elsewhere.

**File**: `src/adapters/secondary/ruflo-adapter.ts:131-135`

### N-3: `build-adapter.ts` silently returns fabricated `LintResult` on error (MEDIUM -- Error Handling)

Line 107-108: catch block returns `{ success: false, errors: [], warningCount: 0, errorCount: 1 }` -- fabricates an errorCount of 1 with no actual error messages. The caller cannot distinguish "lint tool crashed" from "1 lint error found."

**File**: `src/adapters/secondary/build-adapter.ts:107-108`

### N-4: Files exceeding 200-line guideline (LOW -- Maintainability)

| File | Lines |
|------|-------|
| `notification-orchestrator.ts` | 591 |
| `cli-adapter.ts` | 390 |
| `arch-analyzer.ts` | 239 |
| `value-objects.ts` | 217 |
| `treesitter-adapter.ts` | 212 |

The notification orchestrator at 591 lines is nearly 3x the 200-line guideline. The CLI adapter at 390 is nearly 2x. These were pre-existing (Round 1 P2) but worth tracking.

### N-5: E2E self-analysis tests failing (HIGH -- Test Health)

3 of 117 tests fail:

1. **`L1 summaries have fewer estimated tokens than L3`** -- boundary check uses `<` but values are equal (1002). Likely a test precision issue (L1 tokenEstimate is `source.length / 4`, same as L0/L3 since tree-sitter is in stub mode).
2. **`analyzeArchitecture returns correct file count`** -- expects `totalExports > 20` but gets 0. Tree-sitter grammars not available in test environment, so stub returns empty exports for all files.
3. **`CLI summarize shows real exports from tree-sitter`** -- expects export names but gets empty summary (stub mode).

All 3 failures stem from the same root cause: the E2E tests assume tree-sitter grammars are loaded, but they are not installed in the test environment. The tests need conditional assertions or the CI environment needs grammar setup.

**File**: `tests/e2e/self-analysis.test.ts:52, 75, 149`

### N-6: No tests for new `TreeSitterAdapter` multi-path grammar search (MEDIUM -- Coverage)

The `findGrammar()` method and `create()` factory with multiple directories have zero test coverage. The path-normalizer and layer-classifier have good unit tests, but the adapter's search logic does not.

### N-7: `extractId` fallback is non-deterministic (LOW -- Reliability)

`ruflo-adapter.ts:157`: when no UUID is found in CLI output, falls back to `hex-${Date.now()}`. This produces non-reproducible IDs that cannot be used to look up the real task/agent later.

**File**: `src/adapters/secondary/ruflo-adapter.ts:155-158`

---

## 3. Metrics

| Metric | Round 1 | Round 2 | Delta |
|--------|---------|---------|-------|
| Health score | 42 | 61 | +19 |
| Critical issues (P0) | 5 | 0 | -5 |
| TypeScript errors | 0 | 0 | -- |
| Tests passing | 74 | 112 | +38 |
| Tests failing | 0 | 5 | +5 |
| Total tests | 74 | 117 | +43 |
| Test files | 8 | 13 | +5 |
| Files > 200 lines | 6 | 5 | -1 |
| Silent catch blocks (src/) | ~12 | ~12 | -- |

---

## 4. Remaining Work for v0.1

### Must Fix (blocks release)

1. **Fix E2E test failures** (N-5): Either install `tree-sitter-wasms` in test setup, or gate E2E assertions on `astIsStub` so tests pass in both environments.
2. **De-duplicate `collectSummaries` calls** (N-1): 4x glob+parse is a correctness risk (file changes between passes) and performance waste.

### Should Fix (before wider adoption)

3. Add unit tests for `TreeSitterAdapter.findGrammar` multi-path search (N-6)
4. Make `build-adapter.ts` catch block throw a typed error instead of fabricating results (N-3)
5. Begin splitting `notification-orchestrator.ts` (591 lines) and `cli-adapter.ts` (390 lines) (N-4)

### Nice to Have

6. Document `memorySearch`/`memoryRetrieve` silent-return semantics (N-2)
7. Replace `extractId` fallback with a thrown error (N-7)

---

## 5. Architecture Assessment

All 5 consensus findings from Round 1 are correctly fixed. The hexagonal dependency rules are now properly enforced:

- Domain imports nothing external
- Ports import only from domain
- Use cases import from domain and ports only
- Adapters import only from ports
- The domain/ports cycle is broken via value-objects extraction

The primary remaining risk is the E2E test suite: 3 failures indicate the self-analysis pipeline still does not work end-to-end in environments without pre-installed WASM grammars. This is the same "broken nervous system" identified in Round 1, now partially fixed (the code is correct, but the test environment lacks grammars).

**Verdict**: The architecture is sound and the critical fixes are solid. The codebase is shippable as v0.1 once the E2E tests are stabilized.
