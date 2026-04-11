# Dashboard Views vs CLI/MCP Coverage Audit

**Date:** 2026-03-22
**Auditor:** Claude Opus 4.6 (automated)

## View Components Inventory

### `/hex-nexus/assets/src/components/views/`

| Component | Data Shown | Data Source | CLI Equivalent | MCP Equivalent |
|-----------|-----------|-------------|----------------|----------------|
| `ControlPlane.tsx` | Multi-project overview: project cards with swarm/agent/task counts, connection status (SpacetimeDB, Agent Registry, Inference, Fleet) | SpacetimeDB subscriptions (`swarms`, `swarmTasks`, `swarmAgents`, `registryAgents`), projects store | `hex status` (partial) | `hex_status` (partial) |
| `AgentFleet.tsx` | All registered agents split into LOCAL/REMOTE sections with status, role, model, uptime, heartbeat, current task, terminate button | SpacetimeDB `registryAgents`, `swarmTasks` | `hex agent list` | `hex_agent_list` (if exists) |
| `ADRBrowser.tsx` | ADR list with search/filter, detail view with markdown content, workplan cross-references, inline editing | REST `/api/adrs`, `/api/adrs/:id/content`, `/api/workplan/list`, SpacetimeDB for save sync | `hex adr list`, `hex adr status <id>`, `hex adr search <q>` | `hex_adr_list`, `hex_adr_status`, `hex_adr_search` |
| `ConfigPage.tsx` | 7-section config browser: Blueprint, MCP Tools, Hooks, Skills, Context, Agent Defs, SpacetimeDB | REST `/api/config/sync` + sub-component fetches | No CLI equivalent | No MCP equivalent |
| `FileTreeView.tsx` | Interactive file browser with directory tree, file preview, markdown rendering | REST `/api/files?path=...&list=true`, `/api/files?path=...` | No CLI equivalent | No MCP equivalent |
| `ProjectDetail.tsx` | Project detail: health grade, branch picker, agents, worktrees, commits, diff viewer | SpacetimeDB `registryAgents`, health store, git store (worktrees, log), REST for diffs | `hex status`, `hex analyze .` (partial) | `hex_status`, `hex_analyze` (partial) |
| `ProjectHierarchy.tsx` | Agent-Worktree-Commit hierarchy tree for a project | Props from ProjectDetail (agents, worktrees, commits) | No CLI equivalent | No MCP equivalent |
| `WorkplanView.tsx` | Workplan execution dashboard: active banner, execution history, workplan files list, execute/pause/resume controls | REST `/api/workplans`, `/api/workplan/*` (workplan store) | `hex plan list`, `hex plan status` | No MCP equivalent |

### `/hex-nexus/assets/src/components/project/`

| Component | Data Shown | Data Source | CLI Equivalent | MCP Equivalent |
|-----------|-----------|-------------|----------------|----------------|
| `ProjectOverview.tsx` | Project cards grid with stats (projects/agents/swarms), register/hide/archive/delete | SpacetimeDB `registryAgents`, `swarms`, projects store | `hex project list` (partial) | No MCP equivalent |
| `ProjectCard.tsx` | Individual project card with health indicator, actions menu | Props from parent | N/A (sub-component) | N/A |
| `AgentList.tsx` | Project-scoped agent list with heartbeat-based liveness (online/stale/dead), toggle for all/active | SpacetimeDB `registryAgents`, `swarmAgents`, `agentHeartbeats` | `hex agent list` (partial) | No MCP equivalent |
| `AgentDetail.tsx` | Single agent detail: metadata, assigned tasks, worktree, recent commits | SpacetimeDB `swarmAgents`, `swarmTasks`, `registryAgents`, `agentHeartbeats`, REST for git log | `hex agent id` (partial) | No MCP equivalent |
| `AgentLog.tsx` | Agent inspector: live status, controls (kill/restart/reassign), tasks, heartbeat, scoped memory | SpacetimeDB `registryAgents`, `agentHeartbeats`, `swarmTasks`, `hexfloMemory`, REST for agent control | No CLI equivalent | No MCP equivalent |
| `TaskBoard.tsx` | Kanban-style task board (pending/in_progress/completed/failed) for a swarm | SpacetimeDB `swarmTasks`, `swarmAgents` | `hex task list` (partial) | `hex_task_list` (partial) |
| `SwarmDetail.tsx` | Swarm detail: metadata, task list, agent roster, progress bar | SpacetimeDB `swarms`, `swarmTasks`, `swarmAgents`, `agentHeartbeats` | `hex swarm status` (partial) | `hex_swarm_status` (partial) |
| `GovernancePipeline.tsx` | Horizontal ADR -> WorkPlan -> HexFlo pipeline banner with counts | REST for ADR/workplan counts, SpacetimeDB for swarm counts | No CLI equivalent | No MCP equivalent |
| `GovernanceTimeline.tsx` | Timeline of governance events | Props/REST | No CLI equivalent | No MCP equivalent |
| `WorkPlanDetail.tsx` | Workplan detail: phases with tier/gate info, action buttons, linked swarm | REST `/api/workplans/:id`, SpacetimeDB `hexfloMemory` for swarm linkage | `hex plan status <id>` (partial) | No MCP equivalent |
| `BranchPicker.tsx` | Git branch selector dropdown | REST git API | No CLI equivalent | No MCP equivalent |
| `FileTree.tsx` | Sub-component file tree | REST file API | N/A (sub-component) | N/A |
| `ProjectLayout.tsx` | Layout wrapper for project pages | N/A (layout) | N/A | N/A |
| `ProjectSidebar.tsx` | Project navigation sidebar | N/A (navigation) | N/A | N/A |
| `ProjectHome.tsx` | Project home page wrapper | Composition of sub-components | N/A | N/A |

### Other Component Directories

| Directory | Key Components | Data Source | CLI Equivalent | MCP Equivalent |
|-----------|---------------|-------------|----------------|----------------|
| `health/` | `HealthPane.tsx` — Score ring, violation breakdown, stat boxes | Health store (REST `/api/analyze`) | `hex analyze .` | `hex_analyze` |
| `fleet/` | `FleetView.tsx` — Compute node cards (hostname, agents, health); `InferencePanel.tsx` — Provider cards, model lists, RPM/TPM meters, cost tracking, token stats | SpacetimeDB `fleet-state`, `inference-gateway` subscriptions, REST | No CLI for fleet; `hex inference list` for inference | No MCP for fleet; partial for inference |
| `swarm/` | `SwarmMonitor.tsx` — Phase progress + TaskDAG + Timeline; `SwarmInitDialog.tsx` — Create new swarm; `TaskDAG.tsx` — Dependency graph visualization; `SwarmTimeline.tsx` — Event timeline | SpacetimeDB `swarms`, `swarmTasks`, `swarmAgents` | `hex swarm status` (text only) | `hex_swarm_status` (text only) |
| `chat/` | `ChatView.tsx` — Full chat interface with sessions, model selector, streaming; `ProjectChatWidget.tsx` — Inline project chat | WebSocket chat store, projects store, git store | No CLI equivalent | No MCP equivalent |
| `config/` | `BlueprintView.tsx`, `MCPToolsView.tsx`, `HooksView.tsx`, `SkillsView.tsx`, `ContextView.tsx`, `AgentDefsView.tsx`, `SpacetimeDBView.tsx` | REST config APIs | `hex skill list` (skills only) | No MCP equivalent |

## Dashboard-Only Features (No CLI/MCP Access)

These features are visible in the dashboard but have **no CLI or MCP tool equivalent**:

1. **Chat Interface** (`chat/ChatView.tsx`, `ProjectChatWidget.tsx`) — Full conversational AI interface with session management, model selection, streaming responses. No CLI chat command exists.

2. **File Browser** (`views/FileTreeView.tsx`) — Interactive file tree with preview and markdown rendering. No `hex files` command exists.

3. **Agent Inspector/Log** (`project/AgentLog.tsx`) — Live agent status with kill/restart/reassign controls and scoped memory viewer. No `hex agent inspect` command exists.

4. **Configuration Browser** (`views/ConfigPage.tsx` + `config/*`) — Visual browser for Blueprint, MCP Tools, Hooks, Skills, Context files, Agent Definitions, and SpacetimeDB status. Only `hex skill list` covers skills; the other 6 sections have no CLI.

5. **Governance Pipeline** (`project/GovernancePipeline.tsx`, `GovernanceTimeline.tsx`) — Visual ADR -> WorkPlan -> HexFlo flow with counts and status. No CLI equivalent for the aggregated pipeline view.

6. **Task DAG Visualization** (`swarm/TaskDAG.tsx`) — Dependency graph visualization of swarm tasks. `hex task list` shows flat list only.

7. **Swarm Timeline** (`swarm/SwarmTimeline.tsx`) — Chronological event log for swarm activity. No CLI equivalent.

8. **Fleet Management** (`fleet/FleetView.tsx`) — Compute node registration, health monitoring, agent distribution across nodes. No `hex fleet` command exists.

9. **Inference Monitoring** (`fleet/InferencePanel.tsx`) — Provider health, model lists, RPM/TPM meters, cost tracking, token budgets. `hex inference list` exists but lacks cost/token analytics.

10. **Branch Picker** (`project/BranchPicker.tsx`) — Git branch switching within the dashboard. No CLI equivalent needed (git handles this natively).

11. **Diff Viewer** (`code/DiffViewer.tsx`) — Visual diff viewer within project detail. No CLI equivalent (git diff handles this natively).

12. **Project Hierarchy View** (`views/ProjectHierarchy.tsx`) — Agent-Worktree-Commit tree visualization. No CLI equivalent.

13. **Workplan Execution Controls** (`views/WorkplanView.tsx`) — Execute, pause, resume workplans from UI. `hex plan` has limited execution support.

14. **Project Archive/Delete** (`views/ControlPlane.tsx`, `project/ProjectOverview.tsx`) — Archive and delete projects from dashboard. No `hex project archive` or `hex project delete` CLI commands.

15. **Agent Terminate** (`views/AgentFleet.tsx`) — Kill agents from dashboard via REST `DELETE /api/agents/:id`. No `hex agent kill` CLI command.

16. **Connection Status Banner** (`views/ControlPlane.tsx`) — Real-time SpacetimeDB/Agent Registry/Inference/Fleet connection health. `hex nexus status` provides partial info.

## CLI/MCP Features Missing from Dashboard

These features exist in CLI/MCP but have **no dashboard representation**:

1. **`hex inbox list/notify/ack`** — Agent notification inbox (ADR-060). No dashboard inbox panel.

2. **`hex memory store/get/search`** — Direct memory CRUD. Dashboard shows memory read-only in AgentLog but has no dedicated memory management UI.

3. **`hex init`** — Project initialization. Dashboard has project registration but not full hex init.

4. **`hex hook *`** — Hook execution/testing. Dashboard shows hook config but no hook execution UI.

5. **`hex test *`** — Integration test runner. No dashboard test panel.

6. **`hex readme *`** — README specification management. No dashboard equivalent.

7. **`hex enforce *`** — Enforcement rule management. No dashboard equivalent.

8. **`hex stdb *`** — SpacetimeDB management. Dashboard has SpacetimeDBView in config but no direct stdb CLI parity.

9. **`hex adr abandoned`** — Stale ADR detection. Dashboard ADR browser has no abandoned filter.

10. **`hex secrets status/has`** — Secret management. No dashboard secrets panel.

## Summary

| Category | Count |
|----------|-------|
| Dashboard view components | 9 |
| Dashboard project components | 16 |
| Dashboard other component groups | 5 (health, fleet, swarm, chat, config) |
| Dashboard-only features (no CLI/MCP) | 16 |
| CLI/MCP features missing from dashboard | 10 |
| Full parity (dashboard + CLI + MCP) | 4 (ADR browse, analyze, swarm status, task list) |

**Key gap:** The dashboard is significantly richer than CLI/MCP for visualization and real-time monitoring. The CLI/MCP is richer for operational commands (inbox, hooks, enforcement, testing). The biggest dashboard-only gap is the **chat interface** and **agent lifecycle controls** (inspect/kill/restart). The biggest CLI-only gap is **inbox notifications** and **enforcement rules**.
