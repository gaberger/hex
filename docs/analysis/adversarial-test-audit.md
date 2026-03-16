# Adversarial Test Quality Audit

**Date**: 2026-03-15
**Scope**: All test files in `tests/`, `examples/*/tests/`
**Source cross-reference**: All `.ts` files in `src/`

---

## 1. Coverage Gap Analysis

### Source files with NO corresponding test

| Source File | Criticality | Risk |
|---|---|---|
| `src/core/usecases/scaffold-service.ts` | **9/10** | Generates project scaffolds (README, CLAUDE.md, scripts). Zero tests. Contains branching logic for language/runtime detection, dev server config, and file generation. A bug here silently ships broken projects. |
| `src/core/usecases/notification-orchestrator.ts` | **8/10** | 600-line class with rate limiting, stall detection, convergence checks, decision escalation. Zero tests. Contains `setInterval` timers and complex state machine logic. |
| `src/core/usecases/status-formatter.ts` | **6/10** | ANSI formatting, progress bars, JSON payload construction. Zero tests. Lower risk since output is visual, but `buildJsonPayload` is consumed by webhooks. |
| `src/core/domain/errors.ts` | **4/10** | Error class hierarchy. Simple constructors, but `BoundaryViolation` encodes `fromLayer`/`toLayer` which consumers rely on. |
| `src/adapters/secondary/ruflo-adapter.ts` | **8/10** | Shell command construction via `execFile`. `extractJson` parsing logic. `toSwarmStatus` field mapping with fallback chains. Zero tests. A parsing bug means silent swarm failures. |
| `src/adapters/secondary/git-adapter.ts` | **7/10** | Git operations via `execFile`. Untested. |
| `src/adapters/secondary/llm-adapter.ts` | **7/10** | API key handling, streaming, token counting. Untested. |
| `src/adapters/secondary/build-adapter.ts` | **6/10** | Compile/lint/test orchestration. Untested. |
| `src/adapters/secondary/worktree-adapter.ts` | **6/10** | Git worktree management. Untested. |
| `src/adapters/secondary/terminal-notifier.ts` | **5/10** | Terminal output formatting. Untested. |
| `src/adapters/secondary/event-bus-notifier.ts` | **5/10** | Event bus notification delivery. Untested. |
| `src/adapters/secondary/file-log-notifier.ts` | **5/10** | File-based log notification. Untested. |
| `src/adapters/secondary/webhook-notifier.ts` | **5/10** | HTTP webhook delivery. Untested. |
| `src/adapters/secondary/registry-adapter.ts` | **5/10** | Registry operations. Untested. |
| `src/adapters/primary/dashboard-adapter.ts` | **6/10** | HTTP server for dashboard. Untested. |
| `src/adapters/primary/dashboard-hub.ts` | **5/10** | SSE hub for dashboard. Untested. |
| `src/adapters/primary/mcp-adapter.ts` | **7/10** | MCP tool definitions and routing. Untested. Tool definitions are data but `MCPAdapter.handleToolCall` has dispatch logic. |
| `src/adapters/primary/notification-query-adapter.ts` | **5/10** | Notification query delegation. Untested. |
| `src/composition-root.ts` | **Partial** | Has integration test checking property existence, but does NOT test actual wiring correctness (e.g., that the right adapter is injected for each port). |
| `src/cli.ts` | **7/10** | Entry point. Untested directly (partially covered by E2E). |
| `src/index.ts` | **3/10** | Public API re-exports. Low risk. |

**Summary**: 20 of ~35 source files have zero test coverage. The tested files represent ~43% of the codebase by file count, but the untested files include critical business logic (scaffold-service, notification-orchestrator) and all secondary adapters.

---

## 2. Test-Code Mirroring Analysis

The CLAUDE.md warns: *"Tests can mirror bugs when the same LLM writes code AND tests."*

### Detected mirroring patterns

**quality-score.test.ts -- Hardcoded formula coefficients**
The test at line 36 asserts `allPass.score - halfPass.score` equals exactly `30`. This encodes the exact weight (60% * 0.5 = 30) from the `QualityScore.score` getter. If the weight formula is *wrong* (e.g., test pass ratio should be weighted at 40%, not 60%), both code and test agree on the wrong answer. The test verifies the implementation matches itself rather than verifying a behavioral property like "higher test pass rate always produces higher score."

**code-generator.test.ts -- Prompt content assertions**
Tests assert that specific strings appear in prompts sent to the LLM mock (`toContain('JWT validation')`). This tests prompt assembly implementation, not generation behavior. If the prompt format changes but still conveys the same requirements, these tests break without a real bug.

**arch-analyzer.test.ts -- Weak violation assertions**
Line 109: `expect(violations.length).toBeGreaterThan(0)` -- this passes whether there are 1 or 100 violations. It does not verify the violation is the *correct* one (i.e., that `domain -> adapter` is caught vs some other spurious violation).

**path-normalizer.test.ts -- Direct output matching**
Tests like `expect(normalizePath('src/core/foo.js')).toBe('src/core/foo.ts')` are appropriate here since path normalization has a clear, well-defined contract. This is NOT mirroring; it is correct behavioral testing.

### Verdict
The quality-score and code-generator tests show moderate mirroring risk. The quality-score tests should use property-based assertions ("monotonically increases with test pass ratio") rather than hardcoded deltas.

---

## 3. Missing Edge Cases

### code-generator.ts
- **Empty spec requirements/constraints**: What happens with `requirements: []`?
- **LLM returns empty string**: No test for empty/malformed LLM response
- **LLM response with markdown fences**: `stripCodeFences` is untested directly
- **`toFileName` edge cases**: Titles with unicode, all-special-chars, very long titles
- **`archAnalyzer` feedback loop**: `refineFromArchAnalysis` has a `MAX_ARCH_REFINE_PASSES=2` loop -- untested
- **Build failure -> refine path**: `refineFromBuildErrors` is exercised only by mocking success

### arch-analyzer.ts
- **Empty project** (zero files): Does `analyzeArchitecture` crash or return reasonable defaults?
- **Files with no exports/imports**: Edge case for dead export detection
- **Non-TypeScript files** in glob results
- **Very deep circular dependency chains** (A->B->C->D->...->A)

### filesystem-adapter.ts
- **Symlink traversal**: `../` is blocked, but what about symlinks that escape the root?
- **Unicode filenames**: Not tested
- **Very long paths**: Platform-specific limits
- **Concurrent write/read**: No concurrency test
- **Empty file read**: Not tested
- **glob with no matches**: Returns `[]`? Not explicitly tested.

### cli-adapter.ts
- **No arguments at all**: `runCLI([])` is not tested
- **Analyze with missing path argument**: Not tested
- **Summarize with missing file argument**: Not tested
- **Very large output**: Not tested

### layer-classifier.ts
- **Windows-style paths** (`src\core\domain\foo.ts`): Not tested
- **Paths with extra slashes** (`src//core//domain/foo.ts`): Not tested
- **Case sensitivity**: `src/Core/Domain/foo.ts`

### workplan-executor.ts
- **Empty requirements list**: `createPlan([], 'typescript')`
- **Malformed LLM JSON response**: The LLM mock always returns valid JSON. What if it returns garbage?
- **Circular dependencies in plan steps**: Steps that depend on each other

### task-graph.ts
- **Duplicate step IDs**: `addStep` with same ID twice
- **Self-referencing dependency**: Step depends on itself
- **Circular dependencies in topologicalSort**: Does it hang, throw, or return partial order?

### feedback-loop.ts
- **Large iteration counts** (thousands): Performance/memory concern
- **Concurrent record calls**: Thread safety

---

## 4. Test Isolation Analysis

### Tests with external state dependencies

| Test | External Dependency | Risk |
|---|---|---|
| `filesystem-adapter.test.ts` | Real filesystem via `tmpdir()` | **Acceptable** -- uses temp dir with cleanup. Deterministic. |
| `composition-root.test.ts` | Hardcoded path `/Volumes/ExtendedStorage/PARA/01-Projects/hex` | **CRITICAL**: Fails on any other machine. Not portable. |
| `self-analysis.test.ts` (E2E) | Same hardcoded path + real tree-sitter WASM grammars | **CRITICAL**: Fails on any other machine and requires grammar installation. |

### Determinism issues

- **NotificationOrchestrator** uses `Date.now()` and `setInterval` -- if tested, these would need time mocking
- **FeedbackLoop/QualityScore tests**: Fully deterministic, no time dependencies. Good.
- **All unit tests except filesystem**: Use in-memory mocks only. Good.

### Verdict
The integration and E2E tests are machine-specific. The hardcoded `PROJECT_ROOT` path makes CI impossible without modification. This should use `process.cwd()` or `import.meta.dir`.

---

## 5. Property-Based Testing Assessment

### Current state
The only property-based test in the project is `examples/flappy-bird/tests/property/physics-properties.test.ts` for the Flappy Bird example. The core `hex` framework has **zero property tests**.

### Modules that SHOULD have property tests

| Module | Property to Test | Criticality |
|---|---|---|
| `QualityScore` | `score` is monotonically non-decreasing with better inputs; always in [0, 100]; `compileSuccess=false` always yields 0 | **9/10** |
| `path-normalizer` | `resolveImportPath(a, b)` always returns a valid path; `normalizePath` is idempotent: `normalize(normalize(x)) === normalize(x)` | **8/10** |
| `layer-classifier` | `isAllowedImport(a, a)` is always true (same-layer); `classifyLayer` output is always from the known enum | **7/10** |
| `TaskGraph.topologicalSort` | Output contains all input steps; for every edge A->B, A appears before B in output | **8/10** |
| `import-boundary-checker` | `checkImport(f, f, [])` is always null (same file); consistency with `allowedImportsFor` | **7/10** |
| `stripCodeFences` (code-generator) | `stripCodeFences(stripCodeFences(x)) === stripCodeFences(x)` (idempotent) | **5/10** |
| `FeedbackLoop.isConverging` | With strictly increasing scores, always returns true | **6/10** |

---

## 6. Integration Test Gap Analysis

### composition-root.test.ts
The existing test only verifies that `createAppContext` returns an object with the expected property names and that `typeof` checks pass. It does NOT verify:

- That `archAnalyzer` actually uses the real `TreeSitterAdapter` (not a stub)
- That `fs` is rooted at the correct project path
- That ports are wired to the correct adapters
- That the context works end-to-end (e.g., `archAnalyzer.analyzeArchitecture()` returns real data)

The E2E test (`self-analysis.test.ts`) partially covers this, but is machine-specific.

### Missing adapter contract tests
No adapter has a contract test that verifies it implements its port interface correctly with real (or simulated) external dependencies:

- **TreeSitterAdapter**: No test that it can parse a real TypeScript file
- **GitAdapter**: No test against a real git repo
- **BuildAdapter**: No test with a real TypeScript project
- **RufloAdapter**: No test against a running ruflo daemon (even a mock server)
- **LLMAdapter**: No test against a mock HTTP server simulating the API
- **FileSystemAdapter**: Has real tests (good), but no contract verification against `IFileSystemPort` interface completeness

### Missing cross-layer integration tests
- CLI -> ArchAnalyzer -> TreeSitter -> FileSystem: Covered only by E2E (machine-specific)
- CLI -> CodeGenerator -> LLM -> Build: Not tested at all
- WorkplanExecutor -> Swarm -> Agent lifecycle: Not tested end-to-end
- NotificationOrchestrator -> Notifiers -> Terminal/Webhook: Not tested at all

---

## 7. Mutation Testing / Weak Assertion Analysis

### Tests that would pass with key logic deleted

**composition-root.test.ts** -- Almost all assertions are `toHaveProperty` or `typeof` checks:
```
expect(ctx).toHaveProperty('archAnalyzer')  // passes even if archAnalyzer is null
expect(typeof ctx.fs.read).toBe('function') // passes even if read() always throws
```
Deleting the entire body of `createAppContext` and returning `{ archAnalyzer: { analyzeArchitecture: () => {} }, ... }` would pass all assertions except the `fs.exists('package.json')` check.

**arch-analyzer.test.ts line 109, 123, 143, 161**:
```
expect(violations.length).toBeGreaterThan(0)
expect(cycles.length).toBeGreaterThan(0)
```
These pass with any non-empty array. If `validateHexBoundaries` returned `[{bogusViolation}]` for every input, these tests would still pass. The violation content (fromLayer, toLayer, rule) is only spot-checked in one test.

**code-generator.test.ts** -- `toContain` on prompt strings:
If the system prompt accidentally included ALL possible strings (a bug), every `toContain` check passes. No test verifies that irrelevant content is ABSENT.

**workplan-executor.test.ts line 115-116**:
```
expect(failed.length).toBeGreaterThan(0)
expect(failed[0].errors![0]).toContain('Swarm down')
```
Good -- this checks specific error content. However, no test verifies that successful execution does NOT produce errors.

### Specific weak assertion patterns

| File | Line | Assertion | Issue |
|---|---|---|---|
| `composition-root.test.ts` | 9 | `toHaveProperty('archAnalyzer')` | Does not verify value is functional |
| `composition-root.test.ts` | 19 | `typeof ... === 'function'` | Does not verify function produces correct results |
| `arch-analyzer.test.ts` | 109 | `toBeGreaterThan(0)` | Does not verify violation correctness |
| `arch-analyzer.test.ts` | 123 | `toBeGreaterThan(0)` | Does not verify violation correctness |
| `arch-analyzer.test.ts` | 211 | `toBeLessThan(100)` | Accepts score of 99 (nearly perfect) for a project with violations |
| `self-analysis.test.ts` | 31 | `toBeGreaterThan(5)` | Magic number; should assert specific known exports |

---

## 8. Summary and Prioritized Recommendations

### Critical (must fix, 8-10)

1. **Add scaffold-service tests** (9/10): This service generates entire project structures. Bugs silently produce broken scaffolds that users cannot run. Test `analyzeRuntime` for each language, `generateScripts` output, and `scaffold` file writing.

2. **Add notification-orchestrator tests** (8/10): 600 lines of untested state machine logic including stall detection, convergence analysis, and rate limiting. Test `handleEvent` state transitions, `checkForStalls` timer behavior, `shouldEmit` rate limiting, and `checkConvergence` escalation.

3. **Add ruflo-adapter unit tests** (8/10): The `extractJson` method, `toSwarmStatus` field mapping with fallback chains, and `mcpExec` error wrapping are all untested. Mock `execFile` and test parsing edge cases.

4. **Fix hardcoded paths in integration/E2E tests** (8/10): Replace `/Volumes/ExtendedStorage/PARA/01-Projects/hex` with `import.meta.dir` or `process.cwd()`. These tests cannot run on CI or any other developer's machine.

5. **Add property tests for QualityScore** (9/10): The current tests encode exact formula weights. Add property tests: score always in [0,100], monotonically increases with better inputs, `compileSuccess=false` always yields 0.

### Important (should fix, 5-7)

6. **Add property tests for TaskGraph.topologicalSort** (7/10): Verify ordering invariant (dependencies before dependents) and completeness (all steps present in output) for arbitrary graphs.

7. **Test code-generator edge cases** (7/10): Empty spec, malformed LLM response, `stripCodeFences` and `toFileName` as isolated functions.

8. **Strengthen arch-analyzer assertions** (6/10): Replace `toBeGreaterThan(0)` with specific violation checks (exact `fromLayer`, `toLayer`, `rule` values).

9. **Add status-formatter tests** (6/10): `formatCompact` JSON mode, `buildJsonPayload` structure, `progressBar` edge cases (0%, 100%).

10. **Test domain error hierarchy** (5/10): Verify `BoundaryViolation` stores `fromLayer`/`toLayer` correctly, `ValidationError` stores `field`, all extend `DomainError`.

11. **Add mcp-adapter routing tests** (7/10): `handleToolCall` dispatch logic should be tested with each tool name.

### Test quality improvements (3-4)

12. **Add negative assertions to code-generator tests** (4/10): Verify prompts do NOT contain irrelevant content.

13. **Test TaskGraph with circular dependencies** (4/10): Verify `topologicalSort` behavior is defined (throws or returns partial order).

14. **Test FeedbackLoop with large iteration counts** (3/10): Memory/performance sanity check.

---

## 9. Positive Observations

- **filesystem-adapter.test.ts**: Excellent. Tests real I/O with temp directories, covers path traversal security (4 attack vectors), cleans up after itself.
- **layer-classifier.test.ts**: Comprehensive coverage of all layer combinations with `isAllowedImport`. Tests both allowed and forbidden directions.
- **import-boundary-checker.test.ts**: Good behavioral testing with `validatePlannedImports` covering multi-violation scenarios and edge cases like unknown layers.
- **task-graph.test.ts**: Tests diamond dependencies and reverse-order insertion for topological sort. Strong behavioral focus.
- **feedback-loop.test.ts**: Good factory-based fixtures (`makeQualityScore`, `makeFeedbackIteration`) that make tests readable and maintainable.
- **self-analysis.test.ts (E2E)**: Despite machine-specificity, this is a valuable "eats its own dog food" test that verifies hex can analyze itself.
- **todo-app tests**: The example todo-app has excellent London-school mock tests demonstrating the framework's testing philosophy. Good edge case coverage (empty title, not found, filter combinations).
- **Test fixtures**: The shared `fixtures.ts` file with factory functions and sensible defaults follows best practices for test data management.

---

## 10. Overall Verdict

**Test suite maturity: 4/10**

The tested modules are tested well (good mocking, clear assertions, behavioral focus), but coverage is dangerously sparse. Over half the codebase -- including critical paths like scaffold generation, notification orchestration, all secondary adapters, and the MCP adapter -- has zero tests. The integration tests are machine-specific and would fail in CI. There are no property tests in the core framework despite several modules (QualityScore, TaskGraph, path-normalizer) being ideal candidates.

The CLAUDE.md warning about test-code mirroring is partially realized: the QualityScore tests encode exact formula weights rather than behavioral properties. However, the architecture-related tests (layer-classifier, import-boundary-checker) demonstrate good behavioral testing that would catch real regressions.

**Priority action**: Add tests for scaffold-service, notification-orchestrator, and ruflo-adapter. Fix hardcoded paths. Add property tests for QualityScore and TaskGraph.
