# Mission Control UX — Operator Landing Surface Behavioral Spec

*status*: proposed  ·  *date*: 2026-05-11

Mission Control UX — Operator Landing Surface Behavioral Spec

*status*: proposed  ·  *date*: 2026-05-10  
*references*: ADR-066 (Dashboard Visibility), cost-gate-refinement.md, MissionControl.tsx (423 lines), mission_control.rs (293 lines)

---

## Overview

**Mission Control** is the operator's single landing surface at `http://localhost:5555/#/mission-control` (SolidJS SPA route). It aggregates six operational domains—pending decisions, persona health, resource anomalies, recent activity, top processes, board ask compose—into one screen with a unified 5s refresh cadence.

**Target users**: Operators (human developers) running multi-project hex AIOS loops.

**Success criterion**: Operator can answer "Is hex healthy? What needs my attention?" in <5 seconds without leaving this view.

---

## Layout (12-column responsive grid)

```
┌──────────────────────────────────────────────────────────────────┐
│ HEADER: "Mission Control" · 5s refresh · STDB ✓/✗ · drill-downs │
├──────────────────────────────────────────────────────────────────┤
│ BOARD ASK COMPOSE (full width, sticky)                          │
├────────────────────────────────────┬─────────────────────────────┤
│ PENDING DECISIONS (8 cols)         │ PERSONA HEALTH (4 cols)     │
│ • Actions (proposed_action)        │ • role, paused, last_tick   │
│ • Commitments (commitment table)   │                             │
├────────────────────────────────────┼─────────────────────────────┤
│ RECENT ACTIVITY (8 cols)           │ ANOMALIES (4 cols)          │
│ • Last 12 executed_action          │ • resource_anomaly open     │
├────────────────────────────────────┴─────────────────────────────┤
│ TOP PROCESSES BY RSS (full width, sortable table)               │
└──────────────────────────────────────────────────────────────────┘
```

---

## API Contract

### Endpoint

**GET** `/api/mission-control`

**Implementation**: `hex-nexus/src/routes/mission_control.rs::get_mission_control()`

**Refresh cadence**: Every 5s (frontend `setInterval`, const `REFRESH_MS = 5000`)

**Timeout**: 4s (`STDB_TIMEOUT_SECS`)

### Response Schema

```typescript
interface MissionControlPayload {
  ts: string;                         // ISO8601 timestamp
  stdb_alive: boolean;                // true if any STDB query succeeded
  activity: {
    recent_executed: ExecutedRow[];   // last 12, newest first
    open_merge_requests: MergeRow[];  // up to 10 voting/open (excludes /tmp/cli-*)
  };
  pending_decisions: {
    actions: ActionRow[];             // up to 20 pending|escalated, newest first
    commitments: CommitmentRow[];     // up to 20 open|overdue, newest first
    anomalies: AnomalyRow[];          // up to 15 unhandled, newest first
  };
  personas: PersonaRow[];             // all persona_pool rows
  top_processes: ProcessRow[];        // top 8 by rss_kb
}

interface ExecutedRow {
  id: number; kind: string; path: string | null;
  success: boolean; error: string; executed_at: string; evidence: string;
}
interface MergeRow {
  worktree_path: string; branch: string; status: string; opened_at: string;
}
interface ActionRow {
  id: number; kind: string; proposed_by: string; status: string;
  twin_verdict: string; twin_rationale: string; escalate_reason: string;
}
interface CommitmentRow {
  id: number; role: string; action: string; success_artifact: string;
  status: string; created_at: string;
}
interface AnomalyRow {
  id: number; detected_at: string; kind: string; severity: string;
  pids: string; note: string;
}
interface PersonaRow {
  role: string; display_name: string; paused: boolean; last_tick_at: string;
}
interface ProcessRow {
  pid: number; argv: string; rss_kb: number; cpu_pct: number; state: string;
}
```

### Backend Query Strategy

**Parallel tokio::join!** of 7 SpacetimeDB SQL queries:
1. `executed_action` → recent_executed
2. `commitment` → open/overdue commitments
3. `proposed_action` → pending/escalated actions
4. `resource_anomaly` → unhandled anomalies
5. `persona_pool` → persona health
6. `merge_request` → voting/open merges
7. `process_observation` → top processes by rss_kb

Each query has 4s timeout; partial failure = empty array for that section, but other panels render.

---

## Panel-by-Panel Behavior

### 1. Header Bar

**Elements**:
- Title: "Mission Control"
- Subtitle: "Single landing for hex operators · refreshes 5s"
- STDB health indicator: green "STDB ✓" if `stdb_alive`, red "STDB ✗" if false
- Drill-down buttons (right): Merge Gate, Resources, Commitments, Personas, Thoughts

**Behavior**:
- Drill-down buttons call `navigate({ page: "<target>" })` from `hex-nexus/assets/src/stores/router`
- No refresh on drill-down—opens separate view (operator can cmd+click for new tab)

**Implementation**: Lines 177-205 in `MissionControl.tsx`

---

### 2. Board Ask Compose (sticky, full width)

**Purpose**: Operator sends natural-language board asks to personas (e.g., "@cto fix cache bug") or broadcast to board.

**Elements**:
- Text input (mono font, placeholder: "board ask (no @mention) or @cto / @cpo / ...")
- "Send" button (cyan, disabled if empty)
- Status line (appears for 4s after send)

**Button behavior**:
- **Send**: POST `/api/org/send-message` with `{ from: "ceo", content: <text> }`
- Clears input on success
- Shows `routed → <persona list>` for 4s
- Calls `refresh()` to update pending decisions

**Keyboard shortcut**: Cmd+Enter / Ctrl+Enter triggers send

**Error handling**: If POST fails, status shows red "error: <message>"

**Implementation**: Lines 207-230 in `MissionControl.tsx`

---

### 3. Pending Decisions (8 cols, left column)

**Purpose**: Show operator what needs approval, escalation, or manual resolution.

**Sections**:
- **Actions** (from `proposed_action` table): status=pending|escalated, newest first, up to 20
- **Commitments** (from `commitment` table): status=open|overdue, newest first, up to 20

**Action row display**:
- Badge: status (pending=yellow, escalated=orange)
- Cyan text: kind (e.g., "adr_draft", "code_patch")
- Gray text: proposed_by
- Twin rationale (if present): "twin: <rationale>"
- Escalate reason (if present): orange text
- Gray ID: `#<id>`

**Commitment row display**:
- Badge: status (open=yellow, overdue=red)
- Cyan text: role
- White text: action (line-clamp-2)
- Mono gray text: success_artifact (if present)
- **"Mark satisfied" button**: calls `satisfyCommitment(id)` → POST `/api/commitments/satisfy` with `{ id, evidence: "mission-control manual mark" }` → refreshes

**Empty state**: "Nothing waiting. Operator is idle." (gray, bordered box)

**Implementation**: Lines 232-283 in `MissionControl.tsx`

---

### 4. Persona Health (4 cols, right column)

**Purpose**: Show operator which personas are running and which are paused.

**Display**:
- Title: "Personas (N)" where N = count
- Scrollable list (divide-y border)
- Each row: status dot (green ● = ready, yellow ● = paused) · role (cyan mono) · "paused"/"ready" (gray, right-aligned)

**Empty state**: "No personas registered."

**Refresh behavior**: No button—auto-refreshes via 5s global cadence

**Future enhancement** (not in current 423-line impl): Click persona → drill-down to `/persona/<role>` detail view

**Implementation**: Lines 285-303 in `MissionControl.tsx`

---

### 5. Recent Activity (8 cols, left column)

**Purpose**: Last 12 executed actions (successful + failed), newest first. Operator can spot-check what the loop is doing.

**Display**:
- Title: "Recent activity (last N executed actions)"
- Each row: success icon (✓ green / ✗ red) · kind (cyan) · id (gray) · path (mono, truncated) · evidence (gray, truncated) · error (red, if !success)

**Empty state**: "No actions executed yet."

**Sorting**: Backend pre-sorts by `id DESC`, truncates to 12

**Future enhancement** (not in current impl): Click row → modal with full payload_json + evidence

**Implementation**: Lines 305-331 in `MissionControl.tsx`

---

### 6. Anomalies (4 cols, right column)

**Purpose**: Show operator open `resource_anomaly` rows (runaway processes, disk pressure, etc.) requiring ack.

**Display**:
- Title: "Anomalies (N open)"
- Each row: severity badge (critical=red, warn=yellow, info=blue) · kind (cyan) · id (gray) · note (line-clamp-2, gray) · **"Ack" button**

**Ack button behavior**:
- Calls `ackAnomaly(id)` → POST `/api/resources/anomalies/ack` with `{ id, source: "mission-control" }` → refreshes
- Button disabled while request in flight (`busyId() === id`)

**Empty state**: "No anomalies."

**Escalation threshold** (from spec): Not enforced in UI—backend should auto-escalate anomalies >30min unhandled to `proposed_action` (ADR-060 inbox contract).

**Implementation**: Lines 333-361 in `MissionControl.tsx`

---

### 7. Top Processes by RSS (full width, table)

**Purpose**: Real-time visibility into memory + CPU consumption. Operator can spot runaway processes before OOM.

**Display**:
- Title: "Top processes by RSS — total X.X GiB"
- Table columns: pid (cyan mono) · state (gray) · cpu% (right-aligned, tabular-nums) · rss (right-aligned, tabular-nums) · argv (mono, truncated max-w-2xl)
- **Color thresholds**:
  - **RSS**: red bold >30 GiB, yellow 20-30 GiB, gray <20 GiB
  - **CPU%**: red bold >800%, yellow 200-800%, gray <200%

**Sorting**: Backend pre-sorts by `rss_kb DESC`, returns top 8

**Refresh cadence**: 5s (same as global)

**Future enhancement** (not in current impl): Click row → modal with process tree, open FDs, /proc/<pid>/cmdline

**Implementation**: Lines 363-401 in `MissionControl.tsx`

---

## Button Behaviors Summary

| Button | Endpoint | Payload | Success Action | Error Handling |
|--------|----------|---------|----------------|----------------|
| Send (board ask) | POST `/api/org/send-message` | `{ from: "ceo", content: <text> }` | Clear input, show "routed → <personas>" for 4s, refresh | Show "error: <msg>" in status line |
| Mark satisfied | POST `/api/commitments/satisfy` | `{ id, evidence: "mission-control manual mark" }` | Refresh `/api/mission-control` | Show red banner at top: "satisfy failed: <msg>" |
| Ack (anomaly) | POST `/api/resources/anomalies/ack` | `{ id, source: "mission-control" }` | Refresh `/api/mission-control` | Show red banner at top: "ack failed: <msg>" |

**Loading state**: Button shows disabled + `busyId() === id` check prevents double-click

---

## Escalation Thresholds

These are **backend** policy, not UI enforcement—listed here for operator awareness:

1. **Anomalies >30min unhandled** → Auto-escalated to `proposed_action` with `escalate_reason = "Anomaly unresolved for 30m"` (ADR-060 contract)
2. **Commitments overdue >24h** → Status set to "overdue", appears in Pending Decisions panel (backend sets via scheduled tick)
3. **Proposed actions pending >1h with twin_verdict="approve"** → Backend should auto-execute if within tier autonomy (ADR-[PHONE] autonomy envelope); if not, escalate to operator

**UI does not enforce these**—it surfaces the escalated state for operator action.

---

## Acceptance Criteria

1. **Operator sees all pending decisions** (actions, commitments, anomalies) in <5s on page load.
2. **5s auto-refresh** runs in background; operator never manually refreshes (unless error).
3. **Ack anomaly** button removes row from panel within next 5s refresh cycle.
4. **Mark satisfied** button removes commitment from panel within next 5s refresh.
5. **Board ask send** routes to correct persona (validated via "routed → <role>" status).
6. **STDB ✗ indicator** appears if SpacetimeDB unreachable (operator knows to check `spacetime start`).
7. **Top processes RSS** highlights red any process >30 GiB, operator can spot OOM risk at a glance.

---

## Observable Artifacts

### Frontend (implemented, 423 lines)

- `hex-nexus/assets/src/components/views/MissionControl.tsx`
  - Lines 14-59: TypeScript interfaces matching API schema
  - Lines 61-75: Helper functions (fmtRss, sevColor, statusBadge)
  - Lines 87-170: Async handlers (refresh, ackAnomaly, overrideAction, satisfyCommitment, sendBoardAsk)
  - Lines 177-401: Render (header, compose box, 6 panels)

### Backend (implemented, 293 lines)

- `hex-nexus/src/routes/mission_control.rs::get_mission_control()`
  - Lines 68-91: Parallel tokio::join! of 7 STDB SQL queries
  - Lines 93-154: Activity feed transform (executed_action → recent_executed)
  - Lines 156-214: Pending decisions transform (proposed_action, commitment → actions, commitments)
  - Lines 216-236: Anomalies transform (resource_anomaly → open_anomalies)
  - Lines 238-250: Persona health transform (persona_pool → personas)
  - Lines 252-274: Merge requests transform (merge_request → open_merge)
  - Lines 276-289: Top processes transform (process_observation → top_processes, sort by rss_kb DESC)
  - Lines 291-293: Final JSON assembly + return

### Routes (registration)

- `hex-nexus/src/routes/mod.rs` line 513: `.route("/api/mission-control", get(mission_control::get_mission_control))`

---

## Drill-Down Navigation Map

Mission Control is the **hub**; each panel links to a detail view:

| Panel | Drill-Down Route | View |
|-------|------------------|------|
| Pending Decisions | `navigate({ page: "merge-gate" })` | Merge gate queue |
| Pending Decisions | `navigate({ page: "commitments" })` | Full commitment list |
| Anomalies | `navigate({ page: "resources" })` | Resource monitor |
| Personas | `navigate({ page: "persona-health" })` | Persona detail grid |
| Recent Activity | `navigate({ page: "thoughts" })` | Full thought log (executed_action + agent turns) |

**Buttons in header** (lines 188-201) provide 1-click navigation to these views.

---

## Future Enhancements (not in 423-line implementation)

1. **Burn-Rate Widget** (from cost-gate-refinement.md):  
   - 4-col card, top-right  
   - Hour/day/week spend + sparkline  
   - Color-coded green <$5, yellow $5-20, red >$20  
   - "View Breakdown" button → `/admin/cost-metrics`

2. **Cache Hit Rate Indicator** (from cost-gate-refinement.md):  
   - Below burn-rate widget  
   - Shows Anthropic prompt cache hit% last 24h  
   - Green >50%, yellow 20-50%, red <20%

3. **Pre-flight Cost Preview Modal** (from cost-gate-refinement.md):  
   - Auto-displays when `proposed_action.kind == "sop_cost_preview"`  
   - Operator approves/downgrades/cancels SOP runs >$0.50

4. **Workplan Progress Bar** (ADR-066 Phase P1c):  
   - Show active workplan → ADR → executing swarm link  
   - Progress: N/M tasks done

5. **Task → Agent → Worktree → Commit Drill-Down** (ADR-066 Phase P1b):  
   - Click swarm task → agent detail → worktree → git log

6. **Inbox Notification Panel** (ADR-066 Phase P1a, ADR-060):  
   - Priority-2 notifications highlighted red  
   - Ack button calls `/api/inbox/{id}/ack`

---

## References

- **ADR-066**: Dashboard Visibility Overhaul — defines Mission Control as operator's single landing surface
- **cost-gate-refinement.md**: Burn-rate widget + cache hit rate + pre-flight cost modal (future enhancements)
- **ADR-060**: Agent notification inbox (future inbox panel integration)
- **ADR-052**: Project-centric navigation (Mission Control scoped to selected project in multi-project mode)
- **Source files**:
  - `hex-nexus/assets/src/components/views/MissionControl.tsx` (423 lines, current implementation)
  - `hex-nexus/src/routes/mission_control.rs` (293 lines, API implementation)
  - `hex-nexus/src/routes/mod.rs` line 513 (route registration)

---

## Success Metrics (Observability)

1. **Operator time-to-decision** <5s (from page load to "I know what needs my attention")
2. **Auto-refresh latency** <500ms for `/api/mission-control` 95th percentile
3. **Zero missed anomalies** within 5s refresh window (operator acks within 1 refresh cycle)
4. **Board ask routing accuracy** >95% (correct persona receives message)
5. **STDB health false-negative rate** <1% (indicator accurately reflects SpacetimeDB connectivity)

---

*End of spec. Operator should be able to use this doc + the 423-line MissionControl.tsx to understand every button, every refresh, every threshold.*