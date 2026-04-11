# Hex-Hub System Research Report

**Date:** 2026-03-16
**Scope:** Complete hex-hub system audit — messaging, SSE removal, race conditions, routing, statusline

---

## Executive Summary

The hex-hub is a Rust binary (port 5555) + TypeScript Node.js fallback that provides real-time observability and bidirectional command dispatch for Claude Code projects using hexagonal architecture. The system has **2 critical**, **6 high**, **5 medium**, and **2 low** severity concurrency issues. SSE and WebSocket carry non-overlapping data today — SSE handles observability events while WS handles commands only. Removing SSE requires migrating 5 event types to new WS topics. The dashboard has zero URL routing — no deep links, no `history.pushState`, no SPA catch-all.

---

## 1. Architecture Overview

```
Claude Code Session              hex-hub (Rust :5555)              Browser Dashboard
  │                                    │                                │
  ├─[hook] hub-push.cjs               │                                │
  │   POST /api/event ──────────────>  │                                │
  │                                    │──SSE /api/events ────────────> │
  ├─[adapter] DashboardAdapter         │                                │
  │   POST /api/push (10s interval) ─> │──SSE state-update ──────────> │
  │   POST /api/event (file-change) ─> │──SSE file-change ───────────> │
  │   WS subscribe project:{id}:cmd <──│                                │
  │                                    │<─POST /api/{id}/command ──────│
  │   <── WS command dispatch ─────────│                                │
  │   POST /api/{id}/cmd/{cid}/result >│──WS project:{id}:result ────> │
  │   WS publish (DUPLICATE!) ────────>│──WS project:{id}:result ────> │
  │                                    │                                │
  └─[adapter] HubLauncher             │                                │
      start/stop Rust binary           │                                │
```

---

## 2. SSE Removal — Migration Plan

### Current State: Zero Overlap

| Channel | Carries | Direction |
|---------|---------|-----------|
| **SSE** | state-update, project-registered/unregistered, file-change, decision-response, hook events | hub → browser |
| **WS** | command dispatch, command results | hub ↔ browser/project |

### New WS Topics Required

| SSE Event | New WS Topic | Notes |
|-----------|-------------|-------|
| `connected` (project list snapshot) | `hub:projects` | Send on WS open |
| `project-registered` | `hub:projects` | Global scope |
| `project-unregistered` | `hub:projects` | Global scope |
| `state-update` | `project:{id}:state` | Per-project |
| Arbitrary events (file-change, agent-*) | `project:{id}:events` | Per-project |
| `decision-response` | `project:{id}:decisions` | Per-project |

### Migration Checklist

**Rust (`hex-hub/`)**
- [ ] `state.rs` — remove `sse_tx: broadcast::Sender<SseEvent>`, `SseEvent` struct, `SseParams`
- [ ] `routes/sse.rs` — delete entire file
- [ ] `routes/mod.rs` — remove `mod sse`, remove `/api/events` route
- [ ] `routes/push.rs` — replace 2× `sse_tx.send()` with `ws_tx.send()` using new topic format
- [ ] `routes/projects.rs` — replace 2× `sse_tx.send()` with `ws_tx.send()` on `hub:projects`
- [ ] `routes/decisions.rs` — replace `sse_tx.send()` with `ws_tx.send()` on `project:{id}:decisions`

**Browser (`hex-hub/assets/index.html`)**
- [ ] Remove `connectSSE()` function (~140 lines) and `state.sseRetryTimer`
- [ ] Add WS subscriptions for `hub:projects`, `project:{id}:state`, `project:{id}:events`, `project:{id}:decisions`
- [ ] On project switch, re-subscribe to new project's topics
- [ ] Send initial project list on WS `connected` welcome message

**TypeScript Node.js fallback (`dashboard-hub.ts`)**
- [ ] Remove `SSEClient` interface, `sseClients` Set, `handleSSE()`, `broadcast()`, `broadcastToProject()`
- [ ] Replace broadcast call sites with WS fan-out

**Dead code removal**
- [ ] Delete `src/adapters/secondary/sse-broadcast-adapter.ts` entirely
- [ ] Evaluate `src/core/ports/broadcast.ts` — `IBroadcastPort` may become dead

---

## 3. Race Conditions & Pathologies

### Critical

| ID | Location | Issue | Fix |
|----|----------|-------|-----|
| **C1** | `commands.rs:41–68` | **Project TOCTOU**: read lock checks project exists, drops, write lock inserts command — `unregister` can remove project between the two locks | Hold read lock through command insertion, or accept orphaned commands |
| **C2** | `commands.rs:64–91` | **Two-phase status write**: command inserted as `"pending"`, WS broadcast fires, then second write lock sets `"dispatched"` — fast client can report result before status becomes `"dispatched"`, final status is nondeterministic | Single write lock: insert as `"dispatched"` before broadcasting |

### High

| ID | Location | Issue | Fix |
|----|----------|-------|-----|
| **C3** | `main.rs:51–68` | Eviction acquires `commands.write()` then drops, then `results.write()` — inconsistent view between reads | Acquire both locks in consistent order |
| **C4** | All `sse_tx.send()`/`ws_tx.send()` sites | Broadcast overflow (cap 256) silently drops with `let _ =` — missed command dispatch means command stuck `"pending"` forever | Log warnings; increase capacity; dead-letter queue for commands |
| **C5** | `push.rs:11–58` | Write lock on `projects` held across `sse_tx.send()` — blocks all concurrent readers | Snapshot data, drop lock, then broadcast |
| **C6** | `projects.rs:34–77` | Same pattern: write lock held across broadcast | Same fix |
| **H1** | `ws.rs:63–82` | Subscription `tokio::Mutex` acquired on every broadcast message | Use `RwLock` or `ArcSwap` for lock-free reads |
| **H2** | `dashboard-adapter.ts:142–149` | Non-atomic check on `_isListening` — two concurrent calls both pass guard, create duplicate WS connections | Set `_isListening = true` before `connectWs` |
| **H3** | `dashboard-adapter.ts:214–217` | `close` event schedules reconnect after `stopListening` clears timer but before it nulls `this.ws` | Check `this.stopped` as primary guard; remove listeners before nulling |

### Medium

| ID | Location | Issue | Fix |
|----|----------|-------|-----|
| **H4** | `main.rs:80–127` | Lock file written before TCP listener bound — early client gets connection refused | Write lock file after `TcpListener::bind` |
| **H5** | `hub-launcher.ts:51–53` | Concurrent `ensureHubRunning` spawns two daemons racing for port 5555 | Advisory lock file before spawn |
| **M1** | `commands.rs:151–184` | Two separate read locks allow eviction between reads | Single lock scope |
| **M2** | `sse-broadcast-adapter.ts:24–35` | Map mutated during iteration skips next client | Collect failed IDs, delete after loop |
| **M3** | `hub-push.cjs:119` | `process.exit(0)` fires before HTTP response drained under load | Exit in response callback, not timer |
| **M4** | `decisions.rs:10–28` | Decision result not persisted — missed SSE = lost decision, agent hangs forever | Store decisions; add polling endpoint |
| **M5** | `composition-root.ts:154–203` | Concurrent `writeFile` to `status.json` from multiple async paths | Serialize writes through queue |

### Low

| ID | Location | Issue | Fix |
|----|----------|-------|-----|
| **L1** | `push.rs:65–89` | Write lock acquired just to update `last_push_at` timestamp | Use read lock + atomic timestamp |
| **L2** | `main.rs:55,66` | RFC3339 string comparison instead of `DateTime` parse — fragile under timezone variance | Parse to `DateTime<Utc>` before comparing |

---

## 4. Messaging Architecture

### Complete Message Catalog

| # | Message | Direction | Transport | Payload |
|---|---------|-----------|-----------|---------|
| 1 | Project register | project→hub | HTTP POST | `{ name, rootPath, astIsStub }` |
| 2 | State push | project→hub | HTTP POST `/api/push` | `{ projectId, type, data, filePath? }` |
| 3 | Event push | project/hook→hub | HTTP POST `/api/event` | `{ projectId, event, data }` |
| 4 | Command issue | browser→hub | HTTP POST `/api/{id}/command` | `{ type, payload?, source? }` |
| 5 | Command result | project→hub | HTTP POST `/api/{id}/command/{cid}/result` | `{ status, data?, error? }` |
| 6 | SSE state-update | hub→browser | SSE | `{ projectId, type, timestamp }` |
| 7 | SSE events | hub→browser | SSE | `{ event, data }` (arbitrary) |
| 8 | SSE connected | hub→browser | SSE (on connect) | `{ projects: [...] }` |
| 9 | WS welcome | hub→client | WS | `{ topic: "hub:health", event: "connected" }` |
| 10 | WS command dispatch | hub→project | WS `project:{id}:command` | `{ commandId, type, payload, issuedAt }` |
| 11 | WS command result | hub→browser | WS `project:{id}:result` | `{ commandId, status, data?, error? }` |
| 12 | WS subscribe | client→hub | WS | `{ type: "subscribe", topic }` |
| 13 | WS publish | client→hub | WS | `{ type: "publish", topic, event, data? }` |

### Known Issue: Duplicate Command Results

`DashboardAdapter.handleCommand()` sends the result via BOTH:
1. HTTP POST `/api/{id}/command/{cid}/result` (which triggers WS broadcast from Rust)
2. WS Publish to `project:{id}:result` (direct WS broadcast)

**Any WS subscriber receives the result twice.** Fix: Remove the WS Publish; the HTTP POST already triggers WS broadcast.

### Back-Pressure

Broadcast channels (cap 256) use silent drop on overflow. Slow browser connections cause `Lagged` errors — the receiver loop `continue`s past them. No dead-letter, no logging, no acknowledgment that messages were lost.

---

## 5. Dashboard UI & Routing

### Current State: No Routing

- Single page at `/` — no hash routes, no query params, no `history.pushState`
- Project switching is in-memory JS: `state.currentProject` + `switchProject(id)`
- No catch-all SPA route in Rust — any path besides `/` returns 404
- Cannot bookmark or share a specific project view
- Back/forward browser navigation does nothing

### What's Needed

1. **URL scheme**: `/?project={id}` or `/#/project/{id}` (hash is simpler — no Rust changes)
2. **`history.pushState`** in `switchProject()` to update URL on project change
3. **`popstate` listener** to handle back/forward navigation
4. **Initial load from URL**: parse `location.hash` on page load, auto-select project
5. **Catch-all route** in Rust (if using path-based routing instead of hash)

### Statusline Integration

**Current**: `scripts/hex-statusline.cjs` shows a "dashboard" link using OSC 8 terminal hyperlinks pointing to `http://localhost:5555` (the root). It reads `~/.hex/daemon/hub.lock` for port and checks PID liveness.

**What's needed**: The statusline should link to the specific project: `http://localhost:5555/#/project/{id}`. The project ID is available from the hub registration response, stored in `.hex/status.json`. The statusline already reads `status.json` — it just needs to extract the project ID and append it to the URL.

### Hub Discovery Chain

```
hex-hub starts → writes ~/.hex/daemon/hub.lock { pid, token, port }
project starts → HubLauncher reads hub.lock → DashboardAdapter registers → receives projectId
project writes .hex/status.json { dashboard: "http://localhost:{port}" }
statusline reads .hex/status.json → renders clickable link
```

**Gap**: `status.json` has the hub URL but NOT the project ID. Adding `projectId` to `status.json` enables per-project deep links.

---

## 6. Recommended Fix Priority

### Phase 1: Critical Fixes (do first)
1. **C1+C2**: Fix command TOCTOU and two-phase status — single write lock, insert as `"dispatched"`
2. **C4**: Add logging on broadcast `SendError`; separate command channel from event channel (commands must not be silently dropped)
3. **Duplicate result**: Remove WS Publish from `DashboardAdapter.handleCommand()` — HTTP POST already triggers WS broadcast

### Phase 2: SSE Removal
4. Add 4 new WS topics (`hub:projects`, `project:{id}:state`, `project:{id}:events`, `project:{id}:decisions`)
5. Migrate all `sse_tx.send()` to `ws_tx.send()` with topic routing
6. Update browser to subscribe to WS topics instead of EventSource
7. Delete `sse.rs`, `sse-broadcast-adapter.ts`, SSE types from `state.rs`
8. Remove SSE from `dashboard-hub.ts` Node.js fallback

### Phase 3: Routing & Statusline
9. Add hash-based routing to `index.html` (`/#/project/{id}`)
10. Store `projectId` in `.hex/status.json`
11. Update `hex-statusline.cjs` to link `http://localhost:{port}/#/project/{id}`

### Phase 4: Remaining Pathologies
12. **C5+C6**: Snapshot-then-broadcast pattern for all lock-holding broadcast sites
13. **H2+H3**: Fix reconnection races in `dashboard-adapter.ts`
14. **M4**: Persist decision results, add polling endpoint
15. **M5**: Serialize `status.json` writes
16. **H4**: Write lock file after TCP bind
17. **H1**: Replace subscription Mutex with RwLock/ArcSwap
