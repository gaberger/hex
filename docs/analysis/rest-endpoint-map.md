# REST Endpoint Map — All Consumers

**Generated**: 2026-03-22
**Source**: `hex-nexus/src/routes/mod.rs` (`build_router()`)

## Legend

| Symbol | Meaning |
|--------|---------|
| CLI | Called from `hex-cli/src/commands/*.rs` via nexus_client |
| MCP | Called from `hex-cli/src/commands/mcp.rs` dispatch |
| Dash | Called from `hex-nexus/assets/src/` (dashboard frontend) |
| Hook | Called from `hex-cli/src/commands/hook.rs` |

---

## Meta / Static

| Endpoint | Method | CLI | MCP | Dash | Hook | Notes |
|----------|--------|-----|-----|------|------|-------|
| `/api/openapi.json` | GET | | | | | OpenAPI spec (ADR-039) |
| `/api/docs` | GET | | | | | Swagger UI redirect |
| `/api/version` | GET | nexus, status | mcp | nexus-health | | Version check |
| `/api/health` | GET | | | | hook | Lightweight health probe |
| `/api/tools` | GET | | | | | MCP tool registry; dashboard test command reads it |

## Project Management

| Endpoint | Method | CLI | MCP | Dash | Hook | Notes |
|----------|--------|-----|-----|------|------|-------|
| `/api/projects` | GET | project, analyze | | | | List projects |
| `/api/projects/register` | POST | project | | git store | | Register project |
| `/api/projects/init` | POST | init | | projects store | | Initialize project |
| `/api/projects/{id}` | DELETE | project | | | | Unregister project |
| `/api/projects/{id}/archive` | POST | | | projects store | | Archive project |
| `/api/projects/{id}/delete` | POST | | | projects store | | Delete project |

## Push / Event (Project -> Hub)

| Endpoint | Method | CLI | MCP | Dash | Hook | Notes |
|----------|--------|-----|-----|------|------|-------|
| `/api/push` | POST | | | | | Push state from project |
| `/api/event` | POST | | | | hook (as `/api/events`) | Push events |

## Per-Project Queries (Browser)

| Endpoint | Method | CLI | MCP | Dash | Hook | Notes |
|----------|--------|-----|-----|------|------|-------|
| `/api/{project_id}/health` | GET | analyze | | health store | | Project health |
| `/api/{project_id}/tokens/overview` | GET | | | | | Token overview |
| `/api/{project_id}/tokens/{file}` | GET | | | | | Token file detail |
| `/api/{project_id}/swarm` | GET | | | | | Project swarm state |
| `/api/{project_id}/graph` | GET | | | | | Dependency graph |
| `/api/{project_id}/project` | GET | | | | | Project metadata |

## Architecture Analysis

| Endpoint | Method | CLI | MCP | Dash | Hook | Notes |
|----------|--------|-----|-----|------|------|-------|
| `/api/analyze` | POST | | mcp | health, commands, graph stores | | Analyze path |
| `/api/{project_id}/analyze` | GET | | | | | Analyze project (JSON) |
| `/api/{project_id}/analyze/text` | GET | | | | | Analyze project (text) |
| `/api/analyze/adr-compliance` | POST | | | | | ADR compliance check |
| `/api/{project_id}/analyze/adr-compliance` | GET | | | | | Project ADR compliance |

## ADR Number Reservation

| Endpoint | Method | CLI | MCP | Dash | Hook | Notes |
|----------|--------|-----|-----|------|------|-------|
| `/api/adr/next` | GET | | | | | Next ADR number |
| `/api/adr/reserve` | POST | | | | | Reserve ADR number |

## Commands (Browser/MCP -> Hub -> Project)

| Endpoint | Method | CLI | MCP | Dash | Hook | Notes |
|----------|--------|-----|-----|------|------|-------|
| `/api/{project_id}/command` | POST | | | | | Send command |
| `/api/{project_id}/command/{command_id}` | GET | | | | | Get command |
| `/api/{project_id}/command/{command_id}/result` | POST | | | | | Report result |
| `/api/{project_id}/commands` | GET | | | | | List commands |

## Decisions

| Endpoint | Method | CLI | MCP | Dash | Hook | Notes |
|----------|--------|-----|-----|------|------|-------|
| `/api/{project_id}/decisions/{decision_id}` | POST | | | | | Handle decision |

## SpacetimeDB

| Endpoint | Method | CLI | MCP | Dash | Hook | Notes |
|----------|--------|-----|-----|------|------|-------|
| `/api/stdb/hydrate` | POST | stdb | | | | Hydrate SpacetimeDB |
| `/api/stdb/health` | GET | | | | | SpacetimeDB health |

## Swarm / HexFlo

| Endpoint | Method | CLI | MCP | Dash | Hook | Notes |
|----------|--------|-----|-----|------|------|-------|
| `/api/swarms` | POST | swarm | mcp | SwarmInitDialog | | Create swarm |
| `/api/swarms/active` | GET | swarm, nexus, agent, task | mcp | | | List active swarms |
| `/api/swarms/{id}` | GET | | mcp | | | Get swarm detail |
| `/api/swarms/{id}` | PATCH | | | | | Complete swarm |
| `/api/swarms/{id}/tasks` | POST | task | mcp | | | Create task |
| `/api/swarms/{id}/tasks/{task_id}` | PATCH | task | | | hook | Update task |
| `/api/hexflo/tasks/{task_id}` | PATCH | | mcp | | hook | Update task (no swarm ID) |
| `/api/work-items/incomplete` | GET | | | | | Incomplete work items |

## Coordination

| Endpoint | Method | CLI | MCP | Dash | Hook | Notes |
|----------|--------|-----|-----|------|------|-------|
| `/api/coordination/instance/register` | POST | | | | | Register instance |
| `/api/coordination/instance/heartbeat` | POST | | | | | Instance heartbeat |
| `/api/coordination/instances` | GET | | | | | List instances |
| `/api/coordination/worktree/lock` | POST | | | | | Acquire lock |
| `/api/coordination/worktree/locks` | GET | | | | | List locks |
| `/api/coordination/worktree/lock/{key}` | DELETE | | | | | Release lock |
| `/api/coordination/task/claim` | POST | | | | | Claim task |
| `/api/coordination/task/claim/{task_id}` | DELETE | | | | | Release task |
| `/api/coordination/tasks` | GET | | | | | List claims |
| `/api/coordination/activity` | POST | | | | | Publish activity |
| `/api/coordination/activities` | GET | | | | | Get activities |
| `/api/coordination/unstaged` | GET | | | | | Get unstaged |
| `/api/coordination/cleanup` | POST | | | | | Cleanup stale sessions |

## RL (Reinforcement Learning)

| Endpoint | Method | CLI | MCP | Dash | Hook | Notes |
|----------|--------|-----|-----|------|------|-------|
| `/api/rl/action` | POST | | | | | Select action |
| `/api/rl/reward` | POST | | | | | Submit reward |
| `/api/rl/stats` | GET | | | | | Get stats |
| `/api/rl/patterns` | GET/POST | | | | | Search/store patterns |
| `/api/rl/patterns/{id}/reinforce` | POST | | | | | Reinforce pattern |
| `/api/rl/decay` | POST | | | | | Decay patterns |

## Agent Orchestration

| Endpoint | Method | CLI | MCP | Dash | Hook | Notes |
|----------|--------|-----|-----|------|------|-------|
| `/api/agents/spawn` | POST | | | AgentLog, SpawnDialog | | Spawn agent |
| `/api/agents/health` | POST | | | | | Agent health check |
| `/api/agents` | GET | nexus | mcp | | test | DEPRECATED (ADR-039): list agents |
| `/api/agents/{id}` | GET | | | | | Get agent detail |
| `/api/agents/{id}` | DELETE | | | AgentFleet, AgentCard | test | Terminate agent |
| `/api/agents/connect` | POST | | mcp | | hook | DEPRECATED (ADR-065): use hex-agents |
| `/api/agents/disconnect` | POST | | mcp | | hook | Disconnect agent |
| `/api/agents/spawn-remote` | POST | agent | | | | Spawn remote agent (not in router -- CLI-only?) |
| `/api/agents/fleet` | GET | agent | | | | Fleet capacity (not in router -- CLI-only?) |

## Workplan

| Endpoint | Method | CLI | MCP | Dash | Hook | Notes |
|----------|--------|-----|-----|------|------|-------|
| `/api/workplan/execute` | POST | plan | mcp | WorkPlanDetail | | Execute workplan |
| `/api/workplan/status` | GET | | | | | Workplan status |
| `/api/workplan/pause` | POST | | mcp | WorkPlanDetail, workplan store | | Pause workplan |
| `/api/workplan/resume` | POST | | mcp | WorkPlanDetail, workplan store | | Resume workplan |
| `/api/workplan/list` | GET | plan | mcp | workplan store, ADRBrowser, GovernanceTimeline | | List workplans |
| `/api/workplan/{id}` | GET | | mcp | | | Get workplan |
| `/api/workplan/{id}/report` | GET | plan | mcp | workplan store | | Workplan report |
| `/api/workplans` | GET | | | WorkplanView, WorkPlanDetail | | Workplan file definitions |

## Fleet

| Endpoint | Method | CLI | MCP | Dash | Hook | Notes |
|----------|--------|-----|-----|------|------|-------|
| `/api/fleet` | GET | | | | | List fleet nodes |
| `/api/fleet/register` | POST | | | FleetView | | Register node |
| `/api/fleet/health` | POST | | | | | Fleet health check |
| `/api/fleet/select` | GET | | | | | Select best node |
| `/api/fleet/{id}` | GET | | | | | Get node |
| `/api/fleet/{id}` | DELETE | | | FleetView | | Unregister node |
| `/api/fleet/{id}/deploy` | POST | | | | | Deploy to node |

## Secrets / Vault

| Endpoint | Method | CLI | MCP | Dash | Hook | Notes |
|----------|--------|-----|-----|------|------|-------|
| `/secrets/claim` | POST | | | | | Claim secrets |
| `/secrets/grant` | POST | | | | | Grant secret |
| `/secrets/revoke` | POST | | | | | Revoke secret |
| `/secrets/grants` | GET | | | | | List grants |
| `/api/secrets/health` | GET | | | | | Secrets health |
| `/api/secrets/vault` | POST | secrets | | | | Store secret |
| `/api/secrets/vault/{key}` | GET | secrets | | | | Retrieve secret |

## Inference

| Endpoint | Method | CLI | MCP | Dash | Hook | Notes |
|----------|--------|-----|-----|------|------|-------|
| `/api/inference/register` | POST | inference | mcp | | | Register provider |
| `/api/inference/endpoints` | GET | nexus, inference | mcp | ModelSelector | | List endpoints |
| `/api/inference/endpoints/{id}` | DELETE | | | | | Remove endpoint |
| `/api/inference/health` | POST | | | InferencePanel | | Check inference health |
| `/api/inference/complete` | POST | | | | | Synchronous inference completion |

## Git Integration (ADR-044)

| Endpoint | Method | CLI | MCP | Dash | Hook | Notes |
|----------|--------|-----|-----|------|------|-------|
| `/api/{project_id}/git/status` | GET | | | git store | | Git status |
| `/api/{project_id}/git/log` | GET | | | git store, AgentDetail, ProjectHome | | Git log |
| `/api/{project_id}/git/diff` | GET | | | git store | | Git diff |
| `/api/{project_id}/git/diff/{refspec}` | GET | | | git store | | Git diff refs |
| `/api/{project_id}/git/branches` | GET | | | git store | | Git branches |
| `/api/{project_id}/git/worktrees` | GET/POST | | | git store | | Git worktrees |
| `/api/{project_id}/git/worktrees/{name}` | DELETE | | | | | Delete worktree |
| `/api/{project_id}/git/log/{sha}` | GET | | | | | Commit detail |
| `/api/{project_id}/git/task-commits` | GET | | | | | Task commits |
| `/api/{project_id}/git/violation-blame` | POST | | | | | Violation blame |
| `/api/{project_id}/git/timeline` | GET | | | | | Git timeline |

## ADR (Architecture Decision Records)

| Endpoint | Method | CLI | MCP | Dash | Hook | Notes |
|----------|--------|-----|-----|------|------|-------|
| `/api/adrs` | GET | | mcp | ADRBrowser, GovernanceTimeline | | List ADRs |
| `/api/adrs/{id}` | GET/PUT | | mcp | ADRBrowser | | Get/save ADR |
| `/api/projects/{id}/adrs` | GET | | | ADRBrowser, GovernancePipeline, GovernanceTimeline | | Project ADRs |
| `/api/projects/{id}/adrs/{adr_id}` | GET/PUT | | | ADRBrowser | | Project ADR detail |

## Files / Config

| Endpoint | Method | CLI | MCP | Dash | Hook | Notes |
|----------|--------|-----|-----|------|------|-------|
| `/api/files` | GET/PUT | | | settings, FileTreeView, ContextView, AgentDefsView, SkillsView, WorkPlanDetail, BlueprintView | | Read/write files |
| `/api/files` | DELETE | | | SkillsView | | Delete file |
| `/api/config/sync` | POST | | | commands, ConfigPage, AgentDefsView, SkillsView | | Re-sync config |

## HexFlo Coordination (ADR-027)

| Endpoint | Method | CLI | MCP | Dash | Hook | Notes |
|----------|--------|-----|-----|------|------|-------|
| `/api/hexflo/memory` | POST | memory | mcp | | hook | Store memory |
| `/api/hexflo/memory/search` | GET | memory | mcp | | | Search memory |
| `/api/hexflo/memory/{key}` | GET | memory | mcp | | hook | Retrieve memory |
| `/api/hexflo/memory/{key}` | DELETE | | | | hook | Delete memory |
| `/api/hexflo/cleanup` | POST | | | | | Cleanup stale agents |
| `/api/hexflo/enforcement-rules` | GET/POST | enforce | | | | Enforcement rules |
| `/api/hexflo/enforcement-rules/toggle` | PATCH | enforce | | | | Toggle enforcement |

## Agent Notification Inbox (ADR-060)

| Endpoint | Method | CLI | MCP | Dash | Hook | Notes |
|----------|--------|-----|-----|------|------|-------|
| `/api/hexflo/inbox/notify` | POST | inbox | mcp | | | Send notification |
| `/api/hexflo/inbox/expire` | POST | inbox | | | | Expire stale notifications |
| `/api/hexflo/inbox/{agent_id}` | GET | inbox | mcp | | hook | Query inbox |
| `/api/hexflo/inbox/{id}/ack` | PATCH | inbox | mcp | | | Acknowledge notification |

## Unified Agent Registry (ADR-058)

| Endpoint | Method | CLI | MCP | Dash | Hook | Notes |
|----------|--------|-----|-----|------|------|-------|
| `/api/hex-agents/connect` | POST | nexus, agent | mcp | | hook | Connect agent |
| `/api/hex-agents/evict` | POST | | | | hook | Evict dead agents |
| `/api/hex-agents` | GET | agent | | | | List hex agents |
| `/api/hex-agents/{id}` | GET | agent | | | | Get hex agent |
| `/api/hex-agents/{id}` | DELETE | agent | | | | Disconnect hex agent |
| `/api/hex-agents/{id}/heartbeat` | POST | | | | | Agent heartbeat |

## Test Sessions

| Endpoint | Method | CLI | MCP | Dash | Hook | Notes |
|----------|--------|-----|-----|------|------|-------|
| `/api/test-sessions` | POST/GET | test | | | | Record/list test sessions |
| `/api/test-sessions/trends` | GET | test | | | | Test trends |
| `/api/test-sessions/flaky` | GET | | | | | Flaky tests |
| `/api/test-sessions/{id}` | GET | test | | | | Get test session |

## Sessions (ADR-036/042)

| Endpoint | Method | CLI | MCP | Dash | Hook | Notes |
|----------|--------|-----|-----|------|------|-------|
| `/api/sessions` | POST/GET | nexus | | session store | | Create/list sessions |
| `/api/sessions/search` | GET | | | | | Search sessions |
| `/api/sessions/{id}` | GET/PATCH/DELETE | | | session store | | Session CRUD |
| `/api/sessions/{id}/messages` | GET/POST | | | chat-ws service | | Session messages |
| `/api/sessions/{id}/fork` | POST | | | SessionListPanel | | Fork session |
| `/api/sessions/{id}/compact` | POST | | | | | Compact session |
| `/api/sessions/{id}/revert` | POST | | | | | Revert session |
| `/api/sessions/{id}/archive` | POST | | | | | Archive session |

## WebSocket

| Endpoint | Method | CLI | MCP | Dash | Hook | Notes |
|----------|--------|-----|-----|------|------|-------|
| `/ws` | GET | | | connection store | | Main WebSocket |
| `/ws/chat` | GET | | | chat-ws service | | Chat WebSocket |

---

## Summary

| Category | Total Endpoints | CLI | MCP | Dashboard | Hook | Zero Consumers |
|----------|----------------|-----|-----|-----------|------|----------------|
| Meta/Static | 5 | 2 | 1 | 1 | 1 | 1 |
| Projects | 6 | 4 | 0 | 3 | 0 | 0 |
| Push/Event | 2 | 0 | 0 | 0 | 1 | 1 |
| Per-Project Queries | 6 | 1 | 0 | 0 | 0 | 5 |
| Analysis | 5 | 0 | 1 | 1 | 0 | 3 |
| ADR Reservation | 2 | 0 | 0 | 0 | 0 | 2 |
| Commands | 4 | 0 | 0 | 0 | 0 | 4 |
| Decisions | 1 | 0 | 0 | 0 | 0 | 1 |
| SpacetimeDB | 2 | 1 | 0 | 0 | 0 | 1 |
| Swarm/HexFlo | 8 | 3 | 4 | 1 | 2 | 0 |
| Coordination | 13 | 0 | 0 | 0 | 0 | 13 |
| RL | 6 | 0 | 0 | 0 | 0 | 6 |
| Agent Orchestration | 8 | 1 | 3 | 2 | 2 | 1 |
| Workplan | 8 | 2 | 4 | 4 | 0 | 0 |
| Fleet | 7 | 0 | 0 | 2 | 0 | 5 |
| Secrets/Vault | 7 | 2 | 0 | 0 | 0 | 5 |
| Inference | 5 | 2 | 2 | 2 | 0 | 1 |
| Git | 11 | 0 | 0 | 6 | 0 | 5 |
| ADR | 4 | 0 | 2 | 4 | 0 | 0 |
| Files/Config | 3 | 0 | 0 | 3 | 0 | 0 |
| HexFlo Memory | 7 | 4 | 3 | 0 | 3 | 0 |
| Inbox | 4 | 3 | 3 | 0 | 1 | 0 |
| Hex Agents | 6 | 4 | 1 | 0 | 2 | 1 |
| Test Sessions | 4 | 3 | 0 | 0 | 0 | 1 |
| Sessions | 8 | 1 | 0 | 3 | 0 | 4 |
| WebSocket | 2 | 0 | 0 | 2 | 0 | 0 |
| **TOTAL** | **148** | **33** | **24** | **34** | **12** | **60** |

### Key Findings

1. **60 endpoints (41%) have zero identified consumers** -- most are in Coordination (13), RL (6), Per-Project Queries (5), Git (5), Fleet (5), Secrets (5), and Commands (4)
2. **Coordination API (13 endpoints)** has no CLI, MCP, dashboard, or hook consumers -- likely used only by inter-instance protocols
3. **RL API (6 endpoints)** has no consumers -- either not yet integrated or dead code
4. **Git API** is consumed only by dashboard -- no CLI or MCP tools for git queries
5. **Fleet API** is dashboard-only for register/unregister; 5 of 7 endpoints unused
6. **Per-Project Queries** (tokens, swarm, graph, project) are mostly unconsumed
