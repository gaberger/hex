# hex-nexus Dashboard Redesign Blueprint

**Date:** 2026-03-20
**Based on:** UX Audit, Competitor Analysis (9 tools), Design Pattern Research (40+ sources), Component Architecture Map
**Goal:** Transform hex-nexus from a confusing multi-app dashboard into an OpenCode-like chat-first developer experience

---

## Diagnosis: Why It Feels Useless

The current dashboard has **5 fatal problems**:

1. **Split-brain**: 3 HTML entry points (`index.html`, `chat.html`, `dashboard.html`) with 2 rendering engines (vanilla JS + SolidJS) sharing zero state
2. **Identity crisis**: Is it a dashboard? A chat app? A fleet manager? A swarm monitor? It tries to be all 5 and fails at each
3. **Hidden everything**: File browser needs 5 steps. Chat view needs Ctrl+P. Agent spawn is a 14px icon. Swarm init uses `prompt()`
4. **No mental model**: "Projects" means 3 different things across 3 views. No navigation between entry points
5. **Broken plumbing**: BottomBar creates a new WebSocket per message (memory leak, no context). Vanilla JS and SolidJS maintain duplicate connections

---

## The Target: OpenCode/Crush Model

OpenCode (now Crush by Charmbracelet) proved the winning formula for AI dev tools:

```
+--------------------------------------------------+
| [Status Bar: model | tokens | mode | session]     |
+--------------------------------------------------+
|                    |                               |
|  Messages          |  Sidebar (collapsible)        |
|  (chat stream)     |  - Session list               |
|                    |  - Active agents               |
|                    |  - Health badge                |
|                    |                               |
|                    |                               |
+--------------------------------------------------+
|  > Input (universal: chat + /commands + @files)   |
+--------------------------------------------------+
```

**Key principles from ALL 9 tools studied:**
- Chat-first, tools-second (all 9/9)
- Streaming/inline results (all 9/9)
- @-mention file context (all 9/9)
- Plan/Build mode separation (7/9)
- Inline diffs with accept/reject (6/9)
- Git-native changes (5/9)
- Command palette as escape hatch (7/9)

---

## Architecture Decision: Consolidate on SolidJS

**Kill `index.html` and `chat.html`.** The SolidJS app (`dashboard.html`) already has:
- Tiling pane system (max 4 panes)
- Command palette (Ctrl+P) with fuzzy search
- SpacetimeDB reactive data layer (4 modules)
- Sidebar with projects/agents/swarms
- Lazy-loaded components

The vanilla JS layer (`chat-*.js`) has better chat features (streaming, tool calls, sessions, markdown, file browser) — these must be **ported to SolidJS components**, then the vanilla layer deleted.

### Migration Plan

| Vanilla JS Feature | Port To | Priority |
|---|---|---|
| WebSocket streaming (`chat-stream.js`) | `ChatView.tsx` persistent WS | P0 |
| Tool call rendering (`chat-tools.js`) | New `ToolCallCard.tsx` component | P0 |
| Markdown + syntax highlight (`chat-markdown.js`) | Enhance `CodeBlock.tsx` | P0 |
| Session management (`chat-sessions.js`) | New `SessionStore` signal | P1 |
| File browser (`chat-file-panel.js`) | Enhance `FileTree.tsx` | P1 |
| Health dashboard (`chat-health.js`) | New `HealthPane.tsx` | P1 |
| HexFlo swarm cards (`chat-hexflo.js`) | Already exists in `SwarmMonitor.tsx` | Done |
| Dependency graph (`index.html` canvas) | New `DependencyGraphPane.tsx` | P2 |

---

## New Layout: Three-Column with Chat-First Center

### Desktop (>1200px)

```
+--------+----------------------------+------------------+
| Left   | Center                     | Right            |
| 240px  | flexible                   | 320px            |
+--------+----------------------------+------------------+
|        |                            |                  |
| [hex]  | [Mode: Plan | Build]       | Context Panel    |
|        |                            |                  |
| -----  | Messages stream            | - Active project |
| PROJ   | (tool calls inline,        | - Health badge   |
| -----  |  diffs inline,             | - Agent status   |
| > Proj |  collapsible sections)     | - Token budget   |
|   A    |                            | - SpacetimeDB    |
| > Proj |                            |   connections    |
|   B    |                            | - File context   |
|        |                            |   pills          |
| -----  |                            |                  |
| AGENTS |                            | ---- Detail ---- |
| -----  |                            |                  |
| o cdr  |                            | (expands when    |
| o plnr |                            |  you click an    |
|   [+]  |                            |  agent, task,    |
|        |                            |  or swarm from   |
| -----  |                            |  the sidebar)    |
| SWARMS |                            |                  |
| -----  |                            |                  |
| > feat |                            |                  |
|   auth |                            |                  |
|        +----------------------------+                  |
|        | > [input] [@files] [/cmds] |                  |
|        | [Plan|Build] [model badge] |                  |
+--------+----------------------------+------------------+
```

### Key Layout Decisions

1. **Chat IS the center** — not a panel, not a pane, not a bottom card. The chat stream is the primary interaction surface. This matches OpenCode, Claude Code, Aider, and Warp.

2. **Left sidebar = navigation** — Projects (switchable), Agents (clickable for logs), Swarms (clickable for monitor). Always visible. Collapsible to 48px icon column.

3. **Right panel = context** — Shows details for whatever is selected. Default: project health + connections. Click an agent: agent log. Click a swarm: task DAG. Click a file: diff viewer. **This replaces the tiling pane system for most users** — the pane system remains available via Ctrl+\ for power users.

4. **Universal input bar at bottom** — Accepts:
   - Plain text → chat message
   - `/` prefix → slash command (triggers command palette inline)
   - `@` prefix → file picker overlay
   - `Tab` → toggles Plan/Build mode

5. **Mode indicator** — Visible in status bar and input area:
   - **Plan mode** (default): AI discusses, analyzes, suggests — no file changes
   - **Build mode**: AI edits files, runs commands, creates tasks
   - Visual: Plan = blue accent, Build = green accent

### Tablet (768-1200px)

- Left sidebar collapses to icon strip (48px)
- Right panel becomes overlay sheet (triggered by selection)
- Chat remains full center

### Mobile (<768px)

- Bottom tab bar: Chat | Tasks | Agents | Health
- Single column, full width
- Details open as full-screen sheets

---

## Component Architecture (New)

### Delete

```
hex-nexus/assets/
  index.html                    # DELETE — replaced by unified SolidJS app
  css/chat-*.css (8 files)      # DELETE — replaced by Tailwind
  js/chat-*.js (14 files)       # DELETE — features ported to SolidJS
```

### Rename

```
dashboard.html → index.html     # The SolidJS app becomes the sole entry point
```

### New/Modified Components

```
src/
  components/
    layout/
      App.tsx                   # MODIFY — new three-column layout
      Sidebar.tsx               # MODIFY — add project switcher, improve nav
      RightPanel.tsx            # MODIFY → ContextPanel.tsx — detail-on-select
      BottomBar.tsx             # REWRITE — persistent WS, universal input,
                                #           @-file picker, /command inline,
                                #           Plan/Build toggle
    chat/
      ChatView.tsx              # REWRITE — persistent WebSocket, streaming,
                                #           tool call rendering, inline diffs
      MessageList.tsx           # MODIFY — add collapsible tool call sections
      Message.tsx               # MODIFY — support tool_use, diff, permission types
      ChatInput.tsx             # MERGE INTO BottomBar — single input point
      ToolCallCard.tsx          # NEW — renders tool calls with expand/collapse
      InlineDiff.tsx            # NEW — accept/reject per hunk
      FilePickerOverlay.tsx     # NEW — @-mention file search overlay
    health/
      HealthPane.tsx            # NEW — architecture health (from index.html)
      HealthBadge.tsx           # NEW — compact health indicator for sidebar
    graph/
      DependencyGraphPane.tsx   # NEW — canvas dep graph (from index.html)
    session/
      SessionList.tsx           # NEW — sidebar session list with status badges
      SessionCard.tsx           # NEW — individual session with rename/fork/delete
  stores/
    chat.ts                     # NEW — persistent WebSocket, message history,
                                #       streaming state, session management
    session.ts                  # NEW — session CRUD, auto-compact at 95% tokens
    mode.ts                     # NEW — Plan/Build mode signal
    health.ts                   # NEW — architecture health polling
```

### State Management

```
Signals (SolidJS createSignal):
  mode: "plan" | "build"
  activeProject: Project | null
  activeSession: Session | null
  chatMessages: Message[]
  isStreaming: boolean
  contextFiles: string[]          # @-mentioned files
  rightPanelContent: PanelContent  # what the right panel shows

WebSocket (single persistent connection):
  /ws/chat — maintained by chat store
  Reconnect with exponential backoff
  Session ID sent on connect for continuity
```

---

## Interaction Flows (Redesigned)

### "I want to analyze my project's architecture"

**Before (current):** Navigate to index.html → wait 60s auto-refresh OR open chat.html → find unlabeled grid icon → open dashboard panel → scroll to health section
**After:** Type "analyze" in input → inline streaming results with health score, violations, and fix suggestions

### "I want to create a swarm"

**Before:** Ctrl+P → search "swarm" → browser prompt() → hardcoded topology
**After:** Type "/swarm init feature-auth mesh" → swarm card appears in sidebar → task DAG shows in right panel

### "I want to browse files"

**Before:** Click unlabeled icon → scroll to Files → expand → select project → navigate
**After:** Type "@" → fuzzy file search overlay → select file → diff/content shows in right panel

### "I want to spawn an agent"

**Before:** Find 14px "+" icon OR know Ctrl+N
**After:** Click visible "+" button in Agents sidebar section (now 24px with label) OR type "/spawn coder" → SpawnDialog with proper form

### "I want to see what agents are doing"

**Before:** Click agent name in sidebar → pane opens (if you knew to click)
**After:** Agents section in sidebar shows live status dots. Click agent → right panel shows agent log. Agent activity also streams inline in chat when in Build mode.

---

## Command Palette Enhancement

The existing Ctrl+P command palette is good. Enhancements:

1. **Also trigger with Cmd+K** (muscle memory from Linear, Superhuman, Raycast)
2. **Show keyboard shortcuts inline** (passive learning — Superhuman pattern)
3. **Add `/` prefix support in the main input** (type `/` to see available commands)
4. **Context-aware results** — when a swarm is selected, show swarm-related commands first
5. **Recent commands** section at the top

---

## Design System Unification

### Problem
Vanilla JS uses CSS custom properties (`--accent: #58a6ff` blue). SolidJS uses Tailwind (`cyan-500` teal). Different accent colors, different spacing, different typography.

### Solution
Consolidate on Tailwind with CSS custom properties for theming:

```css
:root {
  --hex-accent: #58a6ff;        /* Keep the blue — it's the hex brand */
  --hex-accent-dim: rgba(88, 166, 255, 0.12);
  --hex-green: #3fb950;
  --hex-yellow: #e3b341;
  --hex-red: #f85149;
  --hex-purple: #bc8cff;
  --hex-bg: #0d1117;
  --hex-card: #161b22;
  --hex-border: #30363d;
  --hex-text: #e6edf3;
  --hex-text2: #9eaab8;
  --hex-text3: #6e7a88;
}
```

Reference these in `tailwind.config.ts`:
```typescript
colors: {
  hex: {
    accent: 'var(--hex-accent)',
    green: 'var(--hex-green)',
    // ...
  }
}
```

---

## Implementation Phases

### Phase 1: Fix the Plumbing (1-2 days)
- [ ] Fix BottomBar.tsx: persistent WebSocket instead of new-per-message
- [ ] Add Plan/Build mode toggle (Tab key + visual indicator)
- [ ] Wire BottomBar input to ChatView's WebSocket
- [ ] Replace `prompt()` in swarm init with proper SwarmInitDialog
- [ ] Add toast notification system for operation feedback

### Phase 2: Unify Entry Points (2-3 days)
- [ ] Move `dashboard.html` to `index.html` (SolidJS becomes the sole app)
- [ ] Port streaming chat from `chat-stream.js` to SolidJS `ChatView.tsx`
- [ ] Port tool call rendering from `chat-tools.js` to `ToolCallCard.tsx`
- [ ] Port markdown rendering from `chat-markdown.js` to SolidJS
- [ ] Delete `chat.html`, old `index.html`, and all `chat-*.js` files

### Phase 3: New Layout (2-3 days)
- [ ] Implement three-column layout in `App.tsx`
- [ ] Convert `RightPanel.tsx` to `ContextPanel.tsx` (detail-on-select)
- [ ] Add project switcher to sidebar header
- [ ] Add `@`-file picker overlay to input
- [ ] Add session list to sidebar
- [ ] Add `/` inline command search to input

### Phase 4: Port Dashboard Features (2-3 days)
- [ ] Create `HealthPane.tsx` (architecture health from old index.html)
- [ ] Create `HealthBadge.tsx` (compact health in sidebar)
- [ ] Create `DependencyGraphPane.tsx` (canvas graph from old index.html)
- [ ] Port event log to SolidJS component
- [ ] Port coordination panel (worktree locks, task claims)

### Phase 5: Polish (1-2 days)
- [ ] Unify design tokens (CSS vars → Tailwind config)
- [ ] Add responsive breakpoints (tablet icon sidebar, mobile bottom tabs)
- [ ] Add keyboard shortcut hints throughout UI
- [ ] Add inline diff accept/reject for Build mode
- [ ] Remove all vanilla JS remnants and unused CSS

---

## Success Criteria

After redesign, a user should be able to:

1. **Open the dashboard and immediately understand what they're looking at** — project health, active agents, active swarms visible at a glance
2. **Start chatting without finding hidden panels** — the input is always visible at the bottom, chat is the center
3. **Switch between Plan and Build modes** with one keypress (Tab)
4. **Find any feature in under 2 seconds** via Cmd+K or visible sidebar navigation
5. **Attach file context** by typing `@` and searching
6. **See agent activity inline** — tool calls, diffs, and status updates stream into the chat
7. **Monitor swarm progress** by glancing at the sidebar (no navigation required)
8. **Create swarms, tasks, and agents** via `/commands` or visible buttons (no `prompt()` dialogs)

---

## References

- [UX Audit — Current State](./ux-audit-current-state.md)
- [Competitor Analysis — 9 AI Dev Tools](./ux-research-competitor-analysis.md)
- [Design Pattern Research — 40+ Sources](./ux-research-design-patterns.md)
- [Component Architecture Map](./ux-component-architecture.md)
