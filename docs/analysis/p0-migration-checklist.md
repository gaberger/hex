# P0 Migration Checklist — Legacy → SolidJS Consolidation

**Date:** 2026-03-22
**Source Audit:** Three parallel agent audits of index.html, chat.html, and SolidJS dashboard

---

## Migration Matrix

| Feature | Legacy Source | SolidJS Status | Action | Priority |
|---------|-------------|----------------|--------|----------|
| **Streaming Chat** | chat-stream.js, chat-websocket.js | COMPLETE | None | — |
| **Markdown + Copy Buttons** | chat-markdown.js | COMPLETE | None | — |
| **Tool Call Cards** | chat-tools.js (104 LOC) | PARTIAL — missing fallback matching | Enhance ToolCallCard.tsx | HIGH |
| **Session Management** | chat-sessions.js (389 LOC) | Backend wired, NO UI | Create SessionListPanel.tsx | HIGH |
| **File Browser** | chat-file-panel.js (218 LOC) | NOT PRESENT | Create FileBrowser.tsx | MEDIUM |
| **Token Budget Gauge** | chat-messages.js (SVG circle) | NOT PRESENT | Create TokenGauge.tsx | MEDIUM |
| **Model Selector** | chat-init.js (56 LOC) | Signal exists, no UI | Create ModelSelector.tsx | MEDIUM |
| **Dashboard Quick Stats** | chat-dashboard.js (231 LOC) | Separate components exist | Wire into sidebar/overlay | LOW |
| **HexFlo Swarm Cards** | chat-hexflo.js (173 LOC) | SwarmMonitor exists | Verify parity | LOW |
| **Agent Status Pills** | chat-sidebar.js | PARTIAL | Wire WebSocket events | MEDIUM |
| **Architecture Health Ring** | chat-health.js (272 LOC) | HealthPane exists | Verify parity | LOW |
| **Slash Commands** | chat-init.js | BottomBar: DONE, ChatInput: STUB | Wire ChatInput to commands store | LOW |
| **Control Buttons** | chat-init.js (/clear /tokens /status) | BottomBar has these | None | — |
| **@agent Routing** | chat-websocket.js | COMPLETE | None | — |
| **HexFlo Events** | chat system messages | COMPLETE | None | — |
| **Chat Input (Enter/Shift+Enter)** | chat-init.js | COMPLETE | None | — |
| **Auto-reconnect** | chat-websocket.js | COMPLETE | None | — |

## Legacy Files to Delete (P0.3)

```
hex-nexus/assets/index.html          (if exists — may be index.legacy.html)
hex-nexus/assets/index.legacy.html
hex-nexus/assets/chat.html           (if exists — may be chat.legacy.html)
hex-nexus/assets/chat.legacy.html
hex-nexus/assets/js/chat-*.js        (14 files, ~2016 LOC total)
hex-nexus/assets/js/                 (entire directory if only chat-*.js)
hex-nexus/assets/styles.css          (legacy CSS vars — replaced by Tailwind)
```

## What SolidJS Already Has That Legacy Doesn't

- **Session Persistence** — REST + SpacetimeDB (legacy was in-memory only)
- **Reactive Architecture** — Signals auto-update UI (legacy: manual DOM manipulation)
- **Component Modularity** — Separate ChatView, MessageList, Message, MarkdownContent, ToolCallCard
- **Tiling Panes** — Multiple views side-by-side (legacy: single view)
- **Command Palette** — Ctrl+P universal search (legacy: none)
- **SpacetimeDB Subscriptions** — Real-time data from 4 modules (legacy: REST polling)

## P0.2 Scope (What to Port)

### Must Port (blocks P0.3 deletion)
1. **Session UI** — list, create, switch, fork, delete sessions (389 LOC equivalent)
2. **Tool Result Fallback** — add prefix-based matching when toolUseId is missing
3. **Model Selector** — dropdown wired to /api/inference/endpoints

### Can Port Later (doesn't block P0.3)
4. File Browser (will be redesigned in P1 as project-scoped FileTree)
5. Token Gauge (nice-to-have, move to sidebar)
6. Dashboard Quick Stats (will be redesigned in P2 as ProjectHome)

### Drop (superseded by new architecture)
- Legacy CSS variable system → Tailwind
- Legacy polling → SpacetimeDB subscriptions
- Legacy DOM manipulation → SolidJS reactivity
- RL sidebar stats → not needed in new design
