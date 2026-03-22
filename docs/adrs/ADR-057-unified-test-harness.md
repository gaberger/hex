# ADR-057: Unified Test Harness & Linting Pipeline

**Status:** Accepted
**Accepted Date:** 2026-03-22
## Date: 2026-03-22

- **Informed by**: ADR-005 (quality gates), ADR-014 (no mock.module, DI Deps), ADR-018 (multi-language build)
- **Authors**: Gary (architect), Claude (analysis)

## Context

hex is a polyglot project spanning 6 Rust crates, a TypeScript library, a Solid.js dashboard, and 18 SpacetimeDB WASM modules. Testing today is fragmented:

### Current State

| Component | Test runner | Invocation | Coverage |
|-----------|-----------|------------|----------|
| Rust crates (hex-core, hex-agent, hex-nexus) | `cargo test` | `hex test unit` | Unit tests, some lib tests |
| Rust CLI (hex-cli) | `cargo check` only | `hex test unit` | Compilation check, no unit tests |
| TypeScript library | `bun test` | Manual | Unit + property + smoke |
| Solid.js dashboard | None | N/A | Zero test coverage |
| SpacetimeDB WASM modules | `cargo test` (separate workspace) | `hex test unit` | 6 of 18 modules tested |
| Integration (API endpoints) | `hex test services` | Requires running nexus | HTTP smoke tests |
| Architecture | `hex analyze .` | `hex test arch` | Boundary, cycle, dead code |

### Problems

1. **No linting command**: `cargo clippy` and `bun run check` must be run manually — no unified `hex lint` or `hex test lint` level.

2. **No dashboard tests**: The Solid.js SPA (`hex-nexus/assets/`) has zero tests. Store logic, component rendering, and SpacetimeDB subscription handling are untested.

3. **12 of 18 WASM modules untested**: Only `file-lock-manager`, `architecture-enforcer`, `conflict-resolver`, `inference-gateway`, `hexflo-coordination`, and `secret-grant` have cargo tests. The remaining 12 modules have no test coverage.

4. **TypeScript tests disconnected**: `bun test` runs independently from `hex test`. The quality gate pipeline (ADR-005) references `IBuildPort.test()` but the Rust CLI doesn't invoke `bun test`.

5. **No E2E tests**: No browser-level testing of the dashboard. No full-cycle test that validates `hex init` → code → `hex analyze` → `hex validate` as a user would.

6. **Integration tests leak state**: `hex test all` creates swarms and memory entries in SpacetimeDB without cleanup. Test state accumulates across runs.

7. **hex-cli has no unit tests**: The CLI is only compilation-checked (`cargo check`), not tested. The new `readme`, `interview`, and `adr` modules have testable logic but no test harness.

## Decision

### 1. Test Pyramid — 5 Levels

```
Level 5: E2E         Browser smoke tests (Playwright)     — hex test e2e
Level 4: Integration  API + SpacetimeDB lifecycle tests    — hex test services
Level 3: Architecture Boundary, cycle, dead code analysis  — hex test arch
Level 2: Lint         clippy + tsc + biome                 — hex test lint
Level 1: Unit         cargo test + bun test                — hex test unit
```

Each level is independently runnable. `hex test all` runs levels 1-4. E2E (`hex test e2e`) is opt-in because it requires a browser runtime.

### 2. `hex test lint` — Unified Linting

New subcommand that runs all linters in sequence:

| Linter | Scope | Gate behavior |
|--------|-------|---------------|
| `cargo clippy --workspace -- -D warnings` | All Rust crates | Fail on any warning |
| `cargo clippy --workspace -- -D warnings` in `spacetime-modules/` | WASM modules | Fail on any warning |
| `bun run check` | TypeScript library | Fail on type errors |
| `biome check src/` (if installed) | TypeScript style | Warn only (non-blocking initially) |

### 3. Dashboard Test Harness

The Solid.js dashboard (`hex-nexus/assets/`) gets a test setup:

- **Runner**: Vitest (already compatible with Vite build pipeline)
- **Component tests**: `@solidjs/testing-library` for store logic and component rendering
- **Priority targets**: Stores first (they contain business logic), then complex components

```
hex-nexus/assets/
  src/
    stores/__tests__/        # Store unit tests
      toast.test.ts
      hexflo-monitor.test.ts
      connection.test.ts
    components/__tests__/    # Component tests (phase 2)
  vitest.config.ts
```

Integrated into `hex test unit` via:
```rust
fn run_dashboard_tests(r: &mut TestResults) {
    let ok = Command::new("npx")
        .args(["vitest", "run", "--reporter=verbose"])
        .current_dir("hex-nexus/assets")
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    r.check("Dashboard store tests pass", ok);
}
```

### 4. hex-cli Unit Tests

The CLI crate gets a `tests/` directory for testing pure logic:

| Module | Testable logic |
|--------|---------------|
| `readme.rs` | `parse_adr_summaries()`, `sync_adr_section()`, `generate_readme()` |
| `interview.rs` | `is_empty_project()` |
| `adr.rs` | `parse_adr_status()`, `extract_title()`, `parse_enforced_by()` |
| `hook.rs` | `is_destructive_command()`, session state serialization |

These are `#[cfg(test)] mod tests` blocks within each module, plus `hex-cli/tests/` for integration tests that need temp directories.

### 5. SpacetimeDB Module Test Strategy

WASM modules can't access the filesystem or network, but they can test:

- **Reducer logic**: Call reducers with test inputs, assert table state
- **Validation**: Test constraint enforcement (duplicate keys, invalid status transitions)
- **Query correctness**: Verify SQL queries return expected rows

For the 12 untested modules, prioritize by risk:

| Priority | Module | Why |
|----------|--------|-----|
| P0 | `agent-registry` | Agent lifecycle is mission-critical |
| P0 | `workplan-state` | Task status machine drives swarm execution |
| P1 | `chat-relay` | Message routing affects user experience |
| P1 | `inference-bridge` | Model routing affects all LLM calls |
| P2 | Remaining 8 | Lower risk — fleet-state, config, etc. |

### 6. Integration Test Cleanup

Integration tests must clean up after themselves:

```rust
// At end of run_integration_tests():
if let Some(ref id) = swarm_id {
    // Complete the test swarm so it doesn't pollute SpacetimeDB
    let _ = http
        .patch(format!("{}/api/swarms/{}", base, id))
        .send()
        .await;
}

// Clean up test memory entries
let _ = http
    .delete(format!("{}/api/hexflo/memory/hex-test-key", base))
    .send()
    .await;
```

### 7. `hex test` Subcommand Updates

```
hex test unit           # Rust + TS + Dashboard + SpacetimeDB module tests
hex test lint           # clippy + tsc + biome
hex test arch           # hex analyze . (boundaries, cycles, dead code)
hex test services       # API endpoint smoke tests (requires nexus)
hex test e2e            # Browser tests via agent-browser (opt-in)
hex test all            # unit + lint + arch + services
hex test full           # all + e2e
```

### 8. CI Pipeline Matrix

```yaml
# .github/workflows/test.yml
jobs:
  lint:
    runs-on: ubuntu-latest
    steps:
      - cargo clippy --workspace -- -D warnings
      - cargo clippy --workspace -- -D warnings  (spacetime-modules/)
      - bun run check

  unit:
    runs-on: ubuntu-latest
    steps:
      - cargo test --workspace
      - cargo test --workspace  (spacetime-modules/)
      - bun test
      - npx vitest run  (hex-nexus/assets/)

  integration:
    runs-on: ubuntu-latest
    needs: [lint, unit]
    services:
      spacetimedb:
        image: clockworklabs/spacetimedb:latest
    steps:
      - hex nexus start
      - hex test services
      - hex test arch

  e2e:
    runs-on: ubuntu-latest
    needs: [integration]
    steps:
      - hex nexus start
      - hex test e2e    # Uses agent-browser for AI-optimized snapshot testing
```

### E2E via agent-browser

E2E tests use `agent-browser` (Playwright-based, AI-optimized snapshots) rather than raw Playwright test scripts. This provides:

- **93% context reduction**: Accessibility tree snapshots with element refs (`@e1`, `@e2`) instead of full DOM
- **Agent-driven testing**: Hex agents can run E2E tests as part of swarm validation using the `/browser` skill
- **Snapshot assertions**: `agent-browser snapshot -i` captures interactive elements for state verification

```bash
# E2E test flow (invoked by hex test e2e)
agent-browser open http://127.0.0.1:5555        # Open dashboard
agent-browser snapshot -i                         # Capture interactive elements
agent-browser click @sidebar-projects             # Navigate
agent-browser snapshot -i                         # Verify state change
agent-browser screenshot tests/e2e/dashboard.png  # Visual evidence
agent-browser close
```

The `hex test e2e` subcommand orchestrates agent-browser commands against a running nexus instance, validating:
- Dashboard loads and renders project list
- Swarm monitor reflects real-time task state
- ADR browser lists all ADRs
- Agent fleet shows connected agents
- Chat interface sends and receives messages

### 9. Quality Gate Integration (ADR-005)

ADR-005's 6-gate pipeline maps to the test levels:

| ADR-005 Gate | hex test Level | Implementation |
|-------------|---------------|----------------|
| Gate 1: Compile | `hex test unit` | `cargo check`, `bun run check` |
| Gate 2: Lint | `hex test lint` | `cargo clippy`, `biome` |
| Gate 3: Unit Test | `hex test unit` | `cargo test`, `bun test`, `vitest` |
| Gate 4: Integration | `hex test services` | API + SpacetimeDB lifecycle |
| Gate 5: AST Diff | `hex test arch` | `hex analyze .` |
| Gate 6: Token Budget | Agent-only | HexFlo memory tracking |

## Implementation Plan

| Phase | Tasks | Effort |
|-------|-------|--------|
| 1 | Add `hex test lint` subcommand (clippy + tsc) | Small |
| 2 | Add `#[cfg(test)]` blocks to hex-cli modules (readme, adr, interview) | Small |
| 3 | Setup Vitest in `hex-nexus/assets/`, test critical stores | Medium |
| 4 | Integration test cleanup (swarm + memory teardown) | Small |
| 5 | Add tests for P0 SpacetimeDB modules (agent-registry, workplan-state) | Medium |
| 6 | E2E test setup with agent-browser + `/browser` skill | Medium |
| 7 | CI pipeline yaml | Medium |

## Consequences

### Positive
- Single entry point (`hex test`) for all test levels across all languages
- Linting is enforced, not optional — catches issues before agents waste tokens
- Dashboard gets test coverage for the first time
- Integration tests stop polluting SpacetimeDB state
- CI pipeline provides confidence for merges

### Negative
- Vitest adds a dev dependency to the dashboard
- E2E tests require agent-browser installed and a running nexus instance
- SpacetimeDB module tests require the separate workspace, complicating CI

### Mitigations
- E2E is opt-in (`hex test e2e`), not part of default `hex test all`
- CI caches Rust targets and node_modules across runs
- SpacetimeDB module workspace is handled as a separate CI job

## Related

- ADR-005: Quality Gates (agent feedback loop — maps to test levels)
- ADR-014: No mock.module, DI Deps pattern (test isolation strategy)
- ADR-018: Multi-Language Build Enforcement
- ADR-055: README-Driven Specification (interview.rs and readme.rs need unit tests)
