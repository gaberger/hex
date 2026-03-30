# ADR-2603300100: hex-agent as First-Class SpacetimeDB WebSocket Client

**Status:** Accepted
**Date:** 2026-03-30
**Drivers:** Docker sandbox workers use REST polling for task coordination and inference, creating fragile multi-hop chains that fail silently. The correct architecture is hex-agent connecting directly to SpacetimeDB via WebSocket — the same protocol the dashboard uses.
**Supersedes:** ADR-2603291900 (Docker Worker First-Class Execution — subsumed)

## Context

### Current Architecture (Broken)

```
hex-agent (Docker)
  → REST poll: GET /api/hexflo/tasks/claim      (5s polling, 403 on missing header)
  → REST exec: hex dev start --auto <desc>       (full inner pipeline — ADR/workplan/code/test)
  → REST done: PATCH /api/hexflo/tasks/{id}      (silently 403; task_complete reducer 404)
  → Inference: POST /api/inference/complete      (falls through 5 models → trinity-mini:free)
```

Four signal paths, all broken:
1. **Task claim**: polling every 5s → latency and missed assignments
2. **Task execution**: runs a full re-entrant pipeline for a single step (O(n²) work)
3. **Task completion**: PATCH returns 403 (missing agent-id header) OR 404 (SpacetimeDB reducer missing)
4. **Inference routing**: cascading fallback ignores model capability floor, ends at trinity-mini:free

The root cause: hex-agent treats SpacetimeDB as an opaque service reached only through hex-nexus REST.
Yet hex-nexus exists to bridge WASM (which can't do I/O) — not to be a coordination middleman.

### What SpacetimeDB Already Provides

SpacetimeDB has native WebSocket subscription. The dashboard uses this for all live state.
The `hexflo-coordination` WASM module already has reducers for:
- `task_assign(task_id, agent_id)` → status: in_progress
- `task_complete(task_id, result, completed_at)` → status: completed
- `agent_heartbeat(agent_id, status, current_task_id)` → keeps agent alive

The `inference-gateway` WASM module has:
- `inference_request_create(agent_id, model, messages)` → queues request
- hex-nexus processes the queue (it CAN make network calls) and stores the result
- Agent subscribes to `inference_response` table → gets result pushed

### Why the Inner Pipeline Is Wrong

The Docker worker runs `hex dev start --auto "<step description>"`:
- Creates ADR, workplan, swarm, code phases — for a SINGLE step
- The step description is the ONLY context passed; WorkplanStep JSON is discarded
- Inner pipeline cannot spawn further Docker workers (no Docker socket in container)
- Inner pipeline falls back to inline code generation — this was always the actual execution

The Docker worker should execute ONLY the code generation phase for its assigned step.
The outer supervisor owns the TDD feedback loop; the worker owns a single code-generation turn.

## Decision

Redesign hex-agent as a native SpacetimeDB WebSocket client:

### 1. SpacetimeDB WebSocket Connection

hex-agent establishes a WebSocket connection to SpacetimeDB at startup:
```
SPACETIMEDB_URL (env, default: ws://localhost:3033)
SPACETIMEDB_TOKEN (env, from vault)
```

It subscribes to:
- `SELECT * FROM swarm_task WHERE agent_id = {self.agent_id} AND status = 'pending'`
- `SELECT * FROM inference_response WHERE agent_id = {self.agent_id} AND status = 'ready'`

This replaces REST polling. Assignments arrive as push notifications.

### 2. Structured Task Payload

The supervisor serializes `WorkplanStep` to JSON and stores it in the task's `description` field.
The `title` field retains the human-readable summary.

```json
{
  "step_id": "P0.1",
  "description": "Define domain entities and value objects",
  "tier": 0,
  "language": "typescript",
  "output_dir": "/workspace/f1-race-standings",
  "model_hint": "openai/gpt-4o-mini",
  "context": { "workplan": "...", "existing_files": [] }
}
```

### 3. Code Generation Only (No Inner Pipeline)

When the worker receives a task:
1. Deserialize `WorkplanStep` from task payload
2. Load context: read existing source files from `output_dir`
3. Call `inference_request_create` reducer with step prompt + context
4. Wait for `inference_response` push via subscription
5. Parse generated code → write files to `output_dir`
6. Run compile check (`tsc --noEmit`, `cargo check`, etc.)
7. Call `task_complete(task_id, result_json)` reducer via WebSocket

No ADR phase. No workplan phase. No nested swarm. One round-trip.

### 4. Inference via SpacetimeDB Procedure (no hex-nexus bridge)

SpacetimeDB 1.0 supports `#[spacetimedb::procedure]` — functions that run outside transactions
and can make outbound HTTP calls via `ctx.http`. This means the `inference-gateway` WASM module
can call LLM APIs directly, without any external bridge.

```
hex-agent  → request_inference reducer (WebSocket)
           ← SpacetimeDB inserts row, schedules execute_inference procedure immediately
StDB proc  → reads InferenceRequest + InferenceProvider + InferenceApiKey from DB
           → POST to provider base_url (OpenRouter / Anthropic / Ollama)
           ← writes InferenceResponse row via with_tx
hex-agent  ← receives inference_response push via WS subscription
```

**`InferenceApiKey` table (private)**: hex-nexus calls the `set_api_key` reducer on startup
to store resolved API key values (from OS env / vault). The table is not `public`, so
no client can subscribe to the raw key values.

**`InferenceExecuteSchedule` table**: inserted with `ScheduleAt::Interval(Duration::ZERO)`
so the procedure fires in the next scheduling tick — effectively immediate.

This removes the `InferenceRequestProcessor` from hex-nexus entirely. hex-nexus retains its
correct role: filesystem I/O bridge. SpacetimeDB handles inference end-to-end.

#### Requires: `spacetimedb = { version = "1.0", features = ["unstable"] }` in inference-gateway

The procedure feature is behind the `unstable` Cargo feature flag in spacetimedb 1.x.

### 5. Task Completion Signal

```
hex-agent → task_complete(task_id, result, now) via WebSocket
          ← outer supervisor sees status change via subscription (no polling)
```

The supervisor's poll loop is replaced with a SpacetimeDB subscription on `swarm_task`.

### 6. File I/O Still via hex-nexus REST

WASM cannot write files. hex-agent calls:
- `POST /api/files/write` — write generated file to output_dir
- `GET /api/files/read` — read existing files for context

This is the correct use of hex-nexus: filesystem bridge, not coordination hub.

## Consequences

### Positive
- Task assignments arrive in milliseconds (push vs 5s poll)
- Task completion is guaranteed by SpacetimeDB ACID reducers (no more 403/404)
- Inference routing is handled by the inference-gateway module (no fallback chains)
- No nested pipelines — O(n) work instead of O(n²)
- Consistent with the dashboard's architecture

### Negative
- hex-agent must embed the SpacetimeDB Rust client (spacetimedb-client-sdk)
- Requires `spacetimedb-client-sdk` as a Rust crate dependency in hex-agent
- SpacetimeDB modules (hexflo-coordination, inference-gateway) must be deployed and in sync with hex-nexus expectations — the current `coordination_cleanup` 404 must be resolved first

### Migration Path
1. Fix SpacetimeDB module deployment (resolve `coordination_cleanup` 404)
2. Add `spacetimedb-client-sdk` to hex-agent/Cargo.toml
3. Implement `StdbTaskPoller` replacing `TaskExecutor`'s REST polling with WS subscription
4. Implement `StdbInferenceClient` replacing the nexus REST inference path
5. Update supervisor to encode `WorkplanStep` JSON in task description
6. Update supervisor to subscribe to task status changes instead of REST polling
7. Remove `hex dev start --auto` from task_executor — replace with direct code phase execution

## Alternatives Considered

**REST polling + structured payload only** (ADR-2603291900 approach): Fixes the task payload problem but leaves the polling latency, the 403/404 signal failures, and the inference fallback chain broken. Rejected as a partial fix.

**Server-Sent Events from hex-nexus**: Avoids SpacetimeDB client dependency, but doesn't fix the inference routing or task_complete signal path. Rejected.

## Implementation Order

The spec (`docs/specs/hex-agent-stdb-websocket-client.json`) defines the behavioral acceptance criteria.
The workplan (`docs/workplans/feat-hex-agent-stdb-websocket-client.json`) defines the phased implementation.

Phase 0: Fix SpacetimeDB module deployment (unblock everything) ✅
Phase 1: StdbTaskPoller (subscription replaces polling) ✅
Phase 2: StdbInferenceClient (inference via inference-gateway) ✅
  Phase 2.1: hex-nexus InferenceRequestProcessor (REST bridge — superseded by P2.2) ✅ removed
  Phase 2.2: execute_inference procedure in inference-gateway WASM module ✅
             - `InferenceApiKey` private table + `set_api_key` reducer
             - `InferenceExecuteSchedule` table with `scheduled(execute_inference)`
             - `#[spacetimedb::procedure] fn execute_inference` with `ctx.http.send`
             - hex-nexus `InferenceRequestProcessor` deleted (3 files)
Phase 3: CodePhaseWorker (replace inner pipeline with direct code phase) ✅
Phase 4: Supervisor encodes WorkplanStep as TaskPayload JSON; fix blocking sleeps ✅
Phase 5: Integration test (end-to-end: task → code → complete → outer supervisor sees it) ✅
