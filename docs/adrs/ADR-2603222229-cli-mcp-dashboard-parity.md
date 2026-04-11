# ADR-2603222229: CLI / MCP / Dashboard Parity Investigation

**Status:** Accepted
**Scope:** CLI/MCP parity only. Dashboard parity is a separate effort (see ADR-066).
**Date:** 2026-03-22
**Drivers:** hex has three user-facing surfaces — CLI (`hex-cli`), MCP tools (`hex mcp`), and the dashboard (`hex-nexus/assets`). Feature parity across these surfaces has never been audited. Some operations are available in CLI but not MCP, some dashboard views have no CLI equivalent, and some MCP tools delegate to endpoints the dashboard doesn't use.

## Context

hex provides three ways to interact with the system:

| Surface | Technology | User | Access |
|---------|-----------|------|--------|
| **hex-cli** | Rust binary | Developer terminal | Direct commands |
| **hex mcp** | JSON-RPC stdio | Claude Code / MCP clients | Tool calls |
| **Dashboard** | Solid.js SPA | Browser | Real-time UI at `:5555` |

ADR-019 established the principle that every CLI command should have an MCP equivalent. But the dashboard was built independently, and enforcement/test-tracking/audit features were added to CLI/MCP without dashboard panes.

### Known gaps (suspected)

| Feature | CLI | MCP | Dashboard |
|---------|-----|-----|-----------|
| Enforcement rules | `hex enforce list` | ? | ? |
| Agent audit | `hex agent audit` | ? | ? |
| Test history/trends | `hex test history/trends` | ? | ? |
| Workplan activate | ? | `hex_workplan_activate` | ? |
| ADR compliance | `hex analyze --adr-compliance` | `hex_analyze` | ? |
| Session heartbeat | hook | `hex_session_heartbeat` | ? |

## Decision

Conduct a systematic parity audit across all three surfaces, document gaps, and create a remediation workplan.

## Implementation

| Phase | Description | Status |
|-------|------------|--------|
| P1 | Audit CLI commands → map to MCP tools → identify missing tools | Pending |
| P2 | Audit MCP tools → map to CLI commands → identify missing commands | Pending |
| P3 | Audit dashboard views → map to CLI/MCP → identify missing panes | Pending |
| P4 | Audit nexus REST endpoints → identify unused/undocumented routes | Pending |
| P5 | Create remediation workplan with prioritized gaps | Pending |

## Known Findings

| Issue | Surface | Status |
|-------|---------|--------|
| `hex_adr_list` returned wrong directory, wrong IDs, missing files | MCP | Fixed 2026-03-25 (two separate implementations drifted) |

## References

- ADR-019: CLI-MCP Parity
- ADR-2603250838: CLI/MCP Shared Implementation — One Function, Two Skins (root-cause fix for drift)
- ADR-039: Nexus Agent Control Plane
- ADR-066: Dashboard Visibility Overhaul
