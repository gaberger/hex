# ADR-065: Registration Lifecycle Gaps — Project and Agent Identity

**Status:** Accepted
**Date:** 2026-03-22
**Drivers:** End-to-end system test (hex-monitor on bazzite) revealed that project registration, agent registration, and session identity are fragmented across multiple endpoints and tables. Agents can connect but not receive inbox notifications. Projects exist on disk but not in SpacetimeDB. Session files resolve to the wrong agent.

## Context

During the hex-monitor integration test on bazzite, we found these registration gaps:

### 1. Project Registration Is Manual and Disconnected

When `hex init` or `go mod init` creates a project on a remote host, nothing registers it in SpacetimeDB. The `project` table exists but:
- `hex agent connect` doesn't register the project it's working in
- `hex swarm init` accepts a `project_id` but doesn't validate it exists in the `project` table
- The dashboard shows projects that were manually registered but misses remote projects entirely
- `notify_all_agents(project_id, ...)` broadcasts to agents filtered by `project_id`, but if agents registered without a `project_id`, they receive nothing

### 2. Agent Registration Endpoint Fragmentation

Three separate registration paths exist:

| Path | Endpoint | Table | Used By |
|------|----------|-------|---------|
| Session start hook | `POST /api/hex-agents/connect` | `hex_agent` | Claude Code sessions (local) |
| Remote agent connect | `POST /api/agents/connect` (was) | `remote_agent` | `hex agent connect` CLI |
| Nexus auto-register | `POST /api/hex-agents/connect` | `hex_agent` | hex-nexus daemon startup |

The `hex agent connect` endpoint was fixed in this session to use `/api/hex-agents/connect`, but the underlying problem remains: there's no guarantee that an agent's `project_id` is set correctly, and remote agents don't create session files compatible with `hex inbox list`.

### 3. Session File Resolution Is Ambiguous

`hex inbox list` calls `resolve_agent_id()` which picks the **newest** session file in `~/.hex/sessions/`. This fails because:
- hex-nexus writes its own session file on startup (newer than Claude's)
- Remote agents don't write session files at all
- Multiple Claude Code sessions produce multiple files with no way to distinguish "current"
- The `CLAUDE_SESSION_ID` env var is available in hooks but not in standalone CLI invocations

### 4. `hex agent connect` Doesn't Persist Connection State

After `hex agent connect http://nexus:5555`, the agent ID is printed but not saved anywhere on the remote host. Subsequent CLI commands (`hex inbox list`, `hex task list`) can't resolve "who am I" without a session file.

## Decision

### 1. Auto-Register Projects on Agent Connect

When `hex agent connect` or the session-start hook registers an agent, the project MUST also be registered if it doesn't exist:

```
agent_connect(agent_id, project_dir, ...) {
    1. Derive project_name from basename(project_dir)
    2. Call project_find(project_name)
    3. If not found: call register_project(generated_id, project_name, project_dir)
    4. Set agent.project_id = resolved project_id
}
```

### 2. Unify Agent Registration to Single Endpoint

All agent registration MUST go through `POST /api/hex-agents/connect` (the ADR-058 unified registry). Remove or redirect:
- `POST /api/agents/connect` → redirect to `/api/hex-agents/connect`
- `register_remote_agent` reducer → deprecate, use `agent_connect` reducer

### 3. Write Session File on `hex agent connect`

`hex agent connect` must write a session file to `~/.hex/sessions/agent-{session_id}.json` on the remote host, identical in format to what the session-start hook writes. This enables subsequent CLI commands to resolve the agent ID.

### 4. Session File Resolution Priority

`resolve_agent_id()` must use this priority order:
1. `CLAUDE_SESSION_ID` env var → look up `agent-{session_id}.json` (exact match)
2. `HEX_AGENT_ID` env var → use directly (for scripts/CI)
3. `claude_pid` match — walk the PPID chain to find the `claude` process PID, then match session files by `claude_pid` field (supports multiple concurrent Claude instances in the same project directory)
4. Newest session file with `status != "completed"` (skip nexus agent files)
5. Error: "Cannot resolve agent ID"

Filter out nexus auto-registered agents by checking `name.starts_with("nexus-agent")`.

### 4a. `claude_pid` — Multi-Instance Disambiguation

The `SessionState` struct gains a `claude_pid: Option<u32>` field. The session-start hook sets it to `std::os::unix::process::parent_id()` — the PID of the `claude` binary that spawned the hook process. This PID is stable for the Claude session's lifetime and unique per instance.

**Statusline resolution**: The statusline script walks its own PPID chain (via `ps`) to find the `claude` process PID, then matches session files by `claude_pid`. This ensures each concurrent Claude instance sees only its own agent identity on the status bar.

**Display format**: Agent identity on the statusline uses `{name}:{shortId}` format (e.g., `claude-jaco2.lan:d9c3b1c0`) to provide both context (hostname) and uniqueness (8-char UUID prefix).

### 5. Project Scoping for Broadcasts

`notify_all_agents(project_id, ...)` currently filters by `hex_agent.project_id`. Agents registered without a `project_id` are invisible to broadcasts. Fix:
- `hex agent connect` must resolve and set `project_id` (see Decision 1)
- The `agent_connect` reducer must reject empty `project_id` when `project_dir` is provided (derive it server-side if needed)

### 6. `hex init` Registers the Project

`hex init <path>` already scaffolds the project structure. It must also call `register_project` so the project appears in SpacetimeDB immediately, not only when an agent connects.

## Consequences

**Positive:**
- Single registration path for all agent types (local, remote, nexus)
- Projects auto-registered — no manual step needed for remote projects
- `hex inbox list` works correctly on remote hosts
- `notify_all_agents` reliably reaches all agents in a project
- Session file format is consistent across local and remote agents

**Negative:**
- Breaking change: `POST /api/agents/connect` must be deprecated (add redirect + deprecation header)
- Remote agents now write state files, adding filesystem dependency
- Project auto-registration may create duplicate entries if project names collide across hosts

**Mitigations:**
- Deprecation middleware already exists (ADR-039) — add `/api/agents/connect` to the deprecated list
- Project registration uses `project_find()` (ADR-059) to deduplicate by name
- Session files use atomic writes to prevent corruption

## Implementation

| Phase | Description | Status |
|-------|------------|--------|
| P1 | `hex agent connect` writes session file on remote host + sends project_dir | Done |
| P2 | `hex agent connect` auto-registers project via `/api/hex-agents/connect` | Done (client sends project_dir; server-side auto-register pending P6) |
| P3 | `resolve_agent_id()` priority order (CLAUDE_SESSION_ID → HEX_AGENT_ID → claude_pid → filtered newest) | Done |
| P3a | `claude_pid` field in SessionState + statusline PPID-chain matching | Done |
| P3b | `hex agent id` CLI subcommand for self-discovery | Done |
| P3c | Auto-reconnect: `hex agent id` re-registers with nexus when agent unknown | Done |
| P3d | `hex agent list` shows full UUIDs + `◀ you` marker for current agent | Done |
| P3e | `hex agent disconnect` uses unified `/api/hex-agents/` endpoint (was `/api/agents/`) | Done |
| P3f | `resolve_agent_id_detailed()` returns resolution method, session file, parsed data | Done |
| P4 | `hex init` registers project in SpacetimeDB via POST /api/projects | Done |
| P5 | Deprecate `POST /api/agents/connect` — forwards to /api/hex-agents/connect with Deprecation + Sunset headers | Done |
| P6 | REST handler accepts client-provided `agent_id` for identity persistence across nexus restarts | Done |
| P6a | Project auto-registration on agent connect (project_find → project_register) | Done |

## References

- ADR-058: Unified agent identity
- ADR-059: Canonical project identity contract
- ADR-060: Agent notification inbox (exposed these gaps during testing)
- Integration test: hex-monitor on bazzite (2026-03-22)
