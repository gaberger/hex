# Production Readiness Verdict — hex-intf

**Date:** 2026-03-15
**Method:** 5-agent adversarial swarm evaluation
**Verdict: NOT READY for production** — 8 blockers must be resolved first

---

## Swarm Summary

| Agent | Focus | Findings |
|-------|-------|----------|
| Security Auditor | Injection, XSS, traversal, secrets | 2 HIGH, 2 MEDIUM |
| Architecture Reviewer | Hex boundary violations, coupling | 2 CRITICAL, 5 HIGH, 2 MEDIUM |
| Silent Failure Hunter | Error swallowing, races, leaks | 3 CRITICAL, 6 HIGH, 10 MEDIUM, 5 LOW |
| Test Quality Analyst | Coverage gaps, mirroring, assertions | 57% untested, maturity 4/10 |
| Type Design Analyzer | `any`, casts, null safety, API surface | 3 CRITICAL, ~30 unsafe casts |

**Totals: 8 CRITICAL, 15 HIGH, 16 MEDIUM, 5 LOW**

---

## Blockers (Must Fix Before Production)

### B1 — Registry TOCTOU Race Condition (Resilience C-01)
`registry-adapter.ts` reads then writes `registry.json` with no locking. Concurrent dashboard instances corrupt the file.
**Fix:** Use `flock`-style advisory locking or atomic write-rename.

### B2 — Dashboard Hub Accepts Arbitrary rootPath (Security VULN-03)
`POST /api/projects/register` accepts any filesystem path. An attacker can register `/etc` and read any `.ts` file.
**Fix:** Validate `rootPath` is an existing directory containing a `package.json` or `.hex-intf/` marker.

### B3 — Symlink Traversal in FileSystemAdapter (Security VULN-01)
`safePath()` does not resolve symlinks. An attacker can symlink past the root boundary.
**Fix:** Use `fs.realpath()` after path resolution, before the `startsWith` check.

### B4 — No Top-Level Error Boundary in CLI (Resilience C-02)
Unhandled rejections in `cli.ts` produce raw stack traces to stdout.
**Fix:** Wrap the CLI entry point in try/catch with user-friendly error formatting.

### B5 — Stub AST Produces Fake "Healthy" Results (Resilience H-06)
When tree-sitter fails to load, the stub returns 0 violations and 100% health score — actively misleading.
**Fix:** Stub must return an explicit error state or throw, never fake success.

### B6 — Unsafe `as` Casts on External Data (Type C-01/C-02/C-03)
LLM adapter, ruflo adapter, and MCP adapter all cast external JSON responses with `as T` and no runtime validation. API errors produce cryptic TypeErrors.
**Fix:** Add runtime validation (Zod schemas or manual checks) at every system boundary.

### B7 — CORS Origin Bypass (Security VULN-04)
`origin.startsWith('http://localhost')` passes `http://localhost.evil.com`.
**Fix:** Use exact match or a proper URL parser that checks hostname.

### B8 — Error Information Disclosure (Security VULN-02)
Dashboard error responses include internal filesystem paths and exception messages.
**Fix:** Return generic error messages; log details server-side only.

---

## High Priority (Should Fix)

| ID | Source | Issue | Fix |
|----|--------|-------|-----|
| H-01 | Resilience | Webhook notifier silently drops messages after retry exhaustion | Log + emit error event |
| H-02 | Resilience | Ruflo memory returns null on error — indistinguishable from empty | Return Result type or throw |
| H-03 | Architecture | `path-normalizer.ts` imports `node:path` — usecase purity violation | Inline the 2 string ops |
| H-04 | Resilience | LLM response parser uses blind `as` casts | Add runtime validation |
| H-05 | Architecture | `ruflo-adapter.ts` bare built-in specifiers | Add `node:` prefix |
| H-06 | Architecture | Tests import adapter implementations directly | Import through ports |
| H-07 | Resilience | BuildAdapter/GitAdapter/WorktreeAdapter have no exec timeouts | Add 30s timeout to all execFile calls |
| H-08 | Resilience | Dashboard `server.listen()` hangs on EADDRINUSE | Add error handler on listen |

---

## Systemic Issues

### 21 Bare `catch {}` Blocks
Eleven silently swallow errors with no logging. This is the single biggest resilience risk — failures are invisible.
**Recommendation:** Adopt a project-wide rule: every catch must either re-throw, log, or return an explicit error type. Add an ESLint rule (`no-empty` or `@typescript-eslint/no-empty-function`).

### 57% Source Files Have Zero Tests
Critical paths untested: `scaffold-service`, `notification-orchestrator`, all secondary adapters, MCP adapter.
**Recommendation:** Prioritize contract tests for secondary adapters and behavioral tests for usecases.

### 67 Type Assertions, ~30 on Untrusted Data
**Recommendation:** Add Zod or a validation layer at every external boundary (CLI args, HTTP requests, subprocess stdout, LLM responses).

### No Property Tests
QualityScore, TaskGraph, and path normalization are ideal candidates.
**Recommendation:** Add `fast-check` property tests for domain invariants.

---

## Scorecard

| Dimension | Score | Notes |
|-----------|-------|-------|
| Security | 4/10 | Path traversal, CORS bypass, info disclosure |
| Architecture | 7/10 | Clean hex structure with 2 entry-point violations |
| Error Handling | 3/10 | 21 silent catches, no error boundary, TOCTOU race |
| Test Coverage | 4/10 | 57% untested, weak assertions, hardcoded paths |
| Type Safety | 5/10 | Zero `any`, but 30 unsafe casts on external data |
| API Design | 6/10 | Clean ports, but no error contracts or branded types |
| **Overall** | **4.8/10** | **Not production-ready** |

---

## Recommended Fix Order

1. **B1-B3** (Security + Data integrity) — highest blast radius
2. **B4-B5** (User-facing error quality) — affects every user
3. **B6-B8** (Type safety + CORS) — external attack surface
4. **Systemic: empty catches** — add logging to all 21
5. **Systemic: test coverage** — contract tests for adapters
6. **Systemic: runtime validation** — Zod at boundaries

---

## Detailed Reports

- [Security Audit](adversarial-security-audit.md)
- [Architecture Audit](adversarial-architecture-audit.md)
- [Resilience Audit](adversarial-resilience-audit.md)
- [Test Coverage Audit](adversarial-test-audit.md)
- [Type Safety Audit](adversarial-type-audit.md)
