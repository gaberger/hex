# Workplan: AIIDE Remaining Tasks

**Baseline:** Commit 98f164e
**Date:** 2026-03-21
**Status:** in_progress
**Status note:** P0 complete (rebuild, agents scoped, file write API). P1 largely done (ADR skills, description field). P2-P3 partially done. Superseded for future work by aiide-phase2.md.
**Priority:** Complete the Hex Nexus AIIDE to production-ready state

## Priority Matrix

| Priority | Task | Effort | Dependencies |
|----------|------|--------|-------------|
| P0 | Rebuild nexus binary | 5 min | None |
| P0 | Agents scoped to projects | Medium | SpacetimeDB schema change |
| P0 | File write API (save ADRs, CLAUDE.md) | Medium | Rust endpoint |
| P1 | Real worktree data | Medium | git2 API or shell calls |
| P1 | ADR skill files (.md) | Small | Skill template |
| P1 | Project description field | Small | SpacetimeDB schema + UI |
| P2 | File tree view | Medium | Rust file listing API |
| P2 | Health auto-fetch on project navigate | Small | Frontend only |
| P2 | Real MCP/hooks/skills discovery | Medium | Parse settings.json |
| P3 | Inference full-width Pencil design | Small | Frontend only |
| P3 | Fleet Nodes full page | Small | Frontend only |
| P3 | Mobile responsive testing | Small | Frontend only |
| P3 | ADR-032 duplicate cleanup | Trivial | File rename |

## Swarm Tasks (for HexFlo)

### Phase 1: Backend (P0)
- T1: Rebuild hex-nexus binary with all route changes
- T2: Add project_id to agent-registry SpacetimeDB table
- T3: Create file write API endpoint (POST /api/files)
- T4: Regenerate agent-registry TS bindings

### Phase 2: Data Integration (P1)
- T5: Wire real git worktree data into ProjectDetail
- T6: Create ADR skill .md files (create, review, search, status)
- T7: Add description field to project SpacetimeDB table
- T8: Health auto-fetch when navigating to project

### Phase 3: Discovery (P2)
- T9: Parse .claude/settings.json for MCP servers
- T10: Parse .claude/settings.json for hooks
- T11: Scan .claude/skills/ for skill files
- T12: File tree component with nested directory browsing

### Phase 4: Polish (P3)
- T13: Inference page Pencil design match
- T14: Fleet Nodes page enhancement
- T15: Mobile responsive breakpoint testing
- T16: Clean up ADR-032 duplicate
