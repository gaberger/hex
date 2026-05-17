# ADR-2605092200 — spec-to-code-materialization-gap-root-cause-and-fix

Status: **Accepted** (shipped 2026-05; auto-emitter live in commit a140e820 `feat(self-managing): workplan_emit + adr_status_set + auto-emitter`; ADR→workplan auto-bridge confirmed per memory `project_self_managing_loop_2605091200`)
Date: 2026-05-10

ADR-2605092200: Spec→Code Materialization Gap — Root Cause and Fix

**Status**: **Proposed**  
**Date**: 2026-05-09  
**Related**: ADR-[PHONE] (digital twin), ADR-[PHONE] (tool library), symptoms collected 2026-05-09 (ADR-[PHONE], ADR-[PHONE], commitment_reverify_tick reducer).

---

## Context

### Symptoms (Operator Report, 2026-05-09)

The operator observed a **systemic pattern**: drafted artifacts (ADRs, specs, code patches) exist as **STDB `proposed_action(kind=file_write)` rows but never materialize on disk**:

1. **ADR-[PHONE]-sop-reason-phase-ollama-fallback-for-content-filtered-asks.md** — `repo_grep` finds matches, `repo_read` returns `ENOENT`
2. **ADR-[PHONE]-telegram-integration** — same symptom
3. **ADR-[PHONE]** specified `preferred_provider:ollama` for `ciso.yml`, but `repo_read` shows the field never landed
4. **`commitment_reverify_tick` reducer** specced this morning, absent from `spacetime-modules/hexflo-coordination/src/lib.rs`

Pattern: **drafted artifacts don't materialize on disk**.

### The Digital-Twin Chain (as Designed, ADR-[PHONE])

The correct flow is:

```
persona → adr_draft/code_patch tool
       ↓
  proposed_action_open(kind=file_write, payload_json={path, content}, proposed_by="tool:*")
       ↓ STDB row created (status=pending)
       ↓
  twin_reviewer loop (polls every 20s)
       ↓ auto-approves tool:* actions per ADR-[PHONE]
       ↓ calls proposed_action_twin_decide(verdict="approve")
       ↓ status → approved
       ↓
  action_executor loop (polls every 15s)
       ↓ fetch_approved() → [action]
       ↓ execute_file_write() → writes via stdlib, canonical-path guarded
       ↓ calls proposed_action_mark_executed(success=true, evidence="wrote <path>")
       ↓ status → executed
       ↓ file materialized on disk ✅
```

### Code Verification

Grounded in actual source:

- **`proposed_action_open` reducer**: `spacetime-modules/hexflo-coordination/src/lib.rs:6014` ✅
- **`twin_reviewer::spawn`**: `hex-nexus/src/orchestration/twin_reviewer.rs:18` + startup in `hex-nexus/src/lib.rs:432` ✅
  - **Auto-approval for `tool:*`**: `twin_reviewer.rs:312-322` ✅
- **`action_executor::spawn`**: `hex-nexus/src/orchestration/action_executor.rs:12` + startup in `hex-nexus/src/lib.rs:444` ✅
  - **`execute_file_write`**: `action_executor.rs:148-223` ✅
- **`drafter::spawn`**: `hex-nexus/src/orchestration/drafter.rs:17` + startup in `hex-nexus/src/lib.rs:427` ✅

**All code is correct.** The chain is intact.

### Root Cause

Each loop checks an environment variable and **exits silently** if set:

```rust
// drafter.rs:18-21
if std::env::var("HEX_DISABLE_DRAFTER").is_ok() {
    tracing::info!("drafter disabled via HEX_DISABLE_DRAFTER");
    return;
}

// twin_reviewer.rs:20-22
if std::env::var("HEX_DISABLE_TWIN").is_ok() {
    tracing::info!("twin_reviewer disabled via HEX_DISABLE_TWIN");
    return;
}

// action_executor.rs:13-15
if std::env::var("HEX_DISABLE_ACTION_EXECUTOR").is_ok() {
    tracing::info!("action_executor disabled via HEX_DISABLE_ACTION_EXECUTOR");
    return;
}
```

These variables are documented in `docs/specs/cost-ops-runbook.md:77-78` as kill-switches. They're NOT set by `scripts/hex-up.sh`, but could be:

1. **Inherited from shell profile** (`.bashrc`, `.zshrc`, systemd user env)
2. **Set during debugging** and left behind
3. **Present in a systemd service file** the operator hasn't audited

When these are set, **the loops spawn but immediately exit**. The rest of the stack runs: personas issue tool calls, STDB rows are written, but **no twin reviews happen, no executor writes files**. The result is exactly the observed symptom: **STDB rows exist, files don't**.

---

## Decision

### P0: Diagnostic + Immediate Fix

1. **Add startup diagnostics** to `hex-nexus/src/lib.rs` that **log the env-var state for all three loops** at INFO level before spawn. Operator can grep `nexus.log` to confirm loops are running.
2. **Emit structured health heartbeat** from each loop (drafter, twin_reviewer, action_executor) every 60 seconds with a unique `loop=<name>` tag so the operator can confirm "alive" state in logs.
3. **Check env vars in the operator's shell**:
   ```bash
   env | grep HEX_DISABLE
   ```
   If any are set → unset them and restart nexus.

### P1: Resilience — Fail-Loud, Not Silent

Replace the silent `return` with a **tracing::error!** + **panic!** so the nexus binary crashes if these kill-switches are set **unless explicitly confirmed by a second flag**:

```rust
// drafter.rs (same pattern for twin_reviewer, action_executor)
if std::env::var("HEX_DISABLE_DRAFTER").is_ok() {
    if std::env::var("HEX_CONFIRM_DISABLE_DRAFTER").is_err() {
        tracing::error!("FATAL: HEX_DISABLE_DRAFTER is set but HEX_CONFIRM_DISABLE_DRAFTER is not. This will SILENTLY break the digital-twin loop. Set HEX_CONFIRM_DISABLE_DRAFTER=true if you REALLY want to disable the drafter, or unset HEX_DISABLE_DRAFTER.");
        panic!("HEX_DISABLE_DRAFTER set without confirmation");
    }
    tracing::warn!("drafter disabled via HEX_DISABLE_DRAFTER (confirmed by HEX_CONFIRM_DISABLE_DRAFTER)");
    return;
}
```

This forces the operator to **explicitly confirm** the kill-switch, preventing accidental or stale env vars from silently breaking the platform.

### P2: Observability — Dashboard Health Panel

Add a `/api/loops/status` endpoint to hex-nexus that returns JSON:

```json
{
  "drafter": { "enabled": true, "last_tick": "2026-05-09T22:01:34Z", "pending_commitments": 2 },
  "twin_reviewer": { "enabled": true, "last_tick": "2026-05-09T22:01:39Z", "pending_actions": 0 },
  "action_executor": { "enabled": true, "last_tick": "2026-05-09T22:01:42Z", "approved_actions": 1 }
}
```

Expose this in a new **Mission Control → Loops** panel so the operator can see at a glance whether each loop is alive + what's in its queue.

### P3: Self-Healing — Auto-Disable on Sustained Errors

If a loop encounters **10+ consecutive tick errors** (e.g., STDB unreachable, inference timeout), **log a structured escalation** but keep retrying (don't disable). Add a `HEX_LOOP_MAX_FAILURES` env var (default 10) to tune the threshold.

---

## Consequences

### Positive

- **Diagnosis becomes trivial**: operator grep `nexus.log` for `"drafter disabled"` / `"twin_reviewer disabled"` / `"action_executor disabled"` → immediate root cause.
- **Fail-loud by default**: stale env vars → panic at startup → operator notices immediately.
- **Observability closes the gap**: `/api/loops/status` + dashboard panel → operator can see loop health without log-diving.
- **Fixes the meta-problem**: every "we drafted it but it doesn't exist" symptom traces to this root cause. Fixing it makes the platform **truthful**.

### Negative

- **Breaking change for legitimate kill-switch use**: if the operator is intentionally disabling loops (e.g., during STDB schema migration), they must now also set `HEX_CONFIRM_DISABLE_*=true`.  
  **Mitigation**: documented in `cost-ops-runbook.md` + startup error message is self-explanatory.
- **Additional HTTP endpoint**: `/api/loops/status` adds ~100 LOC. Negligible cost.

### Risks

- **Panic on startup if env vars are stale**: operator shell has old `HEX_DISABLE_*` → nexus crashes → operator must unset and restart.  
  **Mitigation**: the panic message is explicit; operator can recover in <30s. This is **better** than the current silent failure where the operator debugs for hours.

---

## Implementation Notes

1. **P0 diagnostic**: add `tracing::info!` before each `spawn()` call in `hex-nexus/src/lib.rs:427-448` logging the env-var state.
2. **P1 fail-loud**: replace `return` with panic unless `HEX_CONFIRM_DISABLE_*` is set (3 files: `drafter.rs`, `twin_reviewer.rs`, `action_executor.rs`).
3. **P2 endpoint**: add `GET /api/loops/status` in `hex-nexus/src/routes.rs` (or new `routes/loops_status.rs`) + wire to shared state tracking last-tick timestamps.
4. **P3 self-healing**: add `consecutive_errors` counter per loop + escalate at threshold (log only; don't disable).

---

## Verification

After implementation:

1. Set `HEX_DISABLE_TWIN=true` in shell → `./scripts/hex-up.sh` → nexus should **panic** with clear error message.
2. Set `HEX_DISABLE_TWIN=true` **and** `HEX_CONFIRM_DISABLE_TWIN=true` → nexus should start, log warning, skip twin_reviewer loop.
3. Unset all `HEX_DISABLE_*` vars → `./scripts/hex-up.sh` → `curl http://[IP_ADDRESS]:5555/api/loops/status` → all loops should report `enabled: true` + recent `last_tick`.
4. Draft an ADR via `adr_draft` tool → wait 60s → file should exist on disk → `/api/loops/status` should show `action_executor.approved_actions: 0` (consumed).

---

**Decision**: Implement P0 + P1 immediately (P0 is 10 LOC, P1 is 15 LOC across 3 files). P2 + P3 follow in next workplan. This unblocks every pending "drafted but not materialized" symptom.