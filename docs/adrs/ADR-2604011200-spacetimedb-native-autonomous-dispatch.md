# ADR-2604011200: SpacetimeDB-Native Autonomous Inference Dispatch

**Status:** Superseded by ADR-2604112000 (Hex Standalone Dispatch)
**Date:** 2026-04-01
**Supersedes:** Nothing — closes the final gap in ADR-2604010000 (Unified Execution Path)
**Extends:** ADR-2604010000 (Path B routing), ADR-046 (SpacetimeDB single authority), ADR-027 (HexFlo coordination), ADR-060 (agent inbox)

---

## Context

ADR-2604010000 established two execution paths for the workplan executor:

- **Path A**: spawn `hex-agent` subprocess (non-CC environment)
- **Path B**: enqueue to `hexflo_memory` + inbox notification → outer CC session dispatches Agent tool

Path B was proven correct today (`feat-fix-task-list` ran 3 phases, 4 tasks, 2 gates, 0 failures, 7 minutes). However, one structural gap remains:

**Path B requires an active `UserPromptSubmit` cycle to drain the inference queue.**

The outer CC session checks its inbox inside the `hex hook route` (`UserPromptSubmit` hook). This means:

1. If no user message arrives, queued tasks sit idle indefinitely
2. The "autonomous" label is not yet earned — a human must be present to trigger each dispatch cycle
3. Between phases, the executor waits for a human-triggered hook to fire

This gap exists because we are using `hexflo_memory` (a general key-value store) as the inference queue. It has no event semantics — it cannot push notifications. The executor polls every 5 seconds; the outer session polls only on user input.

**SpacetimeDB solves this natively.**

SpacetimeDB is our coordination backbone precisely because it has real-time push semantics. Every connected client receives table-row insertions via WebSocket subscription immediately — no polling, no `UserPromptSubmit` dependency. hex-nexus is already a SpacetimeDB client with active subscriptions. We are not using this capability for inference dispatch.

---

## Decision

Replace `hexflo_memory` as the inference queue with a **dedicated SpacetimeDB table** (`inference_task`). hex-nexus subscribes to this table and relays new tasks to connected CC agents via a persistent WebSocket channel (`/ws/inference`). CC agents connect at session start and receive task pushes in real-time — no polling, no user input required.

This makes the AAIDE loop fully autonomous:

```
workplan executor
  → STDB reducer: inference_task_create(id, prompt, workplan_id, task_id)
  → STDB subscription fires in hex-nexus (< 10ms)
  → hex-nexus broadcasts to /ws/inference subscribers
  → hex-agent WS listener receives task
  → hex-agent dispatches Agent tool (background, bypassPermissions)
  → Agent completes → PATCH inference_task status=Completed
  → executor STDB subscription fires completion
  → executor advances to next phase
```

No `UserPromptSubmit`. No human present. No polling delay beyond STDB latency.

---

## Architecture

### 1. STDB Table: `inference_task` (hexflo-coordination WASM)

```rust
#[table(name = inference_task, public)]
pub struct InferenceTask {
    #[primary_key]
    pub id: String,           // UUID
    pub workplan_id: String,
    pub task_id: String,      // e.g. "P1.T2"
    pub phase: String,        // phase name
    pub prompt: String,       // full agent prompt
    pub role: String,         // agent role (coder, reviewer, etc.)
    pub status: String,       // "Pending" | "InProgress" | "Completed" | "Failed"
    pub agent_id: String,     // assigned CC agent (empty until claimed)
    pub result: String,       // populated on completion
    pub created_at: String,
    pub updated_at: String,
}
```

New reducers:
- `inference_task_create(id, workplan_id, task_id, phase, prompt, role, created_at)`
- `inference_task_claim(id, agent_id, updated_at)` — CAS: only transitions Pending → InProgress
- `inference_task_complete(id, result, updated_at)`
- `inference_task_fail(id, reason, updated_at)`
- `inference_task_list_pending()` — query helper

### 2. hex-nexus: STDB Subscription + WebSocket Relay

hex-nexus adds a subscription to `inference_task` on startup (alongside existing subscriptions). On new row insert with `status = "Pending"`:

1. Find all connected CC agents via `/ws/inference` channel
2. Broadcast `InferenceTaskPush { id, task_id, workplan_id, prompt, role }` as JSON
3. Target the agent registered for this workplan_id first (from HexFlo memory); fall back to broadcast

New endpoint: `GET /ws/inference` — WebSocket upgrade. Authenticated via `X-Hex-Agent-Id` header. Nexus maintains a `HashMap<AgentId, WsSender>` of connected inference subscribers.

### 3. Workplan Executor: Write to STDB

Replace in `workplan_executor.rs`:

```rust
// BEFORE (Path B via hexflo_memory)
hexflo.memory_store(&key, &json_payload, "global").await?;

// AFTER (Path B via STDB inference_task)
state.inference_task_create(&id, workplan_id, task_id, phase, &prompt, role, &now).await?;
```

Replace completion polling:

```rust
// BEFORE: poll hexflo_memory every 5s
loop {
    let val = hexflo.memory_retrieve(&key).await?;
    if val["status"] == "Completed" { break; }
    sleep(5s).await;
}

// AFTER: subscribe to inference_task table, wait for status change
// (executor already holds a live STDB connection — subscribe to row updates for this task_id)
```

### 4. CC Agent: WebSocket Listener at Session Start

`hex hook session-start` (after agent registration) spawns a background process:

```bash
hex inference watch --agent-id $AGENT_ID
```

`hex inference watch` connects to `ws://localhost:5555/ws/inference`, authenticates with agent ID, and on each `InferenceTaskPush` message:

1. Claims the task via `inference_task_claim` (CAS — only one agent wins per task)
2. If claim succeeds: spawns `claude -p --dangerously-skip-permissions` with the prompt...

Wait — we don't use `claude -p`. This is the outer CC session.

**Revised**: `hex inference watch` is not a subprocess. Instead, the `UserPromptSubmit` hook already fires on every CC session interaction. The missing piece is a hook that fires on **WebSocket push without user input**.

Claude Code does not have a WebSocket-triggered hook today. Therefore the approach must be:

**Option A (short-term)**: Keep `UserPromptSubmit` as the trigger, but use STDB subscription in hex-nexus to ensure the inbox notification arrives within milliseconds of task creation (vs. 5-10s polling delay). The CC session still needs to be active — but latency drops from seconds to milliseconds and reliability improves significantly.

**Option B (medium-term)**: `hex inference watch` is a standalone process that runs alongside the CC session. It connects to `/ws/inference` and spawns subagent dispatches via the MCP tool protocol (stdio). This requires the `hex mcp` server to expose an `invoke_agent` tool that spawns background agents. The watch process sends MCP tool calls to the already-running `hex mcp` server instance.

**Option C (long-term — the true AAIDE)**: Claude Code exposes a hook event for external push (e.g., a FIFO or Unix domain socket that triggers a synthetic `UserPromptSubmit`). hex could write a synthetic prompt to this socket when a task arrives, waking the CC session without human input.

### Chosen Path: A now, B concurrent, C as Anthropic API matures

- **Immediately**: STDB table replaces hexflo_memory → better reliability, native event semantics, audit trail per task
- **Phase 2**: `/ws/inference` endpoint + `hex inference watch` with Option B dispatch via MCP stdio
- **Phase 3**: Contribute synthetic wake-on-push to Claude Code (or use cc.ai API when available)

---

## State Port Extension

`IStatePort` gains inference task methods:

```rust
async fn inference_task_create(&self, id: &str, workplan_id: &str, task_id: &str, phase: &str, prompt: &str, role: &str, created_at: &str) -> Result<(), StateError>;
async fn inference_task_claim(&self, id: &str, agent_id: &str, updated_at: &str) -> Result<(), StateError>;
async fn inference_task_complete(&self, id: &str, result: &str, updated_at: &str) -> Result<(), StateError>;
async fn inference_task_fail(&self, id: &str, reason: &str, updated_at: &str) -> Result<(), StateError>;
async fn inference_task_list_pending(&self) -> Result<Vec<InferenceTaskInfo>, StateError>;
async fn inference_task_get(&self, id: &str) -> Result<Option<InferenceTaskInfo>, StateError>;
```

---

## Consequences

**Positive:**
- Inference tasks are first-class STDB rows — visible in dashboard, queryable, auditable
- Push semantics eliminate all polling in the executor hot path
- `hexflo_memory` queue namespace (`inference:queue:*`) is retired — no more ephemeral key-value hacks
- Task history is permanent — post-mortem analysis, cost attribution, retry logic all become trivial
- Multiple CC agents can compete for tasks via `inference_task_claim` CAS — horizontal scale

**Negative:**
- WASM module rebuild required (hexflo-coordination) — ~2 min compile
- Option B (`hex inference watch`) introduces a sidecar process — operators must be aware
- Option C depends on Claude Code platform capability not yet available

**Neutral:**
- Path A (non-CC environment) is unaffected — subprocess spawn continues unchanged
- Existing `hexflo_memory` queue entries from in-flight workplans drain naturally (executor falls back to memory poll if STDB task not found)

---

## Migration

1. Deploy updated `hexflo-coordination` WASM module (additive — no existing tables changed)
2. Update hex-nexus and hex-cli binaries
3. Workplan executor detects at runtime: if STDB supports `inference_task_create` → use new path; else → fall back to `hexflo_memory` (one release overlap)
4. After one release cycle, remove `hexflo_memory` fallback

---

## Success Criteria

1. `hex plan execute <workplan>` completes all phases without any `UserPromptSubmit` event from a human (Option B active)
2. Inference task rows visible in SpacetimeDB with full lifecycle: Pending → InProgress → Completed
3. Dashboard shows live inference task status alongside swarm tasks
4. Workplan executor latency from task-queued to agent-dispatched: < 500ms (vs. current ~5-10s poll interval)
5. `hex inference watch` process survives nexus restart and reconnects automatically
