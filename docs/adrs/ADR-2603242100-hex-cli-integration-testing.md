# ADR-2603242100: Comprehensive hex-cli Integration Testing

**Status:** Proposed
**Date:** 2026-03-24
**Drivers:** hex-cli has 30 command modules and zero integration tests. CLI output contracts, exit codes, and command routing can regress silently — no CI gate catches broken commands before they ship.

## Context

hex-cli is the canonical user entry point for the entire hex system. All 30 command modules (`adr`, `agent`, `analyze`, `dev`, `enforce`, `git_cmd`, `hook`, `inbox`, `inference`, `init`, `interview`, `mcp`, `memory`, `neural_lab`, `nexus`, `opencode`, `plan`, `project`, `readme`, `report`, `secrets`, `skill`, `status`, `stdb`, `swarm`, `task`, `test`, `assets_cmd`, `adr_review`, `plan`) are exercised only by hand today.

Current gaps:
- **No regression safety**: a refactor in `hex-nexus` REST routes or `hex-cli` argument parsing can silently break any command
- **No output contract validation**: table formatting (ADR-2603241226), JSON mode, and error messages are untested
- **No exit code verification**: `hex analyze` on a clean repo vs. a violating repo — no test confirms the correct exit code
- **No CI enforcement**: nothing blocks a merge that breaks `hex swarm status` or `hex adr list`

### What was considered

| Approach | Verdict |
|----------|---------|
| Unit tests per command module | Too isolated — misses CLI argument parsing, routing, and output formatting |
| `assert_cmd` integration tests (Rust) | **Chosen** — spawns the real binary, captures stdout/stderr/exit code |
| Python/shell CLI smoke tests | Fragile, not composable with `cargo test`, adds language dependency |
| End-to-end tests against live nexus | Valuable but slow; separate tier from command-level tests |

### Dependency: nexus mock mode

Most commands delegate to hex-nexus REST endpoints. Tests must not require a live nexus daemon. Two strategies exist:

1. **`--dry-run` / `--offline` flag**: CLI short-circuits before HTTP calls, returns canned responses — fast but tests less
2. **Mock nexus server** (`wiremock` or `httpmock`): starts an in-process HTTP stub that responds to the exact routes the CLI calls — tests the full HTTP path including serialization/deserialization

We will use **httpmock** (in-process, no external process spawning) for commands that call nexus, and direct binary invocation for commands that are purely local (`hex adr list`, `hex analyze .`, `hex assets`).

## Decision

We will create a `hex-cli/tests/` integration test suite using `assert_cmd` + `httpmock` with the following structure:

### Test Tiers

**Tier 0 — Local commands (no nexus required)**
Tests the CLI binary directly. No network. Fast.
- `hex adr list` — exit 0, table output contains expected columns
- `hex adr search <query>` — filters correctly, empty result exits 0
- `hex adr status <id>` — known ADR returns detail, unknown returns exit 1
- `hex analyze .` — clean repo exits 0, known violation exits 1
- `hex assets list` — lists embedded asset categories
- `hex --version` — prints semver
- `hex --help` — each subcommand appears in help text

**Tier 1 — Nexus-backed commands (httpmock)**
Spins an in-process stub on a random port. Passes `HEX_NEXUS_URL=http://127.0.0.1:<port>` to the CLI process.
- `hex status` — stub returns mock project/agent data; output matches expected format
- `hex swarm init <name>` — stub accepts POST, returns swarm ID; CLI prints confirmation
- `hex swarm status` — stub returns active swarms; table rendered correctly
- `hex task create <swarm-id> <title>` — stub accepts POST; CLI exits 0
- `hex task list` — stub returns task list; table rendered correctly
- `hex agent list` — stub returns agent list; columns present
- `hex memory store <key> <val>` — stub accepts POST; CLI exits 0
- `hex memory get <key>` — stub returns value; CLI prints it
- `hex nexus status` — stub at `/health`; CLI reports connected
- `hex inbox list` — stub returns notifications; table rendered
- `hex inference list` — stub returns providers; table rendered
- `hex plan list` — stub returns workplans; table rendered

**Tier 2 — Error path contracts**
- Commands that require nexus return a clear error (not panic) when nexus is unreachable
- Unknown subcommands print help and exit 2 (not 0, not 1)
- Missing required arguments print usage and exit 2
- `hex adr status NONEXISTENT` exits 1 with human-readable error

**Tier 3 — Output format contracts**
- `--json` flag (where supported) produces valid JSON parseable by `serde_json`
- Table output (default) contains expected column headers
- Colors are suppressed when `NO_COLOR=1` is set

### Test Infrastructure

```
hex-cli/
  tests/
    common/
      mod.rs          # Shared: binary path resolution, httpmock factory, env helpers
      nexus_stub.rs   # Pre-built stubs for each nexus route (returns realistic fixtures)
      fixtures/       # JSON fixture files (swarm list, task list, agent list, etc.)
    tier0_local.rs    # Tier 0 tests
    tier1_nexus.rs    # Tier 1 tests
    tier2_errors.rs   # Tier 2 tests
    tier3_format.rs   # Tier 3 tests
```

### Cargo dependencies (dev only)

```toml
[dev-dependencies]
assert_cmd = "2"       # Spawn binary, assert stdout/stderr/exit code
httpmock = "0.7"       # In-process HTTP stub server
predicates = "3"       # assert_cmd output matchers (contains, matches regex)
serde_json = "1"       # Parse --json output
tempfile = "3"         # Isolated config dirs per test
```

### Environment isolation

Each test that touches filesystem state (init, config) must use `tempfile::TempDir` and set `HEX_HOME` to the temp path. This prevents test cross-contamination and allows parallel execution.

### CI gate

`cargo test -p hex-cli --test '*'` runs all integration tiers. This is added to the CI pipeline as a required check before merge. Tier 0 and Tier 1 must pass; Tier 2 and 3 are required but can be introduced incrementally.

### Fixture maintenance

Nexus response fixtures live in `hex-cli/tests/common/fixtures/*.json`. When the nexus REST API changes (new fields, renamed routes), fixture files are the single update point — not scattered across test files. A compile-time check (`include_str!` + `serde_json::from_str` in a `#[test]`) ensures fixtures remain valid JSON.

## Consequences

**Positive:**
- CLI regressions caught before merge, not after user reports
- Output contracts are explicit and verifiable — ADR-2603241226 (`tabled` formatting) is now testable
- New command authors have a pattern to follow immediately
- Parallel test execution (httpmock is in-process, random ports prevent collisions)

**Negative:**
- Binary must be compiled before tests run — adds to CI time
- Fixtures drift from real nexus responses if not maintained
- Commands that do complex TUI output (ratatui, ADR-2603241500) are harder to assert on

**Mitigations:**
- Use `cargo test --test '*'` which triggers compilation automatically
- Fixture validation test catches JSON drift at compile time
- TUI commands tested only for exit code and non-interactive flag behavior

## Implementation

| Phase | Description | Status |
|-------|-------------|--------|
| P1 | Add dev dependencies, `tests/common/` scaffolding, fixture files | Pending |
| P2 | Tier 0: local command tests (adr, analyze, assets, version, help) | Pending |
| P3 | Tier 1: nexus-backed command tests with httpmock stubs | Pending |
| P4 | Tier 2 + 3: error paths and output format contracts | Pending |
| P5 | Wire `cargo test -p hex-cli --test '*'` into CI as required gate | Pending |

## References

- ADR-057: Unified Test Harness & Linting Pipeline (TypeScript side — this ADR covers the Rust CLI side)
- ADR-2603241226: Structured CLI Table Output (`tabled`) — contracts this testing ADR validates
- ADR-2603231900: Fix `hex test all` False Skips
- ADR-019: CLI–MCP Parity (MCP tools and CLI commands share backend — CLI tests also validate MCP contract indirectly)
- [assert_cmd crate](https://crates.io/crates/assert_cmd)
- [httpmock crate](https://crates.io/crates/httpmock)
