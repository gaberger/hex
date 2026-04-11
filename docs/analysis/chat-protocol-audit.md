# hex-hub WebSocket Chat Protocol Audit

**Date**: 2026-03-18
**Scope**: Complete protocol analysis of the hex-hub ↔ hex-agent ↔ browser chat WebSocket system
**Files analyzed**:
- `hex-nexus/src/routes/chat.rs` — chat WS handler
- `hex-nexus/src/routes/ws.rs` — general WS handler
- `hex-nexus/src/state.rs` — WsEnvelope, SharedState, broadcast channel
- `hex-agent/src/ports/hub.rs` — HubMessage enum
- `hex-agent/src/adapters/secondary/hub_client.rs` — agent WS adapter
- `hex-nexus/assets/chat.html` — browser chat UI
- `hex-agent/src/main.rs` (lines 285–413) — hub-managed agent loop

---

## 1. Architecture Overview

There are two distinct WebSocket endpoints:

| Endpoint | Handler | Purpose |
|----------|---------|---------|
| `/ws` | `ws::ws_handler` | General pub/sub with topic subscriptions and wildcard matching |
| `/ws/chat` | `chat::chat_ws_handler` | Chat-specific handler with LLM bridge fallback |

Both share the same `broadcast::Sender<WsEnvelope>` channel (`state.ws_tx`, capacity 512). All messages flow through this single broadcast bus.

### WsEnvelope structure

```rust
struct WsEnvelope {
    topic: String,   // Routing key (e.g. "agent:abc123", "chat:sid:llm")
    event: String,   // Message type (e.g. "stream_chunk", "tool_call")
    data: Value,     // Arbitrary JSON payload
}
```

### Two operating modes

The chat handler supports two modes depending on whether a hex-agent is connected:

1. **LLM Bridge mode** — No agent connected, hub has `ANTHROPIC_API_KEY`. Hub calls the Anthropic API directly (non-streaming, single-turn request/response).
2. **Agent Relay mode** — A hex-agent is connected via the same `/ws/chat` endpoint. Messages are relayed through the broadcast channel.

---

## 2. Message Flow Maps

### 2.1 User sends a chat message (Agent Relay mode)

```
Browser                  Hub (chat.rs)              Broadcast Channel         Agent (main.rs)
   |                         |                            |                       |
   |-- {"type":"chat_message" -->                         |                       |
   |    "content":"hello"}   |                            |                       |
   |                         |-- WsEnvelope{              |                       |
   |                         |     topic: "agent:broadcast:input"                 |
   |                         |     event: "chat_message"  |                       |
   |                         |     data: {sessionId, content}                     |
   |                         |   } ---------------------->|                       |
   |                         |                            |-- recv() unwraps ---->|
   |                         |                            |   to HubMessage::     |
   |                         |                            |   ChatMessage{content}|
```

**Key detail**: When no `agent_id` is specified, the hub publishes to topic `agent:broadcast:input`. The agent receives this because the chat handler's send_task forwards any envelope whose `topic.starts_with("agent:")`.

### 2.2 Agent streams a response back

```
Agent (main.rs)          Hub (chat.rs recv_task)    Broadcast Channel         Browser (chat.html)
   |                         |                            |                       |
   |-- HubMessage::StreamChunk{text:"Hello"} ----------->|                       |
   |   (serialized as {"type":"stream_chunk","text":"Hello"})                     |
   |                         |                            |                       |
   |                         |-- raw JSON not a           |                       |
   |                         |   ChatInbound, so:         |                       |
   |                         |   broadcast as WsEnvelope{ |                       |
   |                         |     topic: "agent:<session_id>:output"             |
   |                         |     event: "stream_chunk"  |                       |
   |                         |     data: {type,text}      |                       |
   |                         |   } ---------------------->|                       |
   |                         |                            |-- send_task matches-->|
   |                         |                            |   (event=="stream_chunk")
   |                         |                            |                       |
   |                         |                            |   Browser receives:   |
   |                         |                            |   {type:"stream_chunk",
   |                         |                            |    data:{type,text}}  |
```

**Critical path**: Agent messages arrive at the hub's `recv_task` as raw JSON. They fail to parse as `ChatInbound` (which expects `chat_message`, `connect_agent`, or `spawn_agent`). The fallback code (lines 156-166) re-broadcasts them as `WsEnvelope` with `topic: "agent:<session_id>:output"` and `event` extracted from the `type` field.

### 2.3 Tool call lifecycle

```
Agent                    Hub broadcast             Browser
  |                           |                       |
  |-- ToolCall{tool_name,  -->|                       |
  |   tool_input}             |-- event:"tool_call"-->|
  |                           |                   handleToolCall():
  |                           |                   creates collapsible card
  |                           |                   shows "running..."
  |                           |                       |
  |-- ToolResultMsg{       -->|                       |
  |   tool_name,content,      |-- event:"tool_result">|
  |   is_error}               |                   handleToolResult():
  |                           |                   fills result section
  |                           |                   marks error if is_error
```

### 2.4 LLM Bridge mode (no agent)

```
Browser                  Hub (chat.rs)              Anthropic API
   |                         |                            |
   |-- chat_message -------->|                            |
   |                         |-- agent_status "thinking"->| (broadcast)
   |                         |                            |
   |                         |-- POST /v1/messages ------>|
   |                         |<-- JSON response ----------|
   |                         |                            |
   |                         |-- chat_message (response)->| (broadcast, topic: chat:<sid>:llm)
   |                         |-- token_update ----------->| (broadcast)
   |                         |-- agent_status "idle" ---->| (broadcast)
   |                         |                            |
   |<-- receives all three --|                            |
```

**Note**: LLM bridge is non-streaming. The entire response is sent as a single `chat_message`, not as `stream_chunk` events. The browser's `handleMessage` for `chat_message` calls `addAssistantMessage()` which calls `endStream()` first, so any in-flight stream is properly terminated.

---

## 3. Protocol Gap Analysis

### 3.1 Agent Registration (PARTIALLY BROKEN)

| What | Status | Detail |
|------|--------|--------|
| Agent sends `Register` | SENT | `main.rs:302` — sends `HubMessage::Register{agent_id, agent_name, project_dir}` |
| Hub processes `Register` | NOT PROCESSED | The hub `recv_task` tries to parse incoming messages as `ChatInbound`. `Register` has `type: "agent_register"` which does not match any `ChatInbound` variant. It falls through to the raw-JSON fallback (line 157) and gets re-broadcast with `event: "agent_register"`. |
| Browser handles `agent_register` | NOT HANDLED | `handleMessage()` has no `case "agent_register"` in its switch statement. The message is silently dropped. |
| Send_task forwards `agent_register` | PARTIALLY | The send_task filter (line 96-104) does NOT include `"agent_register"` in its event allowlist. However, the topic `agent:<session_id>:output` passes the `topic.starts_with("agent:")` check on line 96, so it IS forwarded. |

**Impact**: Registration "works" in that the agent can send it and the hub re-broadcasts it, but:
- The hub does not record the agent in any registry
- The browser ignores the message
- No agent metadata (name, project_dir) is displayed in the sidebar

### 3.2 StreamChunk forwarding (WORKS)

| What | Status | Detail |
|------|--------|--------|
| Agent sends `StreamChunk` | SENT | `main.rs:343` |
| Hub re-broadcasts | YES | Falls through ChatInbound parsing, re-broadcast with `event: "stream_chunk"` |
| Send_task forwards | YES | `"stream_chunk"` is in the allowlist (line 103) |
| Browser handles | YES | `handleStreamChunk()` appends text, shows cursor |

### 3.3 ToolCall / ToolResult (WORKS)

| What | Status | Detail |
|------|--------|--------|
| Agent sends `ToolCall` | SENT | `main.rs:346` |
| Hub re-broadcasts | YES | `event: "tool_call"` |
| Send_task forwards | YES | `"tool_call"` in allowlist (line 99) |
| Browser renders | YES | `handleToolCall()` creates collapsible card |
| Agent sends `ToolResultMsg` | SENT | `main.rs:355` |
| Hub re-broadcasts | YES | `event: "tool_result"` |
| Send_task forwards | YES | `"tool_result"` in allowlist (line 100) |
| Browser renders | YES | `handleToolResult()` fills in result section |

### 3.4 TokenUpdate (WORKS)

| What | Status | Detail |
|------|--------|--------|
| Agent sends `TokenUpdate` | SENT | `main.rs:361` |
| Hub re-broadcasts | YES | `event: "token_update"` |
| Send_task forwards | YES | `"token_update"` in allowlist (line 98) |
| Browser renders | YES | `handleTokenUpdate()` updates gauge, counters |

**Caveat**: The agent sets `total_input = input_tokens as u64` and `total_output = output_tokens as u64` (main.rs:363-364), meaning totals always equal the current turn's tokens, not cumulative totals. The browser stores running totals in `state.totalInput`/`state.totalOutput` but uses `msg.total_input` if present (which overwrites the running total). This means the gauge resets to per-turn values rather than accumulating.

### 3.5 AgentStatus (WORKS with caveats)

| What | Status | Detail |
|------|--------|--------|
| Agent sends `AgentStatus` | SENT | `main.rs:327,384,393` (thinking, error, idle) |
| Hub re-broadcasts | YES | `event: "agent_status"` |
| Send_task forwards | YES | topic starts with `"agent:"` |
| Browser renders | YES | `handleAgentStatus()` updates status pill |

**Caveat**: The `agent_status` event is NOT in the explicit event allowlist (lines 98-104). It only gets forwarded because the topic (`agent:<session_id>:output`) passes the `topic.starts_with("agent:")` check. This is correct but fragile — if the topic format changes, status updates would silently stop.

### 3.6 Agent Done (NOT RENDERED)

| What | Status | Detail |
|------|--------|--------|
| Agent sends `Done` | NOT SENT by agent | The agent never sends `Done` — it only handles receiving it (main.rs:403) |
| Hub sends `Done` | NOT IMPLEMENTED | No code path in the hub generates a `Done` message |
| Browser handles `agent_done` | NOT HANDLED | No case in `handleMessage()` |

**Impact**: There is no clean "conversation complete" signal. The agent signals completion implicitly via `AgentStatus{status: "idle"}`, which triggers `endStream()` in the browser.

---

## 4. Envelope Wrapping / Unwrapping Analysis

### 4.1 Hub → Browser (chat.rs send_task)

The hub wraps all messages in `ChatOutbound{msg_type, data}` (line 121-124), which serializes as:
```json
{"type": "stream_chunk", "data": {"type": "stream_chunk", "text": "Hello"}}
```

The browser's `ws.onmessage` handler (line 322-328) checks for `raw.event && raw.data` to unwrap `WsEnvelope` format. But `ChatOutbound` uses `type` not `event` as the key name. So the browser falls through to `msg = raw`, receiving:
```json
{"type": "stream_chunk", "data": {"type": "stream_chunk", "text": "Hello"}}
```

The `handleMessage` switch matches on `msg.type`, which is `"stream_chunk"`. Then `handleStreamChunk(msg)` accesses `msg.text` — but the text is inside `msg.data.text`, not `msg.text`.

**CRITICAL BUG**: There is a mismatch between the chat handler's `ChatOutbound` envelope and the browser's unwrapping logic:

- `ChatOutbound` serializes with field name `type` (via `#[serde(rename = "type")] msg_type`) and `data`
- The browser checks for `raw.event && raw.data` (WsEnvelope format), not `raw.type && raw.data`
- Since `ChatOutbound` has `type` not `event`, the browser treats it as a flat message
- `msg.type` = `"stream_chunk"` correctly routes to `handleStreamChunk`
- But `msg.text` is undefined — the actual text is in `msg.data.text`

**However**, looking more carefully: the `ChatOutbound.data` field contains the raw re-broadcast JSON, which for agent messages includes `{"type":"stream_chunk","text":"Hello"}`. So the browser gets:
```json
{"type": "stream_chunk", "data": {"type": "stream_chunk", "text": "Hello"}}
```

Since `raw.event` is undefined (ChatOutbound uses `type` not `event`), the browser uses `msg = raw`, giving `msg.type = "stream_chunk"`. But `msg.text` is still undefined — the text is at `msg.data.text`.

**Wait** — re-examining chat.rs line 96-104 more carefully. The `dominated` variable is checked, then at line 121 a `ChatOutbound` is constructed. But `ChatOutbound` has fields `msg_type` (serialized as `"type"`) and `data`. This is NOT a `WsEnvelope`. The browser's unwrap code checks `raw.event && raw.data` — `ChatOutbound` has no `event` field, so `raw.event` is falsy, and the browser uses `msg = raw` directly.

So `msg = {"type":"stream_chunk","data":{"type":"stream_chunk","text":"Hello"}}`.

`handleStreamChunk(msg)` accesses `msg.text` (line 457) — this is **undefined**. The text is at `msg.data.text`.

**This appears to be a real bug** unless there is something else at play. Let me verify what the browser actually does:

```javascript
// line 457
body.dataset.raw = (body.dataset.raw || "") + msg.text;
```

If `msg.text` is `undefined`, then `body.dataset.raw` would become `"undefined"` (string concatenation of undefined). This would render the literal word "undefined" in the chat.

**Resolution path**: Either:
1. The chat handler should send `WsEnvelope` (with `event` field) instead of `ChatOutbound`, or
2. The browser should also check for `raw.type && raw.data` and unwrap appropriately, or
3. The `ChatOutbound` should flatten the data fields into the top level

**Alternatively**, in practice, the agent may connect via the `/ws/chat` endpoint directly, meaning agent messages arrive as raw JSON in `recv_task`, get re-broadcast as `WsEnvelope{topic, event, data}` (line 159-163), and then the send_task picks them up from the broadcast channel. The send_task wraps them in `ChatOutbound{type: event, data: envelope.data}`. So for a stream_chunk, `envelope.data` is the original raw JSON `{"type":"stream_chunk","text":"Hello"}`, and `ChatOutbound` becomes `{"type":"stream_chunk","data":{"type":"stream_chunk","text":"Hello"}}`.

The browser gets this, `raw.event` is undefined (no `event` field), so `msg = raw`. `msg.text` is undefined. **This is a bug.**

**HOWEVER** — there is another possibility. If the LLM bridge mode is used (no agent), the hub sends:
```json
{"type": "chat_message", "data": {"content": "response text"}}
```
And the browser handler does `msg.data ? msg.data.content : (msg.content || "")` (line 357) — this explicitly reaches into `msg.data`. So the `chat_message` handler is correctly written for the ChatOutbound format.

But `stream_chunk`, `tool_call`, `tool_result`, and `token_update` handlers all expect flat fields (`msg.text`, `msg.tool_name`, `msg.input_tokens`, etc.), not nested under `msg.data`.

**Verdict**: The LLM bridge path works correctly. The agent relay path has an envelope mismatch bug where agent event payloads are nested under `data` but browser handlers expect flat fields. This bug would cause:
- `stream_chunk`: renders "undefined" text
- `tool_call`: tool_name is undefined, tool_input is undefined
- `tool_result`: tool_name is undefined, content is undefined
- `token_update`: all values are undefined/0

### 4.2 Agent → Hub (hub_client.rs recv)

The agent's `recv()` correctly handles both envelope and flat formats (lines 99-118):
1. Tries to parse as JSON Value
2. If it has `event` + `data` fields (WsEnvelope format), reconstructs as `{"type": event, ...data_fields}`
3. Falls back to direct HubMessage parse

This is robust and handles the `ChatOutbound` format from the hub correctly since it checks for `event` (WsEnvelope) but falls back to direct parse (which would handle `ChatOutbound`'s `type` field).

**Wait** — the agent receives from the hub's send_task which sends `ChatOutbound` format (type + data). The agent's recv tries `envelope.get("event")` — ChatOutbound has `type` not `event`, so this is None. Falls to direct parse: `serde_json::from_str::<HubMessage>(&text)`. ChatOutbound has `{"type":"chat_message","data":{"content":"hello"}}` — this would need to match `HubMessage::ChatMessage{content}` but the content is nested under `data.content`, not at top level.

**Actually** — the agent connects to `/ws/chat`. When the browser sends a `chat_message`, the hub's `recv_task` parses it as `ChatInbound::ChatMessage{content}` and broadcasts a `WsEnvelope{topic:"agent:broadcast:input", event:"chat_message", data:{sessionId, content}}`. The agent's recv reads from its own WebSocket stream (not the broadcast channel). But wait — the agent is connected to the same `/ws/chat` endpoint. The hub's send_task for this connection subscribes to the broadcast channel and forwards matching messages wrapped in `ChatOutbound`.

So the agent receives `{"type":"chat_message","data":{"sessionId":"...","content":"hello"}}`. The agent's recv:
1. Parses as Value — gets `{type, data}`
2. Checks for `event` field — not present (it's `type`)
3. Falls to direct parse: `serde_json::from_str::<HubMessage>` on `{"type":"chat_message","data":{...}}`
4. HubMessage::ChatMessage expects `{"type":"chat_message","content":"..."}` — but the actual JSON has `data` not `content`

**This is another envelope mismatch bug in the agent relay direction.** The agent would fail to parse the chat_message and it would become `HubMessage::Unknown`, which is silently skipped.

**UNLESS** — looking at ChatOutbound more carefully: the send_task creates `ChatOutbound{msg_type: envelope.event, data: envelope.data}`. For a `chat_message` broadcast, `envelope.data = {"sessionId":"...", "content":"hello"}`. So ChatOutbound serializes as `{"type":"chat_message","data":{"sessionId":"...","content":"hello"}}`.

The agent's direct parse expects `{"type":"chat_message","content":"hello"}`. The JSON has `data` wrapping the content. **Parse fails → Unknown → skipped.**

**This means the agent never receives chat messages from the hub in the current implementation.**

Wait — let me re-read the agent's recv more carefully. Line 101-111:

```rust
let msg: HubMessage = if let Ok(envelope) = serde_json::from_str::<serde_json::Value>(&text) {
    if let (Some(event), Some(data)) = (envelope.get("event").and_then(|e| e.as_str()), envelope.get("data")) {
        // WsEnvelope path — NOT taken (ChatOutbound has "type" not "event")
    } else {
        // Direct parse
        serde_json::from_str(&text).unwrap_or(HubMessage::Unknown)
    }
}
```

Direct parse of `{"type":"chat_message","data":{"sessionId":"...","content":"hello"}}` against `HubMessage::ChatMessage{content}` — serde would look for a `content` field at the top level. It's not there (it's nested under `data`). **Parse fails.**

**BUT** — serde's `#[serde(tag = "type")]` internally tagged enum representation means the JSON object is the variant's fields. For `ChatMessage{content}`, serde expects `{"type":"chat_message","content":"..."}`. The presence of extra fields (`data`) depends on serde's `deny_unknown_fields` setting. By default, serde **ignores** unknown fields. But the issue is that `content` is MISSING at the top level — it's inside `data`. So this still fails.

**Unless** there is a `#[serde(flatten)]` or similar. Looking at the HubMessage enum (hub.rs), `ChatMessage` just has `content: String`. No flatten. So yes, the parse fails.

### 4.3 Summary of Envelope Issues

| Direction | Format Sent | Format Expected | Works? |
|-----------|-------------|-----------------|--------|
| Hub → Browser (LLM bridge) | `ChatOutbound{type, data:{content}}` | `handleMessage` checks `msg.data.content` | YES |
| Hub → Browser (agent relay) | `ChatOutbound{type, data:{type,text,...}}` | handlers expect `msg.text`, `msg.tool_name` | NO — fields nested under `data` |
| Hub → Agent (chat_message) | `ChatOutbound{type:"chat_message", data:{content}}` | `HubMessage::ChatMessage{content}` | NO — content nested under data |
| Agent → Hub (all types) | Flat `HubMessage` JSON `{type, ...fields}` | ChatInbound parse → fallback re-broadcast | YES (via fallback path) |
| LLM bridge → Browser (status) | `ChatOutbound{type:"agent_status", data:{status}}` | `handleAgentStatus(msg)` checks `msg.status` | NO — status nested under `data` |

**Root cause**: The `ChatOutbound` struct wraps envelope data inside a `data` field, but:
1. It uses `type` instead of `event` so the browser's WsEnvelope unwrapper doesn't activate
2. Browser handlers for agent events expect flat fields, not `data`-wrapped fields
3. The agent's recv expects WsEnvelope (`event` field) or flat HubMessage — ChatOutbound is neither

---

## 5. Missing Features for Production Readiness

### 5.1 Error Handling Gaps

| Gap | Severity | Detail |
|-----|----------|--------|
| No error message to browser on parse failure | LOW | Hub silently drops unparseable inbound messages (chat.rs:154, ws.rs:101) |
| No error feedback on unauthorized publish | MEDIUM | ws.rs:117 silently drops unauthorized publish attempts |
| Agent disconnect not signaled to browser | HIGH | When agent's WS closes, no `agent_disconnected` event is sent to the chat UI. The status pill stays on its last value |
| Broadcast channel overflow | MEDIUM | Channel capacity is 512. Lagged receivers skip messages (lines chat.rs:131, ws.rs:78) with no notification to the client |
| LLM bridge timeout | MEDIUM | No timeout on the Anthropic API call (chat.rs:291). A hung request blocks indefinitely |

### 5.2 Reconnection Logic

| Aspect | Browser | Agent |
|--------|---------|-------|
| Auto-reconnect | YES — exponential backoff, 1s → 30s max (chat.html:334-341) | NO — agent exits loop on disconnect (main.rs:319) |
| State recovery on reconnect | NO — conversation history lost, turn count preserved only in JS state | NO |
| Reconnect indicator | YES — conn-dot turns red, label shows "disconnected" | N/A |
| Session continuity | NO — new session_id on each connect, no way to resume | NO |

### 5.3 Message Ordering Guarantees

| Aspect | Status |
|--------|--------|
| Per-connection ordering | YES — WebSocket guarantees in-order delivery per connection |
| Cross-connection ordering | NO — broadcast channel is unordered between subscribers |
| Sequence numbers | NOT IMPLEMENTED — no message IDs or sequence numbers |
| Duplicate detection | NOT IMPLEMENTED — no dedup mechanism |
| Message acknowledgment | NOT IMPLEMENTED — fire-and-forget on all paths |

### 5.4 Authentication Flow

| Aspect | Status | Detail |
|--------|--------|--------|
| Token validation | BASIC | Query param `?token=xxx` compared against `state.auth_token` |
| Token per message | NO | Auth checked once at connection time only |
| Token rotation | NO | No mechanism to rotate tokens without restart |
| Agent auth | SAME | Agent uses same `?token=xxx` mechanism |
| Browser auth | PROMPT | Falls back to `prompt()` dialog if no token in URL |
| Auth failure feedback | PARTIAL | `authenticated` flag sent in welcome message, but unauthenticated clients can still receive broadcasts — only publishing and agent spawning are blocked |

### 5.5 Additional Missing Features

| Feature | Status |
|---------|--------|
| Message persistence | NO — messages exist only in broadcast channel (ephemeral) and browser DOM |
| Typing indicators | NO |
| Read receipts | NO |
| Multi-agent routing | PARTIAL — `agent_id` filter exists but agent doesn't send its ID in most messages |
| Rate limiting | NO — no throttle on inbound messages |
| Message size limits | NO — no payload size validation |
| Streaming responses (LLM bridge) | NO — LLM bridge uses non-streaming API call |
| Conversation export | NO |
| Agent lifecycle management | PARTIAL — `SpawnAgent` exists but no list/stop/restart controls |

---

## 6. Protocol Specification (Current State)

### 6.1 Connection

**Browser → Hub**:
```
GET /ws/chat?token=<auth_token>[&agent_id=<id>] → WebSocket upgrade
```

**Agent → Hub**:
```
GET /ws/chat?token=<auth_token> → WebSocket upgrade
```

### 6.2 Hub → Client Welcome

```json
{"type": "connected", "data": {"sessionId": "uuid", "authenticated": true, "agentId": null, "llmBridge": true}}
```

### 6.3 Inbound Messages (Browser/Agent → Hub)

| Type | Format | Auth Required |
|------|--------|---------------|
| `chat_message` | `{"type":"chat_message","content":"text","agent_id":"optional"}` | No (but response may be limited) |
| `connect_agent` | `{"type":"connect_agent","agent_id":"id"}` | No |
| `spawn_agent` | `{"type":"spawn_agent","project_dir":"path","model":"optional","agent_name":"optional"}` | Yes |

Any JSON with a `type` field that doesn't match `ChatInbound` variants is re-broadcast as-is.

### 6.4 Outbound Messages (Hub → Browser)

All wrapped in `ChatOutbound`:
```json
{"type": "<event>", "data": <payload>}
```

| Event | Payload | Source |
|-------|---------|--------|
| `connected` | `{sessionId, authenticated, agentId, llmBridge}` | Hub welcome |
| `stream_chunk` | `{type, text}` | Agent relay |
| `tool_call` | `{type, tool_name, tool_input}` | Agent relay |
| `tool_result` | `{type, tool_name, content, is_error}` | Agent relay |
| `token_update` | `{input_tokens, output_tokens, total_input, total_output}` | Agent or LLM bridge |
| `agent_status` | `{status, detail}` | Agent or LLM bridge |
| `chat_message` | `{content}` | LLM bridge |
| `agent_connected` | `{agentId}` | Hub control |
| `agent_spawned` | `{agent}` | Hub control |
| `spawn_error` | `{error}` | Hub control |

### 6.5 Agent-Specific Messages (HubMessage enum)

| Variant | Wire Name | Direction | Fields |
|---------|-----------|-----------|--------|
| Register | `agent_register` | Agent → Hub | `agent_id, agent_name, project_dir` |
| StreamChunk | `stream_chunk` | Agent → Hub | `text` |
| ToolCall | `tool_call` | Agent → Hub | `tool_name, tool_input` |
| ToolResultMsg | `tool_result` | Agent → Hub | `tool_name, content, is_error` |
| TokenUpdate | `token_update` | Agent → Hub | `input_tokens, output_tokens, total_input, total_output` |
| AgentStatus | `agent_status` | Agent → Hub | `status, detail` |
| ChatMessage | `chat_message` | Hub → Agent | `content` |
| Done | `agent_done` | Hub → Agent | `agent_id, summary, exit_code` |
| Connected | `connected` | Hub → Agent | `session_id, authenticated` |

---

## 7. Critical Findings Summary

### Bugs (must fix)

1. **Envelope double-wrapping in agent relay path**: `ChatOutbound` nests agent event data under a `data` field, but browser handlers expect flat fields. Stream chunks would render as "undefined", tool calls would show undefined tool names, and token updates would be NaN/0. This is the most critical bug.

2. **Agent cannot receive chat messages**: The `ChatOutbound` wrapper puts `content` inside `data`, but `HubMessage::ChatMessage` expects `content` at the top level. The agent's recv parser cannot unwrap `ChatOutbound` format (it checks for `event` field, which `ChatOutbound` doesn't have). Chat messages are silently dropped.

3. **Token totals reset each turn**: Agent sends `total_input = input_tokens as u64` (per-turn value, not cumulative). Browser's `handleTokenUpdate` uses `msg.total_input` to overwrite `state.totalInput`, so the gauge reflects only the latest turn.

### Design Issues (should fix)

4. **No agent registry**: Registration messages are re-broadcast but not stored. No way to list connected agents or route messages to specific agents.

5. **No agent disconnect notification**: Browser has no way to know when an agent drops off.

6. **`agent_status` forwarding is implicit**: Works only because the topic starts with `"agent:"`, not because it's in the event allowlist. Fragile.

7. **LLM bridge is non-streaming**: Full response is buffered before sending, giving poor UX for long responses.

### Missing (production gaps)

8. No message persistence or replay
9. No agent reconnection logic
10. No message ordering guarantees beyond WebSocket ordering
11. No rate limiting or payload size validation
12. Auth only at connection time, no per-message auth
13. No timeout on Anthropic API calls in LLM bridge

---

## 8. Recommended Fix Priority

| Priority | Item | Effort |
|----------|------|--------|
| P0 | Fix envelope mismatch (item 1) — either send raw WsEnvelope from send_task or flatten ChatOutbound | Small |
| P0 | Fix agent chat_message receipt (item 2) — same root cause as item 1 | Small |
| P1 | Cumulative token tracking in agent (item 3) | Small |
| P1 | Agent disconnect notification (item 5) | Small |
| P2 | Agent registry in hub state (item 4) | Medium |
| P2 | Streaming LLM bridge (item 7) | Medium |
| P3 | Message persistence (item 8) | Large |
| P3 | Rate limiting and payload validation (items 11, 12) | Medium |
