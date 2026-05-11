# ADR-2605091430: Workspace Boundary Enforcement

**Status:** Proposed  
**Date:** 2026-05-09

## Context

ADR-008 established hexagonal architecture principles within each crate (domain → ports → adapters). However, **cross-crate dependencies** in the hex workspace lack explicit rules and enforcement, creating architectural drift risks:

1. **`hex-cli` can silently import `hex-nexus`** — violating the intent that CLI is a thin primary adapter, not an orchestration client.
2. **`hex-analyzer` could import orchestration logic** — conflating static analysis (tool) with dynamic coordination (runtime).
3. **No CI gate detects workspace-boundary violations** — the only signal is post-merge code review or runtime coupling.
4. **ADR-008's enforcement was intra-crate only** — CI lint rule `no-cross-boundary-imports` checked `src/core/domain/ → src/adapters/` but not `hex-cli → hex-nexus`.

The workspace has 4 principal crates with different architectural roles:

- **`hex-core`**: Shared domain types, port traits, value objects — the gravity center. Zero runtime orchestration.
- **`hex-cli`**: Primary adapter (CLI commands) — drives local actions, invokes `hex-nexus` API over HTTP, but **must not link `hex-nexus` as a Rust dependency**.
- **`hex-nexus`**: Orchestration nexus (library + binary) — HTTP server, agent management, swarm coordination, SpacetimeDB client. May depend on `hex-core` for shared types.
- **`hex-analyzer`**: Standalone architectural-health tool (orphan port detection, god-type scan, cohesion metrics). **Must only depend on `hex-core`** for shared types — no orchestration, no CLI, no nexus runtime.

Current `Cargo.toml` state (grounded 2026-05-09):
- `hex-cli/Cargo.toml`: depends on `hex-core` ✓ (no `hex-nexus` ✓, no `hex-analyzer`)
- `hex-nexus/Cargo.toml`: depends on `hex-core` ✓ (no `hex-cli`, no `hex-analyzer`)
- `hex-analyzer/Cargo.toml`: **no workspace dependencies** ❌ — should declare `hex-core` for shared types

**The risk**: without enforcement, a contributor adds `hex-nexus = { path = "../hex-nexus" }` to `hex-cli/Cargo.toml` to "just call one orchestration function directly" — collapsing the boundary.

## Decision

### 1. Workspace Dependency Rules (Canonical)

| Crate           | May Depend On          | Forbidden                                  | Rationale                                                                 |
|-----------------|------------------------|--------------------------------------------|---------------------------------------------------------------------------|
| `hex-core`      | (none — leaf)          | Any workspace crate                        | Shared domain/port layer; zero dependencies keeps it the gravity center.  |
| `hex-cli`       | `hex-core`             | `hex-nexus`, `hex-analyzer`, `hex-agent`   | CLI is a primary adapter; orchestration via HTTP API, not Rust link.      |
| `hex-nexus`     | `hex-core`             | `hex-cli`, `hex-analyzer`, `hex-agent`     | Nexus is orchestration runtime; CLI/analyzer are separate binaries.       |
| `hex-analyzer`  | `hex-core` (only)      | `hex-cli`, `hex-nexus`, `hex-agent`        | Standalone tool; static analysis only; no runtime orchestration coupling. |
| `hex-agent`     | `hex-core`             | `hex-cli`, `hex-nexus`, `hex-analyzer`     | Agent runtime is independent; coordinates via SpacetimeDB, not Rust link. |

**Exception**: integration-test crates under `tests/` or `examples/` may depend on multiple workspace crates for E2E scenarios.

### 2. Tool: `workspace_boundary_check.rs`

Implement `hex-nexus/src/tools/workspace_boundary_check.rs` (~200 lines) to enforce the above rules via static analysis:

#### 2.1 Algorithm

```
FOR each crate in workspace (walk Cargo.toml files):
  1. Parse [dependencies] section
  2. Extract `path = "../<crate>"` entries (workspace-internal deps)
  3. Build import graph: crate → [deps]
  4. FOR each (crate, dep) edge:
       IF dep is in workspace AND dep is forbidden by rule table:
         EMIT violation { crate, dep, rule, severity: error }
  5. Walk crate's src/**/*.rs files (ripgrep or walkdir):
       - Regex: `use\s+(hex_[a-z_]+)::`
       - Extract module prefix (e.g. `use hex_nexus::` → `hex-nexus` crate)
       - IF imported crate is forbidden AND not declared in Cargo.toml:
           EMIT violation { file, line, imported_crate, reason: "undeclared Cargo.toml dep" }
  6. Return violations[]
```

#### 2.2 Interface (JSON output for CI + hex CLI integration)

```rust
// hex-nexus/src/tools/workspace_boundary_check.rs

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize)]
pub struct BoundaryCheckResult {
    pub violations: Vec<BoundaryViolation>,
    pub scanned_crates: usize,
    pub scanned_files: usize,
}

#[derive(Debug, Serialize)]
pub struct BoundaryViolation {
    pub kind: ViolationKind,
    pub crate_name: String,
    pub forbidden_dep: String,
    pub file: Option<PathBuf>,
    pub line: Option<usize>,
    pub snippet: Option<String>,
    pub rule: String, // e.g. "hex-cli → hex-nexus FORBIDDEN"
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ViolationKind {
    CargoToml,       // Forbidden dep in Cargo.toml [dependencies]
    SourceImport,    // `use hex_nexus::` in source but Cargo.toml correct
    UndeclaredImport // `use hex_nexus::` but NOT in Cargo.toml (transitive leak)
}

pub fn check_workspace_boundaries(
    workspace_root: &Path,
) -> anyhow::Result<BoundaryCheckResult> {
    // 1. Discover workspace crates via workspace Cargo.toml [workspace.members]
    // 2. For each crate: parse Cargo.toml, extract [dependencies]
    // 3. Build adjacency map: crate_name → Vec<dep_name>
    // 4. Walk src/**/*.rs per crate, regex `use hex_[a-z_]+::`
    // 5. Cross-reference against RULE_TABLE (const)
    // 6. Emit violations
    todo!()
}

const RULE_TABLE: &[(&str, &[&str])] = &[
    ("hex-core", &[]), // no workspace deps allowed
    ("hex-cli", &["hex-core"]),
    ("hex-nexus", &["hex-core"]),
    ("hex-analyzer", &["hex-core"]),
    ("hex-agent", &["hex-core"]),
];
```

#### 2.3 Wiring

1. **`cargo check` wrapper integration**  
   - `hex-nexus/src/tools/cargo_check.rs` (existing tool) already invokes `cargo check --workspace`.
   - Add `workspace_boundary_check::check_workspace_boundaries(repo_root)` call **before** `cargo check`.
   - If violations found, prepend to cargo-check JSON output as synthetic errors.
   - Result: `hex check` (CLI) or nexus `cargo_check` tool invocation surfaces boundary violations.

2. **`.git/hooks/pre-commit` integration**  
   - Document in ADR (do not auto-install; respect operator's hook strategy).
   - Suggested hook script:

   ```bash
   #!/usr/bin/env bash
   # .git/hooks/pre-commit — workspace boundary enforcement
   set -e
   hex analyze --workspace --fail-on-violations
   ```

   - `hex analyze --workspace --fail-on-violations` invokes `workspace_boundary_check`, exits 1 if violations found.
   - Operator installs manually or via `hex init --install-hooks`.

3. **CI integration** (GitHub Actions, ADR-2605020830 compliance)  
   - `.github/workflows/ci.yml`:
     ```yaml
     - name: Workspace boundary check
       run: hex analyze --workspace --fail-on-violations
     ```
   - Fails PR if boundary violations detected.

### 3. Immediate Remediation

- **`hex-analyzer/Cargo.toml`**: Add `hex-core = { path = "../hex-core" }` to `[dependencies]` (it currently has zero workspace deps — likely an oversight).
- No other crates violate the rules as of 2026-05-09 (grounded via Cargo.toml reads).

## Consequences

### Positive

- **Architectural drift prevention**: Workspace boundaries are now explicit, machine-checked, and CI-enforced.
- **ADR-008 extension**: Hexagonal architecture enforcement scales from intra-crate (ports/adapters) to inter-crate (CLI/nexus/analyzer separation).
- **Self-documenting**: `RULE_TABLE` in `workspace_boundary_check.rs` is the canonical source of truth.
- **Fast feedback**: Pre-commit hook catches violations locally before CI or review.
- **Composable**: `workspace_boundary_check` is a typed hex tool (callable by nexus, CLI, or agent).

### Negative

- **Maintenance burden**: Rule table must be updated when new workspace crates are added (mitigated by making violations explicit).
- **False positives risk**: Conditional compilation (`#[cfg(test)]` imports) might trigger false violations — tool must skip `dev-dependencies` and `#[cfg(test)]` modules.
- **Bootstrap problem**: Cannot enforce until tool is implemented (P0 task below).

### Risks

- **Transitive dependency leaks**: `hex-cli` → `hex-core` → (if hex-core re-exports hex-nexus types) → implicit nexus coupling. Mitigation: `hex-core` has zero workspace deps (rule table enforces).

## References

- **ADR-008**: Dogfooding — hex Built with Hexagonal Architecture (intra-crate boundaries)
- **ADR-2605020830**: hex CI Enforcement — Done-Conditions, `hex ci`, and Deployed Workflows (CI integration)
- **Cargo.toml state** (grounded 2026-05-09):
  - `hex-cli/Cargo.toml`: `hex-core` only ✓
  - `hex-nexus/Cargo.toml`: `hex-core` only ✓
  - `hex-analyzer/Cargo.toml`: no workspace deps ❌ (should add `hex-core`)

## Implementation

**Phase P0** (blocking: tool scaffold):
1. Create `hex-nexus/src/tools/workspace_boundary_check.rs` (200 lines)
   - Implement `check_workspace_boundaries(workspace_root) → BoundaryCheckResult`
   - Parse workspace `Cargo.toml` → discover members
   - Parse per-crate `Cargo.toml` → extract `[dependencies]` with `path = "../*"`
   - Walk `src/**/*.rs` → regex `use hex_[a-z_]+::`
   - Cross-reference against `RULE_TABLE`
   - Return violations array (JSON-serializable)

2. Wire into `hex-nexus/src/tools/mod.rs`:
   ```rust
   pub mod workspace_boundary_check;
   tools.register(Box::new(WorkspaceBoundaryCheck));
   ```

3. Add `hex analyze --workspace --fail-on-violations` CLI command:
   - `hex-cli/src/commands/analyze.rs`:
     ```rust
     #[derive(Args)]
     pub struct AnalyzeArgs {
         #[arg(long)]
         workspace: bool,
         #[arg(long)]
         fail_on_violations: bool,
     }
     ```
   - Invoke nexus tool `workspace_boundary_check`, parse JSON, exit 1 if violations.

**Phase P1** (integration):
4. Update `hex-nexus/src/tools/cargo_check.rs`:
   - Prepend `workspace_boundary_check()` call before `cargo check --workspace`
   - Merge violations into cargo-check output as synthetic errors

5. Document `.git/hooks/pre-commit` in `docs/specs/pre-commit-hook.md`

**Phase P2** (CI):
6. Add GitHub Actions job `.github/workflows/workspace-boundary-check.yml`:
   ```yaml
   name: Workspace Boundary Check
   on: [pull_request]
   jobs:
     boundary:
       runs-on: ubuntu-latest
       steps:
         - uses: actions/checkout@v4
         - run: cargo build --bin hex
         - run: ./target/debug/hex analyze --workspace --fail-on-violations
   ```

7. Remediate `hex-analyzer/Cargo.toml`: add `hex-core = { path = "../hex-core" }`
