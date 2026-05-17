# ADR-2604012137: Tool Call Observability via WebSocket Event Log

**Status:** Accepted
**Date:** 2026-04-01
**Related:** ADR-2604012110 (hooks-first enforcement), ADR-025 (SQLite fallback)

---

## Context

The [agents-observe](https://github.com/simple10/agents-observe) project demonstrates a lightweight pattern for real-time agent observability:

```
Claude Code hooks → HTTP POST (local) → SQLite event log → WebSocket push → browser dashboard
```

hex already uses this technique at the SpacetimeDB layer for multi-agent coordination. However, for solo/local workflows SpacetimeDB is heavyweight. Additionally, hex currently has no tool-call timeline — the dashboard shows agent status and task state but not *which tools fired, in what order, with what arguments*.

The hook-first architecture migration (ADR-2604012110) creates a natural event source: every PreToolUse/PostToolUse hook call already has the full tool name, arguments, and result available in the hook environment.

---

## Decision

Add a lightweight tool-call event log to hex-nexus:

1. **SQLite event table** (`tool_events`) — one row per tool call, stored in `~/.hex/hub.db` (existing SQLite file, ADR-025)
2. **HTTP endpoint** `POST /api/events` — accepts hook event JSON, writes to SQLite
3. **WebSocket broadcast** — on each insert, push to all connected dashboard clients via existing WebSocket infrastructure
4. **Dashboard feed** — new "Activity" panel showing live tool-call timeline per session

Hooks post events via a small shell one-liner or by extending `hex hook route` — no new binary.

---

## Event Schema

```sql
CREATE TABLE tool_events (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id   TEXT NOT NULL,
    agent_id     TEXT,
    event_type   TEXT NOT NULL,  -- PreToolUse | PostToolUse | SubagentStart | SubagentStop | Stop
    tool_name    TEXT,
    -- Tool input/output (truncated at 4KB each)
    input_json   TEXT,
    result_json  TEXT,
    exit_code    INTEGER,
    duration_ms  INTEGER,
    -- Full audit fields: model routing + context + cost
    model_used      TEXT,        -- e.g. "claude-sonnet-4-6", "MiniMax-M2.7", "local"
    context_strategy TEXT,       -- "aggressive" | "balanced" | "conservative"
    rl_action       TEXT,        -- raw RL compound action: "model:minimax|context:conservative"
    input_tokens    INTEGER,
    output_tokens   INTEGER,
    cost_usd        REAL,
    hex_layer       TEXT,        -- "domain" | "ports" | "adapters/primary" | etc.
    created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE INDEX tool_events_session ON tool_events(session_id, created_at DESC);
CREATE INDEX tool_events_model ON tool_events(session_id, model_used);
```

### Audit Trace Per Tool Call

Every tool call produces a correlated record linking:
- **What happened**: tool_name + input + result
- **Which model handled it**: model_used + rl_action (which Q-table decision drove routing)
- **Context strategy**: how much history/tool-result budget was allocated
- **Cost**: input_tokens + output_tokens + cost_usd (links to existing USD cost tracking)
- **Hex layer**: which architecture boundary was being modified

This enables queries like:
- "Show all Bash tool calls this session with their cost"
- "Which model handled each code generation task?"
- "What was the avg latency for Opus vs MiniMax on Write operations?"
- "Did any domain-layer edits happen without RL routing through Haiku?"

---

## Hook Integration

Extend `hex hook route` (UserPromptSubmit) and `hex hook pre_edit`/`post_edit` to POST to `/api/events` alongside existing coordination calls. Single additional HTTP call per hook event — adds <5ms.

Alternative: new `hex hook observe` command that can run as a standalone hook:
```json
{
  "PreToolUse": [{ "type": "command", "command": "hex hook observe pre", "blocking": false }],
  "PostToolUse": [{ "type": "command", "command": "hex hook observe post", "blocking": false }]
}
```

---

## Dashboard Integration

New "Activity" panel in the hex dashboard (Solid.js, `hex-nexus/assets/`):
- Live WebSocket subscription to `tool_events` stream
- Timeline view: PreToolUse → PostToolUse pairs collapsed into one row
- Expandable tool input/output (collapsed by default)
- Session filter + agent filter
- Latency column (PostToolUse timestamp − PreToolUse timestamp)

---

## What This Does NOT Replace

- SpacetimeDB `hexflo-coordination` — still required for multi-agent task assignment
- SpacetimeDB `agent-registry` — still required for cross-host agent visibility
- SpacetimeDB `inference-gateway` — still required for multi-provider LLM routing

This is additive observability only, not coordination infrastructure.

---

## Consequences

**Positive:**
- Tool-call timeline visible in dashboard — "what did the agent just do?"
- Works without SpacetimeDB (SQLite-only path via ADR-025 fallback)
- Minimal: one SQLite table, one HTTP endpoint, one WebSocket message type
- Composable with agents-observe — can run both; different ports (4981 vs 5555)

**Negative:**
- Event log grows unbounded — needs TTL cleanup (7-day retention, same as cron jobs)
- `input_json` / `result_json` may be large (file contents) — truncate at 4KB

---

## Related

- https://github.com/simple10/agents-observe — inspiration and reference implementation
- `docs/analysis/hooks-prototype/README.md` — agents-observe pattern documented
- ADR-2604012110 — hook-first enforcement (provides the PreToolUse/PostToolUse event source)
- ADR-025 — SQLite fallback state (reuses `~/.hex/hub.db`)
