# ADR Gap Analysis

**Date**: 2026-03-17
**ADRs Analyzed**: 21 (ADR-001 through ADR-021)
**Ports Analyzed**: 35 interfaces
**Adapters Analyzed**: 56 implementations

---

## Executive Summary

**Status**: 🟡 **Moderate Gaps** — 9 proposed ADRs need status updates, 7 architectural areas lack ADRs

### Key Findings

- ✅ **No abandoned ADRs** — All 21 ADRs have recent commit activity
- 🟡 **9 proposed ADRs** are fully implemented but not marked "accepted"
- 🔴 **7 missing ADRs** for implemented features (error handling, testing strategy, logging, etc.)
- ✅ **Strong coverage** for core architecture (hexagonal, tree-sitter, multi-language, swarm)

---

## 1. Proposed ADRs That Should Be Accepted

These ADRs have status "proposed" but are **fully implemented** in the codebase:

| ADR | Title | Evidence | Recommendation |
|-----|-------|----------|----------------|
| **ADR-011** | Coordination Multi-Instance | `CoordinationAdapter` (343 lines), `ICoordinationPort` used in 8 files | **Accept** — Implemented and working |
| **ADR-012** | ADR Lifecycle Tracking | `ADRAdapter` (212 lines), `hex adr` commands functional | **Accept** — Implemented and working |
| **ADR-013** | Secrets Management | 4 adapters: Infisical, LocalVault, Env, Caching (569 total lines) | **Accept** — Implemented and working |
| **ADR-014** | No mock.module | All tests use DI, mock.module banned per commit e933f19 | **Accept** — Enforced in codebase |
| **ADR-015** | Hub SQLite Persistence | Implemented in hex-hub/src/persistence.rs (commit 0049209) | **Accept** — Implemented and working |
| **ADR-016** | Hub Binary Version Verification | `--build-hash` CLI flag, version checks in hub launcher | **Accept** — Implemented and working |
| **ADR-020** | Feature Progress UX | `IFeatureProgressPort`, `FeatureProgressDisplay` (commits 875cb4e, 94bb2da, bec3dad) | **Accept** — Implemented and working |
| **ADR-021** | Init Memory Exhaustion | Streaming walker with ignore engine (commit ad4ce74, just added today) | **Accept** — Implemented and working |

**Proposed → Accepted count**: 8 ADRs

---

## 2. Missing ADRs (Implemented Features Without Documentation)

### 🔴 High Priority (User-Facing / Architectural)

#### ADR-022: Error Handling Strategy
**What exists**:
- Custom error types: `PathTraversalError`, `WASMModuleNotFoundError`
- Ports return `Promise<T>` (no explicit Result type)
- CLI exits with non-zero codes
- No standardized error codes or error hierarchy

**Why it needs an ADR**:
- Inconsistent error handling across adapters
- No guidance on when to throw vs. return error states
- No error telemetry/logging strategy

**Decision to make**:
- Use Result<T, E> pattern (like Rust) or stick with throws?
- Error codes for machine-readable failures?
- Error serialization for MCP/CLI boundaries?

---

#### ADR-023: Testing Strategy and Coverage Targets
**What exists**:
- 3 test levels mentioned in README: unit, property, smoke
- Tests in `tests/unit/` and `tests/integration/`
- No formal coverage targets
- Property tests exist but no documented invariants

**Why it needs an ADR**:
- No guidance on when to write each test type
- No coverage thresholds (line/branch/mutation)
- No guidance on test naming, structure, or organization
- Property testing strategy undocumented

**Decision to make**:
- Coverage targets (e.g., 80% line, 70% branch)
- Property test invariants for each domain entity
- Test file naming conventions (`.test.ts` vs `.spec.ts`)
- Integration test scope (real adapters vs. stubs)

---

#### ADR-024: Logging and Observability Strategy
**What exists**:
- Terminal output via `TerminalNotifier`
- File logging via `FileLogNotifier` (`.hex/activity.log`)
- No structured logging library (no winston/pino)
- No log levels enforced
- No distributed tracing

**Why it needs an ADR**:
- Developers don't know what to log or at what level
- No guidance on PII redaction or secret masking
- No correlation IDs for multi-agent workflows
- No integration with external observability tools (Datadog, Honeycomb, etc.)

**Decision to make**:
- Structured logging library (winston? pino? console.log?)
- Log levels: DEBUG, INFO, WARN, ERROR, FATAL
- Correlation ID propagation for swarm agents
- PII redaction rules
- Log retention policy

---

### 🟡 Medium Priority (Developer Experience)

#### ADR-025: Dependency Management and Vendoring
**What exists**:
- `bun install` for TypeScript deps
- `cargo build` for Rust (hex-hub)
- `go mod` for Go (examples/weather)
- No vendoring strategy
- No lockfile diff policy

**Why it needs an ADR**:
- No guidance on when to update dependencies
- No security scanning policy (dependabot, snyk?)
- No guidance on major version updates
- Rust/Go deps in examples not enforced in CI

**Decision to make**:
- Dependency update cadence (monthly? quarterly?)
- Lockfile commit policy (always commit? only for releases?)
- Security scanning automation
- Vendoring policy (commit node_modules? cargo vendor?)

---

#### ADR-026: Performance Benchmarking and Profiling
**What exists**:
- README claims "5-10x faster" for native tree-sitter (ADR-010)
- No benchmark suite
- No performance regression tests
- No profiling tooling documented

**Why it needs an ADR**:
- Performance claims not validated in CI
- No way to detect regressions
- No guidance on profiling tools (flamegraph, clinic.js, etc.)
- No performance targets (e.g., "summarize 1000 files in <5s")

**Decision to make**:
- Benchmark suite location (`benches/` directory?)
- CI performance tests (run on every PR? weekly?)
- Profiling tools (node --inspect? perf? cargo flamegraph?)
- Performance targets per operation

---

#### ADR-027: Deployment and Release Process
**What exists**:
- npm package `@anthropic-hex/hex`
- Prebuilt binaries for hex-hub (shipped in npm)
- No documented release process
- No semantic versioning policy
- No changelog automation

**Why it needs an ADR**:
- Contributors don't know how to cut a release
- No guidance on version bumping
- No changelog format (keep-a-changelog? conventional-commits?)
- No beta/canary release strategy

**Decision to make**:
- Semantic versioning policy (breaking changes = major bump)
- Release process (manual? automated via GitHub Actions?)
- Changelog automation (conventional-commits? release-please?)
- Pre-release channels (beta, canary, next)

---

### 🟢 Low Priority (Implementation Details)

#### ADR-028: Unused Port Cleanup Strategy
**What exists**:
- `IVaultManagementPort` defined but never implemented (hex analyze reports it as unused)
- `IComparisonPort` has only one adapter (dual swarm comparator)
- `IADRQueryPort` separate from `IADRPort` (may be over-abstraction)

**Why it might need an ADR**:
- Unused ports create confusion
- Decision needed: delete them or keep as future extension points?

**Decision to make**:
- Delete unused ports or keep with "// FUTURE" comments?
- Port deprecation policy (mark deprecated for 1 release, then delete?)

---

## 3. ADR-Code Drift (Accepted ADRs Without Evidence)

**Status**: ✅ **No drift detected**

All accepted ADRs have corresponding implementations:
- ADR-001: Hexagonal architecture enforced by `hex analyze`
- ADR-002: Tree-sitter adapters exist (`TreeSitterAdapter`, `NativeTreeSitterAdapter`)
- ADR-003: Multi-language support in `config/languages/`
- ADR-004: Worktree management in `WorktreeAdapter`
- ADR-005: Quality gates in `ValidationAdapter`
- ADR-006: Skills/agents shipped in npm package
- ADR-007: Notification system implemented (4 adapters)
- ADR-008: Dogfooding (project uses hex architecture)
- ADR-009: Ruflo integration via `RufloAdapter`
- ADR-017: macOS inode workaround in `cli-adapter.ts`
- ADR-018: Multi-language build enforcement in `BuildAdapter`
- ADR-019: CLI-MCP parity verified (60 MCP tools)

---

## 4. Cross-Reference: CLAUDE.md Rules vs. ADRs

### Behavioral Rules in CLAUDE.md

| Rule | ADR Coverage | Status |
|------|--------------|--------|
| "ALWAYS read a file before editing it" | No ADR | 🟡 Implicit best practice, no ADR needed |
| "ALWAYS run `bun test` after changes" | ADR-005 (quality gates) | ✅ Covered |
| "ALWAYS run `bun run build` before commit" | ADR-005 (quality gates) | ✅ Covered |
| "NEVER use `mock.module()` in tests" | ADR-014 | ✅ Covered |
| "Adapters NEVER import other adapters" | ADR-001 (hexagonal) | ✅ Covered |
| "All imports MUST use `.js` extensions" | No ADR | 🟡 Build config, no ADR needed |
| "FileSystemAdapter has path traversal protection" | No ADR | 🔴 **Missing: ADR-022 (error handling / security)** |
| "RufloAdapter uses `execFile` not `exec`" | No ADR | 🔴 **Missing: ADR-022 (security patterns)** |
| "Primary adapters MUST NOT use innerHTML" | No ADR | 🔴 **Missing: ADR-029 (XSS prevention)** |
| "Single composition root" | ADR-001 (hexagonal) | ✅ Covered |
| "London-school testing" | ADR-014 (no mock.module) | ✅ Covered |

**Missing ADR**: Security best practices (XSS, command injection, path traversal)

---

## 5. Prioritized Recommendations

### Immediate Actions (This Week)

1. **Update ADR status to "accepted"** for 8 implemented ADRs:
   ```bash
   # Update frontmatter in these files:
   docs/adrs/ADR-011-coordination-multi-instance.md
   docs/adrs/ADR-012-adr-lifecycle-tracking.md
   docs/adrs/ADR-013-secrets-management.md
   docs/adrs/ADR-014-no-mock-module-di-deps.md
   docs/adrs/ADR-015-hub-sqlite-persistence.md
   docs/adrs/ADR-016-hub-binary-version-verification.md
   docs/adrs/ADR-020-feature-ux-improvement.md
   docs/adrs/ADR-021-init-memory-exhaustion.md
   ```

2. **Write ADR-022: Error Handling Strategy** (1 hour)
   - Decide: throw vs. Result type
   - Define error codes
   - Document error serialization for boundaries

3. **Write ADR-023: Testing Strategy** (1 hour)
   - Set coverage targets (80% line, 70% branch)
   - Document when to use unit vs. integration vs. property tests
   - Define property test invariants

### Short-Term (Next Sprint)

4. **Write ADR-024: Logging and Observability** (2 hours)
   - Choose structured logging library (winston vs. pino)
   - Define log levels and correlation ID strategy
   - Document PII redaction rules

5. **Write ADR-029: Security Best Practices** (1 hour)
   - Consolidate: XSS prevention, command injection, path traversal
   - Document security review checklist
   - Link to OWASP top 10

### Medium-Term (Next Month)

6. **Write ADR-025: Dependency Management** (1 hour)
7. **Write ADR-026: Performance Benchmarking** (2 hours)
8. **Write ADR-027: Deployment Process** (1 hour)

### Low-Priority (Backlog)

9. **Write ADR-028: Unused Port Cleanup** (30 minutes)
10. **Delete `IVaultManagementPort`** if truly unused

---

## 6. Success Metrics

### Current State
- **Total ADRs**: 21
- **Accepted**: 9 (43%)
- **Proposed**: 12 (57%)
- **Abandoned**: 0 (0%)
- **Coverage**: Core architecture ✅, Operations 🔴

### Target State (After Recommendations)
- **Total ADRs**: 29
- **Accepted**: 25 (86%)
- **Proposed**: 4 (14%)
- **Coverage**: Core architecture ✅, Operations ✅

---

## 7. Gap Summary Table

| Category | Count | Status | Action |
|----------|-------|--------|--------|
| **Proposed → Accept** | 8 | 🟡 | Update frontmatter |
| **Missing ADRs (High Priority)** | 3 | 🔴 | Write ADR-022, 023, 024 |
| **Missing ADRs (Medium Priority)** | 3 | 🟡 | Write ADR-025, 026, 027 |
| **Missing ADRs (Low Priority)** | 1 | 🟢 | Write ADR-028, 029 |
| **Abandoned ADRs** | 0 | ✅ | None |
| **ADR-Code Drift** | 0 | ✅ | None |

**Total Gaps**: 15 (8 status updates + 7 missing ADRs)

---

## Conclusion

The hex project has **strong architectural ADR coverage** but **gaps in operational areas** (error handling, testing strategy, logging, deployment). The immediate priority is to:

1. Accept 8 proposed ADRs that are already implemented
2. Write ADR-022 (error handling) and ADR-023 (testing strategy)
3. Write ADR-029 (security best practices)

After these actions, ADR coverage will be comprehensive across both architecture and operations.

**Estimated Effort**: 8 hours (1 hour for status updates + 7 hours for new ADRs)
