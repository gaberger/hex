# Workplan: AIIDE Phase 2 — Config Sync, Project Init, Production

**Baseline:** Commit 167c651
**Date:** 2026-03-21
**Depends on:** ADR-044 (Config Sync to SpacetimeDB)
**Previous:** `aiide-remaining.md` (P0-P3 complete)

## Priority Matrix

| Priority | Task | Effort | Dependencies |
|----------|------|--------|-------------|
| P0 | `hex init` project scaffolding | Medium | None |
| P0 | Config sync layer (files → SpacetimeDB) | Large | SpacetimeDB tables |
| P0 | Production build + embedded assets | Small | Rust rebuild |
| P1 | SpacetimeDB config tables (project_config, skill_registry, agent_definition) | Medium | Module publish |
| P1 | Dashboard config views read from SpacetimeDB subscriptions | Medium | P1 tables |
| P1 | Dashboard config edits write back to repo files | Medium | File write API |
| P1 | Agent-registry project_id field | Small | Module publish |
| P2 | File tree component (browse project files visually) | Medium | File list API |
| P2 | fsnotify auto-sync on file change | Medium | Rust integration |
| P2 | Worktree API endpoint (real git worktree list) | Medium | git2 or shell |
| P2 | Session persistence in SpacetimeDB (replace SQLite hub.db) | Large | New module |
| P3 | Pencil design iteration based on usage feedback | Ongoing | User testing |
| P3 | Light theme support (theme toggle exists, needs palette) | Medium | Design tokens |
| P3 | Keyboard shortcuts help overlay (Ctrl+?) | Small | Frontend only |
| P3 | Command palette search across all entities | Medium | Search index |
| P3 | ADR compliance enforcement (auto-check on PR) | Medium | GitHub Actions |

## Swarm Tasks

### Phase 1: Project Init + Sync Foundation
- T1: Implement `hex init <path>` — scaffold .hex/, .claude/, docs/adrs/
- T2: Add SpacetimeDB tables: project_config, skill_registry, agent_definition
- T3: Publish updated hexflo-coordination module
- T4: Regenerate TypeScript bindings
- T5: Nexus startup sync — read repo files → push to SpacetimeDB
- T6: Production build (cargo build --release) with embedded assets

### Phase 2: Dashboard SpacetimeDB Integration
- T7: Dashboard subscribes to project_config, skill_registry, agent_definition
- T8: MCPToolsView reads from project_config["mcp_servers"] subscription
- T9: HooksView reads from project_config["hooks"] subscription
- T10: SkillsView reads from skill_registry subscription
- T11: AgentDefsView reads from agent_definition subscription
- T12: BlueprintView reads from project_config["blueprint"] subscription

### Phase 3: Bidirectional Sync
- T13: Dashboard config edits → SpacetimeDB reducer → file write API → repo
- T14: File change detection (fsnotify or poll) → re-sync to SpacetimeDB
- T15: "Refresh Config" button triggers manual re-sync
- T16: Config change history tracking (who changed what, when)

### Phase 4: Missing Features
- T17: File tree component — nested directory browser with file preview
- T18: Worktree API — GET /api/projects/{id}/worktrees (real git data)
- T19: Agent-registry project_id field + SpacetimeDB module update
- T20: Session persistence migration from SQLite to SpacetimeDB

### Phase 5: Polish + DX
- T21: Keyboard shortcuts overlay (Ctrl+? shows all shortcuts)
- T22: Command palette entity search (projects, agents, ADRs, skills)
- T23: Light theme color palette
- T24: ADR compliance CI check via GitHub Actions
- T25: Performance audit — lazy loading, bundle size, subscription count

## Success Criteria

After Phase 2 completion:
- `hex init my-project` creates a fully configured project
- Dashboard config views are reactive (no REST polling)
- Config edits in dashboard persist to repo files
- All framework config flows: repo → SpacetimeDB → dashboard → repo

## Architecture Diagram

```
Developer                     AIIDE Dashboard
    │                              │
    ├── Edit files ──────┐         │
    │                    ▼         │
    │              Repo Files      │
    │              (.hex/, .claude/)│
    │                    │         │
    │                    ▼         │
    │              Sync Layer      │
    │              (nexus startup) │
    │                    │         │
    │                    ▼         │
    │              SpacetimeDB     │◄── WebSocket subscription
    │              (reactive)      │         │
    │                    │         │         ▼
    │                    └─────────┼── Config Views
    │                              │   (Blueprint, Tools,
    │                              │    Hooks, Skills, Agents)
    │                              │         │
    │                              │         ▼
    │                              │   Edit in Dashboard
    │                              │         │
    │              File Write API ◄┘         │
    │                    │                   │
    │                    ▼                   │
    │              Repo Files ◄──────────────┘
    │              (committed)
    │
    └── git commit ──────────────── Version controlled
```
