# ADR-2605091200 — hex-mission-control-design

Status: **Proposed**
Date: 2026-05-09

## Context

The hex operator currently navigates three domain-specific dashboard views to monitor system health and respond to events:

1. **Resources** (`hex-nexus/assets/src/components/views/Resources.tsx`)  
   Mirrors `/proc` walker output showing system processes (pid, cpu%, RSS), plus a resource anomaly inbox that the operator can acknowledge. Refreshes every 5s. Implements ADR-2605082500 resource supervision.

2. **MergeGate** (`hex-nexus/assets/src/components/views/MergeGate.tsx`)  
   Lists every merge_request awaiting three-voter quorum (validation-judge, adversarial-red, adversarial-blue). The operator can approve or reject via override vote. Refreshes every 4s. Implements ADR-2605082830 worktree merge gate.

3. **Commitments** (`hex-nexus/assets/src/components/views/Commitments.tsx`)  
   Ledger of every persona Confirm/PLAN line with status (open / overdue / satisfied / abandoned) and operator affordances to satisfy or abandon. Refreshes every 5s.

Each view is a deep-dive into a vertical domain. This design serves specialists (SRE, code-reviewer, product owner), but does not serve the operator's actual daily pattern: scan the fleet for problems, handle the next urgent item, compose a board-ask, repeat.

The operator needs:
- **a single landing surface** showing live activity from all loops
- **a prioritised decision queue** merging merge-gate votes, overdue commitments, and resource anomalies into one feed
- **health badges** for each autonomous loop (do I need to escalate?)
- **a compose box** for board-ask messages without switching to chat

Without this, the operator tab-hops between three dashboards, misses overdue items, and has no unified "everything that needs my attention right now" view.

## Decision

Implement a new **#/mission-control** route in hex-nexus/assets that becomes the operator's default landing page.

### Layout

Four quadrants:

1. **Live Activity Feed** (top-left)  
   Tails the last 50 events from STDB `activity_log` (merge approved, commitment satisfied, anomaly detected, persona escalation, worktree exec result). Each row shows timestamp (relative, e.g. "3m ago"), actor (role/agent_id), verb (approved / escalated / detected), and summary. Scrollable, auto-refreshes every 3s.

2. **Loop Health Badges** (top-right)  
   One badge per autonomous loop: resource-observer, validation-judge, adversarial-red, adversarial-blue, commitment-checker, merge-orchestrator. Each badge polls its health endpoint and shows status (healthy / degraded / down), last heartbeat time, and a drilldown link to the relevant domain view (Resources / MergeGate / Commitments). Colour coded: green (healthy), yellow (degraded), red (down / stale heartbeat).

3. **Pending Decisions Queue** (bottom-left)  
   Merges three decision categories into a single priority-sorted list:
   - Open merge_requests needing operator vote (from `/api/merge/requests?status=open`)
   - Overdue commitments (from `/api/commitments?status=overdue`)
   - Unacknowledged resource anomalies severity=critical (from `/api/resources/anomalies?status=open&severity=critical`)
   
   Each row shows kind (merge / commitment / anomaly), summary line, and one-click affordance buttons: "Approve"/"Reject" for merges, "Satisfy"/"Abandon" for commitments, "Ack" for anomalies. Clicking a button fires the relevant POST and removes the item from the queue. Priority: critical anomalies first, then overdue commitments, then open merges. Refreshes every 4s.

4. **Board Ask Compose Box** (bottom-right)  
   Textarea + "Send to Board" button. Operator types a message; clicking Send POSTs to `/api/board_ask` (to be implemented by orchestration layer) and clears the textarea. Displays last 3 board-ask messages sent with timestamp and preview. This gives the operator a lightweight way to task the board without context-switching to the chat interface.

### Route & Navigation

- New route: `#/mission-control` defined in `hex-nexus/assets/src/App.tsx`
- Component: `hex-nexus/assets/src/components/views/MissionControl.tsx`
- Default landing: update browser redirect or App.tsx default route to `#/mission-control`
- Existing routes (`#/resources`, `#/merge-gate`, `#/commitments`) remain but become drill-down destinations

### Data Sources

- Activity feed: new `/api/activity_log` endpoint reading STDB `activity_log` table (to be added)
- Loop health: new `/api/health/loops` endpoint aggregating heartbeat timestamps from each role's health check
- Pending decisions: existing endpoints (`/api/merge/requests`, `/api/commitments`, `/api/resources/anomalies`)
- Board ask: new `/api/board_ask` endpoint (posts to STDB `board_inbox` table)

### Implementation Phases

1. **Stub UI**: build MissionControl.tsx with placeholder data, wire route in App.tsx, verify layout
2. **Backend endpoints**: implement `/api/activity_log`, `/api/health/loops`, `/api/board_ask` in hex-nexus REST handlers
3. **Data integration**: connect real endpoints to the four quadrants
4. **Default landing**: update App.tsx to default to mission-control
5. **Polish**: add relative timestamps, colour coding, keyboard shortcuts (e.g. `j`/`k` to navigate decision queue, `Enter` to approve)

## Consequences

### Positive

- **Single pane of glass**: operator opens one URL and sees everything that needs attention
- **Faster triage**: decision queue merges three domains into one priority list with instant action buttons
- **Proactive monitoring**: loop health badges surface degraded components before they cascade
- **Less context switching**: board-ask compose box eliminates trips to chat for simple tasking
- **Preserve deep-dive views**: Resources / MergeGate / Commitments remain for detailed analysis

### Negative

- **More backend surface area**: four new REST endpoints (`/api/activity_log`, `/api/health/loops`, `/api/board_ask`, plus aggregate decision endpoint)
- **Polling overhead**: mission-control polls six endpoints every 3-4s; consider WebSocket upgrade if load becomes an issue
- **Operator retraining**: operators must adopt new default route instead of bookmarked domain views
- **Incomplete at launch**: board-ask endpoint requires orchestration wiring to persona chat threads; initially may be write-only until board reads its inbox

### Alternatives Considered

1. **Tabs within one view**: keep Resources/MergeGate/Commitments as tabs instead of separate routes. Rejected because tabs don't allow deep-linking and hide drill-down context.
2. **Dashboard sidebar**: add a sidebar to every view with mini health badges and decision count. Rejected because it fragments the operator's attention and doesn't reduce tab-hopping.
3. **Slack/Discord bot**: push decisions to external chat. Rejected because it couples hex to external services and adds latency; the dashboard is the source of truth.

### Follow-on Work

- ADR for STDB `activity_log` schema (actor, verb, subject, timestamp, metadata JSONB)
- ADR for loop health heartbeat protocol (each role writes `loop_heartbeat(role, timestamp)` every 60s)
- UX research: track operator click-through rate from mission-control to drill-down views to validate quadrant utility
- Keyboard navigation: add vim-style keybinds for decision queue traversal (opens accessibility & power-user path)
