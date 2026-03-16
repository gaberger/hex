# Validation Verdict Report

**Date:** 2025-03-15
**Problem Statement:** hex — a hexagonal architecture framework providing token-efficient AST summaries, architecture analysis, code generation, and swarm coordination for LLM-driven development.
**Verdict: PASS (84/100)**

---

## Score Breakdown

| Category | Score | Weight | Weighted |
|----------|-------|--------|----------|
| Behavioral Specs | 90 | 40% | 36.0 |
| Property Tests | 70 | 20% | 14.0 |
| Smoke Tests | 90 | 25% | 22.5 |
| Sign Conventions | 75 | 15% | 11.25 |
| **Total** | | | **83.75 → 84** |

---

## 1. Behavioral Specs (90/100)

### Tested Behaviors — 13 test files, 194 total tests, 0 failures

| Behavior | Test File | Status |
|----------|-----------|--------|
| Architecture analysis (dead exports, hex boundaries, circular deps) | `arch-analyzer.test.ts` | PASS (6 tests) |
| Layer classification + import rules | `layer-classifier.test.ts` | PASS (18 tests) |
| Path normalization for imports | `path-normalizer.test.ts` | PASS (11 tests) |
| Code generation from spec via LLM | `code-generator.test.ts` | PASS (6 tests) |
| Workplan creation + execution with swarm | `workplan-executor.test.ts` | PASS (5 tests) |
| AST summary delegation | `summary-service.test.ts` | PASS (4 tests) |
| Filesystem with path traversal protection | `filesystem-adapter.test.ts` | PASS (10 tests) |
| CLI commands: help, unknown, errors | `cli-adapter.test.ts` | PASS (3 tests) |
| FeedbackLoop iteration + convergence | `feedback-loop.test.ts` | PASS (9 tests) |
| QualityScore computation + passing gate | `quality-score.test.ts` | PASS (11 tests) |
| TaskGraph dependency ordering | `task-graph.test.ts` | PASS (8 tests) |
| Composition root wiring | `composition-root.test.ts` | PASS (3 integration) |
| Self-analysis E2E | `self-analysis.test.ts` | PASS (7 E2E) |

### Untested Behaviors (-10)

| Behavior | Reason |
|----------|--------|
| Dashboard adapter HTTP routes | No test for `DashboardAdapter` — serves HTML + JSON API + SSE |
| MCP adapter tool routing | No test for `MCPAdapter` — maps tool calls to use cases |
| Notification orchestrator filtering | `NotificationQueryAdapter` tested but orchestrator event routing not fully covered |
| Scaffold service project generation | `ScaffoldService` implements `IScaffoldPort` but no dedicated test file |

---

## 2. Property Tests (70/100)

### Existing Property Tests

Property tests exist **only for the Flappy Bird example**, not for hex core:

| Property | File | Status |
|----------|------|--------|
| `applyFlap` always produces negative velocity | `physics-properties.test.ts` | PASS |
| `applyGravity` always increases velocity | `physics-properties.test.ts` | PASS |
| Gravity eventually overcomes flap | `physics-properties.test.ts` | PASS |
| `checkBounds` safe in valid play area | `physics-properties.test.ts` | PASS |
| Position changes match velocity sign | `physics-properties.test.ts` | PASS |

### Missing Property Tests for hex Core (-30)

| Property | Why It Matters |
|----------|----------------|
| `classifyLayer` is total (never returns `undefined` for valid paths) | Layer classification is the foundation of hex validation |
| `resolveImportPath` round-trips correctly | Path resolution errors cause false positive boundary violations |
| `QualityScore.score()` is bounded [0, 100] | Already tested but not as a property over random inputs |
| `TaskGraph.topologicalSort` is deterministic | Sort stability matters for reproducible workplans |
| `ArchAnalyzer.findDeadExports` is idempotent | Running twice should produce identical results |

---

## 3. Smoke Tests (90/100)

| Smoke Test | Result |
|------------|--------|
| `hex help` runs without error | PASS |
| `hex analyze src` produces health score | PASS (70/100) |
| `bun run build` compiles to dist/ | PASS |
| `bun run check` (tsc --noEmit) passes | PASS (after treesitter-adapter fix) |
| `bun test` — 194 tests, 0 failures | PASS |
| `createAppContext` returns valid context | PASS (integration test) |
| E2E self-analysis with real tree-sitter | PASS |

### Minor Gap (-10)

| Gap | Impact |
|-----|--------|
| No smoke test for `hex init` generating a real project on disk | Low — scaffold service is tested in isolation but not E2E |
| No smoke test for `hex dashboard` starting HTTP server | Medium — dashboard has no tests at all |

---

## 4. Sign Convention Audit (75/100)

### Consistent Patterns

| Convention | Status |
|------------|--------|
| All port methods return `Promise<T>` (async throughout) | CONSISTENT |
| Ports use `I` prefix (`IASTPort`, `ILLMPort`, etc.) | CONSISTENT |
| All 14 adapters use `implements IXxxPort` | CONSISTENT |
| Domain types are pure interfaces (no classes except `QualityScore`, `FeedbackLoop`, `TaskGraph`) | CONSISTENT |
| Error types extend `Error` with descriptive names (`GitError`, `SwarmConnectionError`, `SwarmParseError`) | CONSISTENT |
| `.js` extensions in all relative imports | CONSISTENT |

### Inconsistencies (-25)

| Issue | Files | Impact |
|-------|-------|--------|
| `AppContext` uses `null` for optional ports (`llm`, `codeGenerator`, `workplanExecutor`) but `eventBus` always receives a stub | Inconsistent null vs stub pattern | Low — but callers must handle both patterns |
| `NotificationOrchestrator` implements `INotificationQueryPort` but is a use case, not an adapter | Blurs the layer responsibility | Medium — query adapters should wrap use cases, not be use cases |
| `AppContext` has index signature `[key: string]: unknown` | Breaks type safety — any key returns `unknown` | Medium — circumvents the closed interface guarantee |
| `cli-adapter.ts` re-declares `AppContext` as a `Pick<>` type alias (line 28, now dead) | Leftover from pre-port-extraction | Low — dead code, should be removed |

---

## Recommendations

1. **Add property tests for core hex** — The example apps have property tests but the framework itself does not. Priority: `classifyLayer`, `resolveImportPath`, `QualityScore`.

2. **Add dashboard + MCP adapter tests** — These two primary adapters have zero test coverage. Even basic "responds to GET /api/health" would catch regressions.

3. **Remove `[key: string]: unknown` from AppContext** — This index signature undermines the type safety that the port extraction was designed to provide.

4. **Standardize null vs stub** — Pick one pattern: either all optional ports are `null` (caller checks) or all get a no-op stub (caller doesn't check). Currently mixed.

5. **Clean up dead `AppContext` type alias** in `cli-adapter.ts` (line 28) — vestige of the old composition-root import.
