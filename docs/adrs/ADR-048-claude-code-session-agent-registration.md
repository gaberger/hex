# ADR-048: Claude Code Session Agent Registration

## Status: Accepted
## Date: 2026-03-21

## Context

When a developer opens a Claude Code session in a hex project, the session operates as an autonomous agent — reading files, editing code, spawning subagents, and coordinating swarms. However, **hex has no visibility into these sessions**:

- The dashboard shows spawned `hex-agent` processes (via `AgentManager`) and remote agents (via `/api/agents/connect`), but Claude Code sessions are invisible
- SpacetimeDB's `agent-registry` module tracks agents by ID with heartbeats, but no one calls `register_agent` for Claude Code sessions
- The existing `/api/agents/connect` endpoint only accepts a `host` field and hardcodes the name to `remote-{host}`, discarding session metadata (project, model, session ID)
- The existing `DELETE /api/agents/:id` route goes through `AgentManager` which manages child process PIDs — Claude Code sessions are external processes, not spawned children
- Without registration, the dashboard cannot show active developer sessions, and fleet-level coordination cannot account for sessions already working on a project

This creates a blind spot: the AIIDE knows about its own spawned agents but not about the primary agent (Claude Code) that drives all development activity.

## Decision

Implement **automatic agent registration for Claude Code sessions** using the existing hook system and hex-nexus REST API, with three coordinated changes:

### 1. Extended Connect Endpoint

Extend `POST /api/agents/connect` to accept optional metadata fields:

| Field | Type | Default | Purpose |
|-------|------|---------|---------|
| `host` | string | `"unknown"` | Hostname of the machine |
| `name` | string | `"remote-{host}"` | Display name for dashboard |
| `project_dir` | string | `""` | Project root path |
| `model` | string | `""` | LLM model identifier |
| `session_id` | string | — | Claude Code session ID (informational) |

This is backwards-compatible — existing callers that only send `host` continue to work unchanged.

### 2. New Disconnect Endpoint

Add `POST /api/agents/disconnect` that calls `state_port.agent_remove()` directly, bypassing `AgentManager` PID management. This is necessary because:

- Claude Code sessions are not child processes of hex-nexus
- `DELETE /api/agents/:id` goes through `AgentManager.terminate_agent()` which tries to kill a PID that doesn't exist in its `pid_map`
- The disconnect route only needs the `agentId` to remove the registration from SpacetimeDB/SQLite

Request body: `{ "agentId": "<uuid>" }`

### 3. Session Lifecycle Hook

A new hook helper `.claude/helpers/agent-register.cjs` wired into `SessionStart` and `SessionEnd`:

```
SessionStart → agent-register.cjs register
  1. POST /api/agents/connect with session metadata
  2. Save returned agentId to ~/.hex/sessions/agent-{sessionId}.json
  3. Print confirmation for session context

SessionEnd → agent-register.cjs deregister
  1. Read agentId from ~/.hex/sessions/agent-{sessionId}.json
  2. POST /api/agents/disconnect
  3. Delete state file
```

**Design principles:**
- **Fire-and-forget** — registration never blocks the session, even if hex-nexus is not running
- **No in-memory state** — the agentId is persisted to disk so `SessionEnd` can deregister without relying on process memory surviving across hooks
- **Follows existing patterns** — uses the same `hub.lock` auth token, timeout, and error-silencing approach as `hub-push.cjs`

### Registration Flow

```
┌─────────────────┐     POST /api/agents/connect      ┌─────────────────┐
│  Claude Code     │ ──────────────────────────────────▶│  hex-nexus       │
│  SessionStart    │  { name, project_dir, model, ... } │                  │
│  hook            │◀─────────────────────────────────  │  state_port      │
│                  │  { agentId: "uuid-..." }           │  .agent_register │
└────────┬────────┘                                     └────────┬─────────┘
         │                                                       │
         │ save agentId                                          │ SpacetimeDB
         ▼                                                       ▼
  ~/.hex/sessions/                                        agent table
  agent-{session}.json                                    agent_heartbeat
         │
         │ SessionEnd
         ▼
┌─────────────────┐     POST /api/agents/disconnect    ┌─────────────────┐
│  Claude Code     │ ──────────────────────────────────▶│  hex-nexus       │
│  SessionEnd      │  { agentId: "uuid-..." }           │                  │
│  hook            │                                     │  state_port      │
│                  │                                     │  .agent_remove   │
└─────────────────┘                                     └──────────────────┘
```

### WebSocket Broadcast

Both connect and disconnect broadcast events via the existing WebSocket system:
- `agent_connected` — includes `agentId`, `host`, `name`
- `agent_disconnected` — includes `agent_id`

This means the dashboard updates in real-time when sessions start and end.

## Consequences

### Positive

- **Full fleet visibility** — the dashboard shows all active development sessions, not just spawned agents
- **Coordination awareness** — swarm coordinators can see which sessions are active on which projects before assigning work
- **Backwards-compatible** — the extended `/connect` endpoint accepts the same minimal payload as before
- **Zero-cost when nexus is down** — the hook silently skips if the daemon isn't running
- **Clean lifecycle** — sessions register on start and deregister on end, with disk-persisted state bridging the gap

### Negative

- **No heartbeat** — Claude Code sessions register once but don't send periodic heartbeats. If a session crashes without triggering `SessionEnd`, the agent entry will remain in SpacetimeDB until manually cleaned up or the stale-agent cleanup job runs (120s timeout per heartbeat protocol)
- **State file accumulation** — crashed sessions leave orphan files in `~/.hex/sessions/`. These are small (< 200 bytes) and can be cleaned up by the existing `coordination/cleanup` routine

### Future Work

- **Heartbeat integration** — wire `PostToolUse` or `UserPromptSubmit` hooks to send periodic heartbeats via the existing `/api/agents/health` or a new heartbeat endpoint, so the stale-agent detector works for Claude Code sessions too
- **Subagent tracking** — the existing `SubagentStart`/`SubagentStop` hooks already push events via `hub-push.cjs`; these could be linked to the parent session's agentId for hierarchical agent tracking
- **Session metadata** — extend the agent record with token usage, tool call counts, and active file information, sourced from the existing `hub-push.cjs` event stream

## Files Changed

| File | Change |
|------|--------|
| `hex-nexus/src/routes/orchestration.rs` | Extended `connect_agent` to accept metadata fields; added `disconnect_agent` handler |
| `hex-nexus/src/routes/mod.rs` | Registered `POST /api/agents/disconnect` route |
| `.claude/helpers/agent-register.cjs` | New hook helper for session registration lifecycle |
| `.claude/settings.json` | Wired `agent-register.cjs` into `SessionStart` and `SessionEnd` hooks |
