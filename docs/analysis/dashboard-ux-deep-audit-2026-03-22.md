# Dashboard UX Deep Audit — 2026-03-22

**Scope:** Full analysis of hex-nexus dashboard (all entry points, all pages)
**Verdict:** FAIL — layout is fragmented, navigation is incoherent, project-centric model is absent

---

## 1. Critical Finding: Three Separate Applications

The dashboard is not one app — it is **three disconnected HTML files** with no cross-navigation:

| Entry Point | Tech | Purpose | State |
|-------------|------|---------|-------|
| `index.html` | Vanilla JS, CSS vars, polling | "Old dashboard" — 2×2 grid cards | Legacy, should be removed |
| `chat.html` | Vanilla JS, `window.HexChat` | Chat-first with collapsible dashboard | Feature-complete but isolated |
| `dashboard.html` | SolidJS SPA, Tailwind, SpacetimeDB WS | Tiling pane manager with command palette | Newest, incomplete |

**Impact:** Users cannot form a coherent mental model. Projects appear differently in each entry point. No URL-based routing in the SolidJS app (state in localStorage only — can't bookmark or share links).

---

## 2. Intended vs Actual Information Architecture

### ADR-052 Intended Hierarchy (Project-Centric)
```
Control Plane
  └── Project: {name}                    ← THE MAIN ENTITY
        ├── Agents (local + remote)      ← agents belong to projects
        │     └── Worktrees              ← agents own worktrees
        │           └── Commits          ← worktrees own commits
        ├── Swarms (HexFlo)              ← orchestrate agents on project
        │     └── Tasks → Agents         ← tasks assigned to agents
        ├── ADRs                         ← project architecture decisions
        ├── WorkPlans                    ← decomposed feature specs
        ├── Health Analysis              ← hex architecture score
        ├── Dependency Graph
        ├── Chat Sessions
        └── Configuration
              ├── Blueprint, Skills, Hooks
              ├── Agent Definitions
              └── MCP Tools
```

### Actual Navigation (SolidJS dashboard.html)
```
Control Plane (flat grid of cards)
  ├── Project cards (click → project detail)
  │     ├── Overview (basic info)
  │     ├── Files (file tree)
  │     ├── ADRs (list + viewer)
  │     ├── Health (score ring)
  │     ├── Graph (dependency viz)
  │     ├── Chat (scoped chat)
  │     └── Config (7 sub-sections)
  ├── Agent Fleet (GLOBAL, not per-project)    ← WRONG: should be project-scoped
  ├── Inference (GLOBAL)
  ├── Fleet Nodes (GLOBAL)
  └── Workplans (GLOBAL)                       ← WRONG: should be project-scoped
```

### Gaps

| Expected (ADR-052) | Actual | Severity |
|---------------------|--------|----------|
| Agents belong to projects | Agent Fleet is global, not project-scoped | **CRITICAL** |
| Agents own worktrees | No agent→worktree relationship in UI | **CRITICAL** |
| Worktrees own commits | Worktrees shown flat, no commit drill-down | **HIGH** |
| ADR → WorkPlan → HexFlo workflow visible | Three separate disconnected views | **HIGH** |
| Swarms show task→agent→worktree→commit chain | Swarm monitor shows tasks only | **HIGH** |
| Project is THE main entity | Control Plane treats everything as peer-level | **CRITICAL** |
| Command palette as universal escape hatch | Tiny 10px "Ctrl+P" badge, poor discoverability | **MEDIUM** |
| 272px sidebar with icon-mode collapse to 48px | Fixed w-52 (208px), no collapse | **LOW** |

---

## 3. Data Model vs UI Model Mismatch

The SpacetimeDB data model **already supports** the project-centric hierarchy:

```
SpacetimeDB Tables (CORRECT):
  Project ──→ Swarm (via project_id)
  Swarm ──→ SwarmTask (via swarm_id)
  Swarm ──→ SwarmAgent (via swarm_id, has worktree_path)
  SwarmTask ──→ SwarmAgent (via agent_id assignment)
  Project ──→ ProjectConfig (via project_id)
  Project ──→ SkillEntry (via project_id)
  Project ──→ AgentDef (via project_id)
  WorkplanExecution ──→ WorkplanTask (via workplan_id)
```

But the dashboard **does not render these relationships**:
- Agent Fleet page queries `agent-registry` module (global), ignoring `SwarmAgent` (project-scoped)
- Swarm monitor shows tasks but not the agent→worktree→commit chain
- WorkplanView is a separate global page, not linked to swarms or projects
- No drill-down from Project → its Swarms → their Tasks → assigned Agents → Agent's Worktree → Commits

---

## 4. Layout & Usability Issues

### Navigation Confusion
- **Left sidebar** has two nav modes: global (All Projects, Agent Fleet, etc.) and project-scoped (Overview, Files, ADRs, etc.) — but the transition is jarring
- **Horizontal tab bar** (ProjectLayout.tsx) duplicates sidebar items for project sub-pages
- **Breadcrumbs** exist but are auto-generated and shallow (max 2 levels)

### Tiling Pane System — Over-Engineered
- Max 4 panes with draggable dividers — mimics IDE but adds complexity without clear benefit
- Users must use keyboard shortcuts (Ctrl+\, Ctrl+-) or command palette to manage panes
- Pane state persisted in localStorage — fragile, no URL routing
- **Tab types** (9 pane types) overlap with route-based pages — two navigation systems competing

### Panel Discoverability
| Panel | How to Find It | Severity |
|-------|---------------|----------|
| Dashboard (in chat.html) | 32×32px unlabeled grid icon | HIGH |
| File browser | 2-step: open dashboard → click Files | CRITICAL |
| Command palette | Tiny 10px badge or Ctrl+P | HIGH |
| Chat (in dashboard.html) | Command palette or Ctrl+Shift+C | HIGH |
| Inference/Fleet | Command palette or click section title | MEDIUM |

### Visual Inconsistency
- **Two styling systems**: Vanilla JS uses CSS custom properties (blue #58a6ff), SolidJS uses Tailwind (cyan-500/green-500)
- **Three chat implementations**: index.html (500px fixed card), chat.html (full streaming), dashboard.html (ChatView pane + broken BottomBar)
- **No consistent color language** for hex architecture layers in the actual UI (ADR-052 specifies Domain=blue, Ports=purple, etc.)

---

## 5. The Correct Mental Model

The user's description captures the intended hierarchy perfectly:

```
PROJECT (the main entity)
  │
  ├── Governance Layer
  │   ├── ADRs (architecture decisions)
  │   ├── WorkPlans (feature decomposition)
  │   └── Behavioral Specs (acceptance criteria)
  │
  ├── Execution Layer
  │   ├── Swarms (HexFlo orchestration)
  │   │   └── Tasks (adapter-bounded work units)
  │   │       └── Agents (assigned workers)
  │   │           └── Worktrees (isolated git branches)
  │   │               └── Commits (atomic changes)
  │   └── Workflow: ADR → WorkPlan → HexFlo Swarm → Tasks → Code
  │
  ├── Quality Layer
  │   ├── Architecture Health (hex boundary analysis)
  │   ├── Dependency Graph
  │   └── Validation Verdicts
  │
  └── Configuration Layer
      ├── Skills, Hooks, Agent Definitions
      ├── MCP Tools
      └── Inference Providers
```

---

## 6. Recommended Remediation (Priority Order)

### P0 — Consolidate to Single Entry Point
- **Kill `index.html` and `chat.html`** — move all functionality into the SolidJS `dashboard.html`
- Single SPA with hash routing (already started in router.ts)
- One WebSocket connection strategy, one styling system (Tailwind)

### P1 — Project-Centric Navigation Redesign
- **Project is the root entity** — everything else is scoped to a project
- Sidebar: Project list → click project → all sub-pages appear
- Remove global "Agent Fleet" — agents are project-scoped (or show aggregated view with project grouping)
- Add: Project → Swarms → Tasks → Agents → Worktrees → Commits drill-down

### P2 — Render the ADR → WorkPlan → HexFlo Pipeline
- Show the governance→execution flow visually within each project
- ADR page should link to WorkPlans that implement it
- WorkPlan page should link to the HexFlo Swarm executing it
- Swarm page should show task→agent→worktree→commit chain

### P3 — Kill the Tiling Pane System (or Simplify)
- Replace with simpler tab-based navigation within the project context
- The pane system adds complexity without solving a real user problem
- If kept, limit to 2 panes max (main + detail sidebar)

### P4 — Implement URL Routing
- Every page should have a URL (#/project/{id}/swarms/{swarmId}/tasks/{taskId})
- Enable bookmarking, sharing, browser back/forward
- Remove localStorage-based state persistence for navigation

### P5 — Design System Enforcement
- Unified Tailwind-only styling
- Hex layer colors consistently applied (Domain=blue, Ports=purple, etc.)
- Status colors standardized (green/cyan/yellow/red/gray per ADR-052)

---

## 7. Validation Score

| Category | Score | Notes |
|----------|-------|-------|
| Information Architecture | 35/100 | Three apps, no project-centric model |
| Navigation | 40/100 | Two competing nav systems (sidebar + panes) |
| Data Model Utilization | 30/100 | SpacetimeDB has the right tables, UI ignores relationships |
| Visual Consistency | 45/100 | Two styling systems, three chat implementations |
| Discoverability | 35/100 | Hidden panels, undocumented shortcuts |
| ADR Compliance | 50/100 | ADR-046 (SpacetimeDB authority) partially followed, ADR-052 (AIIDE vision) barely implemented |
| **Overall** | **39/100** | **FAIL** |

---

## 8. SpacetimeDB Tables Available (Already Correct)

The backend data model already supports the project-centric hierarchy. The problem is entirely in the frontend:

| Table | Module | Supports |
|-------|--------|----------|
| `Project` | hexflo-coordination | Project entity |
| `Swarm` (has project_id) | hexflo-coordination | Project → Swarm |
| `SwarmTask` (has swarm_id, agent_id) | hexflo-coordination | Swarm → Task → Agent |
| `SwarmAgent` (has swarm_id, worktree_path) | hexflo-coordination | Agent → Worktree |
| `WorkplanExecution` | workplan-state | WorkPlan lifecycle |
| `WorkplanTask` | workplan-state | WorkPlan → Task |
| `Agent` + `AgentHeartbeat` | agent-registry | Global agent lifecycle |
| `RemoteAgent` | hexflo-coordination | Remote agent via SSH |
| `SkillEntry`, `AgentDef` (have project_id) | hexflo-coordination | Project-scoped config |
| `ChatSession` (has project_id) | chat-relay | Project-scoped chat |
| `InferenceRequest/Response` | inference-gateway | Token tracking |

Git worktrees and commits are served via REST (`GET /api/{project_id}/git/worktrees`, `/git/log`) since they require filesystem access (WASM can't read git).
