# ADR-2604011300: Rich CLI Chat TUI (`hex chat`)

**Status:** Proposed
**Date:** 2026-04-01
**Drivers:** hex works well inside Claude Code via MCP, but has no standalone conversational interface — users cannot interact with their hex-connected inference providers directly from a terminal session without Claude Code running.
**Supersedes:** N/A

<!-- ID format: YYMMDDHHMM — use your local time. Example: 2603221500 = 2026-03-22 15:00 -->

## Context

hex's inference gateway (SpacetimeDB + hex-nexus bridge) can route requests to any LLM provider. However, the only way to use this today is through the MCP tools exposed to Claude Code. When a user wants a quick conversational interaction — asking about project architecture, querying ADRs, or exploring workplan status — they must have Claude Code open.

Claude Code itself implements a rich CLI experience: streaming token output, multi-line input, conversation history, markdown + syntax highlighting, tool use display, and vim key bindings. This is built on **Ink** (React reconciler targeting terminal output) with a custom Yoga layout engine.

For hex, the equivalent must be implemented in Rust within `hex-cli`. The idiomatic Rust TUI stack is **ratatui** (retained-mode widget library) + **crossterm** (cross-platform terminal I/O). This combination provides the same interactive feel as Ink without a JavaScript runtime.

**Forces at play:**
- hex-cli is a Rust binary; adding a Node.js/Ink dependency is not acceptable
- The inference gateway is already wired through hex-nexus REST (`POST /api/inference/chat`)
- Users expect parity with the Claude Code experience: streaming, history, code highlighting
- hex has hexagonal architecture — the TUI is a primary adapter, not a domain concern
- Session state must be persisted (conversation history) for multi-turn context
- The chat command must remain aware of the current hex project (ADRs, workplans, agents)

**Alternatives considered:**
1. **Ship a Node.js chat binary separately** — rejected; adds runtime dependency, splits the UX
2. **Use `cursive`** — higher-level but less flexible; ratatui has stronger ecosystem and is the de facto standard
3. **Plain readline loop** — no streaming, no syntax highlighting, no tool use display; far below target UX
4. **Embed a web view (Tauri)** — overkill for CLI; desktop app is already handled by hex-desktop
5. **ratatui + crossterm** — direct Rust, composable, proven in production (Helix, Gitui, etc.)

## Decision

We will add a `hex chat` subcommand to `hex-cli` that provides a rich, interactive TUI chat experience backed by the hex inference gateway.

### Architecture

`hex chat` is a **primary adapter** in hexagonal terms — it drives the `IInferencePort` and `ISessionPort` defined in `hex-core`. It must not contain business logic.

```
hex-cli/src/
  commands/
    chat.rs          # Entry point: parses flags, wires deps, launches TUI
  chat/
    mod.rs           # TUI app struct, ratatui event loop
    input.rs         # Multi-line input widget (crossterm keypress handling)
    messages.rs      # Message list widget (streaming render, scroll)
    renderer.rs      # Markdown → ANSI (pulldown-cmark + syntect)
    history.rs       # Session persistence (~/.hex/sessions/chat-{id}.json)
    keymap.rs        # Key bindings (insert / normal / command modes)
    state.rs         # App state machine (idle | streaming | tool_call | error)
    tool_display.rs  # Tool use display (hex tool calls rendered inline)
```

`hex-core` ports used:
- `IInferencePort` — sends messages, receives token stream
- `ISessionPort` — loads/saves conversation history
- `IProjectContextPort` — injects project context (active ADRs, agent names, workplan status)

hex-nexus provides the concrete adapters via its REST API; `hex chat` calls them through the same HTTP client used by all other hex-cli commands.

### Key Features

**Streaming output**: The message rendering widget consumes a `tokio::sync::mpsc::Receiver<String>` of tokens. Each token triggers a ratatui re-render of the current assistant message bubble, producing the typewriter effect familiar from Claude Code.

**Multi-line input**: `input.rs` implements a minimal editor widget — Enter to submit, Shift+Enter for newline, Up/Down for history navigation, Ctrl+C to cancel. Optional vim mode (insert/normal) gated behind `--vim` flag.

**Markdown rendering**: `renderer.rs` converts assistant output through `pulldown-cmark` → custom ANSI emitter using `syntect` for code block syntax highlighting. Tables, bold, italic, and inline code are all supported.

**Tool use display**: When the model emits a tool call, `tool_display.rs` renders a collapsible block showing tool name, arguments, and result. hex tools (`hex_analyze`, `hex_adr_search`, etc.) are automatically available as tools in the session.

**Project context injection**: On session start, `IProjectContextPort` loads a compact context block: active ADR titles, current workplan status, and agent roster. This is prepended as a system message so the model is project-aware without manual copy-paste.

**Session persistence**: Conversations are stored in `~/.hex/sessions/chat-{uuid}.json` in the same shape as the existing `agent-{session-id}.json` files. `hex chat --resume` lists and resumes prior sessions.

### Constraints

- `hex chat` **must not** import from any `adapters/secondary/` directly — only from `hex-core` ports.
- The ratatui event loop runs in the `hex-cli` binary only; no TUI code in `hex-nexus`.
- Token streaming uses Server-Sent Events (SSE) from the nexus `POST /api/inference/chat/stream` endpoint — this endpoint must be added to hex-nexus as part of this ADR's implementation.

## Consequences

**Positive:**
- hex becomes a self-contained development environment — users can interact conversationally without Claude Code
- Consistent UX with Claude Code's streaming + syntax highlighting experience, all in Rust
- Project-aware context injection means the model always knows ADR/workplan state without manual setup
- Sessions are persistent and resumable — long-running exploration work is not lost
- Tool use display makes hex tool invocations visible and debuggable in chat

**Negative:**
- Adds `ratatui`, `crossterm`, `pulldown-cmark`, and `syntect` to `hex-cli` dependencies (~500KB to binary)
- SSE streaming endpoint must be added to hex-nexus (`/api/inference/chat/stream`)
- Terminal width/height edge cases require testing across macOS, Linux, Windows Terminal

**Mitigations:**
- ratatui and crossterm are already used by other hex-adjacent Rust tools (Helix, Gitui) — well-tested
- SSE endpoint is a small addition to hex-nexus axum router, reusing existing inference_tx channel
- A `--no-tui` flag falls back to plain stdout streaming for CI / pipe contexts

## Implementation

| Phase | Description | Status |
|-------|------------|--------|
| P1 | Add `POST /api/inference/chat/stream` SSE endpoint to hex-nexus | **Complete** (a30d6499) |
| P2 | Implement `hex chat` command + ratatui TUI (streaming, markdown, flowing layout) | **Complete** (b9b70e83) |
| P3 | Project context injection — fetch hex state on startup, build system prompt | **Complete** (90eb8dd3) |
| P4 | Slash commands (`/help`, `/clear`, `/model`, `/context`, `/adr`, `/plan`, `/save`) | **Complete** (90eb8dd3) |
| P5 | Tool use loop — define tool schemas, parse model tool_calls, execute via nexus, display inline | Pending |
| P6 | Session persistence — auto-save to `~/.hex/sessions/`, `hex chat --resume` picker | **Complete** (90eb8dd3) |
| P7 | Hooks — `on_session_start`, `on_message_send`, `on_message_receive`, `on_session_end` | Pending |

### Context Injection (P3)

On `hex chat` startup, before the first turn, build a system message from live hex state:

```
You are an AI assistant for the hex project. Current state:

Project: {project_name} ({project_id})
Active workplans: {workplan summaries}
Recent ADRs: {adr titles + status}
Registered agents: {agent names}
Inference providers: {provider list}

Available commands: hex analyze, hex adr search, hex plan list, hex plan status, ...
```

Injected as the `system` field. User `--system` flag appends to this. Fetched via nexus REST API at session start.

### Slash Commands (P4)

Slash commands are parsed client-side before inference — no model involved. They execute hex operations and display results inline in the message area.

| Command | Action |
|---------|--------|
| `/help` | List available slash commands |
| `/clear` | Clear conversation history (keeps system context) |
| `/model <name>` | Switch model for the session |
| `/context` | Show the current injected system context |
| `/adr <query>` | Search ADRs via `/api/adrs/search` |
| `/plan` | List workplans with status |
| `/save` | Save session to `~/.hex/sessions/chat-{uuid}.json` |

### Tool Use Loop (P5)

For models supporting tool use (OpenAI-compat with `tools` parameter), define schemas for hex operations. The SSE stream carries tool call events; the TUI parses them, executes via nexus, and injects results.

Tool schemas: `hex_status`, `hex_adr_search`, `hex_plan_list`, `hex_plan_status`, `hex_analyze`, `hex_git_log`, `hex_inference_list`.

SSE tool call event format (added to nexus stream handler):
```json
{"tool_call": {"id": "tc_01", "name": "hex_adr_search", "arguments": {"query": "authentication"}}}
{"tool_result": {"id": "tc_01", "content": "[{...}]"}}
```

TUI renders tool calls as collapsed inline blocks:
```
  ⚙ hex_adr_search("authentication")
  └─ 3 results found
```

### Session Persistence (P6)

Sessions auto-saved to `~/.hex/sessions/chat-{uuid}.json` after each turn:
```json
{"id": "...", "created_at": "...", "messages": [...], "model": "...", "project_id": "..."}
```

`hex chat --resume` shows an arrow-key picker of recent sessions.

## References

- Reference implementation: `/Volumes/ExtendedStorage/dev/claude-code/src/` (Ink + React TUI)
  - `src/ink/` — custom Ink renderer (Yoga layout, React reconciler)
  - `src/vim/` — vim key bindings
  - `src/history.ts` — conversation history
  - `src/components/` — message, tool use, streaming widgets
- `hex-nexus/src/orchestration/mod.rs` — existing inference_tx channel wiring
- ADR-2604010000 — Unified execution path (inference routing)
- ADR-2604011200 — SpacetimeDB native autonomous dispatch
- ADR-027 — HexFlo swarm coordination (session-level agent context)
- ADR-060 — Inbox notifications (surface in chat sidebar)
- [ratatui](https://ratatui.rs) — Rust TUI framework
- [crossterm](https://github.com/crossterm-rs/crossterm) — cross-platform terminal I/O
- [syntect](https://github.com/trishume/syntect) — syntax highlighting for Rust
