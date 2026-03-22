# ADR-066: Dashboard Visibility Overhaul

**Status:** Accepted
**Date:** 2026-03-22
**Drivers:** The hex-nexus dashboard (localhost:5555) is the intended control plane for multi-project hex development, but multiple views are broken, data sources are fragmented between REST and SpacetimeDB, and critical operational data (inbox notifications, workplan progress, agent activity) is not visible. Developers are forced to use CLI-only workflows instead of the dashboard.

## Context

The dashboard was built incrementally as features were added to hex. Each new system (agents, swarms, workplans, inbox, projects) added its own view but integration was never validated end-to-end.

### Root Problem

The SpacetimeDB data model **already supports** the project-centric hierarchy that ADR-052 mandates. The problem is entirely frontend: views don't query the relationships that exist, and navigation doesn't expose the hierarchy.

### Three Separate Apps, Not One

The dashboard is fragmented across three HTML files:
- **index.html** — Legacy vanilla JS, 2×2 polling grid (should be removed)
- **chat.html** — Vanilla JS + HexChat, isolated from dashboard
- **dashboard.html** — SolidJS SPA + Tailwind + SpacetimeDB WebSocket (newest, incomplete)

### Current View Status

| View | Status | Root Issue |
|------|--------|-----------|
| Control Plane | Partial | Missing project→agent→worktree hierarchy |
| Agent Fleet | Partial | Treats all agents as global, not project-scoped |
| Swarm Monitor | High Gap | Tasks only — no agent→worktree→commit drill-down |
| WorkPlan View | Partial | No link to triggering ADR or executing swarm |
| Health Ring | Broken | Shows 100/100 when tree-sitter is stub (no astIsStub flag) |
| Graph | Broken | Layer colors missing for primary/secondary adapters |
| Chat | Broken | Two competing implementations, malformed BottomBar |
| ADR Browser | Working | Isolated from workplans/swarms |
| File Tree | Working | No architecture layer coloring |
| Config Page | Partial | Not scoped to project |

### Critical Visibility Gaps

Data exists in SpacetimeDB but the UI doesn't render it:

1. **Task→Agent→Worktree→Commit chain** — `SwarmAgent.worktree_path` exists but no UI renders it. Can't answer "why did this task fail?"
2. **Project-scoped agents** — `HexAgent.project_id` exists; UI shows all agents globally
3. **ADR→WorkPlan→Swarm→Task pipeline** — Three separate views with no cross-navigation
4. **Inbox notifications (ADR-060)** — No dashboard UI at all
5. **Swarm task dependency DAG** — No visual representation of tier dependencies

## Decision

### 1. Consolidate to Single SolidJS SPA

Kill `index.html` and `chat.html`. Everything renders through `dashboard.html` (the SolidJS app). Chat becomes a pane within the SPA, not a separate page.

### 2. Project-Centric Navigation (ADR-052)

The sidebar navigates by project. Selecting a project scopes ALL views:

```
Sidebar: [Project List]
  hex-intf ←selected
    ├─ Dashboard (agents, swarms, health summary)
    ├─ Agents (project-scoped fleet)
    ├─ Swarms & Tasks (with dependency DAG)
    ├─ Workplans (linked to ADRs)
    ├─ Inbox (ADR-060 notifications)
    ├─ ADRs
    ├─ Architecture (health + graph)
    └─ Config
```

### 3. Task Drill-Down Chain

Swarm monitor must support: Task → assigned Agent → Worktree → Commits

```
SwarmTask (status, title)
  └─ HexAgent (name, host, status) via agent_id
       └─ worktree_path → git worktree info
            └─ commits via REST /api/{project}/git/log
```

### 4. Inbox Notification Panel (ADR-060)

New component showing unacknowledged notifications for all agents in the selected project. Priority-2 notifications highlighted with red banner. Ack button calls `/api/hexflo/inbox/{id}/ack`.

### 5. Fix Broken Views

| Fix | What |
|-----|------|
| Health ring | Check `astIsStub` in project data, show warning banner instead of fake 100 |
| Graph colors | Add primary/secondary adapter colors to `LAYER_COLORS` map |
| Agent Fleet | Filter by `HexAgent.project_id`, group local vs remote |
| Token bars | Fix API contract — return flat object with `tokenEstimate` field |

### 6. Security Fixes

| Issue | Fix |
|-------|-----|
| CORS wildcard | Already fixed — `is_local_origin()` predicate in routes/mod.rs |
| Path traversal in `/api/tokens/:file` | Validate path doesn't contain `..` |
| No request body size limit | Already fixed — `DefaultBodyLimit` on all POST routes |

## Consequences

**Positive:**
- Single app — one mental model, one deployment, one state
- Project-centric navigation matches how developers think
- Task drill-down enables debugging failed swarm tasks from the browser
- Inbox visibility closes the ADR-060 loop — notifications visible without CLI

**Negative:**
- Removing index.html/chat.html is a breaking change for existing bookmarks
- SolidJS SPA requires full Vite rebuild cycle for iteration
- Adding drill-down increases SpacetimeDB subscription load (more tables per view)

**Mitigations:**
- Redirect `/` and `/chat` to the SolidJS app routes
- Use Vite HMR (port 5173) for development, not cargo rebuild
- SpacetimeDB subscriptions are efficient — only changed rows transmit

## Implementation

| Phase | Description | Status |
|-------|------------|--------|
| P0a | Fix health ring (astIsStub flag + warning) | Pending |
| P0b | Fix graph layer colors (primary/secondary) | Pending |
| P0c | Project-scope Agent Fleet view | Pending |
| P0d | Remove index.html, redirect / to dashboard SPA | Pending |
| P1a | Add Inbox notification panel (ADR-060) | Pending |
| P1b | Add task→agent→worktree→commit drill-down | Pending |
| P1c | Link WorkPlan view to ADR + executing swarm | Pending |
| P1d | Consolidate chat into SPA pane | Pending |
| P2a | Swarm task dependency DAG visualization | Pending |
| P2b | Health score trend line (historical) | Pending |
| P2c | URL routing for bookmarkable views | Pending |

## References

- ADR-039: Nexus agent control plane
- ADR-046: SpacetimeDB single authority for state
- ADR-052: AIIDE navigation model (project-centric)
- ADR-056: Frontend hexagonal architecture
- ADR-059: Canonical project identity contract
- ADR-060: Agent notification inbox
- ADR-065: Registration lifecycle gaps
- docs/analysis/dashboard-ux-deep-audit-2026-03-22.md
- docs/analysis/dashboard-swarm-analysis.md
- docs/analysis/dashboard-adversarial-review.md
