# Escalation Inbox Panel — Operator UX Surface

*status*: proposed  ·  *date*: 2026-05-11

Escalation Inbox Panel — Operator UX Surface

**Status**: Proposed  
**Authors**: CPO  
**Date**: 2025-05-10  
**Implementation Tier**: Primary adapter (UI panel) + secondary adapter (API endpoint)  

---

## Context

Per ADR-2026-05-09-0000, personas use `escalate_to_operator(reason, urgency, options?)` when they cannot proceed autonomously. Currently (as of overnight cycle 4):

- Escalations are **logged** to `nexus.log` (tracing::warn)
- A **Telegram notification** fires if configured (fire-and-forget)
- A placeholder "wave-2 dashboard surface" comment exists in `hex-nexus/src/tools/escalate_to_operator.rs:100–135`
- The operator has **no dedicated inbox panel** to view, prioritise, or resolve escalations

This creates three observable problems:

1. **Invisible escalations**: The operator must grep logs or rely on Telegram pings; no persistent view exists  
2. **No prioritisation surface**: `high`/`med`/`low` urgency is logged but never surfaced; the operator cannot see what blocks other work  
3. **No closure workflow**: Escalations are fire-and-forget; the operator cannot mark "resolved" or "dismissed," leading to duplicate raises and noise  

**Grounding**:

- `hex-nexus/src/tools/escalate_to_operator.rs` emits escalations but does **not** persist them beyond logs  
- `spacetime-modules/hexflo-coordination/src/lib.rs:863–1009` defines `agent_inbox` table + `send_notification`, `broadcast_notification`, `acknowledge_notification` reducers — but these target **agent-to-agent** comms, not operator escalations  
- `hex-nexus/assets/src/App.tsx:1` contains a TODO for `/memory-health` panel (unrelated), but no escalation surface exists  
- `hex-cli/src/commands/go.rs` implements `hex go` with checks for nexus running, binary staleness, workplans, worktrees, tests — but **not** pending escalations  

---

## Decision

Ship a **dedicated Escalation Inbox Panel** at `hex-nexus/assets/src/EscalationInbox.tsx` (or inline in Mission Control) + a REST endpoint `GET /api/escalations` + a SpacetimeDB `operator_escalation` table.

### Observable Artifacts

#### 1. **SpacetimeDB Table** (`spacetime-modules/hexflo-coordination/src/lib.rs`)

Add a new table `operator_escalation` to persist escalations:

```rust
#[table(name = operator_escalation, public)]
pub struct OperatorEscalation {
    #[primarykey]
    #[autoinc]
    pub id: u64,
    pub raised_at: Timestamp,
    pub reason: String,         // 1-500 chars
    pub urgency: String,        // "low" | "med" | "high"
    pub priority: String,       // derived: "info" | "warn" | "critical"
    pub options: String,        // JSON array of strings, empty if none
    pub raised_by: String,      // persona role (e.g., "ciso", "cto")
    pub status: String,         // "open" | "resolved" | "dismissed"
    pub resolved_at: Option<Timestamp>,
    pub resolution_note: Option<String>,
}
```

Add reducer:

```rust
#[reducer]
pub fn raise_escalation(
    ctx: &ReducerContext,
    reason: String,
    urgency: String,
    priority: String,
    options: String,
    raised_by: String,
) -> Result<u64, String> {
    // Validate 1-500 chars, urgency enum, etc.
    let escalation = OperatorEscalation {
        id: 0, // autoinc
        raised_at: Timestamp::now(),
        reason,
        urgency,
        priority,
        options,
        raised_by,
        status: "open".to_string(),
        resolved_at: None,
        resolution_note: None,
    };
    ctx.db.operator_escalation().insert(escalation);
    Ok(escalation.id)
}

#[reducer]
pub fn resolve_escalation(
    ctx: &ReducerContext,
    escalation_id: u64,
    resolution_note: String,
) -> Result<(), String> {
    let mut esc = ctx.db.operator_escalation()
        .id()
        .find(escalation_id)
        .ok_or_else(|| format!("Escalation {} not found", escalation_id))?;
    esc.status = "resolved".to_string();
    esc.resolved_at = Some(Timestamp::now());
    esc.resolution_note = Some(resolution_note);
    ctx.db.operator_escalation().id().update(esc);
    Ok(())
}
```

#### 2. **Tool Patch** (`hex-nexus/src/tools/escalate_to_operator.rs`)

Replace the placeholder log + Telegram flow with a **SpacetimeDB insert** via the new `raise_escalation` reducer. Keep the Telegram notification as a secondary channel.

```rust
// After line 95, replace the tracing::warn + fire-and-forget with:
let escalation_id = call_stdb_reducer(
    &url,
    "raise_escalation",
    json!({
        "reason": reason,
        "urgency": urgency,
        "priority": priority,
        "options": serde_json::to_string(&options).unwrap_or_default(),
        "raised_by": ctx.persona_role, // Add ctx parameter
    }),
).await?;
```

#### 3. **REST Endpoint** (`hex-nexus/src/routes/escalations.rs` + router registration)

Create a new `escalations.rs` route module:

```rust
use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use crate::AppState;

#[derive(Serialize)]
pub struct EscalationDto {
    pub id: u64,
    pub raised_at: String,
    pub reason: String,
    pub urgency: String,
    pub priority: String,
    pub options: Vec<String>,
    pub raised_by: String,
    pub status: String,
}

pub async fn list_escalations(
    State(state): State<AppState>,
) -> Json<Vec<EscalationDto>> {
    // Query STDB operator_escalation table WHERE status='open'
    // ORDER BY priority DESC, raised_at DESC
    // Return JSON array
    Json(vec![]) // stub
}

#[derive(Deserialize)]
pub struct ResolveRequest {
    pub escalation_id: u64,
    pub resolution_note: String,
}

pub async fn resolve_escalation(
    State(state): State<AppState>,
    Json(req): Json<ResolveRequest>,
) -> axum::http::StatusCode {
    // Call STDB resolve_escalation reducer
    axum::http::StatusCode::OK
}
```

Register in `hex-nexus/src/routes/mod.rs` (or wherever `build_router` lives):

```rust
.route("/api/escalations", get(escalations::list_escalations))
.route("/api/escalations/resolve", post(escalations::resolve_escalation))
```

#### 4. **Frontend Panel** (`hex-nexus/assets/src/EscalationInbox.tsx`)

A SolidJS component that:

1. Fetches `GET /api/escalations` on mount + every 10s poll  
2. Displays escalations as cards, grouped by priority (`critical` / `warn` / `info`)  
3. Each card shows:
   - **Urgency badge** (🔴 high / 🟡 med / 🟢 low)
   - **Raised by** (persona role)
   - **Reason** (1-500 chars)
   - **Options** (if present, as a numbered list)
   - **Action buttons**: "Resolve" (opens a textarea for resolution note) | "Dismiss"  
4. Resolved/dismissed escalations move to a collapsible "History" section (or hidden)

**Wire into App.tsx**:

- Add a **"Escalations"** tab/section in the main dashboard  
- Show a **badge count** next to the tab label (e.g., "Escalations (3)")  
- If any `priority=critical` escalation is open, flash the tab label red or show a persistent alert banner  

#### 5. **CLI Integration** (`hex-cli/src/commands/go.rs`)

Add a check to `hex go` that queries `GET /api/escalations` and warns if any `urgency=high` escalations are open:

```rust
async fn check_escalations() -> bool {
    let client = NexusClient::from_env();
    match client.get("/api/escalations").await {
        Ok(resp) => {
            let escalations: Vec<EscalationDto> = resp.json().await.unwrap_or_default();
            let high_urgency: Vec<_> = escalations.iter()
                .filter(|e| e.urgency == "high")
                .collect();
            if high_urgency.is_empty() {
                println!("  {} no critical escalations", "✓".green());
                false
            } else {
                for esc in high_urgency {
                    println!(
                        "  {} critical escalation: {} — {}",
                        "→".red(),
                        esc.raised_by,
                        esc.reason.chars().take(60).collect::<String>()
                    );
                }
                println!("    {} view at: http://localhost:3033/#/escalations", "→".yellow());
                true
            }
        }
        Err(_) => {
            println!("  {} escalation check skipped (nexus offline)", "⚠".dimmed());
            false
        }
    }
}
```

Call `check_escalations().await` in the main `run()` function after the nexus check.

---

## Success Criteria

1. **Persistence**: Escalations survive nexus restarts (stored in SpacetimeDB)  
2. **Visibility**: Operator sees open escalations in Mission Control without grepping logs  
3. **Prioritisation**: Escalations sort by `priority` (critical → warn → info) and `raised_at` (newest first)  
4. **Workflow closure**: Operator can mark escalations "resolved" with a note; resolved escalations disappear from the main view  
5. **CLI integration**: `hex go` surfaces high-urgency escalations with a direct link to the panel  
6. **No duplicate raises**: Personas can query "is escalation X already open?" before raising (future enhancement; not required for cycle 4)  

---

## Implementation Files

| File | Purpose | Layer |
|------|---------|-------|
| `spacetime-modules/hexflo-coordination/src/lib.rs` | Add `operator_escalation` table + reducers | Infrastructure (STDB schema) |
| `hex-nexus/src/tools/escalate_to_operator.rs` | Replace log-only flow with STDB insert | Secondary adapter (tool) |
| `hex-nexus/src/routes/escalations.rs` | REST API for list + resolve | Primary adapter (HTTP) |
| `hex-nexus/src/routes/mod.rs` | Register `/api/escalations` routes | Primary adapter (router) |
| `hex-nexus/assets/src/EscalationInbox.tsx` | SolidJS panel UI | Primary adapter (frontend) |
| `hex-nexus/assets/src/App.tsx` | Wire "Escalations" tab + badge count | Primary adapter (frontend) |
| `hex-cli/src/commands/go.rs` | Add `check_escalations()` to health checks | Primary adapter (CLI) |

---

## User Flow (Nominal)

1. **Persona raises escalation**: CISO calls `escalate_to_operator(reason="Cannot determine if X is a vuln", urgency="high", options=["Accept risk", "Patch now", "Defer to wave-3"])`  
2. **Tool executes**: Inserts row into `operator_escalation` table (STDB), fires Telegram notification  
3. **Operator sees**: Mission Control shows "Escalations (1)" badge; panel lists the CISO escalation with urgency=high, 3 options  
4. **Operator decides**: Clicks option 2 ("Patch now"), types resolution note "Patched via ADR-XXXX", clicks "Resolve"  
5. **System updates**: `resolve_escalation` reducer marks status=resolved, stores note, timestamp  
6. **Panel refreshes**: Badge count drops to 0; resolved escalation moves to History section  
7. **CLI check**: Next `hex go` run shows "✓ no critical escalations"  

---

## Out of Scope (Cycle 4)

- **Deduplication**: Personas can raise duplicate escalations; operator manually reconciles  
- **Threaded discussion**: Escalations are one-shot; no multi-turn operator ↔ persona chat  
- **Slack integration**: Only Telegram is wired for now  
- **Escalation analytics**: No dashboard for "mean time to resolution" or "escalations per persona"  
- **Auto-resolution**: System never marks escalations resolved; only operator can  

---

## Dependencies

- **ADR-2026-05-09-0000**: SOP contracts (defines `escalate_to_operator` tool)  
- **ADR-2026-04-14-2200**: Workplan reconciliation (not directly related, but both touch operator observability)  
- **SpacetimeDB**: `hexflo-coordination` module must be deployed with the new `operator_escalation` table  

---

## Observable Behavior Change

**Before**: Operator has no visibility into escalations except log grep or Telegram  
**After**: Operator opens Mission Control → sees "Escalations (2)" badge → clicks tab → reviews CISO + CTO escalations, resolves one, dismisses one → badge drops to 0  

**CLI Before**: `hex go` shows nexus, binary, workplans, worktrees, tests  
**CLI After**: `hex go` also shows "→ critical escalation: ciso — Cannot determine..." with link to panel  

---

## Testing Surface

- **Unit tests** (`hex-nexus/tests/escalation_flow.rs`): Call `escalate_to_operator`, verify STDB row inserted, query via mock `GET /api/escalations`, resolve, verify status change  
- **Integration test** (`hex-agent/tests/escalation_e2e.rs`): Spawn agent, trigger escalation-worthy condition (e.g., secret_scan finds a key), verify escalation appears in STDB  
- **Manual test**: Start nexus, open Mission Control, use MCP inspector to call `escalate_to_operator`, verify panel updates in <10s, resolve escalation, verify badge clears  

---

## Rollout Plan

1. **Phase 1** (P0): Add STDB table + reducers, patch `escalate_to_operator.rs` tool  
2. **Phase 2** (P1): Build REST API (`/api/escalations`, `/api/escalations/resolve`)  
3. **Phase 3** (P2): Build SolidJS panel, wire into App.tsx  
4. **Phase 4** (P3): Add CLI check to `hex go`  
5. **Phase 5** (P4): Write integration tests, manual QA, ship  

Total estimate: **~200 LOC** (100 Rust, 80 TypeScript, 20 test)  

---

## Consequences

### Positive

- **Operator gains a single pane of glass** for all persona escalations  
- **Escalations persist** across nexus restarts (STDB durability)  
- **Prioritisation is visible**: Critical escalations surface first  
- **Closure workflow** prevents "lost escalations" and duplicate raises over time  
- **CLI integration** makes `hex go` a true health check (not just infra, now includes operator attention needs)  

### Negative

- **Another table to maintain**: STDB schema evolution must migrate `operator_escalation`  
- **No deduplication**: Personas can spam; operator must manually reconcile (mitigated by resolution notes)  
- **Polling overhead**: Frontend polls every 10s; acceptable for <100 escalations/day, but no WebSocket push yet  

### Neutral

- **Telegram remains fire-and-forget**: Escalations still ping Telegram; panel is the persistent view  
- **No persona feedback loop**: Personas don't see "escalation resolved"; they only know the operator handled it if the operator replies in chat  

---

**End of spec.**
