# ADR-2604081320: Claude Code Context Detection

**Status:** Proposed
**Date:** 2026-04-08
**Drivers:** Gary Berger

## Context

When hex MCP tools are invoked by Claude Code, two failure modes occur that degrade
the agent experience:

### Failure Mode 1: CLI-Only Suggestions in MCP Context

The `hex project status` MCP tool returns suggestions like:
> "run `hex fingerprint generate` to get full context"

When Claude Code receives this text, it attempts to call an `hex_fingerprint_generate`
MCP tool. That tool doesn't exist, so Claude falls through to running `hex fingerprint
generate` via `Bash`. The Bash call then fails with 404 because the fingerprint REST
route wasn't wired (fixed in v26.4.29). Even with the route wired, the Bash fallback
is fragile — it produces verbose output, requires correct argument inference, and
silently gives wrong results when the project ID doesn't match.

### Failure Mode 2: No MCP Tools for Common CLI Commands

`hex fingerprint generate`, `hex analyze`, and other commonly-needed commands have
no corresponding MCP tool. Claude has to shell out, producing noisy output and losing
structured return values that MCP tools can return cleanly as JSON.

### Root Cause

hex MCP tools don't know whether they're being called from:
- Claude Code (CLAUDE_SESSION_ID is set)
- A terminal/script (no Claude env vars)
- Another MCP client

When in Claude Code context, suggestions should reference MCP tools by name
(e.g., `hex_fingerprint_generate`) not CLI commands. And the tools themselves
should exist.

### Detection Signal

Claude Code sets `CLAUDE_SESSION_ID` in the environment of every tool call. This is
a stable, first-party signal that doesn't require any hex-side setup. hex-nexus also
sets `CLAUDECODE=1` for bypass mode — this is a secondary confirmation signal.

**Decision type:** `add`

## Decision

### 1. Context Detection Utility

Add `fn is_claude_code_context() -> bool` to `hex-cli/src/commands/mcp.rs`:

```rust
/// Returns true when running inside Claude Code (as an MCP tool call).
/// Claude Code sets CLAUDE_SESSION_ID on every tool invocation.
pub fn is_claude_code_context() -> bool {
    std::env::var("CLAUDE_SESSION_ID").is_ok()
        || std::env::var("CLAUDECODE").as_deref() == Ok("1")
}
```

### 2. Add Missing MCP Tools

Add the following MCP tools to `hex-cli/src/commands/mcp.rs`:

| MCP Tool | Maps To | REST Endpoint |
|----------|---------|---------------|
| `hex_fingerprint_generate` | `hex fingerprint generate <id>` | `POST /api/projects/{id}/fingerprint` |
| `hex_fingerprint_get` | `hex fingerprint get <id>` | `GET /api/projects/{id}/fingerprint` |
| `hex_analyze_project` | `hex analyze <path>` | `POST /api/analysis/analyze` |

These follow the same pattern as existing MCP tools: delegate to the nexus REST API,
return structured JSON.

### 3. Strip CLI-Only Suggestions in MCP Output

When `is_claude_code_context()` is true, any nexus API response that contains
CLI-only command suggestions (`hex fingerprint generate`, `hex analyze .`, etc.)
must be transformed to reference the equivalent MCP tool name, or omitted entirely
if no MCP equivalent exists.

Implementation: the MCP call handler in `mcp.rs` post-processes the nexus API
response to replace or strip CLI suggestions before returning to Claude.

Alternatively (simpler): nexus API responses use a structured `suggestions` field
(array of `{label, mcp_tool, cli_command}`) and the MCP handler renders only the
`mcp_tool` form.

### 4. MCP Tool for `hex_project_status`

The `hex_project_list` MCP tool currently returns free-text suggestions embedded
in the `message` field. These should be moved to a structured `actions` array:

```json
{
  "projects": [...],
  "actions": [
    {
      "label": "Generate fingerprint for sportsbook",
      "mcp_tool": "hex_fingerprint_generate",
      "mcp_args": {"project_id": "sportsbook-1635zm8"},
      "cli_command": "hex fingerprint generate sportsbook-1635zm8"
    }
  ]
}
```

Claude Code can then present `mcp_tool` calls directly. CLI users get `cli_command`.

## Impact Analysis

### Affected Files

| File | Change | Impact |
|------|--------|--------|
| `hex-cli/src/commands/mcp.rs` | Add `is_claude_code_context()`, add 3 new MCP tools | LOW — additive |
| `hex-nexus/src/routes/projects.rs` | Add `actions` array to project list response | LOW — additive |
| `config/mcp-tools.json` | Register new MCP tools for SpacetimeDB sync | LOW — additive |

### Build Verification Gates

| Gate | Command |
|------|---------|
| Workspace compile | `cargo check --workspace` |
| MCP tool registration | `hex mcp list \| grep fingerprint` |

## Consequences

**Positive:**
- Claude Code gets structured action suggestions instead of raw CLI text
- `hex fingerprint generate` works as an MCP tool, not a Bash fallback
- Context detection is a clean, zero-config pattern — no setup required

**Negative:**
- MCP tool registry grows; must keep in sync with CLI commands
- Structured `actions` field is a mild breaking change to `hex_project_list` output

**Mitigations:**
- `actions` is additive — existing consumers that parse `message` still work
- MCP tools follow a consistent template so adding new ones is low-effort

## Implementation

| Phase | Description | Validation Gate | Status |
|-------|-------------|-----------------|--------|
| P0 | Add `is_claude_code_context()` to mcp.rs | `cargo check -p hex-cli` | Proposed |
| P1 | Add `hex_fingerprint_generate` + `hex_fingerprint_get` MCP tools | `cargo check -p hex-cli` + `hex mcp list` | Proposed |
| P2 | Add `hex_analyze_project` MCP tool | `cargo check -p hex-cli` + `hex mcp list` | Proposed |
| P3 | Add structured `actions` array to `hex_project_list` response | `cargo check --workspace` | Proposed |

## References

- ADR-2603301200: Architecture fingerprint (`hex fingerprint` CLI command)
- ADR-049: MCP server configuration
- `hex-cli/src/commands/mcp.rs` — MCP tool dispatch table
- `hex-nexus/src/routes/fingerprint.rs` — fingerprint route handler (wired v26.4.29)
- v26.4.29: fingerprint routes wired but no MCP tool yet
