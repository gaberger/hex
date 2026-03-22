# ADR-060: Agent Notification Inbox

**Status:** Proposed
**Date:** 2026-03-22
**Drivers:** No mechanism exists to notify a running Claude Code agent of critical system events (hex-nexus update, hex-agent restart, config change). Agents operate in a closed process loop — external systems cannot push messages into an active session. This creates risk of agents working against stale binaries or missing coordinated restarts.

## Context

Claude Code agents run in a single-process loop. The only entry points for external information are:

1. **Hooks** — fire on Claude Code events (PreToolUse, UserPromptSubmit), but only when the agent is actively working. Cannot be triggered from outside.
2. **`claude --resume <id> -p`** — sends a new prompt to a session, but starts a new turn rather than injecting into the current one.
3. **File-based signals** — hex already uses `~/.hex/sessions/agent-*.json` for session state, but nothing reads an "inbox" from it.

None of these provide a **durable, prioritized message queue** that an external system can write to and the agent reliably consumes.

### Forces

- **SpacetimeDB is the coordination backbone** (ADR-046) — any new coordination primitive should live there, not in REST or filesystem
- **Agents already have identity** (ADR-058/059) — `agent_id` is registered and heartbeated
- **Hook stdout is the injection vector** — PreToolUse hook stdout appears as system context in Claude's next action
- **Idle agents don't fire hooks** — an agent waiting for user input has no event loop to poll
- **Graceful state preservation** matters — agents may hold in-progress workplan state, swarm coordination data, or uncommitted analysis

### Alternatives Considered

| Approach | Rejected Because |
|----------|-----------------|
| Unix signals (SIGUSR1) | Claude Code doesn't expose signal handlers; no way to inject context |
| WebSocket push to agent | Claude Code has no inbound socket; hooks are the only interface |
| Filesystem watch (inotify) | Adds OS-specific dependency; no guaranteed delivery; not durable |
| REST polling from agent | Violates ADR-046 (SpacetimeDB is authority); adds HTTP coupling |

## Decision

### 1. SpacetimeDB `agent_inbox` Table

A new table in the `hexflo-coordination` WASM module stores notifications for agents:

```rust
#[spacetimedb::table(public, name = agent_inbox)]
pub struct AgentInbox {
    #[primary_key]
    #[auto_inc]
    pub id: u64,
    pub agent_id: String,
    pub priority: u8,              // 0=info, 1=warning, 2=critical
    pub kind: String,              // "restart", "update", "shutdown", "config_change"
    pub payload: String,           // JSON — version info, reason, instructions
    pub created_at: Timestamp,
    pub acknowledged_at: Option<Timestamp>,
    pub expired_at: Option<Timestamp>,
}
```

### 2. Reducers

| Reducer | Purpose |
|---------|---------|
| `notify_agent(agent_id, priority, kind, payload)` | Enqueue a message for a specific agent |
| `notify_all_agents(project_id, priority, kind, payload)` | Broadcast to all agents in a project |
| `acknowledge_notification(id, agent_id)` | Mark as read (only the target agent can ack) |
| `expire_stale_notifications(max_age_secs)` | Cleanup — called by hex-nexus on schedule |

### 3. hex-nexus Integration

hex-nexus is the **producer** for system-level notifications:

- On binary update detection: `notify_all_agents(project_id, 2, "restart", { reason, new_version })`
- On config sync change (ADR-044): `notify_all_agents(project_id, 1, "config_change", { keys_changed })`
- On SpacetimeDB reconnection: `notify_all_agents(project_id, 0, "info", { event: "stdb_reconnected" })`

hex-nexus also exposes a REST endpoint for non-SpacetimeDB producers:

```
POST /api/agents/:agent_id/notify   → calls notify_agent reducer
GET  /api/agents/:agent_id/inbox    → reads agent_inbox table (filtered, unacked)
```

### 4. Hook-Based Delivery (Agent Side)

The `PreToolUse` hook checks for unacknowledged critical notifications:

```bash
# In hex hook pre-tool (runs before every tool call)
inbox=$(curl -s http://localhost:5555/api/agents/$AGENT_ID/inbox?min_priority=2)
if [ -n "$inbox" ] && [ "$inbox" != "[]" ]; then
  echo "⚠ CRITICAL NOTIFICATION:"
  echo "$inbox" | jq -r '.[] | "[\(.kind)] \(.payload)"'
  echo ""
  echo "Action required: save state and prepare for restart."
fi
```

Hook stdout is injected into Claude's context as a system reminder. The agent sees it before its next action and can respond appropriately.

### 5. Priority Semantics

| Priority | Name | Agent Behavior | Delivery |
|----------|------|---------------|----------|
| 0 | Info | No action required — context only | Next tool call |
| 1 | Warning | Agent should complete current task, then address | Next tool call |
| 2 | Critical | Agent must save state and prepare for restart | Every tool call until acked |

Critical notifications (priority 2) are re-delivered on **every** PreToolUse call until acknowledged. This ensures the agent cannot ignore them even if context is compressed.

### 6. State Preservation Protocol

When an agent receives a `kind: "restart"` notification (any priority):

1. **Save session state** — write current task/phase/swarm to `~/.hex/sessions/agent-{id}.json`
2. **Save memory** — persist key decisions via `hex memory store`
3. **Acknowledge** — call `acknowledge_notification(id)` reducer
4. **Exit gracefully** — the agent tells the user a restart is needed

On session restart, `hex hook session-start`:
1. Reads session file for unfinished work
2. Checks inbox history for the notification that triggered restart
3. Injects recovery context: "You restarted because: {reason}. Previous state: {state}"

### 7. Idle Agent Coverage

Agents waiting for user input don't fire hooks. Two mitigations:

- **Heartbeat hook** (`UserPromptSubmit`): checks inbox when user sends any message
- **External resume** (fallback): `claude --resume <session-id> -p "Check your hex inbox for critical notifications"` — triggered by hex-nexus when a critical notification goes unacknowledged for >60 seconds

## Consequences

**Positive:**
- External systems can reliably communicate with running agents
- Durable message queue — notifications survive agent restarts and SpacetimeDB reconnections
- Priority system prevents notification fatigue (info doesn't interrupt, critical does)
- State preservation protocol prevents lost work during coordinated restarts
- Consistent with SpacetimeDB-as-authority (ADR-046)

**Negative:**
- Adds latency — agent only sees notifications on next tool call (seconds to minutes)
- REST endpoint for inbox check adds HTTP coupling in the hook (but SpacetimeDB WebSocket isn't available in bash hooks)
- Idle agents require external `claude --resume` fallback, which starts a new conversation turn

**Mitigations:**
- Critical notifications repeat on every tool call — delivery is guaranteed once the agent is active
- REST endpoint is read-only from the agent side; writes go through SpacetimeDB reducers
- Idle agent coverage is best-effort; hex-nexus can escalate to OS-level notification if unacked after 5 minutes

## Implementation

| Phase | Description | Status |
|-------|------------|--------|
| P1 | Add `agent_inbox` table + reducers to `hexflo-coordination` module | Pending |
| P2 | hex-nexus REST endpoints + notification producers (update detection, config sync) | Pending |
| P3 | PreToolUse hook integration — inbox check + context injection | Pending |
| P4 | State preservation protocol — save/restore on restart notifications | Pending |
| P5 | Idle agent coverage — hex-nexus escalation for unacked criticals | Pending |

## References

- ADR-046: SpacetimeDB single authority for state
- ADR-048: Task state synchronization (hook-based stdin/stdout pattern)
- ADR-058: Unified agent identity
- ADR-059: Canonical project identity contract
- ADR-044: Config sync on startup
