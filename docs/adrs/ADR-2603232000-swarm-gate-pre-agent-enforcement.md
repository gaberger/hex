# ADR-2603232000: Swarm-Gate Enforcement at Pre-Agent Hook

**Status:** Accepted
**Date:** 2026-03-23
**Drivers:** Both Claude Code and OpenCode allow agent spawning without verifying an active HexFlo swarm exists. Agents burn tokens running exploration then get blocked only at first file edit. OpenCode's hook-handler.cjs does string matching on `HEXFLO_TASK` but never validates swarm state against nexus.
**Supersedes:** None (extends ADR-2603221939 agent lifecycle enforcement)

## Context

hex enforces a strict development pipeline: **ADR → Workplan → Swarm → Task → Agent**. The pre-agent hook (`hex-cli/src/commands/hook.rs:654`) currently validates two of these gates for background agents:

1. **Workplan exists** — checks `workplan_id` in SessionState (line 672-696)
2. **HEXFLO_TASK in prompt** — checks for `HEXFLO_TASK:{uuid}` string (line 698-711)

However, it does NOT validate that an **active HexFlo swarm** exists. The swarm check only happens later in `pre_edit()` (line 575-599), meaning:

- An agent can spawn, run exploration, consume tokens, and only get blocked when it attempts its first file write
- The `HEXFLO_TASK` check validates the task exists (line 728-746) but not that the task belongs to an active swarm
- A task could reference a completed or stale swarm

**OpenCode gap**: `hook-handler.cjs` (line 429-457) performs string matching for `HEXFLO_TASK:` in the prompt but never makes a REST call to validate swarm existence. It blocks agents without the marker string, but accepts any UUID without verification.

**Three enforcement surfaces exist today:**

| Surface | Workplan? | Task marker? | Task exists? | Swarm exists? |
|---------|-----------|-------------|--------------|---------------|
| `pre_agent()` (Rust) | ✅ | ✅ | ✅ (best-effort) | ❌ |
| `pre_edit()` (Rust) | ✅ | — | — | ✅ |
| `hook-handler.cjs` (Node) | ❌ | ✅ (string only) | ❌ | ❌ |

The swarm check should happen at the earliest enforcement point — agent spawn — not at first edit.

## Decision

### 1. Add swarm existence check to `pre_agent()` (Rust)

After the workplan check (line 696) and before the `HEXFLO_TASK` check (line 698), add:

```rust
// Check active swarm exists in SessionState
if is_background {
    let has_swarm = SessionState::load()
        .and_then(|s| s.swarm_id)
        .is_some();

    if !has_swarm {
        if mode == "mandatory" {
            println!("⛔ Background agent blocked — no active HexFlo swarm");
            println!("  Pipeline: ADR → Workplan → Swarm → Task → Agent");
            println!("  Create a swarm first: hex swarm init <name>");
            std::process::exit(2);
        } else {
            println!("⚠️ Agent spawned without active swarm — coordination disabled");
        }
    }
}
```

### 2. Add swarm validation to `pre_agent()` task check

When validating that `HEXFLO_TASK:{uuid}` exists (line 728-746), also verify the task's parent swarm is in `active` status. Extend the existing nexus REST call to check the response body:

```rust
Ok(resp) if resp.status().is_success() => {
    if let Ok(body) = resp.json::<serde_json::Value>().await {
        let swarm_status = body["swarm_status"].as_str().unwrap_or("");
        if swarm_status != "active" {
            println!("⛔ HEXFLO_TASK belongs to {} swarm — cannot proceed", swarm_status);
            std::process::exit(2);
        }
    }
}
```

### 3. Add swarm validation to `hook-handler.cjs` (OpenCode path)

After the `HEXFLO_TASK` string check (line 429-435), add a nexus REST call:

```javascript
// Validate swarm exists via nexus
if (dispatchedViaHexAgent && taskId) {
    try {
        const resp = await fetch(`http://localhost:5555/api/hexflo/tasks/${taskId}`);
        if (!resp.ok) {
            console.error(`[BLOCKED] HEXFLO_TASK:${taskId.slice(0,8)} not found in nexus`);
            process.exit(1);
        }
        const task = await resp.json();
        if (task.swarm_status !== 'active') {
            console.error(`[BLOCKED] Task belongs to ${task.swarm_status} swarm`);
            process.exit(1);
        }
    } catch {
        // Nexus unreachable — degrade to advisory
        console.warn('[WARN] Could not validate swarm — nexus unreachable');
    }
}
```

### 4. Extend nexus `/api/hexflo/tasks/:id` response

The task lookup endpoint must include `swarm_status` in its response body so both enforcement surfaces can validate swarm health without a second REST call.

### 5. Enforcement mode applies uniformly

All three checks respect the project's `lifecycle_enforcement` setting (`mandatory` | `advisory`) from `.hex/project.json`. Default: `mandatory`.

## Consequences

**Positive:**
- Agents are blocked at spawn time, not at first edit — saves tokens and reduces confusion
- Both Claude Code and OpenCode enforce the same swarm gate
- Stale/completed swarms are caught before work begins
- Single REST call validates both task and swarm (no extra round-trip)

**Negative:**
- Adds ~50ms latency to agent spawn (one nexus REST call)
- Nexus-unreachable scenario falls back to advisory (fail-open) — offline work cannot validate swarm state
- OpenCode's hook-handler.cjs gains an async dependency (fetch call)

**Mitigations:**
- REST call has 2-second timeout (matches existing `nexus_client(2)` pattern)
- Advisory fallback is logged so developers are aware enforcement is degraded
- hook-handler.cjs already runs in async context (Node.js hook handler supports await)

## Implementation

| Phase | Description | Status |
|-------|------------|--------|
| P1 | Extend `/api/hexflo/tasks/:id` response to include `swarm_status` | Done |
| P2 | Add swarm existence check to `pre_agent()` in hook.rs (SessionState) | Done |
| P3 | Add swarm status validation to `pre_agent()` task check in hook.rs | Done |
| P4 | Add nexus REST validation to `hook-handler.cjs` pre-task handler | Done |
| P5 | Add integration test: spawn agent without swarm → expect exit(2) | Deferred |
| P6 | Update ADR-2603221939 to reference this ADR as extension | Done |

## References

- ADR-2603221939: Agent lifecycle enforcement (workplan + task requirements)
- ADR-2603231700: Worktree enforcement in agent hooks
- `hex-cli/src/commands/hook.rs:654` — `pre_agent()` function
- `hex-setup/helpers/hook-handler.cjs:420` — OpenCode pre-task handler
- `hex-nexus/src/coordination/mod.rs` — HexFlo swarm coordination
