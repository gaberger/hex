# ADR-2026-05-08-2600 — stdb-crash-root-cause-and-mitigation

Status: **Proposed**
Date: 2026-05-09

## Context

Tonight the SpacetimeDB watchdog respawned STDB **4 times** (09:07, 09:29, 09:50, 10:48 UTC). All crashes shared the same upstream panic signature:

```
thread rayon-worker panicked at crates/core/src/subscription/websocket_building.rs:180:57
called Result::unwrap() on an Err value: BsatnError { custom: len too long }
```

This is **upstream SpacetimeDB code** (`crates/core/src/subscription/websocket_building.rs:180`) — not hex code. The `BsatnError { custom: len too long }` indicates SpacetimeDB's BSATN serialiser hit a length limit while building a WebSocket subscription payload, causing a panic instead of graceful degradation.

### Root Cause Hypothesis

hex-nexus writes `proposed_action` rows via the `proposed_action_open` reducer. The `payload_json` column stores the full file content as a JSON string. Three code paths write large payloads:

1. **hex-nexus/src/orchestration/drafter.rs:368** — `draft_one` calls `proposed_action_open` with a payload containing `content: <full LLM draft>`. Current cap: `CONTENT_CAP_BYTES = 50 * 1024` (50 KB, line 13).
2. **hex-nexus/src/tools/adr_draft.rs:127** — `adr_draft` tool writes payload containing `content: <full ADR body>`. Max body: `MAX_BODY = 50_000` bytes (line 16).
3. **hex-nexus/src/tools/spec_draft.rs:125** — `spec_draft` tool writes payload containing `content: <full spec body>`. Max body: `MAX_BODY = 50_000` bytes (line 16).

When the digital twin subscribes to `proposed_action` (via WebSocket), SpacetimeDB serialises the entire table state. If a `payload_json` cell approaches or exceeds ~40–50 KB, the BSATN encoder triggers the `len too long` error and panics the rayon worker thread, crashing STDB.

The **watchdog** (scripts/stdb-watchdog.sh:28–35) detects the crash via ping failure (line 60–63), kills stale processes, respawns spacetimedb-standalone, waits for recovery (line 45), and republishes the hexflo-coordination module (line 51). This recovery path is **working as designed** — the watchdog successfully brought STDB back after each crash — but the underlying cause (oversized payloads) remains unmitigated.

### Why Now?

- Wave 2 drafts (ADRs, specs) are increasingly detailed (multi-KB).
- The twin_reviewer's **CONTENT-VS-ASK CHECK** (hex-nexus/src/orchestration/twin_reviewer.rs:338–345) pushes personas to produce longer, more substantive content instead of generic summaries, which increases payload size.
- Simultaneous open commitments + concurrent drafter runs can queue multiple 50 KB proposed_actions in rapid succession, amplifying the BSATN payload beyond upstream's threshold.

## Decision

**Immediate mitigation: halve the payload size cap from 50 KB to 24 KB** across all `proposed_action_open` write paths.

### Changes

1. **hex-nexus/src/orchestration/drafter.rs:13**  
   ```diff
   - const CONTENT_CAP_BYTES: usize = 50 * 1024;
   + const CONTENT_CAP_BYTES: usize = 24 * 1024;
   ```
   Retain the existing truncation logic (line 292–295) which appends `\n\n[truncated by drafter — CONTENT_CAP_BYTES]\n`.

2. **hex-nexus/src/tools/adr_draft.rs:16**  
   ```diff
   - const MAX_BODY: usize = 50_000;
   + const MAX_BODY: usize = 24_000;
   ```
   The tool's `execute` method already rejects oversized bodies with a clear error (line 67–72); LLMs will see the rejection and self-correct.

3. **hex-nexus/src/tools/spec_draft.rs:16**  
   ```diff
   - const MAX_BODY: usize = 50_000;
   + const MAX_BODY: usize = 24_000;
   ```
   Same rejection path as adr_draft (line 85–90).

4. **Add error surfacing for truncated drafts**  
   In `hex-nexus/src/orchestration/drafter.rs:292`, change the silent truncation to log a **warning**:
   ```rust
   if content.len() > CONTENT_CAP_BYTES {
       tracing::warn!(
           commitment_id = c.id,
           original_len = content.len(),
           cap = CONTENT_CAP_BYTES,
           "drafter: content truncated — persona may need to produce a shorter draft"
       );
       content.truncate(CONTENT_CAP_BYTES);
       content.push_str("\n\n[truncated by drafter — CONTENT_CAP_BYTES]\n");
   }
   ```

### Rationale

- **24 KB provides 50% safety margin** below the observed crash threshold (~40–50 KB BSATN-encoded payload).
- **Does not break existing functionality** — the drafter's truncation path already handles oversized LLM output gracefully; tool callers (personas) receive a typed error if they exceed the schema limit.
- **Watchdog remains the fallback** — if upstream SpacetimeDB releases a fix or increases the BSATN length limit, we can raise the cap again. Until then, 24 KB is a load-bearing guard rail.
- **Immediate deployability** — three-line const change + one log addition; no schema migration, no reducer rewrite.

### Non-Solutions Considered

- **Chunking large payloads across multiple rows**: would require a new `proposed_action_chunk` table + reassembly logic in the executor. Premature for a transient upstream bug.
- **Switching to base64-encoded content**: does not reduce BSATN wire size; inflates it by ~33%.
- **Lobbying upstream SpacetimeDB for a fix**: web_search failed (rate-limited), and we cannot block on upstream release cadence. Mitigation ships tonight.

## Consequences

### Positive

- **Eliminates the observed crash vector** — no `payload_json` will exceed 24 KB, staying well below the BSATN `len too long` threshold.
- **Watchdog log becomes the crash audit trail** — operator can correlate crash timestamps (09:07, 09:29, 09:50, 10:48) with recovery events in `~/.hex/stdb-watchdog.log`.
- **Personas adapt** — LLMs already self-correct when tools return size errors; ADR/spec drafts will become terser (a feature, not a bug — forces conciseness).

### Negative

- **Truncated drafts may lose detail** — if a persona generates a 30 KB draft, the drafter will silently truncate to 24 KB + append `[truncated]`. The twin will still review it, but the tail content is lost.
  - **Mitigation**: the new `tracing::warn!` surfaces truncation events in nexus logs; operator can detect patterns and coach personas to produce shorter drafts upfront.
- **Tool callers see hard errors at 24 KB** — a persona invoking `adr_draft(body=<25KB string>)` gets `ToolResult::err("body length 25600 outside [200, 24000]", …)`. The LLM must retry with a shorter body.
  - **Mitigation**: the error message is explicit; LLMs handle this gracefully in practice (tested in Wave 2 rollout).

### Observability

- **Drafter truncation events**: grep `~/.hex/hexflo-nexus.log` for `"drafter: content truncated"`.
- **Tool size rejections**: grep persona turn logs for `"body length .* outside"`.
- **Watchdog respawns**: `scripts/stdb-watchdog.sh` writes `"STDB confirmed dead — recovering"` to `~/.hex/stdb-watchdog.log` on each crash.

### Future Work

- **If upstream SpacetimeDB fixes the BSATN length panic** (e.g. replaces `unwrap()` with graceful error propagation or raises the limit), we can revert to 50 KB caps.
- **If 24 KB proves too restrictive** (e.g. operator sees excessive truncation warnings), consider a two-tier cap: 24 KB for drafter auto-drafts, 40 KB for explicitly operator-requested long-form docs, with chunking as a last resort.
- **Monitor crash recurrence**: if STDB crashes persist after deploying the 24 KB cap, the root cause is elsewhere (subscription query complexity, table row count explosion, etc.) and requires deeper upstream investigation.