# ADR-058: Test Session Persistence and Outcome Tracking

**Status:** Proposed
**Date:** 2026-03-22
**Drivers:** No visibility into test health trends. Each `hex test all` run produces results that vanish after the terminal scrolls. We cannot answer: "Are tests getting more stable?", "Which tests flake most?", "Did this commit break something that was green yesterday?"

## Context

Today `hex test all` runs 5 test categories (unit, SpacetimeDB modules, lint, services, integration) across Rust and TypeScript. Results are printed to stdout and lost. This creates several problems:

- **No trend analysis** — We fixed 4 hex-nexus test failures today but had no record they were failing for weeks. The only signal was a human noticing "exit code 1".
- **No flake detection** — Integration tests that depend on SpacetimeDB being ready sometimes skip (HTTP 500) and sometimes pass. Without historical data, we cannot distinguish flaky tests from real failures.
- **No regression detection** — When a commit introduces a test failure, we discover it manually. There is no automated comparison against the previous run.
- **No cross-session correlation** — Multiple Claude Code sessions run tests concurrently. Results from different sessions are never aggregated.
- **Workplan completion relies on human judgment** — Closing workplans (e.g., "checkpoint-remaining") requires someone to verify tests pass. With stored outcomes, workplan gates could auto-close.

### Current test infrastructure

| Category | Runner | Count | Location |
|----------|--------|-------|----------|
| Rust unit | `cargo test -p {crate}` | ~230 | hex-core, hex-agent, hex-nexus |
| Dashboard | `vitest` | 8 | hex-nexus/assets/ |
| SpacetimeDB modules | `cargo test -p {module}` | 88 | spacetime-modules/ |
| Lint | `cargo clippy`, `bun run check` | — | workspace-wide |
| Integration | `hex test services/all` | ~12 | hex-cli/src/commands/test.rs |

### Alternatives considered

1. **CI-only tracking (GitHub Actions)** — Standard approach but hex is a local-first tool. Most test runs happen on developer machines, not CI. We'd miss 90% of signals.
2. **Plain log files** — Write test output to `~/.hex/test-runs/`. Simple but no structure for querying trends.
3. **SpacetimeDB tables** — Store structured results in the same database that tracks swarms and agents. Enables real-time dashboard integration and cross-session aggregation.

## Decision

We will persist test session outcomes in SpacetimeDB via a new `test-results` WASM module, with SQLite fallback for offline use.

### Data model

Each `hex test` invocation creates a **TestSession** containing one or more **TestResult** records:

```
TestSession {
    id: String (uuid),
    agent_id: String (from X-Hex-Agent-Id — ties to the agent that ran tests),
    commit_hash: String (git HEAD at time of run),
    branch: String,
    started_at: Timestamp,
    finished_at: Timestamp,
    trigger: String ("manual" | "hook" | "ci" | "workplan-gate"),
    overall_status: String ("pass" | "fail" | "partial"),
    pass_count: u32,
    fail_count: u32,
    skip_count: u32,
    total_count: u32,
    duration_ms: u64,
}

TestResult {
    id: String (uuid),
    session_id: String (FK → TestSession),
    category: String ("unit" | "integration" | "lint" | "dashboard" | "spacetimedb" | "arch"),
    name: String (test name or category label),
    status: String ("pass" | "fail" | "skip" | "error"),
    duration_ms: u64,
    error_message: Option<String> (first 500 chars of failure output),
    file_path: Option<String> (source file if known),
}
```

### Recording flow

1. `hex test` runs as today
2. After each test category completes, results are collected into `TestResult` structs
3. On completion, the full `TestSession` is written via:
   - Primary: `POST /api/test-sessions` → SpacetimeDB `test_session_record` reducer
   - Fallback: Append to `~/.hex/test-sessions/{date}.jsonl`
4. The agent guard applies — only registered agents can write test sessions

### Query surface

| Endpoint | Purpose |
|----------|---------|
| `GET /api/test-sessions` | List recent sessions (paginated) |
| `GET /api/test-sessions/:id` | Full session with all results |
| `GET /api/test-sessions/trends` | Pass rate over last N runs per category |
| `GET /api/test-sessions/flaky` | Tests that alternate pass/fail across recent sessions |
| `hex test history` | CLI view of recent test runs |
| `hex test trends` | CLI sparkline of pass rates per category |

### Flake detection algorithm

A test is marked **flaky** when it has both pass and fail outcomes within the last 10 sessions on the same branch, with no code changes to the test file (verified via `git log --follow`).

### Regression detection

After each test run, compare against the most recent session on the same branch:
- If a previously-passing test now fails → flag as **regression**
- Include the commit range between sessions for quick bisection
- Optionally emit a warning in the `SessionStart` hook banner

### Workplan gate integration

Workplan steps with `"done_condition": "bun test ... passes"` can be auto-verified:
- The workplan executor queries `/api/test-sessions/trends` for the specific test
- If last N runs all pass → mark step as done
- If flaky → flag for human review

## Consequences

**Positive:**
- Test health becomes visible and measurable over time
- Flaky tests are surfaced automatically instead of silently ignored
- Regressions are detected within one test run of introduction
- Workplan gates can auto-close based on test evidence
- Dashboard gains a test health panel for project monitoring
- Cross-session aggregation reveals patterns no single run shows

**Negative:**
- Additional SpacetimeDB module to maintain
- Test runs become slightly slower (~50ms overhead for HTTP POST)
- Storage grows unbounded without pruning

**Mitigations:**
- Prune test sessions older than 30 days (configurable)
- HTTP POST is fire-and-forget (non-blocking)
- Module follows existing patterns (agent-registry, hexflo-coordination)

## Implementation

| Phase | Description | Status |
|-------|------------|--------|
| P1 | Create `test-results` SpacetimeDB module with tables and reducers | Pending |
| P2 | Add `TestSession` collection to `hex test` runner in test.rs | Pending |
| P3 | Add REST endpoints in hex-nexus for session recording and querying | Pending |
| P4 | Add `hex test history` and `hex test trends` CLI commands | Pending |
| P5 | Add flake detection and regression comparison | Pending |
| P6 | Wire workplan gate auto-verification | Pending |
| P7 | Add dashboard test health panel | Pending |

## References

- ADR-057: Unified Test Harness (test runner infrastructure)
- ADR-048: Claude Code Session Agent Registration (agent identity for test attribution)
- ADR-025: State Management (SpacetimeDB primary, SQLite fallback pattern)
- ADR-027: HexFlo Coordination (same pattern for SpacetimeDB modules)
