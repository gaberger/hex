# ADR-2603283000: Rust Workspace Boundary Analysis in hex analyze

**Status:** Accepted
**Date:** 2026-03-28
**Drivers:** `hex analyze` reports A+/100 while ignoring 87% of the codebase — all Rust files are invisible to layer counting and boundary violation detection
**Supersedes:** ADR-034 (partially — replaces its incomplete implementation claim)

## Context

`hex analyze .` on the hex-intf project reports:

```
Hex layers:
  ✓ Domain        (12 files)
  ✓ Ports         (31 files)
  ✓ Use Cases     (5 files)
  ✓ Primary Adapters (8 files)
  ✓ Secondary Adapters (14 files)
452 source files scanned — Score: A+ / 100
```

The 12 domain files, 31 ports files, etc. are the TypeScript `src/core/domain/`, `src/core/ports/` directories.
The 452-file count includes Rust files across hex-core, hex-cli, hex-nexus, hex-agent — but those files are **only counted, never boundary-checked**.

### What is broken

`hex-cli/src/commands/analyze.rs` hard-codes TypeScript path conventions:

```rust
const LAYER_DIRS: &[(&str, &str, Layer)] = &[
    ("core/domain", "Domain", Layer::Domain),
    ("core/ports", "Ports", Layer::Ports),
    ...
];
// Always joins with root.join("src").join(dir)
```

`scan_local_violations()` (line 263) only scans `root/src/` for TypeScript `import` statements.

### The Rust workspace reality

| Crate | hex role | Files |
|-------|----------|-------|
| `hex-core/src/domain/` | Domain layer | ~20 files |
| `hex-core/src/ports/` | Ports layer | ~8 files |
| `hex-nexus/src/adapters/` | Secondary adapters | ~14 files |
| `hex-nexus/src/orchestration/` | Use cases | ~6 files |
| `hex-nexus/src/routes/` | Primary adapters (REST) | ~8 files |
| `hex-cli/src/commands/` | Primary adapters (CLI) | ~15 files |
| `hex-agent/src/adapters/primary/` | Primary adapters | ~3 files |

Total: ~299 Rust files. These are invisible to the health score.

### ADR-034 claim vs reality

ADR-034 (Accepted) states: "Rust analyzer implemented in `hex-nexus/src/analysis/`."
The `hex-nexus/src/analysis/` modules (`layer_classifier.rs`, `boundary_checker.rs`) exist and correctly classify Rust files by path convention — **but `hex analyze` never calls them for workspace-level layer reporting**. It only calls the nexus analysis API when nexus is running, and even then only for tree-sitter violations, not for layer counting.

### Forces

- hex is predominantly a Rust project (87% of source files)
- The health score is used to gate PRs and validate features — a score based on 13% of files is meaningless
- `hex-nexus/src/analysis/layer_classifier.rs` already knows Rust crate-path conventions
- The fix must work offline (without nexus running) since `hex analyze` is used in CI

## Decision

Extend `hex analyze` to detect and report **Rust workspace layers** alongside TypeScript layers.

### Layer mapping for Rust workspaces

When `Cargo.toml` is present at the root, `hex analyze` shall:

1. Detect hex crate roles by matching well-known path patterns:
   - `<crate>/src/domain/` → Domain
   - `<crate>/src/ports/` → Ports
   - `<crate>/src/adapters/primary/` or `<crate>/src/commands/` → Primary Adapters
   - `<crate>/src/adapters/secondary/` or `<crate>/src/adapters/` → Secondary Adapters
   - `<crate>/src/orchestration/` or `<crate>/src/usecases/` → Use Cases
   - `<crate>/src/routes/` → Primary Adapters (HTTP-facing)

2. Count files per layer across all workspace crates.

3. Run a lightweight boundary scan on Rust `use` statements:
   - Secondary adapters must not `use` other secondary adapters directly (only via ports)
   - Primary adapters must not `use` secondary adapters directly
   - Domain must not `use` from `adapters::` or external crate paths outside `std`

4. Report both TypeScript and Rust layer counts when both are present (hybrid project).

5. The health score denominator includes all source files (TypeScript + Rust), not just TypeScript.

### Out of scope

- Full tree-sitter AST analysis for Rust (remains a nexus-only feature)
- Cross-language boundary checking (TypeScript adapter importing Rust via FFI — not applicable here)
- Refactoring `hex-nexus/src/analysis/` API — reuse `layer_classifier.rs` logic, don't replace it

### Implementation approach

Add a `rust_workspace` module to `hex-cli/src/commands/analyze.rs` (or a sibling file) that:
- Reads `Cargo.toml` workspace members
- For each member crate, walks the `src/` subtree
- Classifies each file's layer using the path pattern table above
- Returns `Vec<LayerCount>` parallel to the TypeScript layer counts

Offline boundary scan: grep `use ` statements in each file, classify source and target layers, flag cross-layer violations using the same `hex_core::rules::boundary` types already used for TypeScript.

## Consequences

**Positive:**
- `hex analyze` health score reflects the actual codebase — 452 files, real layer distribution
- Boundary violations in Rust code (e.g., a secondary adapter importing another adapter) are caught by `hex analyze`
- CI gates are meaningful for a Rust-first project
- Reuses `hex_core::rules::boundary` — no new type system

**Negative:**
- `LAYER_DIRS` path-convention heuristic can misclassify crates that don't follow hex naming (e.g., `hex-parser/`)
- Offline `use` statement grep is syntactically naive — `use` inside `#[cfg(test)]` blocks or macro expansions may produce false positives

**Mitigations:**
- Crates without any recognized layer directory are counted as "infrastructure" (not penalized, not boundary-checked)
- Test-only `use` statements are excluded by skipping files in `tests/` and lines after `#[cfg(test)]`
- A `--rust-workspace=off` flag allows opting out for non-hex Rust projects

## Implementation

| Phase | Description | Status |
|-------|------------|--------|
| P0 | Add `scan_rust_workspace_layers(root)` to `analyze.rs` — path-pattern layer detection, returns `Vec<(label, count)>` | Done |
| P1 | Integrate Rust layer counts into the "Hex layers" display block, alongside TypeScript counts | Done |
| P2 | Add offline Rust boundary scan — grep `use` statements, classify, flag violations | Done |
| P3 | Update health score denominator to include all Rust files | Done |
| P4 | Update `run_json()` output to include `rust_layers` field in structured output | Done |
| P5 | Add tests: layer detection for hex-core/hex-nexus/hex-cli structure, boundary violation detection | Done |

## References

- ADR-034: Migrate hex analyzer from TypeScript to Rust (Accepted — partially superseded by this ADR)
- `hex-nexus/src/analysis/layer_classifier.rs` — existing Rust layer classification logic
- `hex-cli/src/commands/analyze.rs:14` — `LAYER_DIRS` TypeScript-only constant
- `hex-cli/src/commands/analyze.rs:263` — `scan_local_violations()` TypeScript-only scan
