# Mission Control UX

*status*: proposed  ·  *date*: 2026-05-09

Mission Control UX

## Overview

Mission Control is the operator's single landing surface for the hex AIOS. It aggregates health, activity, and decision surfaces into one auto-refreshing view (5s cadence) via `/api/mission-control`. Drill-down pages remain accessible for deep work (Merge Gate, Resources, Commitments, Personas, Thoughts).

**Implementation:** `hex-nexus/assets/src/components/views/MissionControl.tsx` (Solid view) + `hex-nexus/src/routes/mission_control.rs` (aggregator endpoint).

---

## Layout & Panels

12-column responsive grid (Tailwind):

### 1. **Board Ask Compose Box** (full width, sticky top)
- **Affordances:** Text input with placeholder "board ask (no @mention) or @cto / @cpo / ..." + Send button (Ctrl/Cmd+Enter shortcut).
- **Behavior:** Posts to `/api/org/send-message` with `from: "ceo"`. Response shows routed persona list for 4s, then clears.
- **Appearance:** Gray-900 background, cyan-500 send button, small status line below showing "routed → cto, cpo" or error.

### 2. **Pending Decisions** (8 cols, left column)
Shows `proposed_action` rows in `pending` or `escalated` status + `commitment` rows in `open` or `overdue` status (last 20 each, newest first).

- **Action cards:** Status badge (color-coded: pending=yellow, escalated=orange, approved=green, rejected=red), kind label (cyan), proposer, twin rationale (if present), escalate reason (orange text).
- **Commitment cards:** Status badge, role (cyan), action text (2-line clamp), success artifact path (mono font), "Mark satisfied" button → POST `/api/commitments/satisfy`.
- **Fallback:** "Nothing waiting. Operator is clear." if empty.

### 3. **Persona Health** (4 cols, right column, top)
Lists all `persona_pool` rows with role, display name, paused state, last tick timestamp.

- **Affordances:** Green dot (ready) or yellow dot (paused), role name (mono, cyan), status text ("paused" / "ready").
- **Appearance:** Divided list in a gray-900/40 card.
- **Fallback:** "No personas registered."

### 4. **Recent Activity** (8 cols, left column, below decisions)
Last 12 `executed_action` rows, newest first.

- **Per row:** Success/failure icon (✓ green / ✗ red), kind label (cyan), id, optional path (mono, truncated), evidence snippet (gray), error text (red, if failed).
- **Fallback:** "No actions executed yet."

### 5. **Open Anomalies** (4 cols, right column, below personas)
Up to 15 unhandled `resource_anomaly` rows, newest first.

- **Per anomaly:** Severity badge (critical=red, warn=yellow, info=blue), kind label (cyan), note (2-line clamp), "Ack" button → POST `/api/resources/anomalies/ack`.
- **Fallback:** "No anomalies."

### 6. **Top Processes by RSS** (full width, bottom)
Up to 8 `process_observation` rows sorted by RSS descending.

- **Table columns:** pid, state, cpu%, RSS (formatted as M/G), argv (60 chars, mono).
- **Header:** Shows total RSS across all processes in GiB.

---

## Header & Navigation

- **Title:** "Mission Control" + subtitle "Single landing for hex operator · refreshes 5s · STDB ✓/✗" (STDB status color-coded green/red).
- **Quick nav buttons (top-right):** Merge Gate, Resources, Commitments, Personas, Thoughts → `navigate({ page: "…" })`.

---

## Error Handling

- **Global error banner:** Red-950 background, border-red-900, displayed below header when any fetch or action fails.
- **Per-action busy state:** Button shows `disabled` + `busyId` signal prevents double-click during async POST.

---

## Refresh Behavior

- **Auto-refresh:** `setInterval(refresh, 5000)` on mount, cleared on cleanup.
- **Manual refresh:** Triggered after every user action (ack anomaly, satisfy commitment, send board message) to immediately reflect changes.

---

## Success Criteria

1. Operator sees all pending work (actions, commitments, anomalies) in one glance without tab-hopping.
2. Zero latency for read-only view (single 5s poll replaces 5+ separate per-domain fetches).
3. One-click triage: ack anomaly, satisfy commitment, send board ask—all from this screen.
4. Drill-down pages remain accessible for deep work; Mission Control is the triage surface, not a replacement.

---

## Observable Artifacts

- **View file:** `hex-nexus/assets/src/components/views/MissionControl.tsx`
- **Endpoint:** `hex-nexus/src/routes/mission_control.rs::get_mission_control` returns single JSON payload with `activity`, `pending_decisions`, `personas`, `top_processes`, `stdb_alive`.
- **Route registration:** Axum router in `hex-nexus/src/routes/mod.rs` mounts `GET /api/mission-control`.
