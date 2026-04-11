# UX Research: Design Patterns for Chat-Based Developer Tool Dashboards

**Date:** 2026-03-20
**Purpose:** Research synthesis of layout patterns, interaction designs, and UX principles that make chat-based developer tools successful. Findings are intended to inform the hex-nexus dashboard redesign.

---

## Table of Contents

1. [The Three-Column Pattern in Dev Tools](#1-the-three-column-pattern-in-dev-tools)
2. [Command Palette vs Hidden Panels](#2-command-palette-vs-hidden-panels)
3. [Progressive Disclosure in Complex UIs](#3-progressive-disclosure-in-complex-uis)
4. [Chat + Dashboard Hybrid Patterns](#4-chat--dashboard-hybrid-patterns)
5. [Project/Session Management in AI Tools](#5-projectsession-management-in-ai-tools)
6. [Mobile-Responsive Patterns for Dev Dashboards](#6-mobile-responsive-patterns-for-dev-dashboards)
7. [Anti-Patterns to Avoid](#7-anti-patterns-to-avoid)
8. [Recommendations for hex-nexus](#8-recommendations-for-hex-nexus)

---

## 1. The Three-Column Pattern in Dev Tools

### The Pattern

The three-column layout divides the screen into:
- **Left column** -- Navigation, sessions, projects (narrow, 200-300px)
- **Center column** -- Main content: chat, code, or primary workspace (flexible width)
- **Right column** -- Context, details, properties, or secondary information (collapsible)

### Why It Works (Cognitive Science)

- **Spatial memory:** Users build a mental map of where things live. The left is always "where am I," the center is always "what am I doing," and the right is always "what else do I need to know."
- **Peripheral awareness:** Side columns provide ambient information without demanding focused attention, leveraging the eye's natural horizontal scan pattern (F-pattern reading).
- **Reduced context switching:** All three information layers are visible simultaneously, eliminating the cognitive cost of navigating between views.

### Specific Examples

#### VS Code
VS Code's layout is the gold standard for developer three-column design:
- **Activity Bar** (far left, icon-only, ~48px): Top-level mode switching (Explorer, Search, Git, Debug, Extensions). Each icon opens a different view in the Primary Sidebar.
- **Primary Sidebar** (left, ~300px): Context-specific tree views (file explorer, search results, git changes). Tightly coupled to the Activity Bar -- clicking an Activity Bar icon swaps the sidebar content.
- **Editor Area** (center, flexible): The main workspace. Supports splits, tabs, and grid layouts.
- **Secondary Sidebar** (right, optional): Introduced to allow views from the Panel or Sidebar to be shown simultaneously (e.g., Outline view, Timeline, or a chat panel).
- **Panel** (bottom, optional): Terminal, output, problems, debug console.

Design rationale: VS Code remembers layout across sessions. Users can drag and drop panels to any zone. The Secondary Sidebar was added specifically because users needed to see context (like an AI chat) alongside code without losing their file explorer.

Source: [VS Code Custom Layout](https://code.visualstudio.com/docs/configure/custom-layout), [VS Code UX Guidelines](https://code.visualstudio.com/api/ux-guidelines/overview)

#### Discord
Discord uses a four-zone variant:
- **Server list** (far left, icon column, ~72px): Visual server switching via circular avatars.
- **Channel sidebar** (~240px): Text and voice channels organized by category, with collapsible groups.
- **Message area** (center): Scrolling chat with message composition at the bottom.
- **Member list** (right, collapsible): Users categorized by role.

Key insight: Discord's server icon column acts as a "workspace switcher" -- analogous to switching between projects. The channel sidebar then provides navigation within that workspace. This two-level left navigation (workspace > channel) maps well to project > session hierarchies.

Source: [Discord UI Architecture (ResearchGate)](https://www.researchgate.net/figure/The-Discord-user-interface-The-far-left-sidebar-lists-all-the-Discord-servers-the-user_fig1_337131371)

#### Slack
Slack's 2023+ redesign introduced:
- **Navigation bar** (far left, icon column): Home, DMs, Activity, Later, More -- each shows a different sidebar view.
- **Sidebar** (~260px): Channels and DMs organized into custom sections (user-created folders). Sections can be configured to show only unreads.
- **Message area** (center): Conversation view.
- **Thread panel** (right, on-demand): Opens when clicking a thread, overlaying or splitting the right side.

Key insight: Slack's custom sidebar sections let users create their own information architecture. This is critical for power users who need to organize dozens of channels into meaningful groups (e.g., "Project Alpha," "On-Call," "Social").

Source: [Slack Sidebar Preferences](https://slack.com/help/articles/212596808-Adjust-your-sidebar-preferences), [Slack Custom Sections](https://slack.com/help/articles/360043207674-Organize-your-sidebar-with-custom-sections)

### Application to hex-nexus

The hex dashboard should adopt a three-column layout:
- **Left:** Project list + swarm/session navigation (collapsible to icons only)
- **Center:** Primary interaction surface (chat, task board, or log view depending on mode)
- **Right:** Context panel showing details for the selected item (agent status, task details, file diffs, architecture health)

The left column should support a two-level hierarchy: workspace/project selection (icon strip) followed by contextual navigation (sessions, tasks, agents) within that project.

---

## 2. Command Palette vs Hidden Panels

### The Pattern

A command palette is a searchable, keyboard-activated overlay that provides access to every action in the application. Activated via a memorable shortcut (Cmd+K or Cmd+Shift+P), it replaces the need for deeply nested menus or hidden panel toggles.

### Why Command Palettes Are Better Than Hidden Panels

| Factor | Command Palette | Hidden Panels |
|--------|----------------|---------------|
| **Discoverability** | Users type what they want and discover actions via fuzzy search | Users must know the panel exists and where to find the toggle |
| **Screen real estate** | Zero permanent screen cost -- appears only when invoked | Panels consume persistent layout space or require toggle buttons |
| **Scalability** | Can expose unlimited actions without UI clutter | Each new panel adds layout complexity |
| **Learning curve** | Self-teaching -- shows keyboard shortcuts alongside results | Requires documentation or exploration |
| **Consistency** | Single entry point for all actions | Actions scattered across different panels and menus |

### Cognitive Science

- **Recognition over recall:** Users don't need to remember where a feature lives -- they type a partial name and recognize it in results (Hick's Law reduction).
- **Passive shortcut learning:** Superhuman's key insight: every time you use Cmd+K to find a command, the palette displays the keyboard shortcut next to it. Users passively learn shortcuts without deliberate memorization.
- **Reduced decision fatigue:** One universal entry point ("when in doubt, Cmd+K") eliminates the "where do I click?" question.

### Specific Examples

#### Superhuman
Superhuman's Cmd+K is the canonical example:
- **Unified command access:** Every command lives in one place, simplifying the mental model.
- **Fuzzy search with synonyms:** Users don't need to memorize exact command names.
- **Inline shortcut display:** Each result shows its keyboard shortcut, creating passive learning.
- **Context-aware results:** The palette prioritizes commands relevant to the current state.

Source: [How to Build a Remarkable Command Palette (Superhuman)](https://blog.superhuman.com/how-to-build-a-remarkable-command-palette/)

#### Linear
Linear's command palette (Cmd+K) exemplifies keyboard-first project management:
- **Multi-path actions:** Every action is accessible via buttons, keyboard shortcuts, context menus, OR the command palette. Users choose their preferred modality.
- **Composable navigation:** `G` then `I` for Inbox, `G` then `V` for current cycle. Two-key sequences create a navigational grammar.
- **Instant filtering:** `/` in any view activates filters without leaving the current context.

Source: [Linear Keyboard Shortcuts](https://shortcuts.design/tools/toolspage-linear/), [Linear Concepts](https://linear.app/docs/conceptual-model)

#### VS Code
VS Code's Cmd+Shift+P (Command Palette) and Cmd+P (Quick Open) split commands from file navigation:
- **Command Palette (Cmd+Shift+P):** Searchable list of all editor commands with shortcut hints.
- **Quick Open (Cmd+P):** File search with fuzzy matching, symbol search (`@`), and line navigation (`:`).
- **Prefix grammar:** `>` for commands, `@` for symbols, `#` for workspace symbols, `:` for go-to-line. A single input box serves multiple purposes based on prefix.

Source: [VS Code User Interface](https://code.visualstudio.com/docs/getstarted/userinterface)

#### Raycast
Raycast extends the command palette pattern to the OS level:
- Acts as a launcher, clipboard manager, automation runner, and AI interface.
- Keyboard-first with no mouse requirement.
- Extensible with plugins, making the palette a platform rather than just a feature.

Source: [Raycast on Windows (Windows Forum)](https://windowsforum.com/threads/raycast-on-windows-a-keyboard-first-command-palette-for-fast-actions.395552/)

### Integration with Chat-First Interfaces

For chat-based tools, the command palette serves as an escape hatch from the conversational paradigm:
- **Chat handles open-ended queries** ("analyze this codebase," "explain this error")
- **Command palette handles discrete actions** ("create swarm," "switch project," "show architecture health," "open settings")

The two are complementary: chat for exploration, palette for execution.

### Application to hex-nexus

The hex dashboard should implement a Cmd+K command palette that provides:
1. **Navigation:** Switch between projects, swarms, agents, tasks
2. **Actions:** Create swarm, create task, start analysis, deploy
3. **Search:** Find tasks by name, search agent logs, find ADRs
4. **Mode switching:** Toggle between chat view, dashboard view, task board view
5. **Inline shortcuts:** Display keyboard shortcuts for every action to enable passive learning

This replaces the need for multiple toolbar buttons, hidden panels, and navigation menus.

---

## 3. Progressive Disclosure in Complex UIs

### The Pattern

Progressive disclosure reveals information in layers: show the essential first, then progressively reveal complexity as the user needs it. The goal is to reduce cognitive load by deferring advanced or rarely-used features to secondary interactions.

### Why It Works (Cognitive Science)

- **Miller's Law:** Working memory holds 7 plus or minus 2 items. Progressive disclosure keeps each layer within this limit.
- **Hick's Law:** Decision time increases logarithmically with the number of choices. Fewer visible options means faster decisions.
- **Cognitive load theory:** Intrinsic load (the task itself) competes with extraneous load (UI complexity). Progressive disclosure minimizes extraneous load.
- **Critical limit:** Research suggests disclosure levels should be kept below three, with clear and intuitive navigation between levels. Too many layers reverses the benefit.

Source: [Progressive Disclosure (NN/g)](https://www.nngroup.com/articles/progressive-disclosure/), [Progressive Disclosure (IxDF)](https://ixdf.org/literature/topics/progressive-disclosure)

### Specific Examples

#### Notion
Notion's block-based interface is a masterclass in progressive disclosure:
- **Default view:** A clean page with minimal chrome. Just content blocks.
- **Hover to reveal:** Hovering over a block reveals a drag handle and `+` button. No persistent clutter.
- **Slash commands:** Typing `/` reveals a searchable menu of block types -- the full power is available but hidden until invoked.
- **Database views:** A database initially shows a simple table. Filters, sorts, grouping, and alternative views (calendar, board, timeline) are behind dropdown menus.
- **Nested pages:** Pages can contain sub-pages, creating infinite hierarchy without showing it all at once.

Key insight: Notion's genius is that every complexity-adding feature is one interaction away but zero interactions visible.

Source: [How Notion Uses Progressive Disclosure (Medium)](https://medium.com/design-bootcamp/how-notion-uses-progressive-disclosure-on-the-notion-ai-page-ae29645dae8d)

#### Linear
Linear achieves complexity management through opinionated defaults:
- **View defaults:** Issues show title, status, priority, and assignee. Additional properties are available but not shown by default.
- **Keyboard grammar:** Simple actions require one key (`C` to create), complex actions use two-key sequences (`G` then `I` for inbox).
- **Progressive filtering:** Views start unfiltered. Users add filters incrementally, each visible as a chip that can be removed.
- **Cycles and projects:** These organizational layers exist but are opt-in. A team can use Linear with just issues and statuses.

Source: [Linear Concepts](https://linear.app/docs/conceptual-model)

#### GitHub
GitHub's Primer design system codifies progressive disclosure:
- **Repository landing page:** Shows README, file tree, and key metrics. Branch switching, settings, actions, and insights are behind tabs.
- **Pull request view:** Shows conversation by default. Files changed, checks, and commits are tabs. Within files changed, diffs are collapsed by file and expandable.
- **Issue detail:** Shows title, body, and comments. Labels, assignees, milestones, and linked PRs are in a sidebar that collapses on narrow screens.

Key insight: GitHub uses tabs and collapsible sidebars as the primary disclosure mechanism. Each tab represents a different depth of detail.

Source: [GitHub Primer Progressive Disclosure](https://primer.style/design/ui-patterns/progressive-disclosure/)

### Mode Switching

Successful tools handle mode switching through:
1. **Tabs** (GitHub, Grafana): Persistent tab bar showing available modes. Current mode is highlighted.
2. **Command palette** (Linear, VS Code): Type the mode name to switch.
3. **View toggles** (Notion databases): Small icon group allowing table/board/calendar/timeline switching.
4. **Activity bar** (VS Code): Icon column where each icon represents a mode.

The key principle: mode switching should be visible but not dominant. Users need to know modes exist without modes competing for attention with the primary content.

### Application to hex-nexus

The hex dashboard should implement three layers of progressive disclosure:

**Layer 1 (Default view):** Chat interface with project health summary badge. Shows: project name, overall health status (green/yellow/red), active agent count, and the chat input.

**Layer 2 (On-demand panels):** Accessible via command palette or tab bar:
- Architecture health details (hex analyze results)
- Active swarms and their task lists
- Agent status and logs
- ADR registry

**Layer 3 (Deep detail):** Accessible from Layer 2 items:
- Individual task details with git diffs
- Agent conversation history
- Dependency graphs
- Performance metrics

Each layer should be reachable from the layer above via a single click or keyboard shortcut, never requiring more than two interactions from the default view.

---

## 4. Chat + Dashboard Hybrid Patterns

### The Challenge

Combining real-time chat with dashboard monitoring is inherently tense. Chat is temporal and conversational (newest at bottom, read sequentially). Dashboards are spatial and at-a-glance (panels arranged by importance, scanned non-linearly). Forcing both into the same view often satisfies neither use case.

### How Successful Tools Handle This

#### Grafana (Dashboard + AI Assistant)
Grafana 12 introduced Grafana Assistant, which carefully separates chat from dashboards:
- **Dashboard is primary:** The monitoring dashboard occupies the full viewport. Panels are arranged in auto-grid layouts that adapt to screen size.
- **Chat is secondary and overlay-based:** The Assistant opens via a left-menu tab or a sparkle icon in the dashboard header. It appears as a side panel or overlay, never replacing the dashboard.
- **Chat generates dashboard artifacts:** The Assistant can create new panels, modify queries, set up alerts, and declare incidents. Chat output becomes dashboard content.
- **Principle:** "Every new feature needs a pretty strong justification because every new feature in almost every case makes it more complicated."

Key insight: The chat does not replace the dashboard -- it augments it. Users monitor via the dashboard and troubleshoot via chat. The two modes serve different cognitive tasks (surveillance vs. investigation).

Source: [Grafana Assistant (Grafana Labs)](https://grafana.com/blog/llm-grafana-assistant/), [Grafana 12 (InfoQ)](https://www.infoq.com/news/2025/05/grafana-12/)

#### Datadog (Monitoring + Collaboration)
Datadog keeps monitoring and communication in separate but linked views:
- **Dashboards** are standalone, shareable, and embeddable.
- **Incidents** have their own timeline view with chat-like updates.
- **Notebooks** combine narrative text with live metric embeds, creating a hybrid document.

Key insight: Datadog uses "notebooks" as the bridge between dashboard and chat. A notebook is neither a dashboard nor a conversation -- it is a structured document with live data. This avoids the dashboard-chat identity crisis.

#### Cursor (Code Editor + AI Chat)
Cursor's approach to mixing a workspace with AI chat is directly relevant:
- **Chat lives in a sidebar panel** (the "Composer"). It can be docked left, right, or as a floating pane.
- **Context pills** at the top of the chat show which files and code sections are in context. Users add context with `#` or `@` mentions.
- **Chat output materializes as code diffs** that can be accepted or rejected inline.
- **Agent mode** (Cursor 2.0+) gives agents their own sidebar section with plans, runs, and diffs as first-class objects.

Key insight: The chat does not try to be the dashboard. The code editor is the primary surface. Chat is a tool for generating edits, and those edits are presented in the editor (the primary surface), not in the chat.

Source: [Cursor Composer](https://docs.cursor.com/composer/overview), [Cursor Features](https://cursor.com/features)

### Successful Hybrid Patterns

| Pattern | Description | Example |
|---------|-------------|---------|
| **Chat as side panel** | Dashboard is primary. Chat overlays or docks to the side. | Grafana Assistant, Cursor Composer |
| **Chat generates artifacts** | Chat output becomes dashboard/editor content (panels, diffs, alerts). | Grafana, Cursor, GitHub Copilot |
| **Notebook bridge** | Structured document mixing narrative text with live data embeds. | Datadog Notebooks, Jupyter |
| **Mode switching** | Dedicated modes for monitoring vs. conversation, switchable via tabs. | Slack (Channels vs. Canvas) |
| **Contextual chat** | Chat scoped to a specific dashboard panel or code file. | Grafana panel-level Assistant |

### Anti-Patterns in Chat+Dashboard Hybrids

1. **Chat-as-homepage:** Making the chat the primary view with dashboard data buried behind navigation. Users who need to monitor at a glance must now scroll through conversation history. (This is the pattern the current hex dashboard appears to be using.)
2. **Dashboard with embedded chat widget:** A tiny chat box crammed into a dashboard corner, too small for meaningful conversation.
3. **Interleaved streams:** Mixing monitoring alerts and conversational messages in the same feed, making both hard to follow.
4. **Full-page mode switching:** Requiring a full page navigation to switch between chat and dashboard, losing context in both directions.

### Application to hex-nexus

The hex dashboard should adopt the **"chat as side panel + dashboard as primary"** pattern:

- **Default view:** A lightweight dashboard showing project health, active swarms, recent tasks, and agent status -- all scannable at a glance.
- **Chat panel:** Dockable to the right side (or bottom). Always accessible via a keyboard shortcut or icon but not occupying the primary viewport.
- **Chat generates artifacts:** Chat commands (e.g., "create a new swarm for feature-auth") should materialize as dashboard items (a new swarm card appears on the dashboard).
- **Contextual scoping:** Opening chat from a specific swarm or task card should pre-scope the chat to that context.

---

## 5. Project/Session Management in AI Tools

### The Challenge

AI coding tools must manage multiple dimensions of state: which project/codebase, which conversation session, which files are in context, and what is the agent's current plan. Users need to switch between these dimensions fluidly.

### How Successful Tools Handle This

#### Cursor
- **Project = VS Code workspace:** Each window is a project. No separate "project" concept needed.
- **Sessions = Composer conversations:** Each Composer chat is a session with its own context and history.
- **Context management:** "Pills" at the top of chat show active files. Add via `#` to reference files, `@` to reference symbols. Auto-context uses embeddings to include relevant code.
- **Agent plans:** In agent mode, plans and runs are first-class sidebar objects showing steps, status, and diffs.

Source: [Cursor Composer Overview](https://docs.cursor.com/composer/overview)

#### Windsurf (Cascade System)
- **Session memory:** Windsurf's Cascade remembers context across the session -- related code changes, file modifications, and architectural decisions.
- **Architectural awareness:** Instead of token-limited context, Cascade builds a structured understanding of the codebase.
- **Continuity:** Session state persists, making it possible to resume work after interruptions.

Source: [AI Code Editors Showdown 2025](https://www.codeant.ai/blogs/best-ai-code-editor-cursor-vs-windsurf-vs-copilot)

#### Claude Code (Terminal-Based)
- **Session = terminal session:** Each terminal invocation is a session.
- **Project context:** Uses CLAUDE.md files for persistent project instructions.
- **Memory:** Explicit memory storage via files (not embedded in the tool).
- **No visual project switching:** Relies on filesystem navigation (cd).

#### GitHub Copilot Workspace
- **Task-oriented sessions:** Each "task" is a session anchored to a GitHub issue.
- **Plan-edit-review loop:** Shows a plan, generates edits across files, and presents them for review.
- **Session persistence:** Tasks can be revisited and continued.

### Key UX Patterns for Session Management

| Pattern | Description | Used By |
|---------|-------------|---------|
| **Workspace = Project** | One window per project, no explicit project concept | VS Code, Cursor |
| **Session list sidebar** | Left sidebar showing past and current sessions | ChatGPT, Claude.ai |
| **Context pills** | Visual indicators of what files/data are in scope | Cursor, Windsurf |
| **Persistent memory** | Project-level instructions that survive across sessions | Claude Code (CLAUDE.md), Cursor (.cursorrules) |
| **Task-anchored sessions** | Sessions tied to issues/tickets, not just conversations | GitHub Copilot Workspace |
| **Session branching** | Fork a session to explore alternatives | ChatGPT (upcoming) |

### Session State Indicators

Users need to see at a glance:
1. **Which project** they are in (name + health badge)
2. **Which session** is active (title or auto-generated summary)
3. **What context** is loaded (file list, agent assignments)
4. **What is happening** (agent status: idle, working, waiting for input, error)
5. **What happened** (recent actions: files modified, tests run, tasks completed)

### Application to hex-nexus

The hex dashboard should implement:
- **Project switcher** in the left column header (dropdown or icon strip for registered projects)
- **Session list** below the project switcher showing active and recent swarm sessions
- **Context indicator** at the top of the chat panel showing: active project, loaded files, active agents
- **Status badges** on each session: active (green pulse), paused (yellow), completed (gray), failed (red)
- **Session continuity** via hex memory: resuming a session restores its swarm state, task list, and chat history

---

## 6. Mobile-Responsive Patterns for Dev Dashboards

### The Challenge

Developer dashboards are information-dense tools primarily used on large screens. Mobile responsiveness is secondary but important for monitoring on-the-go and receiving notifications.

### Responsive Collapse Strategies

#### Strategy 1: Column Stacking
Three columns collapse to a single scrollable column on mobile:
- Left sidebar becomes a hamburger menu (top-left) or bottom tab bar
- Center content occupies full width
- Right panel becomes a swipe-up sheet or secondary screen

#### Strategy 2: Bottom Tab Navigation
- Desktop sidebar navigation converts to a fixed bottom tab bar on mobile
- 4-5 primary destinations (Home, Chat, Tasks, Agents, Settings)
- Active tab shows its content full-screen

Source: [Responsive Navigation Patterns (MDN)](https://developer.mozilla.org/en-US/docs/Web/Progressive_web_apps/Responsive/Responsive_navigation_patterns)

#### Strategy 3: Adaptive Components
Components change form factor based on available space:
- Data tables become stacked cards
- Multi-panel dashboards become swipeable card carousels
- Charts simplify (fewer data points, larger touch targets)

Source: [Responsive Design Trends 2025 (BootstrapDash)](https://www.bootstrapdash.com/blog/9-responsive-design-trends-in-dashboard-templates)

### Modern CSS Techniques (2025-2026)

- **Container queries** (93.92% browser support as of late 2025): Components respond to their container size, not the viewport. This allows panels to adapt independently.
- **CSS Grid with auto-fit/auto-fill:** Dashboard panels reflow naturally as viewport shrinks.
- **`clamp()` for fluid typography and spacing:** Smooth scaling without breakpoint jumps.

Source: [Responsive Web Design Techniques (Lovable)](https://lovable.dev/guides/responsive-web-design-techniques-that-work)

### Mobile-Specific UX Patterns

| Pattern | Desktop | Mobile |
|---------|---------|--------|
| Navigation | Persistent sidebar | Bottom tab bar or hamburger |
| Content | Multi-panel grid | Single-column stack |
| Details | Side panel | Full-screen sheet or modal |
| Actions | Toolbar buttons | FAB (floating action button) or bottom sheet |
| Data tables | Full table | Stacked cards |
| Charts | Full-width with hover tooltips | Simplified with tap-to-inspect |

### Application to hex-nexus

The hex dashboard should define three breakpoints:

**Desktop (>1200px):** Full three-column layout. Left sidebar (240px) + center content (flexible) + right panel (320px, collapsible).

**Tablet (768-1200px):** Two-column layout. Left sidebar collapses to icon-only (48px). Right panel becomes an overlay sheet triggered by item selection.

**Mobile (<768px):** Single-column layout. Bottom tab bar for navigation (Chat, Tasks, Agents, Health). Content is full-width. Details open as full-screen sheets with a back gesture.

---

## 7. Anti-Patterns to Avoid

Based on research into dashboard design failures and observability tool UX mistakes, these are patterns the hex dashboard must avoid:

### 7.1 Information Overload
**Problem:** Displaying all metrics, agents, tasks, logs, and health data on a single screen.
**Why it fails:** When everything appears equally important, nothing is important. Users cannot find critical information.
**Solution:** Prioritize 3-5 key metrics on the default view. Use progressive disclosure for everything else.

Source: [Bad Dashboard Examples (Databox)](https://databox.com/bad-dashboard-examples)

### 7.2 Wrong Visualization Types
**Problem:** Using complex charts (stacked bar, radar) for simple data, or simple charts (pie) for complex data.
**Why it fails:** Mismatched visualizations force users to decode the chart before understanding the data.
**Solution:** Use the simplest visualization that conveys the data accurately. Status badges for binary state. Sparklines for trends. Numbers for counts.

### 7.3 Lack of Context
**Problem:** Showing numbers without baselines, comparisons, or explanations.
**Why it fails:** "5 agents active" means nothing without knowing "out of 5 total" or "down from 8 yesterday."
**Solution:** Always show context: ratios (5/5), trends (arrows up/down), and annotations (why something changed).

Source: [Top 10 Mistakes in Observability Dashboards (Logz.io)](https://logz.io/blog/top-10-mistakes-building-observability-dashboards/)

### 7.4 Chat-as-Homepage
**Problem:** Making the AI chat the landing page, with dashboard data accessible only through conversation or hidden navigation.
**Why it fails:** Monitoring requires at-a-glance scanning, which chat interfaces are fundamentally bad at. Chat is temporal; dashboards are spatial.
**Solution:** Dashboard as primary view. Chat as a side panel or secondary mode.

### 7.5 Feature Creep Without Justification
**Problem:** Adding panels, tabs, and features because they are technically possible.
**Why it fails:** Each addition increases cognitive load. Grafana's principle: "Every new feature needs a pretty strong justification."
**Solution:** For each proposed feature, ask: "What user task does this enable that cannot be accomplished another way?" If the answer is unclear, do not add it.

### 7.6 Excessive Real-Time Updates
**Problem:** Animating every metric change, auto-refreshing all panels, streaming all logs.
**Why it fails:** Constant motion distracts from focused work. Users cannot read data that keeps changing.
**Solution:** Show real-time updates only for actively-monitored items. Use "last updated" timestamps for others. Let users opt into live streaming per-panel.

Source: [Dashboard Design Principles (UXPin)](https://www.uxpin.com/studio/blog/dashboard-design-principles/)

### 7.7 No Keyboard Navigation
**Problem:** Requiring mouse interaction for all actions in a developer tool.
**Why it fails:** Developers live in keyboards. Forcing mouse interaction breaks flow.
**Solution:** Every action should have a keyboard shortcut. The command palette (Cmd+K) should be the universal escape hatch.

---

## 8. Recommendations for hex-nexus

### Priority 1: Layout Architecture

Adopt a three-column layout with the following zones:

```
+--------+------------------------+------------------+
| Left   | Center                 | Right            |
| 240px  | flexible               | 320px            |
| (icon  | (main content)         | (context panel)  |
|  mode: |                        | (collapsible)    |
|  48px) |                        |                  |
+--------+------------------------+------------------+
| Projects | Dashboard / Chat /   | Details for      |
| Sessions | Task Board (mode     | selected item:   |
| Agents   | switched via tabs    | agent log, task  |
| Swarms   | or Cmd+K)            | detail, health   |
|          |                      | report, diff     |
+--------+------------------------+------------------+
```

### Priority 2: Command Palette (Cmd+K)

Implement a command palette as the primary action dispatch mechanism:
- **Trigger:** Cmd+K (Mac), Ctrl+K (Windows)
- **Categories:** Navigation, Actions, Search, Mode Switching
- **Features:** Fuzzy search, inline shortcut display, recent commands, context-aware results
- **Replaces:** Hidden toolbar buttons, navigation menus, modal dialogs

### Priority 3: Dashboard-First, Chat-Second

- Default landing view is a dashboard showing project health at a glance
- Chat is a dockable side panel (right side), not the primary view
- Chat can generate dashboard artifacts (swarms, tasks, analyses)
- Chat context is scoped to the currently selected project/swarm

### Priority 4: Progressive Disclosure

- **Layer 1:** Health badge + key metrics (3-5 items) + chat input
- **Layer 2:** Expanded panels (swarm list, task board, agent grid) via tabs or Cmd+K
- **Layer 3:** Deep detail (individual agent logs, task diffs, dependency graphs) via item selection

### Priority 5: Responsive Design

- Use CSS Grid with container queries for panel layouts
- Three breakpoints: desktop (3-col), tablet (2-col with icon sidebar), mobile (1-col with bottom tabs)
- Data tables become cards on mobile
- Charts simplify (fewer data points, larger targets)

### Priority 6: Session and Project Management

- Project switcher in left sidebar header
- Session list with status badges (active/paused/completed/failed)
- Context pills in chat showing loaded files and active agents
- Session continuity via hex memory (restore state on resume)

---

## Sources

### Three-Column Layout
- [VS Code Custom Layout](https://code.visualstudio.com/docs/configure/custom-layout)
- [VS Code UX Guidelines](https://code.visualstudio.com/api/ux-guidelines/overview)
- [VS Code Activity Bar](https://code.visualstudio.com/api/ux-guidelines/activity-bar)
- [VS Code Secondary Sidebar (GitHub Issue)](https://github.com/microsoft/vscode/issues/132893)
- [Discord UI Architecture](https://www.researchgate.net/figure/The-Discord-user-interface-The-far-left-sidebar-lists-all-the-Discord-servers-the-user_fig1_337131371)
- [Slack Sidebar Preferences](https://slack.com/help/articles/212596808-Adjust-your-sidebar-preferences)
- [Slack Custom Sections](https://slack.com/help/articles/360043207674-Organize-your-sidebar-with-custom-sections)

### Command Palette
- [How to Build a Remarkable Command Palette (Superhuman)](https://blog.superhuman.com/how-to-build-a-remarkable-command-palette/)
- [Command Palette UX Patterns (Medium)](https://medium.com/design-bootcamp/command-palette-ux-patterns-1-d6b6e68f30c1)
- [Command Palette Interfaces (Philip Davis)](https://philipcdavis.com/writing/command-palette-interfaces)
- [Designing Retool's Command Palette](https://retool.com/blog/designing-the-command-palette)
- [Command Palette: Past, Present, Future](https://www.command.ai/blog/command-palette-past-present-and-future/)
- [Command Palette UI Design (Mobbin)](https://mobbin.com/glossary/command-palette)
- [Linear Keyboard Shortcuts](https://shortcuts.design/tools/toolspage-linear/)
- [Raycast on Windows](https://windowsforum.com/threads/raycast-on-windows-a-keyboard-first-command-palette-for-fast-actions.395552/)

### Progressive Disclosure
- [Progressive Disclosure (NN/g)](https://www.nngroup.com/articles/progressive-disclosure/)
- [Progressive Disclosure (IxDF)](https://ixdf.org/literature/topics/progressive-disclosure)
- [Progressive Disclosure (Primer / GitHub)](https://primer.style/design/ui-patterns/progressive-disclosure/)
- [How Notion Uses Progressive Disclosure](https://medium.com/design-bootcamp/how-notion-uses-progressive-disclosure-on-the-notion-ai-page-ae29645dae8d)
- [Progressive Disclosure in SaaS UX](https://lollypop.design/blog/2025/may/progressive-disclosure/)
- [Progressive Disclosure (Decision Lab)](https://thedecisionlab.com/reference-guide/design/progressive-disclosure)

### Chat + Dashboard Hybrids
- [Grafana Assistant](https://grafana.com/blog/llm-grafana-assistant/)
- [Grafana 12 (InfoQ)](https://www.infoq.com/news/2025/05/grafana-12/)
- [Grafana AI for Observability](https://grafana.com/products/cloud/ai-tools-for-observability/)
- [Cursor Composer Overview](https://docs.cursor.com/composer/overview)
- [Cursor Features](https://cursor.com/features)
- [Datadog vs Grafana (SigNoz)](https://signoz.io/blog/datadog-vs-grafana/)

### AI Tool Session Management
- [AI Code Editors Showdown 2025](https://www.codeant.ai/blogs/best-ai-code-editor-cursor-vs-windsurf-vs-copilot)
- [AI Coding Agents 2025 (Kingy AI)](https://kingy.ai/blog/ai-coding-agents-in-2025-cursor-vs-windsurf-vs-copilot-vs-claude-vs-vs-code-ai/)
- [Cursor AI Review (Prismic)](https://prismic.io/blog/cursor-ai)
- [Linear Concepts](https://linear.app/docs/conceptual-model)

### Responsive Dashboard Design
- [Responsive Design Trends 2025 (BootstrapDash)](https://www.bootstrapdash.com/blog/9-responsive-design-trends-in-dashboard-templates)
- [Mobile Dashboard UI (Toptal)](https://www.toptal.com/designers/dashboard-design/mobile-dashboard-ui)
- [Responsive Web Design Techniques (Lovable)](https://lovable.dev/guides/responsive-web-design-techniques-that-work)
- [Responsive Navigation Patterns (MDN)](https://developer.mozilla.org/en-US/docs/Web/Progressive_web_apps/Responsive/Responsive_navigation_patterns)

### Anti-Patterns
- [Bad Dashboard Examples (Databox)](https://databox.com/bad-dashboard-examples)
- [Top 10 Mistakes in Observability Dashboards (Logz.io)](https://logz.io/blog/top-10-mistakes-building-observability-dashboards/)
- [Dashboard Design Principles (UXPin)](https://www.uxpin.com/studio/blog/dashboard-design-principles/)
- [Dashboard UX Patterns (Pencil & Paper)](https://www.pencilandpaper.io/articles/ux-pattern-analysis-data-dashboards)
- [Dashboard Design Dos and Don'ts (Design Monks)](https://www.designmonks.co/blog/dashboard-design-dos-and-donts)
