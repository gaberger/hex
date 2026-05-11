# ADR-2026-04-14-2100: hex Native Filesystem — Self-Sufficient File Operations

**Status:** Accepted (resolved 2026-05-05)
**Date:** 2026-04-14
**Drivers:** hex currently depends on external tools (context-mode, Bash glob, Claude Read/Grep) for filesystem operations. An AIOS that needs third-party plugins for `ls` is not self-sufficient. hex must own its entire filesystem surface.

## Context

Today, agents working with hex invoke:
- `context-mode:ctx_execute(language: "shell", ...)` — for `find`, `grep`, `ls`, `wc`
- `Bash` — for basic file operations
- `Read`, `Grep`, `Glob` — Claude Code primitives

Each of these is a dependency that:
- Might not exist in other environments (e.g. standalone mode, MCP-hosted sessions)
- Has quirks (context-mode swallows output, Bash has context-guidance hooks)
- Makes hex less portable

An AIOS should provide native filesystem primitives via MCP + CLI that:
- Work everywhere hex works (no Claude Code dependency)
- Respect hex's capability system (ADR-010 claims-based auth)
- Integrate with hex's observability (file access events go to SpacetimeDB)

## Decision

### 1. New top-level command: `hex fs`

```bash
hex fs list <path> [--recursive] [--pattern <glob>]    # ls/find
hex fs read <path> [--lines N] [--offset M]            # cat with pagination
hex fs search <pattern> [--path <path>] [--type rs]    # ripgrep
hex fs glob <pattern>                                  # fd/find
hex fs tree <path> [--depth N]                         # tree
hex fs stat <path>                                     # file metadata
hex fs head <path> [-n N]                              # head
hex fs tail <path> [-n N]                              # tail
```

All routed through hex-nexus REST API + MCP tools.

### 2. MCP tools

```
mcp__hex__hex_fs_list
mcp__hex__hex_fs_read
mcp__hex__hex_fs_search
mcp__hex__hex_fs_glob
mcp__hex__hex_fs_tree
mcp__hex__hex_fs_stat
```

### 3. Implementation

- `hex-nexus/src/routes/fs.rs` — REST endpoints, wraps `ignore::WalkBuilder` (same crate ripgrep uses) + `grep-regex`
- `hex-cli/src/commands/fs.rs` — CLI dispatch to REST
- `hex-cli/src/commands/mcp.rs` — add tool definitions

### 4. Capability-gated access

- Use existing claims system (ADR-010) to scope file access:
  - `fs:read:src/**` — read source
  - `fs:read:docs/**` — read docs
  - `fs:write:docs/workplans/**` — write workplans
- Path traversal prevention via `safe_path()` (already exists in primary adapters)

### 5. Performance

- Use tokio async IO
- Default pagination: 500 lines max, 50 entries max
- Response includes `truncated: bool` + `total: N`

### 6. Dog-food: update agent prompts

Agent YAMLs should say:
```
TOOL PREFERENCE: Always use mcp__hex__hex_fs_* before Bash/Read/Grep.
```

Update `hex-cli/assets/templates/claude-md-hex-section.md` to mandate hex fs primitives.

## Consequences

**Positive:**
- hex works standalone (no Claude Code, no context-mode)
- Single source of truth for filesystem access
- Capability-gated = auditable access
- Events stream to SpacetimeDB (who read what, when)

**Negative:**
- Extra layer vs direct shell access
- Must maintain feature parity with ripgrep/fd

**Mitigations:**
- Shell out to ripgrep/fd underneath (if available) for performance
- Document limitations vs native tools

## Implementation

Tracked by `docs/workplans/wp-hex-native-filesystem.json`. Phase IDs differ slightly from this table — the workplan groups list/read/search/glob/tree/stat/head/tail under a single REST phase.

| Phase | Description | Workplan | Status |
|-------|------------|----------|--------|
| P1 | All 8 fs primitives via REST in hex-nexus (`/api/fs/*`) | wp P1 | Pending — `hex fs` CLI returns 404 (routes not implemented) |
| P2 | `hex fs` CLI subcommand | wp P2 | Done (2026-04-14, hex-cli/src/commands/fs.rs) |
| P3 | MCP tool definitions (`mcp__hex__hex_fs_*`) | wp P3 | Pending — `grep hex_fs_ hex-cli/src/commands/mcp.rs` empty |
| P4 | Update agent YAMLs + CLAUDE.md template | wp P4 | Pending — no asset references hex_fs_ |
| P5 | Capability-gated access via claims (ADR-010) | wp P5 | Pending |
| P6 | Event emission to SpacetimeDB | wp P6 | Pending |

> **Workplan integrity note (2026-05-04):** P1/P3/P4 were previously marked `done` in the workplan based on fabricated evidence. Salvage-checked against live nexus (`/api/fs/list` → 404) and asset grep, then flipped back to `pending`. P5 and P6 were missing from the workplan entirely and have been added.

## References

- ADR-010: Claims-based Authorization
- ADR-019: CLI-MCP Parity
- ADR-2026-04-11-2000: Hex Self-Sufficient Dispatch (Standalone Mode)

---
## Operator Resolution Note (2026-05-05T02:09:25.803241440+00:00)

Auto-applied from @adr-reviewer chat verdict
