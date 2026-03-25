# ADR-2603250838: CLI / MCP Shared Implementation — One Function, Two Skins

**Status:** Accepted
**Date:** 2026-03-25
**Drivers:** A bug in `hex adr list` (MCP returned wrong ADRs, CLI returned correct ones) revealed that CLI and MCP had separate filesystem reader implementations that silently diverged. ADR-019 mandates that both adapters *exist*; this ADR mandates that they share the *same underlying implementation*.
**Supersedes:** Extends ADR-019 (does not replace it)

## Context

ADR-019 established that every CLI command must have an MCP equivalent. But it left open *how* those equivalents are implemented. In practice, two patterns have emerged:

### Pattern A — Direct Delegation (correct)

```
hex adr list  →  adr::list()
hex_adr_list  →  adr::list()  (same function)
```

One Rust function, two callers. Output is identical by construction.

### Pattern B — Parallel Implementation (anti-pattern)

```
hex adr list  →  hex-cli/src/commands/adr.rs::collect_adrs()
hex_adr_list  →  mcp.rs → GET /api/adrs → hex-nexus/src/routes/adrs.rs::list_adrs_from_dir()
```

Two separate implementations of the same filesystem logic. They drift independently.

The `hex_adr_list` bug was Pattern B:
- `find_adr_dir()` in nexus used 3 hardcoded relative-depth candidates; CLI walked upward indefinitely
- Nexus didn't filter non-`ADR-*` files (included `TEMPLATE.md`, `README.md`)
- Nexus returned bare numeric IDs (`"059"`); CLI returned full IDs (`"ADR-059"`)
- Nexus sorted descending; CLI sorted ascending
- Nexus ignored the `?status=` query parameter

This class of bug is **invisible at feature-add time** — the new feature "works" when tested via one surface, and the divergence only surfaces when an agent and a human compare outputs, or when an agent uses a stale MCP result to drive a decision.

### Why this matters for AI development

In AI-assisted development workflows, agents observe tool output and humans observe CLI output. If these differ:
- The agent's world-model diverges from the human's
- Context window data is stale or wrong
- Agent decisions based on tool output are unreliable
- Debugging requires checking which surface returned what

The entire value proposition of hex is that agents and humans work from the same ground truth. Divergent implementations undermine this at the foundation.

## Decision

**CLI and MCP handlers for the same command MUST delegate to a shared implementation.** Two implementations of the same logic are forbidden.

### Rule

> For any capability `X`:
> - `hex X` (CLI) calls `impl_x()`
> - `hex_X` (MCP) calls `impl_x()`
> - There is exactly one `impl_x()`

### Implementation patterns

#### For filesystem-local commands (adr, analyze, git)

The MCP handler lives in `hex-cli/src/commands/mcp.rs`, which is in the same crate as all CLI commands. It MUST call the shared function directly:

```rust
// mcp.rs — CORRECT
"hex_adr_list" => {
    let adrs = adr::list_json(status_filter).await?;
    Ok(serde_json::to_string(&adrs)?)
}

// adr.rs — shared implementation used by BOTH
pub async fn list_json(status_filter: &str) -> anyhow::Result<Vec<AdrSummary>> { ... }
pub async fn list() -> anyhow::Result<()> { /* calls list_json, pretty-prints */ }
```

The MCP handler MUST NOT make an HTTP round-trip to nexus for operations that are purely local filesystem reads. Nexus is the filesystem bridge for SpacetimeDB WASM (which cannot access the filesystem); CLI and MCP are not WASM and do not need the bridge.

#### For nexus-backed commands (swarms, agents, inference)

These legitimately go through nexus because the state lives in SpacetimeDB. For these, nexus is the single implementation — both CLI and MCP call the same nexus endpoint. This is already correct.

#### For hybrid commands

If a command reads both local filesystem and SpacetimeDB state, the local-filesystem portion must be a shared Rust function; the nexus call handles only the SpacetimeDB portion.

### What "shared implementation" means concretely

| Command type | Where impl lives | CLI calls | MCP calls |
|-------------|-----------------|-----------|-----------|
| Filesystem-local (adr, analyze) | `hex-cli/src/commands/` | `cmd::fn()` | `cmd::fn()` directly |
| SpacetimeDB-backed (swarm, agent, inference) | hex-nexus REST | nexus endpoint | same nexus endpoint |
| Hybrid | split: local fn + nexus endpoint | local fn + nexus | local fn + nexus |

### Output contract

The shared function returns a structured type (not formatted text). Each adapter formats independently:
- CLI: ANSI table via `tabled`, human-readable
- MCP: JSON serialization of the same type

This ensures semantic content is identical while presentation differs appropriately per surface.

## Enforcement

### Compile-time (strongest)

When MCP handlers call `crate::commands::X` directly, divergence is impossible — there is one code path. This is the target state.

### Code review gate

PRs that add a new `impl_x()` in `mcp.rs` that duplicates logic from `commands/` MUST be blocked. The reviewer SHALL check that the MCP handler calls the shared function.

### Architecture check

`hex analyze .` SHALL flag any nexus route that re-implements logic already present in `hex-cli/src/commands/`. This is tracked as a parity health metric alongside boundary violations.

### Integration test

`hex adr list` CLI output and `hex_adr_list` MCP tool output SHALL be compared in CI:
- Same ADR count
- Same ID format
- Same status values
- Same sort order

Tracked in: ADR-2603242100 (hex-cli integration testing).

## Migration

Existing commands using Pattern B (parallel nexus implementation) must be migrated to Pattern A. Priority order:

| Command | Current pattern | Target | Priority |
|---------|----------------|--------|----------|
| `hex adr *` | Pattern B (nexus reader) | Pattern A (shared fn) | P1 — triggered this ADR |
| `hex analyze` | Verify | Verify | P1 |
| `hex git *` | Verify | Verify | P2 |
| `hex plan *` | Verify | Verify | P2 |

## Consequences

### Positive

- **Correctness by construction**: One code path means CLI and MCP output are identical. No sync needed.
- **Simpler debugging**: When output is wrong, there is one place to fix, not two.
- **Removes unnecessary HTTP round-trips**: MCP handlers for filesystem ops don't need nexus running.
- **Agent reliability**: Agents can trust MCP tool output matches what humans see in the terminal.

### Negative

- **Refactoring cost**: Existing Pattern B implementations need migration (~5-10 commands affected).
- **API shape constraint**: The shared function must return a type serializable to both ANSI table rows and JSON. This occasionally requires a DTO struct that didn't exist before.

## References

- ADR-019: CLI–MCP Parity (every command must have an MCP equivalent)
- ADR-2603222229: CLI / MCP / Dashboard Parity Investigation (gap audit)
- ADR-2603242100: hex-cli Integration Testing
- Bug fixed: `hex_adr_list` MCP tool returning wrong/incomplete ADR list (2026-03-25)
