# ADR-2604171000 — Cross-Adapter Rejection Regression Test

**Status:** Proposed
**Date:** 2026-04-17
**Drivers:** docs/EVIDENCE.md commit `ac4e5aa` — the README claims "cross-layer imports block the commit" but no automated test proves the rejection path; only the pass path is covered.
**Related:** ADR-001 (hexagonal architecture), ADR-2603283000 (Rust workspace boundary analysis), ADR-2604142200-reconcile-evidence-verification (same pattern: test the rejection, not just the accept)

## Context

`docs/EVIDENCE.md` (committed in this branch) maps every mechanical claim in the README to a reproducer. Five of the six claims have automated test coverage. The sixth — "a commit introducing a cross-adapter import fails the analyzer and is rejected by the pre-commit hook" — is documented only as a three-step manual procedure:

```bash
cd examples/hex-weather
echo 'use crate::adapters::other_adapter::*;' >> src/adapters/mod.rs
hex analyze .                       # expected: non-zero exit, violation printed
git checkout src/adapters/mod.rs
```

The 12 hermetic tests in `hex-cli/src/commands/analyze.rs` cover layer classification, import extraction, cycle detection, and score computation — but all of them exercise the **happy path** (well-formed inputs, correct classification). There is no test that asserts the analyzer:

1. Returns a non-zero exit code when given a tree containing a cross-adapter import.
2. Emits a diagnostic naming the offending file and line.
3. Returns zero again when the import is removed.

This is the same failure mode ADR-2604142200 caught for the reconciler (asserting a task is **not** promoted is harder — and more important — than asserting it **is**). Without an explicit rejection test, a silent refactor of the analyzer that flips a comparison, swaps an `||` for `&&`, or loosens a glob can pass all 12 existing tests while eliminating the very guarantee the README leads with.

The rejection path is the entire value proposition of `hex analyze`. It must be regression-tested.

## Decision

Add a hermetic regression test that constructs a tiny on-disk hex project with a known cross-adapter import and asserts the analyzer rejects it. Wire the test into the default CI workflow.

### Test shape

- New integration test: `hex-cli/tests/analyze_rejects_violation.rs`.
- Uses `tempfile::tempdir()` to build a minimal layout:
  ```
  <tmp>/
    src/
      domain/mod.rs        (empty)
      ports/mod.rs         (imports from domain only)
      adapters/
        primary/cli.rs     (imports from ports; no cross-adapter use)
        secondary/db.rs    (imports from ports; no cross-adapter use)
      main.rs              (composition root)
    Cargo.toml
  ```
- Three assertions per test case:

  | Case | Mutation | Assertion |
  |---|---|---|
  | `baseline_passes` | none | `hex analyze <tmp>` exit 0, output contains `0 boundary violations` |
  | `cross_adapter_import_rejected` | append `use crate::adapters::secondary::db::*;` to `primary/cli.rs` | exit non-zero, stderr mentions `primary/cli.rs` and `adapters/secondary` |
  | `ports_imports_adapter_rejected` | append `use crate::adapters::secondary::db::*;` to `ports/mod.rs` | exit non-zero, stderr mentions `ports/mod.rs` and `adapters/secondary` |
  | `domain_imports_adapter_rejected` | append `use crate::adapters::secondary::db::*;` to `domain/mod.rs` | exit non-zero, stderr mentions `domain/mod.rs` |
  | `baseline_passes_after_revert` | apply then revert | final run exit 0 |

- Each case re-runs `baseline_passes` first to catch fixture corruption.

### CI wiring

- Added to the existing `cargo test --workspace` invocation in `.github/workflows/ci.yml` (no new job).
- Test is hermetic: no network, no Ollama, no SpacetimeDB, no nexus daemon. Runs in <5s on the reference CI runner.
- The `hex` binary is resolved via the same helper used by `reconcile_evidence.rs` (`target/debug/hex` → `target/release/hex` → `which hex`).

### EVIDENCE.md update

Replace the "simulate an agent introducing a cross-adapter import" manual block in §1 with:

```bash
cargo test -p hex-cli --test analyze_rejects_violation
```

Keep the manual sequence below it as a human-readable illustration, but the authoritative reproducer becomes the automated test.

## Consequences

**Positive:**
- Closes the last gap in EVIDENCE.md: every mechanical claim in the README now has an automated reproducer.
- Catches silent analyzer regressions that the existing 12 happy-path tests cannot (flipped boolean, loosened glob, dropped layer rule).
- Gives the pre-commit hook a tested contract — if the hook's invocation of `hex analyze` ever stops exit-code-checking, this test still asserts the underlying binary rejects correctly.
- Provides the shape for future boundary rules (circular dep detection, dead-export detection) — they get the same three-assertion structure.

**Negative:**
- Adds ~5s to CI wall-clock time.
- Creates a second fixture maintenance burden (alongside `hex-cli/tests/fixtures/reconcile/`). When the analyzer's layer-classification heuristics change, the fixture's file layout may need to track them.
- Test relies on substring matching in stderr (`primary/cli.rs`, `adapters/secondary`). If the analyzer reformats output, the assertion breaks even though behavior is correct.

**Mitigations:**
- Run the test from a `tempdir` rather than against `examples/hex-weather/` so the example project stays free to evolve.
- Match on stable tokens (file stem + layer name) rather than whole error strings. Add a `--format json` path to `hex analyze` as a follow-up ADR if stderr matching becomes brittle.
- When layer classification changes, update the fixture in the same PR — the test enforces the coupling rather than hiding it.

## Implementation

| Phase | Description | Status |
|-------|------------|--------|
| P1 | Create `hex-cli/tests/fixtures/boundary-violation/` with the 5 source files and a `Cargo.toml` stub | Pending |
| P2 | Write `hex-cli/tests/analyze_rejects_violation.rs` with the 5 test cases above; reuse `hex_bin()` helper pattern from `reconcile_evidence.rs` | Pending |
| P3 | Verify `cargo test -p hex-cli --test analyze_rejects_violation` passes locally (4 violation cases reject, 1 baseline case passes) | Pending |
| P4 | Confirm CI picks it up automatically via the existing workspace test step; no workflow changes expected | Pending |
| P5 | Update `docs/EVIDENCE.md` §1 to cite the automated test as the primary reproducer | Pending |
| P6 | Enqueue follow-up: `hex analyze --format json` for stable machine-readable output (deferred to its own ADR) | Pending |

## References

- docs/EVIDENCE.md §1 (Tree-sitter hexagonal boundary analyzer) — the gap this ADR closes
- hex-cli/src/commands/analyze.rs — analyzer source; inline tests at the bottom cover the accept path
- hex-cli/tests/reconcile_evidence.rs — reference shape for hermetic CLI-driven regression tests
- ADR-2604142200-reconcile-evidence-verification — same principle applied to the reconciler: test the rejection, not just the accept
- ADR-001-hexagonal-architecture — the rules this analyzer enforces
- ADR-2603283000-rust-workspace-boundary-analysis — Rust-side layer classification this test will exercise
