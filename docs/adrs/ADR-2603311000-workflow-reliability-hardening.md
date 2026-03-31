# ADR-2603311000: Workflow Reliability Hardening

**Status:** Accepted
**Date**: 2026-03-31
**Context**: Post-E2E analysis of recurring failure modes in the hex dev pipeline

---

## Problem

E2E testing on 2026-03-31 exposed five structural root causes behind repeated workflow failures:

1. **JSON shape drift** — `report_done` PATCH body and the nexus handler share no type. Missing `agent_id` and wrong `status` values slipped through undetected.
2. **Silent failures** — inference returning empty content scored 0.85 quality; `report_done` sent `"completed"` regardless of inner `success`; fingerprint injection silently skipped.
3. **No inference pre-validation** — OpenRouter model IDs registered with placeholder names (`gpt-5.4`, `grok-4.20`) that don't exist return empty responses; nothing blocks their use.
4. **Agent session lifecycle incomplete** — sessions accumulate as `running` forever; `on_disconnect` doesn't close them; heartbeat protocol has no termination path.
5. **Config managed by hand** — `HEX_MODEL`, `HEX_AGENT_ID` in a shell script; running daemon silently uses stale values until manually restarted.

---

## Decision

### P0 — Shared TaskCompletionBody (hex-core)

Move the task completion JSON contract to `hex-core` as a shared Rust type:

```rust
// hex-core/src/types/task_completion.rs
#[derive(Serialize, Deserialize)]
pub struct TaskCompletionBody {
    pub status: TaskStatus,   // enum: completed | failed
    pub result: String,
    pub agent_id: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus { Completed, Failed }
```

Both `hex-agent` (sender) and `hex-nexus` (receiver) import this type. Mismatches become compiler errors.

### P1 — Inference Quality Gate

`hex inference add` validates the model before persisting:
- Sends a minimal test prompt
- Rejects registration if response is empty or HTTP error
- Prunes existing registered models that return empty on `hex inference discover --prune`

The `hex inference test` command already exists; gate registration on it.

### P2 — Agent Session Lifecycle

Add session termination to the `session-end` hook:

```bash
# hex-cli/assets/helpers/hook-handler  (session-end event)
hex agent disconnect "$CLAUDE_SESSION_ID"
```

`hex agent disconnect` sets `endedAt = now`, `status = completed` on the agent record. This requires a new `POST /api/hex-agents/:id/disconnect` route in nexus.

### P3 — Pre-flight Task Dispatch

Before the supervisor dispatches a task to a remote worker:
1. Check `HexFlo::agent_is_alive(agent_id)` — heartbeat within last 45s
2. Check `inference_reachable(worker_nexus_url)` — HEAD `/api/health`
3. If either fails: skip this agent, try next available, or surface error immediately

### P4 — Stale Model Pruning

`hex inference discover --prune` tests all registered OpenRouter models and removes those returning empty content. Run automatically on `hex nexus start`.

---

## Consequences

- **Positive**: Compiler-enforced contract between daemon and nexus; no more silent empty-inference completions; sessions properly closed; stale models removed.
- **Negative**: P0 requires a hex-core change that touches both hex-agent and hex-nexus; short build time increase.
- **Out of scope**: Full E2E test harness (separate ADR); quality gate on generated code content (tracked in ADR-035 v2 swarm quality orchestration).
