---
name: hex-adr-create
description: Create a new Architecture Decision Record with auto-numbering, dependency impact analysis, and validation gates. Use when the user asks to "create ADR", "write ADR", "new ADR", or "architecture decision".
trigger: /hex-adr-create
---

# Create New ADR

Creates an Architecture Decision Record with comprehensive impact analysis to prevent
downstream breakage. Every ADR that modifies, deletes, or restructures code artifacts
must include a full consumer dependency map before it can be accepted.

## Phase 1: Gather Intent

1. Ask the user for:
   - **Title** (required)
   - **Brief context description** — why this decision is needed
   - **Decision type**: one of `add | modify | delete | restructure | migrate`

2. Find the highest ADR number in `docs/adrs/`:
   ```bash
   ls docs/adrs/ADR-*.md docs/adrs/adr-*.md 2>/dev/null | sort -t- -k2 -n | tail -1
   ```

3. Generate the ADR ID using timestamp format: `ADR-YYMMDDHHMM`

## Phase 2: Dependency Impact Analysis (REQUIRED for modify/delete/restructure/migrate)

**This phase exists because ADR-2604050900 proved that deleting modules without tracing
all consumers leaves compilation broken in downstream crates.**

### 2a. Identify Affected Artifacts

For each artifact being modified/deleted/restructured:

```bash
# Find ALL consumers across the entire workspace
grep -r '<artifact-name>' --include='*.rs' --include='*.ts' --include='*.yml' \
  --include='*.yaml' --include='*.json' --include='*.toml' --include='*.md' \
  --include='*.html' --include='*.js' .
```

Build a **consumer dependency map**:
```
Artifact: <name>
├── Direct consumers (import/use/reference):
│   ├── crate/file:line — how it's used
│   ├── crate/file:line — how it's used
│   └── ...
├── Transitive consumers (depend on direct consumers):
│   └── ...
├── Config references (Cargo.toml, package.json, CI, Dockerfiles):
│   └── ...
├── Documentation references (ADRs, CLAUDE.md, README, workplans):
│   └── ...
└── Test references (unit, integration, e2e):
    └── ...
```

### 2b. Cross-Crate Analysis (Rust-specific)

For Rust workspace changes:
```bash
# Check feature flags that gate the artifact
grep -r 'feature.*=.*"<artifact>"' --include='*.toml' .

# Check re-exports
grep -r 'pub use.*<artifact>' --include='*.rs' .

# Check conditional compilation
grep -r 'cfg.*feature.*<artifact>' --include='*.rs' .

# Check auto-generated bindings
find . -path '*/spacetime_bindings/*' -name '*.rs' | head -20
```

### 2c. Build Verification Gates

Define explicit gates that the workplan MUST include:

| Gate | Command | Scope |
|------|---------|-------|
| Workspace compile | `cargo check --workspace` | All Rust crates |
| TypeScript compile | `bun run check` | All TS code |
| Unit tests | `bun test` / `cargo test` | Per-crate |
| Integration tests | Defined per-ADR | Cross-crate |

**CRITICAL**: The workplan derived from this ADR MUST include a validation step that
runs these gates AFTER every phase that deletes or restructures artifacts. The
ADR-2604050900 migration skipped this, resulting in hex-agent being broken for an
entire session.

### 2d. Blast Radius Classification

Classify each affected artifact:

| Impact | Definition | Action Required |
|--------|-----------|-----------------|
| **CRITICAL** | Breaks compilation in another crate | Must fix in same phase as deletion |
| **HIGH** | Breaks runtime behavior | Must fix before next phase |
| **MEDIUM** | Stale reference (docs, comments) | Fix in cleanup phase |
| **LOW** | Cosmetic (naming, outdated counts) | Fix opportunistically |

## Phase 3: Write the ADR

Copy `docs/adrs/TEMPLATE.md` to `docs/adrs/ADR-{ID}-{kebab-slug}.md`

Fill in all sections:

```markdown
# ADR-{ID}: {Title}

**Status:** Proposed
**Date:** {today}
**Drivers:** {from user input}

## Context
{Why this decision is needed}

## Impact Analysis

### Consumer Dependency Map
{From Phase 2a — every artifact affected, every consumer traced}

### Cross-Crate Dependencies
{From Phase 2b — feature gates, re-exports, conditional compilation}

### Blast Radius
| Artifact | Consumers | Impact | Mitigation |
|----------|-----------|--------|------------|
{One row per affected artifact}

### Build Verification Gates
{From Phase 2c — explicit commands that must pass after each phase}

## Decision
{What was decided and why}

## Consequences
**Positive:** ...
**Negative:** ...
**Mitigations:** ...

## Implementation
| Phase | Description | Validation Gate | Status |
|-------|-------------|-----------------|--------|
{Each phase with its specific validation command}

## References
{Related ADRs, issues, external docs}
```

## Phase 4: Validate the ADR

Before marking the ADR as complete:

1. **Cross-reference check**: Every ADR mentioned in "Supersedes" or "References" exists
2. **Consumer completeness**: Run the grep from Phase 2a and verify every hit is accounted for
3. **Gate completeness**: Every implementation phase has at least one validation gate
4. **Workplan alignment**: If a workplan will be created, verify it includes all gates

## Anti-Patterns (Lessons from ADR-2604050900)

### Anti-Pattern: Module-Scoped Impact Analysis
Analyzing impact only within the module being changed (e.g., only checking
`spacetime-modules/` and `hex-nexus/`) while missing consumers in other crates
(`hex-agent`, `hex-cli`).

**Fix**: Always grep the ENTIRE workspace. Use `--include` filters for file types,
never path restrictions.

### Anti-Pattern: Missing Validation Gates in Workplan
The workplan has "delete X" and "update Y" steps but no "verify everything compiles"
step between them.

**Fix**: Every workplan phase that modifies/deletes artifacts MUST end with
`cargo check --workspace` (or equivalent). This is a BLOCKING gate — next phase
cannot start until the gate passes.

### Anti-Pattern: Documentation-Only Impact Analysis
Listing which docs mention a module but not which code imports it.

**Fix**: Code consumers are CRITICAL impact. Documentation is MEDIUM. Always
prioritize code analysis over documentation analysis.

## Quick Reference

| Command | What it does |
|---------|-------------|
| `/hex-adr-create` | Create a new ADR with full impact analysis |
