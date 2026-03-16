# Adversarial Test Report — hex-intf

**Date**: 2025-03-15
**Suite**: bun test v1.3.10
**Result**: 172 pass, 0 fail, 1651 expect() calls across 18 files (4.38s)

---

## 1. Overall Verdict

All 172 tests pass. The E2E suite (10 tests, 23 assertions) exercises the real stack against the live source tree and is **genuinely strong**. However, several unit tests are mock-plumbing tests that could pass even with broken production code, and the dead-export analyzer has a confirmed false-positive problem.

---

## 2. Test-by-Test Analysis

### E2E (tests/e2e/self-analysis.test.ts) — STRONG

| Test | Real Behavior? | Verdict |
|------|---------------|---------|
| tree-sitter creates real AppContext | YES — asserts >5 exports, >100 lines | GOOD |
| extracts imports from real files | YES — checks arch-analyzer.ts imports | GOOD |
| L1 < L3 token ratio < 30% | YES — verified 12.6% ratio | GOOD |
| L0 fewest tokens | YES — checks exports/imports empty | GOOD |
| analyzeArchitecture file count | YES — checks >15 files, >20 exports | GOOD |
| zero hex violations | YES — real boundary check | GOOD |
| composition-root only cross-boundary | YES — graph traversal | GOOD |
| CLI analyze structured output | YES — checks non-zero data | GOOD |
| CLI summarize real exports | YES — checks QualityScore, FeedbackLoop, TaskGraph | GOOD |
| CLI exit code 1 on low health | PARTIAL — uses mock analyzer | ACCEPTABLE |

**No false greens found in E2E.**

### Unit Tests — MIXED

#### False-Green Risk: HIGH

| Test File | Problem |
|-----------|---------|
| `summary-service.test.ts` | 4 tests only verify delegation: mock AST returns canned data, test checks it arrived. Would pass if SummaryService returned garbage with correct shape. Missing: no test that level parameter is actually forwarded to AST. |
| `code-generator.test.ts` | 6 tests verify prompt construction and passthrough. No test verifies the generated code compiles, has correct structure, or handles LLM returning invalid code. |
| `workplan-executor.test.ts` | Tests verify swarm task creation and ordering but mock LLM returns pre-baked JSON. No test for malformed LLM response (missing steps, invalid JSON). |
| `cli-adapter.test.ts` | Tests use fully mocked context. The "analyze" test only checks output contains "85/100" — does not verify the formatting of dead exports, orphan files, etc. |

#### False-Green Risk: LOW

| Test File | Assessment |
|-----------|-----------|
| `arch-analyzer.test.ts` | Tests real ArchAnalyzer logic with controlled inputs. Good cycle detection, boundary validation. |
| `layer-classifier.test.ts` | Pure function tests. No mocks needed. Solid. |
| `path-normalizer.test.ts` | Pure function tests. Solid. |
| `quality-score.test.ts` | Pure entity tests. Solid. |
| `feedback-loop.test.ts` | Pure entity tests. Solid. |
| `task-graph.test.ts` | Pure entity tests including diamond deps. Solid. |
| `filesystem-adapter.test.ts` | Tests real filesystem ops including path traversal blocking. Solid. |

### Integration (tests/integration/composition-root.test.ts) — WEAK

3 tests only verify property existence and `typeof === 'function'`. No test calls any method with real data. The E2E suite covers this gap, but the integration test itself is near-useless.

---

## 3. CLI Verification

| Command | Status | Notes |
|---------|--------|-------|
| `hex-intf analyze .` | PASS | 39 files, 175 exports, health 70/100 |
| `hex-intf summarize ... --level L1` | PASS | 134 tokens, 10 exports listed |
| `hex-intf summarize ... --level L3` | PASS | 1064 tokens, full AST |
| `hex-intf help` | PASS | All 10 commands listed |

---

## 4. Stress Test Results

| Test | Result | Assessment |
|------|--------|------------|
| L1/L3 ratio | 12.6% | GOOD — meaningful compression |
| Dead exports | 87 | SUSPICIOUS — see below |
| Unused ports | 12 of ~15 | EXPECTED — many ports have no adapter yet |
| Health score | 70 | PLAUSIBLE |
| Violations | 0 | GOOD — project follows its own rules |
| Circular deps | 0 | GOOD |

### BUG: Dead Export False Positives

`runCLI` and `startDashboard` are flagged as "dead exports" despite being the primary entry points consumed by `src/cli.ts`. The dead-export analyzer only scans `src/**/*.ts` via `fs.glob` but `src/cli.ts` (the entry point) imports `runCLI`. The analyzer likely does not trace imports from files that are themselves entry points (consumed externally or via `bun run`).

**Impact**: Health score is penalized (70 instead of higher). The E2E test "analyzeArchitecture returns correct file count" passes because it only checks `healthScore > 0`, not a specific value. This is a real bug that tests do not catch.

---

## 5. Dashboard Verification

| Endpoint | Status | Notes |
|----------|--------|-------|
| `/api/health` | PASS | Returns full summary JSON |
| `/api/tokens/overview` | PASS | 62 files (includes test files — may want to filter) |
| `/api/swarm` | PASS | Returns idle status, empty tasks/agents |
| `/api/graph` | PASS | 46 nodes, 81 edges |
| `/` (HTML) | PASS | 35KB, includes `<script>` tag |

**Observation**: `/api/tokens/overview` returns 62 files while `analyze` reports 39. The token endpoint likely includes test files and examples. This inconsistency is not tested anywhere.

---

## 6. Missing Tests — Concrete Recommendations

### Priority 1 (False-Green Fixes)

1. **`summary-service.test.ts`**: Add test that verifies `summarizeFile('x', 'L2')` actually passes `'L2'` to the AST port (currently the mock ignores the level parameter).
2. **`code-generator.test.ts`**: Add test for LLM returning empty string, malformed code, or code with markdown fences (` ```ts ... ``` `).
3. **`workplan-executor.test.ts`**: Add test for LLM returning invalid JSON, empty steps array, or steps with circular dependencies.
4. **`composition-root.test.ts`**: Either delete (redundant with E2E) or add a test that calls `ctx.ast.extractSummary` on a real file and checks the result.

### Priority 2 (Missing Coverage)

5. **Dead export false-positive bug**: Add E2E test that `runCLI` is NOT in the dead exports list.
6. **Token endpoint vs analyze file count discrepancy**: Add test verifying consistency.
7. **CLI `generate` and `plan` commands**: Zero test coverage for these commands.
8. **CLI `dashboard` command**: No test for startup/shutdown lifecycle.
9. **CLI `status` command**: No test coverage.
10. **CLI `init` command**: No test coverage.

### Priority 3 (Hardening)

11. **Concurrent analysis**: No test runs `analyzeArchitecture` on two paths simultaneously.
12. **Large file handling**: No test for files >10,000 lines.
13. **Non-TypeScript files**: No test for `.js`, `.go`, `.rs` file handling.
14. **Dashboard error responses**: No test for what happens when `/api/health` throws.
15. **Graph endpoint with circular deps**: No test verifying the graph API reflects cycles.
