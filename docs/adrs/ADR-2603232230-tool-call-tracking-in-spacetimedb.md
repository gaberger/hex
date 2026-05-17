# ADR-2603232230: Tool Call Tracking in SpacetimeDB

**Status:** Accepted
**Date:** 2026-03-23
**Drivers:** `hex dev` pipeline calls are logged locally in session JSON files (`~/.hex/sessions/dev/<id>.json`) but not persisted in SpacetimeDB. This means the dashboard can't show tool call history, reports don't survive machine changes, and multi-agent sessions can't correlate calls across agents.
**Supersedes:** None (extends ADR-2603232220, ADR-027)

## Context

The `hex report` command (ADR-2603232220) assembles audit trails from local session files. Each session now tracks a `tool_calls` array with per-call metadata:

```json
{
  "timestamp": "2026-03-23T22:00:29Z",
  "phase": "adr",
  "tool": "POST /api/inference/complete",
  "model": "deepseek/deepseek-r1",
  "tokens": 2060,
  "cost_usd": 0.0034,
  "duration_ms": 26365,
  "status": "ok",
  "detail": "docs/adrs/ADR-2603232200-...md"
}
```

This data exists only on the local filesystem. It should flow into SpacetimeDB for:
1. **Dashboard visibility** — real-time tool call feed in the dashboard
2. **Cross-machine persistence** — reports work from any connected client
3. **Multi-agent correlation** — see which agent made which calls
4. **Cost aggregation** — per-project, per-agent, per-model cost rollups
5. **RL feedback** — tool call success/failure as training signal

## Decision

### 1. New SpacetimeDB Table: `dev_tool_call`

Add to the `hexflo-coordination` WASM module:

```rust
#[spacetimedb::table(name = dev_tool_call, public)]
pub struct DevToolCall {
    #[primary_key]
    #[auto_inc]
    pub id: u64,
    pub session_id: String,      // hex dev session UUID
    pub agent_id: String,        // from X-Hex-Agent-Id
    pub project_id: String,      // from session context
    pub timestamp: String,       // ISO 8601
    pub phase: String,           // adr, workplan, swarm, code, validate
    pub tool: String,            // "POST /api/inference/complete", "POST /api/swarms", etc.
    pub model: String,           // inference model or empty
    pub tokens: u64,             // 0 for non-inference calls
    pub cost_usd: f64,           // 0.0 for non-inference calls
    pub duration_ms: u64,        // wall clock
    pub status: String,          // ok, error, retry
    pub detail: String,          // step ID, file path, error message
}
```

### 2. New Reducer: `log_tool_call`

```rust
#[spacetimedb::reducer]
pub fn log_tool_call(
    ctx: &ReducerContext,
    session_id: String,
    agent_id: String,
    project_id: String,
    timestamp: String,
    phase: String,
    tool: String,
    model: String,
    tokens: u64,
    cost_usd: f64,
    duration_ms: u64,
    status: String,
    detail: String,
) {
    ctx.db.dev_tool_call().insert(DevToolCall {
        id: 0,
        session_id, agent_id, project_id, timestamp,
        phase, tool, model, tokens, cost_usd, duration_ms,
        status, detail,
    });
}
```

### 3. New REST Endpoint: `POST /api/hexflo/tool-calls`

In hex-nexus:

```rust
// POST /api/hexflo/tool-calls — log a tool call
pub async fn log_tool_call(
    State(state): State<SharedState>,
    Json(body): Json<ToolCallRequest>,
) -> (StatusCode, Json<Value>) {
    // Call SpacetimeDB reducer
    // Falls back to no-op if SpacetimeDB unavailable
}

// GET /api/hexflo/tool-calls?session_id=<id> — list calls for a session
pub async fn list_tool_calls(
    State(state): State<SharedState>,
    Query(params): Query<ToolCallQuery>,
) -> (StatusCode, Json<Value>) {
    // SQL query with optional session_id filter
}
```

### 4. Dual-Write from hex-cli

`session.log_tool_call()` writes to both:
1. Local JSON file (immediate, survives nexus downtime)
2. hex-nexus REST endpoint (async, best-effort, for SpacetimeDB persistence)

```rust
pub fn log_tool_call(&mut self, call: ToolCall) -> Result<()> {
    self.tool_calls.push(call.clone());
    self.save()?;
    // Best-effort POST to nexus (fire-and-forget)
    let _ = self.post_tool_call_to_nexus(&call);
    Ok(())
}
```

### 5. Report Reads from SpacetimeDB First

`hex report show <id>` queries:
1. `GET /api/hexflo/tool-calls?session_id=<id>` (SpacetimeDB — authoritative)
2. Falls back to local session file if nexus unavailable

### 6. Dashboard Integration

The dashboard subscribes to `dev_tool_call` table via WebSocket. New panel: **"Tool Call Feed"** showing real-time calls as they happen during `hex dev` runs.

### 7. Aggregation Queries

SpacetimeDB enables queries the local file can't:

```sql
-- Total cost per model
SELECT model, SUM(cost_usd), COUNT(*) FROM dev_tool_call GROUP BY model;

-- Total cost per project this week
SELECT project_id, SUM(cost_usd) FROM dev_tool_call
WHERE timestamp > '2026-03-17' GROUP BY project_id;

-- Error rate per phase
SELECT phase, status, COUNT(*) FROM dev_tool_call GROUP BY phase, status;

-- Slowest inference calls
SELECT * FROM dev_tool_call WHERE duration_ms > 30000 ORDER BY duration_ms DESC;
```

## Implementation

| Phase | Description | Status |
|-------|------------|--------|
| P1 | Add `dev_tool_call` table + `log_tool_call` reducer to hexflo-coordination WASM module | Pending |
| P2 | Add `POST /api/hexflo/tool-calls` and `GET /api/hexflo/tool-calls` to hex-nexus | Pending |
| P3 | Dual-write from `session.log_tool_call()` — local + nexus REST | Pending |
| P4 | Update `hex report` to read from SpacetimeDB first, fallback to local | Pending |
| P5 | Dashboard: Tool Call Feed panel with WebSocket subscription | Pending |
| P6 | Aggregation queries for cost/model/phase reporting | Pending |

## Consequences

### Positive
- **Full audit trail in SpacetimeDB** — survives machine changes, visible from any client
- **Dashboard real-time feed** — see tool calls as they happen
- **Cost aggregation** — per-project, per-model, per-agent cost rollups
- **RL training data** — tool call outcomes feed the model selection engine
- **Multi-agent correlation** — trace which agent made which calls

### Negative
- **Write amplification** — every tool call writes to local file + SpacetimeDB
- **WASM module size** — another table in hexflo-coordination
- **Latency** — dual-write adds ~5ms per call (acceptable, async)

### Mitigations
- SpacetimeDB write is fire-and-forget (async, best-effort)
- Local file remains authoritative for the current session
- Table auto-prunes entries older than 30 days

## References

- ADR-2603232220: Developer Audit Report
- ADR-2603232005: Self-Sufficient hex-agent with TUI
- ADR-027: HexFlo Native Coordination
- ADR-031: RL-Driven Model Selection & Token Management
