# Orphaned Endpoint Decisions

**Date**: 2026-03-23
**Source**: [rest-endpoint-map.md](rest-endpoint-map.md) — 60 endpoints with zero identified consumers
**Author**: Architecture review (automated classification)

## Decision Categories

| Decision | Meaning |
|----------|---------|
| **keep** | Will gain a consumer soon (dashboard pane, MCP tool, or CLI command planned) |
| **deprecate** | Add `X-Deprecated` header, keep for backward compat, sunset in 90 days (2026-06-21) |
| **remove** | Dead code, no planned consumer, safe to delete |

---

## Coordination (`/api/coordination/*`) — 13 endpoints

These support multi-instance hex-nexus coordination (worktree locking, task claiming, activity pub/sub). Implementation exists in `hex-nexus/src/coordination/mod.rs`. Used by inter-instance protocols when multiple hex-nexus daemons run concurrently (e.g., swarm worktree isolation via `feature-workflow.sh`).

| Path | Method | Decision | Reason |
|------|--------|----------|--------|
| `/api/coordination/instance/register` | POST | **keep** | Required for multi-instance protocol; will be called by `hex nexus start` when clustering is enabled |
| `/api/coordination/instance/heartbeat` | POST | **keep** | Instance liveness — paired with register |
| `/api/coordination/instances` | GET | **keep** | Debugging/dashboard visibility into active instances |
| `/api/coordination/worktree/lock` | POST | **keep** | Core worktree isolation primitive — used by `feature-workflow.sh` swarm worktrees |
| `/api/coordination/worktree/locks` | GET | **keep** | Lock visibility for debugging stale locks |
| `/api/coordination/worktree/lock/{key}` | DELETE | **keep** | Lock release — paired with acquire |
| `/api/coordination/task/claim` | POST | **deprecate** | Overlaps with HexFlo task assignment (`/api/hexflo/tasks/{id}` PATCH). Migrate to HexFlo |
| `/api/coordination/task/claim/{task_id}` | DELETE | **deprecate** | Paired with claim — same overlap with HexFlo |
| `/api/coordination/tasks` | GET | **deprecate** | Overlaps with `/api/swarms/active` task listing |
| `/api/coordination/activity` | POST | **deprecate** | Activity pub/sub never integrated; SpacetimeDB subscriptions supersede this |
| `/api/coordination/activities` | GET | **deprecate** | Paired with activity POST — same reasoning |
| `/api/coordination/unstaged` | GET | **remove** | No implementation consumers, no planned use, unclear semantics |
| `/api/coordination/cleanup` | POST | **remove** | Duplicates `/api/hexflo/cleanup` and `/api/hex-agents/evict` |

## RL (`/api/rl/*`) — 6 endpoints

Reinforcement learning engine for model selection and token management (ADR-031). The ADR exists but the RL system has no consumers — no CLI commands, no MCP tools, no dashboard panes, and no hooks integrate with it.

| Path | Method | Decision | Reason |
|------|--------|----------|--------|
| `/api/rl/action` | POST | **deprecate** | ADR-031 accepted but no consumer implemented; keep API contract for future integration |
| `/api/rl/reward` | POST | **deprecate** | Paired with action — same reasoning |
| `/api/rl/stats` | GET | **deprecate** | Monitoring endpoint for RL — useful if RL is reactivated |
| `/api/rl/patterns` | GET/POST | **deprecate** | Pattern storage for RL — useful if RL is reactivated |
| `/api/rl/patterns/{id}/reinforce` | POST | **deprecate** | Pattern reinforcement — useful if RL is reactivated |
| `/api/rl/decay` | POST | **deprecate** | Decay is a maintenance operation — useful if RL is reactivated |

## Per-Project Queries (`/api/{project_id}/*`) — 5 endpoints

These serve project-scoped data for the dashboard. The health endpoint has CLI + dashboard consumers; the remaining 5 are orphaned.

| Path | Method | Decision | Reason |
|------|--------|----------|--------|
| `/api/{project_id}/tokens/overview` | GET | **keep** | Dashboard InferencePanel planned to consume this (token usage per project) |
| `/api/{project_id}/tokens/{file}` | GET | **keep** | Drill-down from tokens/overview — paired endpoint |
| `/api/{project_id}/swarm` | GET | **deprecate** | Overlaps with `/api/swarms/active` which already has CLI+MCP consumers; migrate dashboard to that |
| `/api/{project_id}/graph` | GET | **keep** | Dashboard dependency graph visualization (GraphView pane) is planned |
| `/api/{project_id}/project` | GET | **deprecate** | Overlaps with `/api/projects` list which returns same metadata |

## Fleet (`/api/fleet/*`) — 5 endpoints

Remote compute node management. Active workplans exist (`feat-remote-agent-spawn.json`, `feat-remote-agent-transport.json`, `feat-remote-agent-remaining.json`). Dashboard already consumes register and unregister.

| Path | Method | Decision | Reason |
|------|--------|----------|--------|
| `/api/fleet` | GET | **keep** | Fleet listing — dashboard FleetView will consume once remote-agent workplan completes |
| `/api/fleet/health` | POST | **keep** | Node health probing — required for fleet management |
| `/api/fleet/select` | GET | **keep** | Best-node selection for `hex agent spawn-remote` — active workplan |
| `/api/fleet/{id}` | GET | **keep** | Node detail — paired with fleet list |
| `/api/fleet/{id}/deploy` | POST | **keep** | Deploy agent to remote node — core remote-agent feature |

## Secrets (`/secrets/*`) — 5 endpoints

Secret broker (ADR-026). The claim/grant/revoke pattern is the single-use secret injection protocol. CLI consumes vault set/get; the broker endpoints have no consumers.

| Path | Method | Decision | Reason |
|------|--------|----------|--------|
| `/secrets/claim` | POST | **keep** | Core secret injection protocol — agents will call this when spawned on remote nodes |
| `/secrets/grant` | POST | **keep** | Paired with claim — nexus grants secrets to authenticated agents |
| `/secrets/revoke` | POST | **keep** | Security: revoke compromised grants |
| `/secrets/grants` | GET | **keep** | Audit trail — dashboard SecurityView planned |
| `/api/secrets/health` | GET | **keep** | Health probe for secrets subsystem — consistent with other health endpoints |

## Commands (`/api/{project_id}/command*`) — 4 endpoints

Browser-to-project command dispatch. The dashboard CommandBar sends commands; projects poll for them. This is the bidirectional control channel.

| Path | Method | Decision | Reason |
|------|--------|----------|--------|
| `/api/{project_id}/command` | POST | **keep** | Dashboard command dispatch — used by CommandBar component |
| `/api/{project_id}/command/{command_id}` | GET | **keep** | Project polls for command details |
| `/api/{project_id}/command/{command_id}/result` | POST | **keep** | Project reports command result back to dashboard |
| `/api/{project_id}/commands` | GET | **keep** | List pending commands — project polling endpoint |

## Analysis — 3 endpoints

| Path | Method | Decision | Reason |
|------|--------|----------|--------|
| `/api/{project_id}/analyze` | GET | **keep** | Project-scoped analysis for dashboard HealthPane — alternative to POST `/api/analyze` |
| `/api/{project_id}/analyze/text` | GET | **remove** | Text-format duplicate of JSON analyze endpoint; no consumer and JSON is preferred |
| `/api/{project_id}/analyze/adr-compliance` | GET | **remove** | Project-scoped ADR compliance; POST version at `/api/analyze/adr-compliance` is the canonical path |

## ADR Reservation — 2 endpoints

| Path | Method | Decision | Reason |
|------|--------|----------|--------|
| `/api/adr/next` | GET | **keep** | Multi-agent ADR number coordination — prevents collisions when agents create ADRs concurrently |
| `/api/adr/reserve` | POST | **keep** | Paired with next — atomically reserves the number |

## Other Orphaned Endpoints

| Path | Method | Category | Decision | Reason |
|------|--------|----------|----------|--------|
| `/api/openapi.json` | GET | Meta | **keep** | OpenAPI spec (ADR-039) — consumed by Swagger UI and frontend tooling |
| `/api/push` | POST | Push | **deprecate** | Legacy push-state protocol; SpacetimeDB subscriptions replace polling/push |
| `/api/{project_id}/decisions/{decision_id}` | POST | Decisions | **keep** | Dashboard decision handling (approve/reject) — interactive governance flow |
| `/api/stdb/health` | GET | SpacetimeDB | **keep** | SpacetimeDB connectivity probe — useful for diagnostics |
| `/api/agents/{id}` | GET | Agents | **deprecate** | Legacy agent detail; superseded by `/api/hex-agents/{id}` (ADR-065) |
| `/api/inference/complete` | POST | Inference | **keep** | Synchronous inference bridge — hex-agent HTTP calls for WASM modules |
| `/api/hex-agents/{id}/heartbeat` | POST | Hex Agents | **keep** | Agent heartbeat — called by background heartbeat timer in hex-agent runtime |
| `/api/test-sessions/flaky` | GET | Test Sessions | **keep** | Flaky test detection — dashboard TestView planned |
| `/api/work-items/incomplete` | GET | Swarm | **deprecate** | Overlaps with task list from `/api/swarms/active`; no consumer |
| `/api/sessions/search` | GET | Sessions | **keep** | Session search — dashboard SessionListPanel will use for filtering |
| `/api/sessions/{id}/compact` | POST | Sessions | **keep** | Session compaction — reduces token count for long sessions |
| `/api/sessions/{id}/revert` | POST | Sessions | **keep** | Session revert — undo to checkpoint |
| `/api/sessions/{id}/archive` | POST | Sessions | **keep** | Session archival — move to cold storage |
| `/api/{project_id}/git/worktrees/{name}` | DELETE | Git | **keep** | Worktree cleanup — used by `feature-workflow.sh` merge/cleanup phases |
| `/api/{project_id}/git/log/{sha}` | GET | Git | **keep** | Commit detail — dashboard AgentDetail links to specific commits |
| `/api/{project_id}/git/task-commits` | GET | Git | **keep** | Task-commit correlation — links HexFlo tasks to git commits |
| `/api/{project_id}/git/violation-blame` | POST | Git | **keep** | Blame violations to specific commits/authors — architecture enforcement |
| `/api/{project_id}/git/timeline` | GET | Git | **keep** | Git timeline visualization — dashboard GovernanceTimeline planned |
| `/api/inference/endpoints/{id}` | DELETE | Inference | **keep** | Remove stale inference endpoint — admin operation |

---

## Summary

| Decision | Count | Percentage |
|----------|-------|------------|
| **keep** | 41 | 68% |
| **deprecate** | 15 | 25% |
| **remove** | 4 | 7% |

### Action Items

1. **Remove (4 endpoints)** — Delete handler code and route registrations:
   - `/api/coordination/unstaged` GET
   - `/api/coordination/cleanup` POST
   - `/api/{project_id}/analyze/text` GET
   - `/api/{project_id}/analyze/adr-compliance` GET

2. **Deprecate (15 endpoints)** — Add `X-Deprecated: true` + `Sunset: 2026-06-21` headers via `deprecation_layer`:
   - 5 coordination endpoints (task/claim, activity)
   - 6 RL endpoints (entire subsystem)
   - `/api/{project_id}/swarm` GET
   - `/api/{project_id}/project` GET
   - `/api/push` POST
   - `/api/agents/{id}` GET
   - `/api/work-items/incomplete` GET

3. **Keep (41 endpoints)** — No action needed now; track consumer addition in workplans:
   - Fleet (5) — blocked on remote-agent workplan
   - Secrets broker (5) — blocked on remote-agent secrets injection
   - Commands (4) — dashboard CommandBar integration
   - Coordination worktree locks (6) — multi-instance protocol
   - ADR reservation (2) — multi-agent coordination
   - Remaining (19) — various dashboard panes and CLI commands planned
