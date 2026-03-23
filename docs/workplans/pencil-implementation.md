# Workplan: Implement Pencil Designs

**Commit baseline:** 2ba366f
**Pencil file:** Open in Pencil desktop app (8 screens designed)
**Status:** in_progress
**Status note:** Control Plane and Navigation partially implemented. Project Detail, ADR Browser, Config screens not started. Design tokens extracted but not fully applied. Several dashboard commits address individual items but gap table below is not tracked.
**Priority:** HIGH — current implementation doesn't match designs

## Gap Analysis

### What Pencil shows vs what's implemented:

| Screen | Pencil Design | Current State |
|--------|--------------|---------------|
| Control Plane | Full-width project cards with health/worktrees/swarms/agents, active swarm cards below | Generic project list with empty state form |
| Project Detail | Worktree cards with branch names, layer, agent, commits | Not implemented |
| Project Chat | Context pills bar, scoped messages, breadcrumb | ChatView exists but no context scoping |
| Agent Fleet | LOCAL + REMOTE sections with detailed agent cards | Created but not verified |
| ADR Browser | Left list + center markdown viewer | Not implemented |
| Config: Blueprint | Layer cards with colors + boundary rules | Not implemented |
| Config: MCP Tools | Server cards with tool badges | Not implemented |
| Navigation | Breadcrumbs on every page | Router + Breadcrumbs created |

### Design tokens from Pencil:
- Background: #0a0e14 (content), #111827 (cards/sidebar)
- Borders: #1f2937 (default), #374151 (elevated)
- Text: #e5e7eb (primary), #9ca3af (secondary), #6b7280 (muted), #4b5563 (hint)
- Accent: #22d3ee (cyan), #4ade80 (green), #60a5fa (blue), #a78bfa (purple), #f0883e (orange)
- Font: Inter for UI, JetBrains Mono for code/identifiers
- Sizes: 20-22px titles, 14-16px body, 12-13px labels, 10-11px badges
- Cards: rounded-xl (12px), padding 16-18px
- Hex layer colors: Domain=#58a6ff, Ports=#bc8cff, UseCases=#3fb950, Primary=#f0883e, Secondary=#d29922

## Implementation Order

### Phase 1: Control Plane (matches Pencil Screen 1)
- Rewrite ControlPlane.tsx to match card layout exactly
- Project cards: folder icon, name, health badge, stats row, progress bar, last activity
- Active swarms: name, percentage, topology badge, project association
- Action buttons: Add Project (cyan), New Swarm (outline)

### Phase 2: Project Detail (matches Pencil Screen 2)
- New ProjectDetail.tsx view
- Stats bar: health/files/worktrees/violations
- Worktree cards: branch name, layer, agent, commits, status badge
- Analyze + New Worktree buttons

### Phase 3: ADR Browser (matches Pencil Screen 5)
- New ADRBrowser.tsx view
- Left: searchable ADR list with number, title, status badge
- Center: markdown renderer with metadata bar
- Edit/Raw toggle buttons

### Phase 4: Agent Fleet (matches Pencil Screen 4)
- Verify/fix AgentFleet.tsx against design
- LOCAL section: cards with role/project/task/uptime/model
- REMOTE section: cards with host/transport/inference/latency

### Phase 5: Config: Blueprint (matches Pencil Screen 6)
- New ConfigBlueprint.tsx view
- Layer cards with hex colors and import rules
- Boundary rules list with shield icons

### Phase 6: Config: MCP Tools (matches Pencil Screen 7)
- New ConfigMCPTools.tsx view
- Server cards with status dots, tool count, tool badges
