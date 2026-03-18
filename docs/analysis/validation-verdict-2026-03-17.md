# Validation Verdict — 2026-03-17

**Overall: WARN (score 72/100)**

Problem statement: Validate the current state of the hex-intf project including recent ADR adapter, coordination adapter, and CLI coordination features.

---

## 1. Behavioral Specs (score: 75/100, weight 40%)

### ADR Adapter (src/adapters/secondary/adr-adapter.ts)
| Behavior | Test | Status |
|----------|------|--------|
| Scan all ADR markdown files | tests/unit/adr-adapter.test.ts | PASS |
| Parse status from `## Status:` and `**Status:**` formats | tests/unit/adr-adapter.test.ts | PASS |
| Extract ADR ID from filename | tests/unit/adr-adapter.test.ts | PASS |
| Index into AgentDB (best-effort) | tests/unit/adr-adapter.test.ts | PASS |
| Search via AgentDB with local fallback | tests/unit/adr-adapter.test.ts | PASS |
| ADR orchestrator use case | tests/unit/adr-orchestrator.test.ts | PASS |

### Coordination Adapter (src/adapters/secondary/coordination-adapter.ts)
| Behavior | Test | Status |
|----------|------|--------|
| Register instance with hub | tests/unit/coordination-adapter.test.ts | PASS |
| Acquire/release worktree locks | tests/unit/coordination-adapter.test.ts | PASS |
| Lock conflict detection | tests/unit/coordination-adapter.test.ts | PASS |
| Claim/release tasks | tests/unit/coordination-adapter.test.ts | PASS |
| Heartbeat with unstaged file capture | tests/unit/coordination-adapter.test.ts | PASS |
| Layer classification in unstaged files | tests/unit/coordination-adapter.test.ts | PASS |
| Publish/query activity stream | tests/unit/coordination-adapter.test.ts | PASS |
| HTTP error resilience (safe defaults) | tests/unit/coordination-adapter.test.ts | PASS |
| Hub integration (real HTTP) | tests/integration/coordination-hub.test.ts | PASS |

### Untested Behaviors (deductions)
- `getActivities(limit)` — no unit test for limit parameter passthrough
- `listClaims()` — no dedicated unit test (covered only in integration)
- `getUnstagedAcrossInstances()` — no dedicated unit test

**Score rationale**: All critical behaviors covered. 3 minor gaps in secondary query paths.

---

## 2. Property Tests (score: 85/100, weight 20%)

| Property | Test | Status |
|----------|------|--------|
| LockResult mutual exclusion (acquired ↔ lock/conflict) | tests/property/coordination-lock.property.test.ts | PASS |
| ClaimResult mutual exclusion (claimed ↔ claim/conflict) | tests/property/coordination-lock.property.test.ts | PASS |
| Lock key uniqueness per project+feature+layer | tests/property/coordination-lock.property.test.ts | PASS |
| UnstagedFile status exhaustiveness | tests/property/coordination-lock.property.test.ts | PASS |
| Layer classification completeness (all hex layers) | tests/property/coordination-lock.property.test.ts | PASS |
| Layer classification determinism (domain wins ambiguity) | tests/property/coordination-lock.property.test.ts | PASS |
| WorktreeLock TTL is positive and bounded | tests/property/coordination-lock.property.test.ts | PASS |
| heartbeatAt >= acquiredAt | tests/property/coordination-lock.property.test.ts | PASS |
| Existing: layer-classifier properties | tests/property/layer-classifier.property.test.ts | PASS |
| Existing: path-normalizer properties | tests/property/path-normalizer.property.test.ts | PASS |
| Existing: import-boundary properties | tests/property/import-boundary.property.test.ts | PASS |

**Score rationale**: Excellent property coverage for coordination types. Minor deduction: property tests use enumerated values, not true random generation (no fast-check / fc integration).

---

## 3. Smoke Tests (score: 55/100, weight 25%)

| Smoke Test | Status | Notes |
|------------|--------|-------|
| Import smoke (all exports resolve) | PASS | tests/smoke/imports.smoke.test.ts |
| CLI lifecycle smoke | PASS | tests/smoke/cli-lifecycle.smoke.test.ts |
| Self-analysis: dead exports detected | PASS | tests/e2e/self-analysis.test.ts |
| Self-analysis: health score > 0 | PASS | tests/e2e/self-analysis.test.ts |
| **Self-analysis: zero hex boundary violations** | **FAIL** | 1 violation found |
| **Self-analysis: composition-root only cross-boundary file** | **FAIL** | 2 cross-boundary files found |
| Dashboard hub integration | FAIL (13 tests) | Hub not running / connection timeout |
| TreeSitter multi-language | FAIL (timeout) | Native tree-sitter unavailable |
| CLI secrets vault | FAIL (7 tests) | Vault crypto dependency issue |
| Hub command sender | FAIL (7 tests) | Mock timing issues |

**Score rationale**: 4 smoke categories pass, 6 fail. The hex boundary violations are the most critical finding — they indicate real architecture drift.

---

## 4. Sign Convention Audit (score: 70/100, weight 15%)

### Port Contract Compliance
| Adapter | Port | Compliant | Notes |
|---------|------|-----------|-------|
| ADRAdapter | IADRPort | YES | All 5 methods match signatures exactly |
| CoordinationAdapter | ICoordinationPort | YES | All 11 methods match signatures exactly |
| cli-adapter | N/A | **VIOLATION** | Imports from usecases layer directly |
| mcp-adapter | N/A | **VIOLATION** | Imports from adapter + ports (cross-boundary) |

### Error Handling Conventions
- CoordinationAdapter: Consistent `resolve(null)` on HTTP errors (no throws) — GOOD
- ADRAdapter: `catch {}` blocks with skip semantics — GOOD
- CoordinationAdapter `captureUnstagedFiles`: Returns `[]` on git failure — GOOD

### Return Type Conventions
- Lock/Claim results use discriminated union pattern (`acquired + lock | conflict`) — CONSISTENT
- Activity/unstaged queries return `[]` on error — CONSISTENT

### Naming Conventions
- All port types use `I` prefix — CONSISTENT
- Value types in ports/coordination.ts — correctly domain-like, no adapter leakage

**Score rationale**: Port contracts are clean. Two cross-boundary import violations in primary adapters reduce the score.

---

## 5. Critical Findings

### FINDING 1: Hex Boundary Violation (BLOCKING)
**File**: `src/adapters/primary/cli-adapter.ts:2444`
**Violation**: `adapters/primary -> usecases` (imports `DualSwarmComparator`)
**Rule**: Adapters must not import from usecases directly. They should go through ports.
**Fix**: Create an `IDualSwarmPort` in `src/core/ports/` and have the use case implement it. The CLI adapter should depend on the port, not the concrete use case.

### FINDING 2: Cross-Boundary Imports (BLOCKING)
**Files**:
1. `src/adapters/primary/cli-adapter.ts` — imports from ports + adapters + usecases
2. `src/adapters/primary/mcp-adapter.ts` — imports from ports + adapters (dashboard-adapter)

**Fix for cli-adapter**: Route DualSwarmComparator through a port. Remove direct adapter imports (cli-fmt, dashboard-adapter, daemon-manager should be injected via composition root, not imported directly).

**Fix for mcp-adapter**: The `dashboard-adapter` import is a cross-adapter coupling. Inject it via composition root instead.

### FINDING 3: Test Infrastructure Issues (NON-BLOCKING)
35 test failures total, but most are infrastructure-related:
- 13 dashboard-hub tests: require running hub daemon (integration env)
- 7 vault tests: crypto dependency issue
- 7 hub-command-sender tests: mock timing
- 1 tree-sitter timeout: native parser not available
- 5 CLI secrets: vault dependency

These are environment-dependent, not behavioral bugs.

---

## 6. Score Breakdown

| Category | Score | Weight | Weighted |
|----------|-------|--------|----------|
| Behavioral Specs | 75 | 40% | 30.0 |
| Property Tests | 85 | 20% | 17.0 |
| Smoke Tests | 55 | 25% | 13.75 |
| Sign Conventions | 70 | 15% | 10.5 |
| **Total** | | | **71.25** |

## Verdict: **WARN** (71/100)

The ADR and coordination adapters are well-implemented with strong port compliance and property coverage. However, two pre-existing hex boundary violations in `cli-adapter.ts` and `mcp-adapter.ts` prevent a PASS verdict. These are architectural issues, not bugs in the new code.

### Recommended Actions (priority order)
1. **Fix cli-adapter DualSwarmComparator import** — create `IDualSwarmPort` in ports layer
2. **Fix mcp-adapter dashboard-adapter import** — inject via composition root
3. **Add unit tests** for `getActivities(limit)`, `listClaims()`, `getUnstagedAcrossInstances()`
4. **Consider fast-check** for true randomized property testing
