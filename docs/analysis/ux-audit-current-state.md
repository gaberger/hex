# UX Audit: hex-nexus Dashboard Current State

**Date:** 2025-03-20
**Scope:** All files under `hex-nexus/assets/` -- vanilla JS layer (index.html, chat.html, chat-*.js) and SolidJS SPA layer (src/components/, src/stores/)
**Severity scale:** CRITICAL / HIGH / MEDIUM / LOW

---

## Executive Summary

The hex-nexus dashboard suffers from a fundamental architectural split: it ships THREE separate HTML entry points (`index.html`, `chat.html`, `dashboard.html`) powered by two incompatible rendering paradigms (vanilla JS with global `window.HexChat` state vs. SolidJS with reactive stores). Users encounter duplicated features, hidden panels, conflicting navigation models, and no coherent information architecture. The interface attempts to serve as a chat client, an architecture dashboard, a swarm monitor, a fleet manager, and a project control plane simultaneously, without clear wayfinding between these roles.

---

## 1. Information Architecture Problems

### 1.1 CRITICAL: Three Separate Applications Masquerading as One

The Vite config (`vite.config.js`) defines three entry points:

| Entry | File | Rendering | Purpose |
|-------|------|-----------|---------|
| `main` | `index.html` | Vanilla JS + CSS imports via Vite | "Old dashboard" -- 2x2 grid of cards (health, tokens, swarms, events) + chat + dependency graph |
| `chat` | `chat.html` | Vanilla JS (`window.HexChat` namespace) | Chat-first interface with collapsible left dashboard panel and right sidebar |
| `dashboard` | `dashboard.html` | SolidJS SPA (`<div id="solid-root">`) | Tiling pane manager with command palette, project overview, swarm monitor, fleet view |

There is **no navigation between these three**. A user who lands on `index.html` has no way to discover that `dashboard.html` exists (and vice versa). The SolidJS app and the vanilla JS apps share zero state -- they maintain completely separate WebSocket connections, separate project registries, and separate session management.

### 1.2 HIGH: "Projects" Means Different Things in Each Layer

| Layer | How projects appear | How you create one | How you switch |
|-------|--------------------|--------------------|----------------|
| `index.html` | `<select id="projectSelector">` dropdown in the header + `<nav class="project-tabs">` tab bar below header | No creation UI visible -- waits for projects to appear via API | Click a tab or select from dropdown |
| `chat.html` | No project UI at all. `state.currentProjectId` is read from URL query param `?project_id=` | Not possible from this view | Change the URL manually |
| `dashboard.html` (SolidJS) | `ProjectOverview` component in center pane with card grid. "Projects" sidebar section with single "Overview" button | "Add Project" card with path input, or `hex project register` CLI command | Click a ProjectCard (opens file tree / task board in a new pane) |

**Impact:** A user cannot form a mental model of "what is a project" because the concept is presented three different ways with three different interaction patterns.

### 1.3 HIGH: No Persistent Navigation or URL Routing

The SolidJS app (`dashboard.html`) has zero URL routing. All navigation state is held in the `paneTree` signal and persisted to `localStorage` key `hex_pane_layout`. This means:

- Browser back/forward buttons do nothing
- Bookmarking a specific view is impossible
- Sharing a link to a specific swarm or agent is impossible
- Refreshing the page restores the last pane layout (good) but with potentially stale data references (swarmId, agentId stored in props are string IDs that may no longer exist)

The vanilla JS apps (`index.html`, `chat.html`) have no routing at all.

---

## 2. Navigation Confusion: Two Rendering Paradigms

### 2.1 CRITICAL: Vanilla JS and SolidJS Are Completely Disconnected

**Vanilla JS layer** (`chat-*.js` files):
- Global namespace: `window.HexChat = { state: {...}, dom: {} }`
- DOM references captured by ID at startup: `messages`, `input`, `sendBtn`, `connDot`, `connLabel`, `gaugeFill`, `hexfloSwarms`, `hexfloAgents`, `filePanel`, etc.
- State management: mutable object properties on `window.HexChat.state`
- 14 script files loaded in dependency order via `<script>` tags
- WebSocket connection to `/ws/chat` with raw `onmessage` handler

**SolidJS layer** (`src/` directory):
- Framework: SolidJS with fine-grained reactivity
- State management: `createSignal` in stores (`panes.ts`, `commands.ts`, `ui.ts`, `connection.ts`, `nexus-health.ts`)
- Data source: 4 separate SpacetimeDB module connections (`hexflo-coordination`, `agent-registry`, `inference-gateway`, `fleet-state`) + REST API polling
- Styling: Tailwind CSS utility classes (via `@tailwindcss/vite` plugin)

**These two layers cannot communicate.** If a user creates a swarm in the SolidJS dashboard, the vanilla JS chat view will not reflect it until its next 10-second polling interval hits the REST API independently. Session state, WebSocket connections, and project context are all duplicated.

### 2.2 MEDIUM: Inconsistent Styling Systems

| Layer | Styling approach |
|-------|-----------------|
| `index.html` | CSS custom properties (`:root` vars) + hand-written component CSS in `src/styles.css` and `css/*.css` files |
| `chat.html` | Same CSS custom properties + same `css/*.css` imports via `src/chat.css` |
| `dashboard.html` | Tailwind CSS utility classes. No shared design tokens with the vanilla layer. Different color palette (gray-950 vs `--bg:#0d1117`), different accent color (`cyan-500`/`green-500` vs `--accent:#58a6ff`) |

**Impact:** The dashboard.html looks visually different from chat.html. Colors, spacing, typography, and component styling are inconsistent. The vanilla layer uses blue accent (#58a6ff), while the SolidJS layer uses cyan/teal (cyan-500 = #06b6d4).

---

## 3. Panel Discoverability: Hidden Panels Requiring Commands or Actions

### 3.1 Panels Hidden Behind Toggle Buttons

| Panel | How to reveal | Discovery cue | Severity |
|-------|--------------|----------------|----------|
| **Dashboard panel** (chat.html) | Click the 4-square grid icon button `#dashToggle` in the top bar | Tiny 32x32px button with no label, no tooltip text (only `aria-label`) | HIGH |
| **File browser** (chat.html) | Click "Files" section title inside the already-hidden dashboard panel | Nested inside a hidden panel -- requires TWO clicks to reach from default state | CRITICAL |
| **Right sidebar** (chat.html, mobile) | Click hamburger menu button `#sidebarToggle` | Only visible at viewport width < 768px. On desktop the sidebar is always visible but there is no indication it exists on mobile | MEDIUM |
| **Command palette** (dashboard.html) | Press `Ctrl+P` or type `/` in the bottom bar | The only discovery cue is a tiny `Ctrl+P` kbd badge in the top-right header, styled at 10px font size | HIGH |
| **Spawn Agent dialog** (dashboard.html) | Press `Ctrl+N` or click the tiny `+` icon next to the "Agents" sidebar section header | The `+` icon is 14x14px with no label. No indication that Ctrl+N exists except via command palette | HIGH |
| **Inference panel** (dashboard.html) | Via command palette "Open Inference Panel" or clicking the "Inference" section title in the right panel | The right panel section title is clickable but styled identically to non-clickable titles -- no cursor change visible in the CSS (it is set but is very subtle) | MEDIUM |
| **Fleet view** (dashboard.html) | Via command palette "Open Fleet View" or clicking "Fleet" section title in right panel | Same affordance problem as Inference | MEDIUM |
| **Swarm monitor** (dashboard.html) | Click a swarm row in the left sidebar | Only appears if swarms exist. If no swarms are active, there is no way to discover this view exists | MEDIUM |
| **Agent log** (dashboard.html) | Click an agent row in the left sidebar | Same conditional visibility as swarm monitor | MEDIUM |
| **Chat view** (dashboard.html) | Via command palette "Open Chat" | No dedicated button, no sidebar entry. Chat is a major feature but requires knowing to search for it | HIGH |

### 3.2 The "Two Clicks to File Browser" Problem

In `chat.html`, the file browser is embedded inside the dashboard panel (`<div class="dash-files-section">`). The dashboard panel itself is off-screen by default (`transform: translateX(-100%)`). To browse files:

1. Click the unlabeled grid icon in the top bar (to open the dashboard panel)
2. Scroll to the bottom of the dashboard panel
3. Click the "Files" section title (to expand the file panel within the dashboard)
4. Wait for the project list to load from `/api/projects`
5. Click a project to browse its files

This is a 5-step process for a core feature.

---

## 4. Project Management UX

### 4.1 Registration Flow

**SolidJS (`ProjectOverview.tsx`):**
- Empty state shows a clear call-to-action: "Register a project to get started" with a path input field
- Sends POST to `/api/projects/register` with `{ path: "/absolute/path" }`
- After registration, the project card appears in a responsive grid
- Also shows an "Add Project" dashed-border card when fewer than 4 projects exist
- CLI alternative is documented inline: `hex project register /path/to/project`

**Vanilla JS (`index.html`):**
- No registration UI at all
- The `<select id="projectSelector">` and `<nav id="projectTabs">` are populated from the API but there is no way to add a new project
- Shows "Waiting for projects to connect..." if none exist

**Verdict:** The SolidJS layer has a reasonable registration flow. The vanilla JS layer has none. Users on `index.html` or `chat.html` must use the CLI or switch to `dashboard.html`.

### 4.2 Switching Between Projects

**SolidJS:** No concept of "switching" projects. ProjectOverview shows all projects as cards. Clicking a card opens a pane for that specific project (file tree, task board). Multiple project panes can be open simultaneously (up to 4 panes).

**Vanilla JS (`index.html`):** Project tabs along the top. Clicking a tab presumably changes which project data is shown in the dashboard cards (health, tokens, events). The tab bar is populated by `chat-dashboard.js` or external polling.

**Vanilla JS (`chat.html`):** Project is set via URL query parameter `?project_id=`. No in-app switching mechanism.

### 4.3 What's Missing

- No project settings or configuration view
- No project deletion or unregistration
- No project health summary that shows all projects at a glance (vanilla layer)
- The "dismiss" feature in ProjectOverview hides cards locally but does not remove projects from the server
- No indication of which project the chat session is connected to (in chat.html)

---

## 5. Chat vs Dashboard Metaphor Conflict

### 5.1 Three Chat Implementations

| Location | Implementation | Features |
|----------|---------------|----------|
| `index.html` bottom-left | Vanilla JS. Fixed-height 500px card. Quick-action buttons (Ping, Analyze, Build, Summarize, Validate, Generate, Spawn Agent, Create Task, Claude). Basic input + send button. | No sessions, no history, no streaming indicators, no tool call rendering |
| `chat.html` center | Vanilla JS. Full-height `window.HexChat` system. Streaming support, tool call cards, agent badges, markdown rendering, session management (fork, delete, rename), file panel integration. | Full-featured but isolated from dashboard data |
| `dashboard.html` pane | SolidJS `ChatView` component. Renders inside a tiling pane. | Appears to be a separate implementation -- creates its own WebSocket to `/ws/chat`. Does not share state with vanilla chat |

### 5.2 Where Metaphors Conflict

**The `index.html` page is a dashboard that happens to have a chat box.** The chat is crammed into a 500px-tall card alongside a dependency graph canvas. It has 9 quick-action buttons but no session management. This creates a confusing hierarchy: is this a dashboard you can chat from, or a chat with dashboard widgets?

**The `chat.html` page is a chat app that happens to have a dashboard panel.** The dashboard is hidden by default and slides in from the left. The right sidebar shows agent info, token budget, and RL insights. The chat is the primary interaction mode, but the dashboard panel duplicates information that also appears in `index.html` (codebase stats, events, instances, swarms, agents).

**The `dashboard.html` page is a tiling window manager that can contain chat.** Chat is one of many pane types (`"chat"` in the `PaneType` union). It competes for screen real estate with project overview, swarm monitor, fleet view, etc. The bottom bar also accepts text input, creating an ambiguity: should I type in the bottom bar or in the chat pane?

### 5.3 The BottomBar Input vs Chat Pane Conflict

In `dashboard.html`, the BottomBar component renders a persistent input field at the bottom of the screen with a `>` prompt. It accepts:
- Plain text (sent via WebSocket)
- Slash commands (`/` prefix triggers command search)

But the ChatView pane, if open, also has its own input field. The user now has **two text inputs on screen**, both of which send messages. The BottomBar creates a new WebSocket connection for each message (`new WebSocket(wsUrl)` inside `sendChatMessage()`), while the chat pane presumably maintains a persistent connection. This means:
- Messages sent from the BottomBar and the ChatView go to different WebSocket connections
- Responses from one will not appear in the other
- The user has no way to know which input to use

---

## 6. Cognitive Load Analysis

### 6.1 `index.html` -- Simultaneous UI Sections

Visible on first load (assuming 1920x1080):

1. Header bar (logo, project selector, project path, AST badge, project count, version badge, connection status)
2. Project tabs bar
3. Architecture Health card (ring chart + 6 stats + violations list + unused list)
4. Token Efficiency card (file selector + 4 level bars + summary)
5. Instance Status card (cleanup button + table)
6. Event Log card (filter buttons + scrollable log)
7. Coordination card (worktree locks + task claims + activity stream + unstaged files -- 4 sub-sections)
8. Command Chat card (WS status + 9 quick-action buttons + message area + input)
9. Dependency Graph card (canvas + zoom controls + legend)
10. Decision modal (hidden until triggered)

**Total: 10 distinct sections, 4 of which have sub-sections. Over 40 individual data points visible simultaneously.**

The page requires scrolling to see everything (`dashboard-scroll` container). The bottom row (chat + graph) is a 2-column grid with a minimum height of 420px, pushing the total page height well beyond 100vh.

### 6.2 `chat.html` -- Simultaneous UI Sections

Default state (dashboard panel closed):

1. Top bar (dashboard toggle, logo, model selector, connection status, sidebar toggle)
2. Chat message area (full height)
3. Input area (textarea + send button + hint)
4. Right sidebar: Token Budget (gauge + 4 stats)
5. Right sidebar: Agent Info (status, turns, project)
6. Right sidebar: Controls (3 buttons)
7. Right sidebar: RL Insights
8. Right sidebar: Architecture Health

**Total: 8 sections. Moderate density.**

When dashboard panel is opened, adds:
9. Codebase stats (3 numbers)
10. Event log
11. Instances table
12. Swarms list
13. Agents list
14. File browser

**Total with panel: 14 sections. High density but manageable because the dashboard panel scrolls independently.**

### 6.3 `dashboard.html` -- Simultaneous UI Sections

Default state (single pane, ProjectOverview):

1. Top bar (logo, keyboard shortcuts hints, Ctrl+P hint, theme toggle)
2. Left sidebar: Projects section (Overview button)
3. Left sidebar: Agents section (+ button, agent list)
4. Left sidebar: Swarms section (swarm list)
5. Center pane: ProjectOverview (header + stats bar + project cards + add card)
6. Right panel: Nexus status
7. Right panel: SpacetimeDB connections (4 module statuses + reconnect button)
8. Right panel: Inference providers
9. Right panel: Fleet nodes
10. Right panel: Token stats (in/out/cost/requests)
11. Bottom bar (input field + mode badge)

**Total: 11 sections. High density due to right panel, but the tiling pane model keeps center content focused.**

---

## 7. Action Clarity: Step-by-Step Traces

### 7.1 "I want to see architecture health for my project"

**Via `index.html`:**
1. Open `http://localhost:5555/` (served by hex-nexus)
2. Architecture Health card is visible immediately (top-left)
3. Wait for auto-refresh (60 second interval) or... there is no manual refresh button

**Via `chat.html`:**
1. Open `http://localhost:5555/chat.html`
2. Click the unlabeled grid button in the top-left (to open dashboard panel)
3. Look at the "Codebase" section (shows files, imports, exports -- but NOT the health score)
4. Check the right sidebar "Architecture Health" section (at the bottom of the sidebar)
5. Alternatively, type `/status` in the chat input

**Via `dashboard.html`:**
1. Open `http://localhost:5555/dashboard.html`
2. Use command palette (Ctrl+P) and search for "analyze"
3. Select "Run Architecture Analysis" -- this fires a POST to `/api/analyze` but shows no results in the UI
4. There is no dedicated architecture health view in the SolidJS app

**Verdict:** Architecture health is a first-class citizen only in `index.html`. The SolidJS app can trigger analysis but cannot display results.

### 7.2 "I want to create a swarm and monitor it"

**Via `dashboard.html`:**
1. Ctrl+P to open command palette
2. Search "swarm", select "Initialize New Swarm"
3. Browser `prompt()` dialog asks for swarm name (jarring -- no styled dialog)
4. POST to `/api/swarms` with hardcoded `topology: "hierarchical"`
5. Swarm appears in left sidebar under "Swarms" section
6. Click the swarm to open SwarmMonitor pane

**Via `chat.html`:**
1. Dashboard panel has a "Swarms" section that shows swarm cards
2. No creation UI -- must use CLI (`hex swarm init <name>`)
3. Swarm cards show progress bars and task lists (read-only)

**Via `index.html`:**
1. Instance Status card shows swarms as table rows
2. No creation UI

### 7.3 "I want to chat with an agent about my project"

**Via `chat.html`:**
1. Open `http://localhost:5555/chat.html`
2. Type in the textarea and press Enter
3. Messages flow via WebSocket. Agent responses stream in with tool call cards.
4. Session persists (sessions sidebar is dynamically injected at top of right sidebar)

**Via `dashboard.html`:**
1. Ctrl+P, search "chat", select "Open Chat"
2. ChatView pane opens (if it exists and works)
3. OR type directly in the bottom bar (creates ephemeral WebSocket per message -- broken pattern)

**Via `index.html`:**
1. Scroll to the bottom-left "Command Chat" card
2. Click a quick-action button or type in the input
3. No streaming, no tool calls, no sessions. Basic send/receive only.

### 7.4 "I want to browse my project's files"

**Via `chat.html`:**
1. Click grid icon (open dashboard panel)
2. Scroll to "Files" section at the bottom
3. Click "Files" title to expand file panel
4. Click a project from the project list
5. Navigate directories by clicking

**Via `dashboard.html`:**
1. Click a ProjectCard in ProjectOverview
2. This calls `openPane('filetree', ...)` -- FileTree component opens in a pane
3. FileTree component (lazy loaded) renders

**Via `index.html`:**
1. Not available. No file browsing capability.

### 7.5 "I want to spawn an agent"

**Via `dashboard.html`:**
1. Click the `+` icon next to "Agents" in the left sidebar, OR press Ctrl+N
2. SpawnDialog overlay appears with form fields
3. Fill in name, role, project path, submit

**Via `index.html`:**
1. Click "Spawn Agent" quick-action button in the Command Chat card
2. This sends a `spawn-agent` command via WebSocket with hardcoded payload `{"name":"helper","role":"coder"}`
3. No form, no customization

**Via `chat.html`:**
1. No dedicated UI. Must type a command in the chat input.

---

## 8. Specific Usability Issues

### 8.1 CRITICAL: BottomBar Creates New WebSocket Per Message

In `dashboard.html`, `BottomBar.tsx` line 57:
```typescript
const ws = new WebSocket(wsUrl);
ws.onopen = () => {
  ws.send(JSON.stringify({...}));
  // WebSocket stays open for streaming response
};
```

Every message creates a brand new WebSocket connection. This means:
- No conversation context is maintained
- The server receives disconnected single-message sessions
- Responses have nowhere to render (BottomBar has no message display area)
- Memory leak: WebSocket connections are never explicitly closed

### 8.2 HIGH: Swarm Init Uses `prompt()` and Hardcodes Topology

In `commands.ts` line 137:
```typescript
const name = prompt("Swarm name:");
// ...
body: JSON.stringify({ name, topology: "hierarchical" }),
```

The browser's native `prompt()` dialog is visually jarring in a polished dark-mode UI. The topology is hardcoded to "hierarchical" with no user choice. The SpawnDialog exists as a proper styled dialog -- swarm init should have one too.

### 8.3 HIGH: No Error Feedback for Failed Operations

- `registerProject()` in ProjectOverview catches errors silently (`finally` block only resets `registering` flag)
- Architecture analysis triggered from command palette fires and forgets (no toast, no result display)
- SpacetimeDB connection failures show only in the browser console
- The only visible error indicator is the connection dot in chat.html turning red

### 8.4 MEDIUM: Session Management Injected via DOM Manipulation

In `chat-sessions.js`, the session list is created by dynamically building DOM elements and inserting them at the top of the sidebar via `sidebar.insertBefore(section, sidebar.firstChild)`. Inline styles are injected via a `<style>` element appended to `<head>`. This is fragile:
- The injected styles can conflict with existing styles
- The sidebar's scroll position resets when sessions are re-rendered
- Session state is not synced if the same user has multiple tabs open

### 8.5 MEDIUM: Inconsistent Connection Status Indicators

| Location | Indicator | What it monitors |
|----------|-----------|-----------------|
| `index.html` header | Green/red dot + "Connecting..." label | Unclear -- no WebSocket connection code in index.html |
| `chat.html` top bar | Green/red dot + "connected"/"disconnected" label | WebSocket to `/ws/chat` |
| `chat.html` chat section | Separate green dot + label | Same WebSocket (duplicated indicator) |
| `dashboard.html` right panel | 4 separate dots for SpacetimeDB modules + nexus online/offline | SpacetimeDB connections + REST API health poll |
| `dashboard.html` ProjectOverview | Green/red dot + "SpacetimeDB connected"/"Connecting..." | Aggregate `anyConnected()` signal |

A user sees between 1 and 6 connection indicators depending on which view they are on. None of them tell the complete story.

### 8.6 LOW: `min-width: 1024px` on `index.html` Body

The `index.html` sets `body { min-width: 1024px }` which forces a horizontal scrollbar on any viewport narrower than 1024px. The `chat.html` has a responsive breakpoint at 768px. The `dashboard.html` has no explicit min-width. These are inconsistent responsive strategies.

### 8.7 LOW: External CDN Dependencies in chat.html

`chat.html` loads `marked` and `highlight.js` from CDNs (`cdn.jsdelivr.net`). If the user is offline or behind a firewall, markdown rendering and syntax highlighting silently fail. The SolidJS app does not use these libraries at all (it has its own `CodeBlock.tsx` and `DiffViewer.tsx` components).

---

## 9. Summary of Findings by Priority

### CRITICAL (Must Fix)

| # | Issue | Impact |
|---|-------|--------|
| C1 | Three disconnected HTML entry points with no cross-navigation | Users cannot discover all features |
| C2 | Two rendering paradigms (vanilla JS vs SolidJS) with no shared state | Data inconsistency, duplicated connections, doubled memory usage |
| C3 | BottomBar creates new WebSocket per message (dashboard.html) | Broken chat, memory leak, no conversation context |
| C4 | File browser requires 5 steps through two hidden panels (chat.html) | Core feature is effectively invisible |

### HIGH (Should Fix)

| # | Issue | Impact |
|---|-------|--------|
| H1 | Command palette is the only way to access Chat, Inference, Fleet views | Power-user-only interface with no progressive disclosure |
| H2 | Dashboard toggle button has no label or tooltip (chat.html) | First-time users will not find the dashboard panel |
| H3 | No error feedback for failed API operations | Users cannot tell if actions succeeded |
| H4 | Swarm init uses browser `prompt()` and hardcodes topology | Jarring UX, no user control |
| H5 | "Projects" concept is inconsistent across all three views | No coherent mental model |
| H6 | Chat exists in 3 places with 3 different capability levels | Users get different experiences depending on entry point |
| H7 | Spawn Agent has no obvious entry point (14x14px icon or keyboard shortcut) | Discovery requires command palette knowledge |

### MEDIUM (Should Improve)

| # | Issue | Impact |
|---|-------|--------|
| M1 | Inconsistent color systems (CSS custom properties vs Tailwind) | Visual inconsistency between views |
| M2 | 5-6 simultaneous connection status indicators | Cognitive overload, unclear system health |
| M3 | No URL routing in SolidJS app | Cannot bookmark, share, or use browser navigation |
| M4 | Session management injected via raw DOM manipulation | Fragile, style conflicts, no multi-tab sync |
| M5 | No manual refresh for architecture health data | 60-second auto-refresh is the only option |
| M6 | Architecture analysis can be triggered but results cannot be viewed (dashboard.html) | Feature gap in the newer SPA |

### LOW (Nice to Have)

| # | Issue | Impact |
|---|-------|--------|
| L1 | `min-width: 1024px` on index.html forces horizontal scroll on small screens | Mobile/tablet unusable |
| L2 | External CDN dependencies for markdown/highlight.js in chat.html | Offline mode breaks |
| L3 | `dashboard.html` loads 4 SpacetimeDB connections on startup even if modules are not deployed | Console noise, unnecessary retries |

---

## 10. Recommendations

### Short Term (Unify Entry Points)

1. **Deprecate `index.html`** -- its dashboard cards (health, tokens, coordination, dependency graph) should become pane types in the SolidJS app. The SolidJS tiling pane model is the more flexible and maintainable approach.

2. **Move chat.html features into the SolidJS app** -- the vanilla JS chat is the most feature-complete (streaming, tool calls, sessions, markdown, file browser). Port these capabilities into the SolidJS ChatView component. Then deprecate chat.html.

3. **Add a navigation bar or router** to the SolidJS app so that different "modes" (dashboard, chat, fleet) have URLs and can be bookmarked.

### Medium Term (Fix Broken Interactions)

4. **Fix BottomBar** -- it should not create new WebSocket connections per message. Either connect to the ChatView's WebSocket or remove the text input from the bottom bar entirely (keep only slash commands).

5. **Add visible navigation affordances** -- every pane type should be reachable from the sidebar or a visible toolbar, not just via Ctrl+P. Add an "Open Chat" button, a "Fleet" button, an "Inference" button to the sidebar.

6. **Create a proper SwarmInitDialog** component (like SpawnDialog) instead of using `prompt()`.

7. **Add toast/notification system** for operation feedback (success/failure).

### Long Term (Architecture)

8. **Single application, single rendering framework** -- consolidate on SolidJS. Remove all vanilla JS chat-*.js files once their features are ported.

9. **Single connection layer** -- one SpacetimeDB connection manager shared across all views, with a REST API fallback when SpacetimeDB modules are not deployed.

10. **Design system** -- extract shared design tokens (colors, spacing, typography) into CSS custom properties that both Tailwind config and any remaining vanilla CSS can reference.
