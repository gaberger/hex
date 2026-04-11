# UX Research: Competitor Analysis of AI Developer Tools

> Date: 2026-03-20
> Purpose: Identify common UX patterns across successful AI coding tools to inform hex-nexus dashboard and CLI design.

---

## Table of Contents

1. [Tool-by-Tool Analysis](#tool-by-tool-analysis)
2. [Cross-Tool Pattern Synthesis](#cross-tool-pattern-synthesis)
3. [The Winning Formula](#the-winning-formula)
4. [Recommendations for hex](#recommendations-for-hex)

---

## Tool-by-Tool Analysis

### 1. OpenCode / Crush (Terminal TUI)

**Status:** OpenCode archived Sept 2025; continued as [Crush](https://github.com/charmbracelet/crush) by Charmbracelet.

**Layout Structure:**
- Three-panel split-pane TUI built with Bubble Tea (Go):
  - **Left panel (Messages):** Main conversation display, 1px padding on all sides
  - **Bottom panel (Editor):** Text input area with top border separator; supports up to 5 file attachments
  - **Right panel (Sidebar, optional):** Session info, shown only when a session exists
- File completion overlay: triggered by `@` in the editor, positioned above the input area
- Status bar at the bottom showing model, token count, mode

**Project/Session Management:**
- SQLite-backed persistent sessions
- Session list accessible via keyboard shortcut (`<leader>+s`)
- Sessions store full conversation history, file context, and tool outputs
- Auto-compact feature: when token usage hits 95% of context window, auto-summarizes and creates a new session
- Sub-agent sessions with parent/child navigation (`<leader>+up/down`)

**Chat + Tools Coexistence:**
- Two primary modes toggled with `Tab`:
  - **Plan mode:** AI suggests implementation strategy without making changes
  - **Build mode:** AI actively edits files, runs commands, uses tools
- Tool outputs (file diffs, command results) stream inline within the chat
- Vim-like keybindings with customizable leader key (default: `Ctrl+X`)
- External editor support (`Ctrl+X, Ctrl+E`) for composing longer messages

**What Makes It Intuitive:**
- Modal separation (Plan vs Build) prevents accidental file changes
- Leader-key system avoids terminal keybinding conflicts
- Inline tool output keeps context visible without panel switching
- Auto-compact prevents context window exhaustion transparently

**Key UX Patterns:**
- **Modal chat** (plan/execute split)
- **Inline tool results** (no separate output pane)
- **Session persistence** with auto-summarization
- **`@`-mention file picker** for context attachment

---

### 2. Claude Code (Anthropic CLI)

**Layout Structure:**
- Single-stream terminal interface (no panels/splits in base CLI)
- Streaming output with inline tool-use indicators
- File diffs shown inline with syntax highlighting
- Permission prompts interrupt the stream for tool approval
- VS Code extension (beta) adds: sidebar panel with inline diffs, real-time change visualization

**Project/Session Management:**
- Project-scoped via `CLAUDE.md` files (instructions persist per-project)
- Session memory via `~/.claude/projects/<project>/memory/MEMORY.md`
- Conversation history stored locally; resumable sessions
- No explicit "project" abstraction -- the working directory IS the project

**Chat + Tools Coexistence:**
- Tools fire inline within the conversation stream
- Permission model: tools require approval unless running in `--dangerously-skip-permissions` or via VS Code extension's `bypassPermissions`
- Compact output: tool calls show name + result summary, expandable
- Searchable prompt history (`Ctrl+R`)
- Multi-turn context: reads files, greps, globs, edits -- all within the same chat flow

**What Makes It Intuitive:**
- Zero configuration to start (just `claude` in any directory)
- Permission prompts give users control without breaking flow
- `CLAUDE.md` convention makes project context declarative and versionable
- Inline diffs are immediately visible -- no separate review step

**Key UX Patterns:**
- **Zero-config project detection** (cwd = project)
- **Inline permission gates** for tool use
- **Declarative project context** (`CLAUDE.md`)
- **Stream-first output** with collapsible tool details

---

### 3. Cursor (AI Code Editor)

**Layout Structure:**
- VS Code fork with fixed three-panel layout:
  - **Left sidebar:** File explorer, search, git, extensions
  - **Center:** Code editor (tabs, split panes)
  - **Right sidebar:** AI chat panel (persistent)
- Chat panel supports: conversation threads, inline code blocks, file references
- "Composer" agent operates across files with a dedicated multi-file diff view
- Visual Editor (Dec 2025): DOM manipulation overlay for web projects

**Project/Session Management:**
- VS Code workspace model: `.cursor/` directory for settings
- Chat history persists per workspace
- Agent threads are scoped to the current workspace
- Multi-agent judging (v2.2): multiple agents propose, best answer wins

**Chat + Tools Coexistence:**
- Chat panel is always visible alongside the editor
- `Cmd+K` for inline edits (highlight code, describe change)
- `Cmd+L` to open/focus chat panel
- Tab completion for AI suggestions in the editor
- Agent mode: AI reads files, runs terminal commands, applies multi-file edits
- Changes appear as inline diffs in the editor with accept/reject controls
- MCP Apps: interactive UIs (charts, diagrams) render inside chat

**What Makes It Intuitive:**
- Editor IS the workspace -- no context switching
- Chat lives alongside code, not in a separate window
- Inline edit (`Cmd+K`) is the lowest-friction AI interaction
- Multi-file diffs group related changes for atomic review

**Key UX Patterns:**
- **Persistent side chat** alongside editor
- **Inline edit trigger** (`Cmd+K`) for micro-interactions
- **Multi-file diff view** for atomic change review
- **Agent mode** with full codebase access
- **MCP integration** for rich tool UIs inside chat

---

### 4. Warp (Agentic Terminal)

**Layout Structure:**
- Warp 2.0 has four integrated capabilities: **Code, Agents, Terminal, Drive**
- Block-based output: each command+output is a discrete, navigable "block"
- Universal input bar: accepts both natural language AND shell commands
- Agent Management Panel: unified dashboard for all active agents (local + cloud)
- Blocks support: search within, copy, share, filter

**Project/Session Management:**
- Drive: shared workspace for team files, scripts, workflows
- Agent sessions persist independently
- Oz (cloud agents): unlimited parallel agents, each with its own session
- Notifications when agents complete or need help

**Chat + Tools Coexistence:**
- Universal input: no mode switch between chat and commands
- `@` syntax for attaching files, images, URLs to prompts
- Agents have configurable permissions: auto-accept diffs, read files, run commands
- Allowlist/denylist for agent-executable commands
- Agent output streams as blocks, interleaved with manual terminal usage

**What Makes It Intuitive:**
- Universal input eliminates the "am I in chat or terminal?" question
- Block-based output makes long sessions scannable and navigable
- Agent permissions are explicit and granular
- Parallel agents visible in management dashboard

**Key UX Patterns:**
- **Universal input** (chat + commands in one bar)
- **Block-based output** for structured navigation
- **Agent management dashboard** for multi-agent visibility
- **Granular permission controls** per agent
- **`@`-mention context attachment**

---

### 5. Zed (Collaborative Editor with AI)

**Layout Structure:**
- Three configurable pane areas (left, center, right)
- Agent Panel: dedicated panel (typically right side) for AI interaction
- Two thread types:
  - **Agent threads:** full tool access, file editing, terminal commands
  - **Text threads:** pure chat, larger text, no tool use
- Multibuffer Review tab: editable diff view with multicursor support
- "Agent following": editor follows the agent's file changes in real-time at 120fps

**Project/Session Management:**
- Project = directory opened in Zed
- Agent threads persist per project
- Multiple concurrent agent threads possible
- Model selection per thread via dropdown

**Chat + Tools Coexistence:**
- Agent panel shows: conversation, tool-call indicators (expandable accordion), results
- "Review Changes" button opens multibuffer diff tab
- Individual change hunks can be accepted/rejected independently
- Extensions can add new tool capabilities to the agent
- Terminal commands require explicit permission

**What Makes It Intuitive:**
- Agent following lets you watch edits happen in real-time
- Multibuffer review is a first-class editing experience (not a read-only diff)
- Thread type separation prevents confusion about what the AI can/cannot do
- 120fps rendering makes agent work feel instantaneous

**Key UX Patterns:**
- **Editable multibuffer diff** for change review
- **Agent following** (real-time file change tracking)
- **Thread type separation** (agent vs text)
- **Extension-based tool capabilities**
- **Per-hunk accept/reject** in review

---

### 6. v0.dev (Vercel AI UI Builder)

**Layout Structure:**
- Chat-first layout:
  - **Left panel:** Chat interface with conversation history
  - **Right panel:** Live preview of generated UI (runs full production environment)
  - **Code tab:** VS Code-style editor built into the interface
  - **Vars panel:** Environment variable management in sidebar
- Toggle between Preview and Code views

**Project/Session Management:**
- **Projects:** Connect to actual Vercel deployments (env vars, domains, deployments)
- **Folders:** Organizational grouping for chats (no deployment connection)
- Multiple chats can contribute to the same Project
- Chat history persists and is shareable (Team plan)
- "Blocks" = reusable code/UI snippets

**Chat + Tools Coexistence:**
- Chat generates code; preview updates in real-time
- Iterative: describe changes in chat, see results immediately in preview
- Supports: text prompts, screenshot uploads, design tool exports
- Code tab shows all generated files with syntax highlighting
- Previews run full server-side code, API routes, database connections

**What Makes It Intuitive:**
- Immediate visual feedback (chat prompt -> live preview)
- Production-accurate previews eliminate "works in dev, breaks in prod"
- Project abstraction connects chat work to real deployments
- Iterative refinement through conversational UI

**Key UX Patterns:**
- **Chat-to-preview pipeline** (prompt -> instant visual result)
- **Production-accurate previews**
- **Project = deployment** connection
- **Iterative refinement** through conversation
- **Multi-chat project contribution**

---

### 7. Bolt.new / StackBlitz (AI Full-Stack Dev)

**Layout Structure:**
- Four-panel layout in browser:
  - **Left:** Chat panel (AI conversation, shows "work steps")
  - **Center-left:** File tree
  - **Center:** Code editor (CodeMirror-based)
  - **Bottom:** Terminal (watches file changes for live reload)
  - **Right:** Live preview (browser-in-browser via WebContainers)
- Toggle between Preview and Code views
- DiffView component for reviewing AI-generated changes

**Project/Session Management:**
- Projects are browser-local (WebContainers = in-browser Node.js)
- Chat history per project
- One-click deploy to various hosting platforms
- File management: create, edit, delete through file tree or chat

**Chat + Tools Coexistence:**
- Chat drives everything: AI has full control over filesystem, terminal, package manager
- Work steps shown in left panel as AI executes
- Code changes visible in real-time in the editor
- Terminal output streams during AI operations
- Preview auto-refreshes as changes are applied

**What Makes It Intuitive:**
- Everything in one browser tab -- no local setup required
- AI controls the full environment, reducing manual steps
- Work steps provide transparency into AI actions
- Live preview closes the feedback loop immediately

**Key UX Patterns:**
- **All-in-one browser workspace** (chat + editor + terminal + preview)
- **AI-controlled full environment** (filesystem, terminal, packages)
- **Work step transparency** (visible action log)
- **Zero-install** browser-based development
- **Live preview** with auto-refresh

---

### 8. Aider (Terminal AI Pair Programmer)

**Layout Structure:**
- Single-stream terminal interface (no panels)
- Prompt-toolkit-based input with emacs/vi keybindings
- Color-coded output: file edits in colored diffs, AI responses in distinct style
- No TUI framework -- pure streaming text with rich formatting

**Project/Session Management:**
- Project = git repository (must be in a git repo)
- Files added to "chat context" explicitly via `/add` command
- `/tokens` shows current context usage
- Git integration: every AI change = automatic commit with descriptive message
- Session state: files in chat, conversation history, undo via `git revert`

**Chat + Tools Coexistence:**
- Four chat modes switchable via `/mode`:
  - **code** (default): AI edits files directly
  - **architect**: planning and design discussion, no file changes
  - **ask**: questions about codebase, no changes
  - **help**: tool usage help
- `/diff` shows all changes since last user message
- `/undo` reverts the last AI commit
- Voice input support for hands-free coding
- Browser GUI alternative maintains same capabilities

**What Makes It Intuitive:**
- Git-native workflow: every change is a commit, undo = revert
- Explicit context management: you control exactly what the AI sees
- Modal separation prevents accidental changes
- Minimal interface reduces cognitive overhead

**Key UX Patterns:**
- **Git-native change management** (auto-commit, easy undo)
- **Explicit context control** (`/add`, `/drop`, `/tokens`)
- **Chat modes** (code/architect/ask/help)
- **Color-coded output** for visual parsing
- **Minimal interface** -- terminal text only

---

### 9. Continue.dev (VS Code AI Assistant)

**Layout Structure:**
- VS Code sidebar extension:
  - **Sidebar panel:** Chat interface with conversation history
  - **Inline completions:** Tab-to-accept ghost text in editor
  - **Inline edits:** Select code -> describe change -> AI applies
- `@`-mention context providers in chat: `@Files`, `@Docs`, `@Codebase`, `@Web`
- Model selector in sidebar for switching between providers

**Project/Session Management:**
- Workspace-scoped via VS Code
- Configuration via `config.yaml` (models, context providers, slash commands)
- Chat history persists per workspace
- Codebase indexing for semantic search
- MCP server connections for external tool access

**Chat + Tools Coexistence:**
- Three interaction modes:
  - **Chat:** Sidebar conversation with context providers
  - **Autocomplete:** Inline ghost-text suggestions (FIM-based)
  - **Edit:** Select code, describe intent, AI applies changes
- `/` slash commands for custom workflows
- `@` context providers for attaching relevant information
- Multi-model support: different models for chat vs autocomplete vs edit

**What Makes It Intuitive:**
- Familiar VS Code extension pattern -- no new app to learn
- `@`-mentions provide discoverable context attachment
- Three distinct interaction modes for different needs
- Open-source and fully configurable

**Key UX Patterns:**
- **IDE sidebar integration** (no separate window)
- **`@`-mention context providers** for discoverable context
- **Three-mode interaction** (chat / autocomplete / edit)
- **Multi-model configuration** (right model for each task)
- **Slash commands** for custom workflows

---

## Cross-Tool Pattern Synthesis

### Universal Patterns (Present in 7+ tools)

| Pattern | Tools | Description |
|---------|-------|-------------|
| **Chat as primary interface** | All 9 | Natural language is the entry point for all interactions |
| **Inline/streaming results** | All 9 | Results appear progressively, not after completion |
| **File context attachment** | All 9 | `@`-mentions, `/add`, or file picker for adding context |
| **Project = directory** | 8/9 | Working directory defines project scope (exception: v0 uses deployment-linked projects) |
| **Persistent sessions** | 8/9 | Conversation history survives across restarts |
| **Permission/approval gates** | 7/9 | User must approve destructive actions (file edits, commands) |

### Strong Patterns (Present in 5-6 tools)

| Pattern | Tools | Description |
|---------|-------|-------------|
| **Chat modes (plan/code/ask)** | OpenCode, Aider, Claude Code, Cursor, Continue | Separate modes for thinking vs doing |
| **Side-by-side layout** | Cursor, Zed, v0, Bolt, Warp | Chat alongside workspace (editor/preview) |
| **Diff-based change review** | Cursor, Zed, Bolt, Claude Code, Aider | Changes shown as diffs with accept/reject |
| **Multi-model support** | OpenCode, Aider, Continue, Warp, Zed | Switch models within or across sessions |
| **Git-native workflow** | Aider, Claude Code, OpenCode, Cursor | Auto-commit, easy undo via git |

### Emerging Patterns (Present in 3-4 tools)

| Pattern | Tools | Description |
|---------|-------|-------------|
| **Universal input bar** | Warp, OpenCode, Claude Code | Same input for chat AND commands |
| **Agent management dashboard** | Warp, Cursor, Zed | Visibility into multiple concurrent agents |
| **Live preview** | v0, Bolt, Cursor | Real-time visual output of changes |
| **Editable diffs** | Zed, Cursor | Review diffs as editable code, not read-only |
| **Block-based output** | Warp, OpenCode | Discrete, navigable output blocks |
| **MCP integration** | Cursor, Continue, OpenCode | Extensible tool capabilities via protocol |

---

## The Winning Formula

Based on this analysis, the most successful AI developer tools share a core formula:

### 1. Chat-First, Tools-Second

Every tool puts natural language input at the center. The chat IS the command interface. Tools, file edits, terminal commands, and previews are outputs of chat, not separate workflows.

**Implementation:** Single input bar that accepts both natural language and slash commands. Results stream inline.

### 2. Modal Separation (Think vs Do)

The most productive tools separate planning from execution:
- **Plan/Architect mode:** Discuss, design, explore -- no side effects
- **Code/Build mode:** Execute changes with full tool access
- **Ask mode:** Quick questions without any file context changes

**Implementation:** Mode indicator in status bar, toggle with single keypress (Tab or /mode).

### 3. Context is King (and Explicit)

Users must control what the AI sees:
- `@`-mention for files, docs, URLs
- `/add` and `/drop` for session context
- Token counter showing context usage
- Auto-compact when approaching limits

**Implementation:** `@`-picker overlay, `/tokens` command, context indicator in status bar.

### 4. Inline Everything

The most productive tools avoid context-switching:
- Tool results inline in chat (not separate panes)
- Diffs inline with accept/reject
- Permissions inline (not modal dialogs)
- Preview alongside editor (not separate window)

**Implementation:** Streaming output with collapsible tool-call sections.

### 5. Git-Native Change Management

AI changes should be first-class git citizens:
- Auto-commit with descriptive messages
- Easy undo (`/undo` = `git revert`)
- Diff review before accepting
- Change attribution (human vs AI)

**Implementation:** Every accepted change = git commit. `/undo` reverts last AI commit.

### 6. Progressive Disclosure

Reduce cognitive load by hiding complexity:
- Simple chat by default
- Tool calls collapsible/expandable
- Advanced features via slash commands
- Settings via config file, not UI

**Implementation:** Accordion-style tool output. Power features behind `/` commands.

### 7. Zero-Config Start, Deep Customization Later

- Start with `tool <directory>` -- no setup required
- Project context via convention (`CLAUDE.md`, `.cursor/`, `.opencode.json`)
- Deep customization via config files for power users
- MCP for extensibility

**Implementation:** Sensible defaults. Config file for models, keybindings, tools.

---

## Recommendations for hex

Based on the patterns above, here are specific recommendations for the hex-nexus dashboard and hex CLI:

### For the Terminal TUI (OpenCode-like experience)

1. **Three-panel layout:** Messages (left/center), Editor (bottom), Sidebar (right, collapsible)
2. **Plan/Build mode toggle** with `Tab` key
3. **`@`-mention file picker** with fuzzy search
4. **Leader-key system** (avoid terminal conflicts)
5. **Status bar:** model name, token count, mode indicator, session name
6. **Auto-compact sessions** at 95% context usage
7. **Block-based output** for tool results (collapsible)
8. **Inline diffs** with accept/reject per hunk

### For the Web Dashboard (hex-nexus)

1. **Chat panel on the left** (consistent with v0, Bolt pattern)
2. **Workspace on the right** with tabs: Preview, Code, Terminal, Diff
3. **Agent management panel** showing all active swarm agents
4. **Work step log** showing what agents are doing (Bolt pattern)
5. **File tree** in collapsible left sidebar
6. **Session list** in dropdown or sidebar

### For the CLI (hex commands)

1. **Universal input:** `hex chat` should accept both prompts and slash commands
2. **Git-native:** Auto-commit AI changes with `hex` prefix in messages
3. **Context commands:** `hex context add/drop/show` for managing what the AI sees
4. **Mode switching:** `hex plan` vs `hex build` vs `hex ask`
5. **MCP-first extensibility:** All hex capabilities exposed as MCP tools

### Priority Order

| Priority | Feature | Rationale |
|----------|---------|-----------|
| P0 | Chat-first interface with streaming | Table stakes -- every competitor has this |
| P0 | Plan/Build mode separation | Prevents accidental changes, builds trust |
| P0 | Inline diffs with accept/reject | Core review workflow |
| P1 | `@`-mention context picker | Reduces friction for context management |
| P1 | Session persistence + auto-compact | Essential for long-running tasks |
| P1 | Git-native change management | Differentiator for serious developers |
| P2 | Agent management dashboard | Required for swarm workflows |
| P2 | Block-based output | Improves scanability of long sessions |
| P2 | Multi-model support | Flexibility for different tasks |
| P3 | Live preview | Only needed for web-focused projects |
| P3 | MCP extensibility UI | Power user feature |

---

## Sources

### OpenCode / Crush
- [OpenCode GitHub](https://github.com/opencode-ai/opencode)
- [OpenCode TUI Docs](https://opencode.ai/docs/tui/)
- [OpenCode Keybinds](https://opencode.ai/docs/keybinds/)
- [Crush GitHub](https://github.com/charmbracelet/crush)
- [Crush TUI Architecture (DeepWiki)](https://deepwiki.com/charmbracelet/crush/5.1-tui-architecture)
- [OpenCode Chat Components (DeepWiki)](https://deepwiki.com/opencode-ai/opencode/5.1-chat-components)
- [Crush Review (The New Stack)](https://thenewstack.io/terminal-user-interfaces-review-of-crush-ex-opencode-al/)

### Claude Code
- [Claude Code GitHub](https://github.com/anthropics/claude-code)
- [Claude Code CLI Reference](https://code.claude.com/docs/en/cli-reference)
- [Claude Code Product Page](https://claude.com/product/claude-code)
- [How Claude Code Made Me Fall in Love with the Terminal](https://www.hadijaveed.me/2025/08/04/terminal-is-all-we-need/)
- [Enabling Claude Code Autonomy (Anthropic)](https://www.anthropic.com/news/enabling-claude-code-to-work-more-autonomously)

### Cursor
- [Cursor Features](https://cursor.com/features)
- [Cursor 2.0 Guide](https://skywork.ai/blog/vibecoding/cursor-2-0-ultimate-guide-2025-ai-code-editing/)
- [Cursor AI Review 2026](https://prismic.io/blog/cursor-ai)
- [Cursor Visual Editor](https://www.starkinsider.com/2025/12/cursor-visual-editor-ide-web-design.html)

### Warp
- [Warp 2.0 Blog Post](https://www.warp.dev/blog/reimagining-coding-agentic-development-environment)
- [Warp Agents](https://www.warp.dev/agents)
- [Warp All Features](https://www.warp.dev/all-features)
- [Warp Terminal](https://www.warp.dev/terminal)
- [Warp Goes Agentic (The New Stack)](https://thenewstack.io/warp-goes-agentic-a-developer-walk-through-of-warp-2-0/)

### Zed
- [Zed Agent Panel Docs](https://zed.dev/docs/ai/agent-panel)
- [Zed AI Overview](https://zed.dev/docs/ai/overview)
- [Zed 2025 Recap](https://zed.dev/2025)
- [Is Zed Ready for AI Power Users in 2026?](https://www.builder.io/blog/zed-ai-2026)
- [Zed Gets the AI Editor Right](https://hyperdev.matsuoka.com/p/zed-gets-the-ai-editor-right)

### v0.dev
- [v0 App](https://v0.app)
- [v0 FAQ](https://v0.dev/faq)
- [v0 Docs](https://v0.app/docs/faqs)
- [What Is V0.dev? (2026)](https://capacity.so/blog/what-is-v0-dev)
- [v0 Hands-on Review](https://annjose.com/post/v0-dev-firsthand/)

### Bolt.new
- [Bolt.new](https://bolt.new/)
- [Bolt Code View Docs](https://support.bolt.new/building/using-bolt/code-view)
- [Bolt.new GitHub](https://github.com/stackblitz/bolt.new)
- [Bolt.new Beginner Guide](https://skywork.ai/blog/bolt-new-beginner-guide-build-deploy-web-apps/)
- [Bolt V2 Hidden Features](https://bolt.new/blog/inside-bolt-v2-hidden-power-features)

### Aider
- [Aider.chat](https://aider.chat/)
- [Aider Documentation](https://aider.chat/docs/)
- [Aider Git Integration](https://aider.chat/docs/git.html)
- [Aider In-Chat Commands](https://aider.chat/docs/usage/commands.html)
- [Aider Review 2025 (Blott)](https://www.blott.com/blog/post/aider-review-a-developers-month-with-this-terminal-based-code-assistant)

### Continue.dev
- [Continue VS Code Extension](https://marketplace.visualstudio.com/items?itemName=Continue.continue)
- [Continue Quick Start](https://docs.continue.dev/ide-extensions/quick-start)
- [Continue Tab Autocomplete](https://docs.continue.dev/walkthroughs/tab-autocomplete)
- [Continue.dev Overview](https://www.continue.dev/continuedev/vscode)

### General UX Patterns
- [Where Should AI Sit in Your UI? (UX Collective)](https://uxdesign.cc/where-should-ai-sit-in-your-ui-1710a258390e)
- [The Shape of AI](https://www.shapeof.ai)
- [AI UX Patterns](https://www.aiuxpatterns.com/)
