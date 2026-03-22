# ADR-012: ADR Lifecycle Tracking

**Status:** Accepted
## Date

2026-03-17

## Context

hex is a framework that enforces hexagonal architecture on target projects. Architecture Decision Records (ADRs) are central to this workflow -- they document why architectural boundaries exist and guide agent behavior during code generation. However, ADRs have historically been passive markdown files with no tooling support. Problems observed:

- ADRs drift out of date: a decision is superseded but the file still says "Accepted".
- Proposed ADRs are forgotten: no mechanism detects stale proposals that were never resolved.
- Agents cannot recall relevant ADRs: during code generation, the agent has no way to query "which ADRs affect the adapter I am modifying?" without scanning all files manually.
- No connection between ADRs and features: worktrees implement decisions, but there is no link from a feature branch back to the ADR that motivated it.

Since hex already manages architecture programmatically (boundary validation, dependency analysis, quality gates), tracking ADR lifecycle is a natural extension -- and an act of dogfooding the framework's own principles.

## Decision

Implement two port interfaces for ADR lifecycle management, backed by filesystem scanning and AgentDB pattern storage.

### Port Architecture

| Port | Direction | Purpose |
|------|-----------|---------|
| `IADRPort` | Secondary (driven) | Scans ADR markdown files, parses metadata, indexes into AgentDB |
| `IADRQueryPort` | Primary (driving) | Query interface consumed by CLI (`hex adr list/status/search/abandoned`) and MCP tools |

### ADR Parsing

The `ADRAdapter` parses markdown files matching `docs/adrs/ADR-*.md`. It extracts:

- **ID**: From filename (e.g., `ADR-001` from `ADR-001-hexagonal-architecture.md`)
- **Title**: First `# ` heading, with the ADR ID prefix stripped
- **Status**: From `## Status:` or `**Status:**` lines, normalized to `proposed | accepted | deprecated | superseded | rejected`
- **Date**: From `## Date:` or `**Date:**` lines
- **Sections**: All `## ` headings (for search indexing)
- **Linked features**: `feat/<name>` references found in the body
- **Linked worktrees**: `worktree: <branch>` references found in the body

Both `## Status: Accepted` and `**Status:** Accepted` formats are supported because existing ADRs use both conventions.

### AgentDB Integration

ADRs are indexed into AgentDB as searchable patterns via `ISwarmPort.patternStore`. Each ADR becomes a pattern with:

- **Category**: `adr`
- **Confidence**: `0.9` for accepted ADRs, `0.5` for others
- **Tags**: Status + section headings (lowercased)

This enables semantic search: an agent working on the notification adapter can query "notification" and retrieve ADR-007 without scanning files.

### CLI and MCP Surface

| Command / Tool | Action |
|----------------|--------|
| `hex adr list` | List all ADRs with status and date |
| `hex adr status ADR-007` | Show details for a specific ADR |
| `hex adr search "notification"` | Full-text + AgentDB search |
| `hex adr abandoned` | Find proposed ADRs older than 30 days with no updates |
| MCP `hex_adr_list` | Same as CLI, for LLM agents |
| MCP `hex_adr_search` | Same as CLI, for LLM agents |
| MCP `hex_adr_abandoned` | Same as CLI, for LLM agents |

### Abandoned Detection

An ADR is flagged as abandoned when:
1. Its status is `proposed` (never accepted or rejected)
2. Its file has not been modified in more than N days (default: 30)
3. No linked features or worktrees reference it

This surfaces forgotten proposals so the team can either promote them to accepted or explicitly reject them.

## Consequences

### Positive

- Agents can recall relevant ADRs during code generation, reducing architectural drift.
- Abandoned detection prevents decision debt from accumulating silently.
- AgentDB indexing enables semantic search across ADRs without loading all files.
- CLI and MCP tools give both humans and agents the same query surface.
- Dogfooding: hex tracks its own ADRs using the same ports-and-adapters pattern it enforces on target projects.

### Negative

- ADR parsing is regex-based, not a full markdown AST. Unusual formatting may be missed.
- AgentDB indexing is best-effort -- when AgentDB is unavailable, search falls back to simple text matching.
- Adding new metadata fields to ADRs requires updating the parser.

## Alternatives Considered

1. **adr-tools (Nat Pryce)** -- A well-known shell script toolkit for ADR management. Rejected because it has no programmatic API, no search capability, and no integration with the hex port system.
2. **Manual markdown + grep** -- Status quo. Rejected because it provides no abandoned detection, no agent-queryable interface, and no indexing.
3. **GitHub Issues as ADR tracker** -- Rejected because it introduces an external dependency (GitHub API) and separates the ADR content from the codebase. ADRs should live next to the code they govern.
