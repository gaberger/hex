# Adversarial Testability Audit & E2E Test Design

## Part 1: Testability Audit

### 1.1 Existing Tests Assessment

| Test File | Tests Real Behavior? | Tests Port Contracts? | Verdict |
|---|---|---|---|
| `quality-score.test.ts` | YES -- exercises QualityScore domain math directly | N/A (pure domain) | GOOD |
| `feedback-loop.test.ts` | YES -- exercises FeedbackLoop state machine | N/A (pure domain) | GOOD |
| `task-graph.test.ts` | YES -- exercises TaskGraph topo sort | N/A (pure domain) | GOOD |
| `layer-classifier.test.ts` | YES -- pure function tests | N/A (pure function) | GOOD |
| `arch-analyzer.test.ts` | PARTIAL -- tests use case logic but mocks mask real adapter behavior | Mocks IASTPort + IFileSystemPort | CONCERN (see 1.3) |
| `cli-adapter.test.ts` | YES -- uses `runCLI` with captured output, no stdout side effects | Mocks all ports via AppContext | GOOD design, SHALLOW coverage |
| `filesystem-adapter.test.ts` | YES -- hits real temp filesystem | Tests real IFileSystemPort contract | GOOD (only real adapter test) |
| `composition-root.test.ts` | PARTIAL -- only checks property existence and `typeof === 'function'` | Does NOT call any adapter method except `fs.exists` | WEAK -- proves wiring but not function |

**Summary**: 6 of 8 test files test real behavior. But only ONE (filesystem-adapter) tests a real adapter against real I/O. The integration test is nearly vacuous -- it asserts shapes, not behavior.

### 1.2 Untestable Patterns

**1. `NotificationOrchestrator.start()` uses `setInterval`**
- File: `src/core/usecases/notification-orchestrator.ts`, line 97
- The `stallCheckTimer` fires based on wall-clock time. There is no way to inject a clock or advance time deterministically.
- `Date.now()` is called directly in `registerAgent()` (line 122), `handleEvent()` (lines 137-139), `checkForStalls()` (line 398), and `emit()` (line 466).
- FIX: Inject a `Clock` interface (`now(): number`) and a `Scheduler` interface (`setInterval(fn, ms): Disposable`).

**2. `FileSystemAdapter.glob()` uses `Bun.Glob` directly**
- File: `src/adapters/secondary/filesystem-adapter.ts`, line 39
- This is Bun-specific API. Cannot run under Node.js test runner, cannot be mocked.
- Not a testability problem per se (it IS an adapter), but it couples the entire project to Bun runtime.

**3. `TerminalNotifier` defaults to `process.stdout`**
- File: `src/adapters/secondary/terminal-notifier.ts`, line 85
- GOOD: Already injectable via `WritableOutput` parameter. Testable.

**4. `WebhookNotifier` uses `setTimeout` for batching and retry backoff**
- File: `src/adapters/secondary/webhook-notifier.ts`, lines 148, 174
- The `sleep()` method and `flushTimer` use real timers. Tests would need to either mock timers or call `flush()` directly. The `flush()` escape hatch exists but batch timing behavior is untestable.

**5. `EventBusNotifier.requestDecision()` uses `setTimeout` with real deadline**
- File: `src/adapters/secondary/event-bus-notifier.ts`, line 99
- 30-second default timeout in a Promise makes unit tests hang if the happy path fails.

**6. `RufloAdapter` shells out to `npx @claude-flow/cli@latest`**
- File: `src/adapters/secondary/ruflo-adapter.ts`, line 117
- Every method spawns a child process. Completely untestable without mocking `execFile`. No tests exist for this adapter.

**7. `cli.ts` calls `process.exit()`**
- File: `src/cli.ts`, line 12
- This kills the test runner. The `runCLI` function in cli-adapter.ts is the testable alternative, but the actual bin entry point is untestable.

**8. `composition-root.ts` has a silent fallback that masks failures**
- File: `src/composition-root.ts`, lines 78-89
- If TreeSitterAdapter.create() throws, a stub that returns empty data is silently used. Tests PASS but the system is BROKEN -- all AST summaries return zero exports, zero imports.

### 1.3 Mock vs Real: Port Contract Gaps

The London-school mocks in `arch-analyzer.test.ts` create a fundamental problem:

**IASTPort mock returns whatever you give it.** But the real TreeSitterAdapter:
- Returns imports with `from` as the raw string from the `import` statement (e.g., `'../../core/ports/index.js'`)
- The ArchAnalyzer then compares `edge.to` against file paths returned by `fs.glob`
- `fs.glob` returns paths like `src/core/ports/index.ts` (relative, no `./`, no `.js`)
- These NEVER match. The dependency graph is broken for real inputs.

**This is the single biggest bug in the codebase.** The mock tests pass because the mock AST returns imports with paths that match the mock FS glob results. But in production, the import paths and glob paths use different conventions, so:
- `buildDependencyGraph` produces edges where `to` is `'../../core/ports/index.js'`
- But the file list contains `'src/core/ports/index.ts'`
- They never match, so `findDeadExports` thinks EVERYTHING is dead
- `detectCircularDeps` finds NO cycles (because edges point to non-existent nodes)
- `validateHexBoundaries` partially works (classifyLayer checks `includes('/ports/')`) but `edge.to` like `'../../core/ports/index.js'` does contain `/ports/` so layer detection works by accident

### 1.4 What the Composition Root Integration Test Actually Proves

The test at `tests/integration/composition-root.test.ts`:
- Creates a real AppContext with the real project path
- Checks `typeof ctx.archAnalyzer.analyzeArchitecture === 'function'` -- this proves wiring but not behavior
- Checks `ctx.fs.exists('package.json')` -- the only real I/O assertion

It does NOT:
- Call `analyzeArchitecture` to see if it actually works
- Check if tree-sitter loaded successfully or fell back to the stub
- Verify that AST summaries contain actual data

---

## Part 2: E2E Test Design -- "hex Analyzes Itself"

### The Test

```typescript
// tests/e2e/self-analysis.test.ts
import { describe, it, expect } from 'bun:test';
import { createAppContext } from '../../src/composition-root.js';
import { runCLI, type AppContext as CLIContext } from '../../src/adapters/primary/cli-adapter.js';

const PROJECT_ROOT = '/Volumes/ExtendedStorage/PARA/01-Projects/hex';

describe('E2E: hex analyzes itself', () => {
  let ctx: Awaited<ReturnType<typeof createAppContext>>;

  // Phase 1: Composition root wires real adapters
  it('creates a real AppContext with working tree-sitter', async () => {
    ctx = await createAppContext(PROJECT_ROOT);
    // Verify tree-sitter loaded (not the fallback stub)
    const summary = await ctx.ast.extractSummary(
      'src/core/ports/index.ts', 'L1'
    );
    expect(summary.exports.length).toBeGreaterThan(5);
    expect(summary.imports.length).toBeGreaterThanOrEqual(0);
    expect(summary.lineCount).toBeGreaterThan(100);
  });

  // Phase 2: Token efficiency -- L1 must be smaller than L3
  it('L1 summaries are smaller than L3 for the same file', async () => {
    const l1 = await ctx.ast.extractSummary('src/core/ports/index.ts', 'L1');
    const l3 = await ctx.ast.extractSummary('src/core/ports/index.ts', 'L3');
    expect(l1.tokenEstimate).toBeLessThan(l3.tokenEstimate);
    // L1 should be at MOST 30% the size of L3
    expect(l1.tokenEstimate / l3.tokenEstimate).toBeLessThan(0.3);
  });

  // Phase 3: Full architecture analysis via the ArchAnalyzer
  it('analyzeArchitecture returns plausible results for this project', async () => {
    const result = await ctx.archAnalyzer.analyzeArchitecture(PROJECT_ROOT);
    // The project has ~20+ source files
    expect(result.summary.totalFiles).toBeGreaterThan(15);
    // The project exports many symbols
    expect(result.summary.totalExports).toBeGreaterThan(20);
    // Health score should be reasonable (not 0, not necessarily 100)
    expect(result.summary.healthScore).toBeGreaterThan(0);
  });

  // Phase 4: Hex boundary validation -- the project should follow its own rules
  it('hex has zero dependency violations against its own rules', async () => {
    const violations = await ctx.archAnalyzer.validateHexBoundaries(PROJECT_ROOT);
    if (violations.length > 0) {
      const report = violations.map(v =>
        `  ${v.from} -> ${v.to}\n    ${v.rule}`
      ).join('\n');
      throw new Error(`Hex boundary violations found:\n${report}`);
    }
  });

  // Phase 5: CLI end-to-end
  it('CLI analyze command produces structured output', async () => {
    const cliCtx: CLIContext = {
      rootPath: PROJECT_ROOT,
      archAnalyzer: ctx.archAnalyzer,
      ast: ctx.ast,
      fs: ctx.fs,
    };
    const result = await runCLI(['analyze', '.'], cliCtx, () => {});
    expect(result.exitCode).toBe(0);
    expect(result.output).toContain('Files scanned:');
    expect(result.output).toContain('Health score:');
  });

  // Phase 6: CLI summarize command
  it('CLI summarize shows real exports from tree-sitter', async () => {
    const cliCtx: CLIContext = {
      rootPath: PROJECT_ROOT,
      archAnalyzer: ctx.archAnalyzer,
      ast: ctx.ast,
      fs: ctx.fs,
    };
    const result = await runCLI(
      ['summarize', 'src/core/domain/entities.ts', '--level', 'L1'],
      cliCtx, () => {}
    );
    expect(result.exitCode).toBe(0);
    expect(result.output).toContain('QualityScore');
    expect(result.output).toContain('FeedbackLoop');
    expect(result.output).toContain('TaskGraph');
  });
});
```

### What MUST Work for This Test to Pass

1. **Tree-sitter WASM grammar must be installed and loadable.** The `web-tree-sitter` package plus `tree-sitter-typescript.wasm` must exist at the expected path.

2. **`FileSystemAdapter.glob('**/*.ts')` must return source files.** It uses `Bun.Glob` which must work in the test environment.

3. **Import path resolution must match glob paths.** This is the FATAL gap described in 1.3. Tree-sitter extracts import paths like `'../../core/ports/index.js'` but glob returns `'src/core/ports/index.ts'`. The ArchAnalyzer never normalizes them.

4. **L1 tokenEstimate must actually differ from L3.** Currently, `tokenEstimate` is always `Math.ceil(source.length / 4)` regardless of level. L1 and L3 return the SAME `tokenEstimate` because the estimate is based on the raw source length, not the summary size.

### What Is Currently BROKEN

| Blocker | Severity | File | Line | Description |
|---|---|---|---|---|
| Import path mismatch | CRITICAL | `arch-analyzer.ts` | 69-78 | `edge.to` contains raw import paths (relative, with `.js` extension); file list contains glob-relative paths (e.g., `src/foo.ts`). They never match. Dead export detection is 100% false positives. Circular dep detection misses everything. |
| Token estimate ignores level | HIGH | `treesitter-adapter.ts` | 65 | `tokenEstimate: Math.ceil(source.length / 4)` is computed from the raw source at ALL levels. L1 and L3 have identical `tokenEstimate`, making the "token efficiency" value proposition untestable. |
| Tree-sitter WASM path is wrong | HIGH | `composition-root.ts` | 76 | Grammar is loaded from `node_modules/web-tree-sitter/tree-sitter-typescript.wasm`. The `web-tree-sitter` package does NOT include language grammars. You need a separate `tree-sitter-typescript` package. This means tree-sitter ALWAYS fails and the stub is ALWAYS used in production. |
| `collectSummaries` called 4x | MEDIUM | `arch-analyzer.ts` | 187-194 | `analyzeArchitecture` calls `collectSummaries()` once directly, then `findDeadExports`, `validateHexBoundaries`, and `detectCircularDeps` each call it again. That is 4 full file scans + 4 tree-sitter parses per file. |
| Silent stub fallback | MEDIUM | `composition-root.ts` | 78-89 | When tree-sitter fails (which is always, see above), a stub returning empty summaries is used. No warning is logged. The CLI happily reports "0 files scanned, health 100" for any project. |

### What FAKE/STUB Behavior Masks Real Failures

1. **The tree-sitter stub in composition-root.ts** -- returns empty exports/imports for every file. ArchAnalyzer sees zero edges, zero exports, and reports a "perfect" health score of 100. This makes `hex analyze` ALWAYS succeed with zero findings.

2. **The NULL_EVENT_BUS** -- silently swallows all domain events. No notification, no logging. Tests and production both use it (the real event bus adapter exists but is never wired in).

3. **Mock AST in arch-analyzer.test.ts** -- uses file paths as both import sources AND glob results, bypassing the path normalization bug. The mock makes the test green while production is broken.

### Minimum Changes to Make E2E Green

```
1. INSTALL tree-sitter-typescript grammar (npm package or .wasm file)
   - Fix the WASM path in composition-root.ts to point to the correct grammar

2. ADD import path normalization to ArchAnalyzer
   - When building edges from imports, resolve relative paths against
     the importing file's directory and normalize to match glob output
   - Strip .js/.ts extensions for comparison, or normalize both sides

3. FIX tokenEstimate for L1 summaries
   - L1 should estimate based on the serialized summary size, not raw source
   - e.g., JSON.stringify({exports, imports}).length / 4

4. LOG a warning when tree-sitter falls back to stub
   - At minimum: console.warn('tree-sitter unavailable, using stub AST')
   - Better: make it an option to fail hard vs. fall back

5. CACHE collectSummaries results in analyzeArchitecture
   - Call once, pass to sub-methods instead of re-scanning 4 times
```

---

## Part 3: Top 10 Missing Test Scenarios

### 1. Tree-sitter Failure Mid-Analysis
**What**: Tree-sitter successfully parses 5 files, then throws on the 6th (corrupted syntax, unsupported construct).
**Why it matters**: `Promise.all` in `collectSummaries` will reject ALL results. No partial result is possible.
**Test**: Mock `extractSummary` to throw on the Nth call. Assert the analyzer returns partial results or a meaningful error.

### 2. Empty Project (Zero Source Files)
**What**: `fs.glob('**/*.ts')` returns an empty array.
**Why it matters**: `analyzeArchitecture` computes `healthScore = 100` for zero files. Division by zero is safe but the result is misleading.
**Test**: Assert that empty projects return a distinct status (score 0 or a flag indicating "no files").

### 3. Large Project (1000+ Files)
**What**: Performance and memory under scale.
**Why it matters**: `collectSummaries` does `Promise.all` over ALL files simultaneously. With 1000 files, this creates 1000 concurrent tree-sitter parses and file reads.
**Test**: Verify that the analyzer uses bounded concurrency (e.g., `p-limit`) or assert that 1000-file analysis completes within a time budget.

### 4. Concurrent Worktree Analysis
**What**: Two agents analyze overlapping file sets via different worktrees.
**Why it matters**: `FileSystemAdapter` stores its `root` in the constructor. If two ArchAnalyzers share an FS adapter but analyze different roots, results are silently merged.
**Test**: Create two ArchAnalyzer instances with different FS adapters pointing to different worktrees. Assert results are isolated.

### 5. Event Bus Replay Ordering
**What**: Domain events arrive out of causal order (e.g., `TestsPassed` before `CodeGenerated`).
**Why it matters**: `NotificationOrchestrator.updateAgentProgress` increments `iteration` on test events. Out-of-order delivery could inflate iteration counts.
**Test**: Emit events in reverse order. Assert progress state is still consistent.

### 6. Notification Rate Limiting Under Load
**What**: 100 trace-level events emitted in 1ms.
**Why it matters**: The `shouldEmit` rate limiter uses `Date.now()` which may not advance between calls in fast loops.
**Test**: Emit 100 trace events synchronously. Assert only 1 is emitted (respecting `traceThrottleMs`).

### 7. Decision Timeout Cascade
**What**: Multiple agents hit convergence drops simultaneously, each requesting a decision.
**Why it matters**: `EventBusNotifier.requestDecision` creates a 30-second timer per decision. If 10 decisions are pending, 10 timers run in parallel.
**Test**: Trigger 5 concurrent decisions. Assert all resolve within deadline and no memory leak (handlers cleaned up).

### 8. Git Adapter Error Handling
**What**: `git commit` fails because there are no staged files. `git diff` fails on invalid refs.
**Why it matters**: `GitAdapter` wraps all errors in `GitError` but tests never exercise this. Error messages may be truncated or misleading.
**Test**: Call `git.commit('test')` with no staged changes. Assert `GitError` contains the original stderr.

### 9. FileLogNotifier Rotation Under Concurrent Writes
**What**: Two agents writing to the same log file trigger rotation simultaneously.
**Why it matters**: `rotate()` renames the file, but another `appendEntry` call could race between the size check and the rename.
**Test**: Simulate concurrent `appendEntry` calls that both cross the 10MB threshold. Assert no data loss or duplicate rotation.

### 10. ArchAnalyzer Self-Referential Import (entities.ts imports from ports/index.ts)
**What**: `src/core/domain/entities.ts` imports from `'../ports/index.js'`. This is `domain -> ports`.
**Why it matters**: The layer classifier allows `domain -> ports`. But the ACTUAL hex rule says domain should have ZERO dependencies. The layer rules encode a pragmatic compromise that may silently permit architectural drift.
**Test**: Specifically assert whether `entities.ts` importing from `ports/index.ts` is flagged or allowed, and document the architectural decision either way.

---

## Summary of Critical Findings

1. **The ArchAnalyzer is fundamentally broken for real inputs** due to import path vs glob path mismatch. All unit tests pass because mocks use matching path conventions.

2. **Tree-sitter never loads in production** because the WASM grammar path points to a package that does not contain language grammars. The silent stub fallback makes this invisible.

3. **Token efficiency is a lie** -- L1 and L3 summaries report identical `tokenEstimate` values because the estimate is always based on raw source length.

4. **The composition root integration test proves almost nothing** -- it checks shapes but never calls the analyzer or verifies tree-sitter loaded.

5. **Zero adapter tests exist** for GitAdapter, WorktreeAdapter, BuildAdapter, RufloAdapter, TerminalNotifier, WebhookNotifier, EventBusNotifier, or FileLogNotifier.

6. **The NotificationOrchestrator is untestable** without injecting a clock due to `Date.now()` and `setInterval` throughout.
