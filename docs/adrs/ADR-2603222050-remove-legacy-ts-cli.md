# ADR-2603222050: Remove Legacy TypeScript CLI and Adapters

**Status:** Abandoned — Legacy TS CLI already removed; Rust CLI is canonical
**Date:** 2026-03-22
**Drivers:** CI fails on `bun run check` due to 29 TypeScript errors in legacy adapters that the Rust CLI (`hex-cli`) replaced. The files are `@ts-nocheck` suppressed but still ship in the npm package, adding ~15K LOC of dead code. Attempted removal revealed deep coupling via `composition-root.ts`.

## Context

hex originally had a TypeScript CLI (`src/cli.ts` → `CLIAdapter`). ADR-010 migrated the CLI to Rust (`hex-cli/`). The TS CLI is no longer the entry point — `hex` now runs the Rust binary. However, the TS adapter layer still exists:

### Files to remove (legacy CLI chain)

| File | Lines | Status |
|------|-------|--------|
| `src/cli.ts` | 50 | Dead — not imported by anything |
| `src/adapters/primary/cli-adapter.ts` | 3200+ | Dead — was the TS CLI, replaced by hex-cli |
| `src/adapters/primary/progress-reporter.ts` | 80 | Dead — only imported by cli-adapter |
| `src/adapters/primary/notification-query-adapter.ts` | 60 | Dead — only imported by cli-adapter |

### Files to refactor (still wired into composition-root)

| File | Lines | Used By | Migration |
|------|-------|---------|-----------|
| `src/adapters/primary/dashboard-adapter.ts` | 800 | composition-root.ts | Move dashboard state to SpacetimeDB subscriptions (ADR-046) |
| `src/adapters/primary/daemon-manager.ts` | 200 | mcp-adapter.ts | Replace with `hex nexus start/stop` calls |
| `src/adapters/secondary/build-adapter.ts` | 100 | composition-root.ts, CodeGenerator | Replace with shell exec or remove CodeGenerator |
| `src/adapters/secondary/coordination-adapter.ts` | 150 | composition-root.ts | Replace with HexFlo coordination (ADR-027) |
| `src/adapters/secondary/event-bus-notifier.ts` | 80 | composition-root.ts | Replace with SpacetimeDB pub/sub |

### Why removal is hard

`composition-root.ts` wires these adapters into the TS library's dependency injection. Removing them breaks `createDeps()` which is used by:
- `src/adapters/primary/mcp-adapter.ts` (the TS MCP server — superseded by Rust `hex mcp`)
- `src/core/usecases/*.ts` (use cases that depend on ports implemented by these adapters)

The entire TS `src/` tree is becoming dead code as hex-cli, hex-nexus, and hex-agent (all Rust) replace each subsystem.

## Decision

### Phase 1: Remove clearly dead files

Delete files with zero live imports:
- `src/cli.ts`
- `src/adapters/primary/cli-adapter.ts`
- `src/adapters/primary/progress-reporter.ts`
- `src/adapters/primary/notification-query-adapter.ts`

Update `composition-root.ts` to remove any references to these files.

### Phase 2: Decouple wired adapters

For each adapter still referenced by `composition-root.ts`:
1. Create a no-op stub implementing the same port interface
2. Replace the real adapter with the stub in `composition-root.ts`
3. Delete the real adapter
4. Mark the port as deprecated if no Rust equivalent exists

### Phase 3: Evaluate TS library scope

After phases 1-2, assess what remains in `src/`:
- **Keep**: Port interfaces (used as contracts for documentation), domain types, ADR adapter, tree-sitter adapter
- **Remove**: Use cases that only orchestrated TS adapters (now done by hex-cli/hex-nexus)
- **Decision**: If <30% of `src/` remains, consider moving kept files to `hex-core/` (Rust) or a separate `hex-types` package

### Phase 4: Remove `@ts-nocheck` suppressions

Once all legacy files are removed, verify `bun run check` passes without any `@ts-nocheck` directives. Zero type errors should be the steady state.

## Consequences

**Positive:**
- CI `bun run check` passes without `@ts-nocheck` hacks
- ~15K LOC of dead code removed from the npm package
- Reduced confusion about which CLI is canonical (Rust, not TS)
- Faster `bun run build` (less to transpile)
- Cleaner `hex analyze` results (no false positives from dead adapters)

**Negative:**
- Breaking change for anyone importing from `@anthropic-hex/hex` npm package
- Port interfaces may lose their TypeScript implementation examples

**Mitigations:**
- Major version bump on npm package (semver)
- Port interfaces preserved as documentation-only `.ts` files
- CLAUDE.md already states hex-cli (Rust) is canonical

## Implementation

| Phase | Description | Status |
|-------|------------|--------|
| P1 | Remove dead files (cli.ts, cli-adapter, progress-reporter, notification-query) | Pending |
| P2 | Stub out wired adapters (dashboard, daemon-manager, build, coordination, event-bus) | Pending |
| P3 | Evaluate remaining TS scope — keep ports/domain, remove dead use cases | Pending |
| P4 | Remove all @ts-nocheck suppressions, verify clean type check | Pending |

## References

- ADR-010: TypeScript-to-Rust Migration
- ADR-035: Hex Architecture V2 — Rust-First
- ADR-032: Deprecate hex-hub (related — removing TS daemon)
