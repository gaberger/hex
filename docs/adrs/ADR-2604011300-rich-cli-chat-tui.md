# ADR-2604011300: Rich CLI Chat TUI (`hex chat`)

**Status:** Deprecated
**Date:** 2026-04-01
**Updated:** 2026-04-01
**Deprecated:** 2026-04-01 — Superseded by ADR-2603231800. The Rust TUI approach was abandoned in favour of `hex chat` launching opencode directly. P1–P10 (the Rust TUI) are archived; P11 was never started. The `hex-cli/src/tui/` module has been removed.
**Supersedes:** ADR-036 (hex-chat session architecture — deprecated 2026-03-22)

<!-- ID format: YYMMDDHHMM — use your local time. Example: 2603221500 = 2026-03-22 15:00 -->

## Context

hex's inference gateway (SpacetimeDB + hex-nexus bridge) can route requests to any LLM provider. However, the only way to use this today is through the MCP tools exposed to Claude Code. When a user wants a quick conversational interaction — asking about project architecture, querying ADRs, or exploring workplan status — they must have Claude Code open.

Claude Code itself implements a rich CLI experience: streaming token output, multi-line input, conversation history, markdown + syntax highlighting, tool use display, and vim key bindings. This is built on **Ink** (React reconciler targeting terminal output) with a custom Yoga layout engine.

**opencode** (`github.com/anomalyco/opencode`) is a second reference implementation: a Rust-native TUI chat client with provider switching, file context display, inline diff rendering, and a session sidebar. Its architecture directly parallels what hex needs — it is the closest analog in the Rust ecosystem.

For hex, the equivalent must be implemented in Rust within `hex-cli`. The idiomatic Rust TUI stack is **ratatui** (retained-mode widget library) + **crossterm** (cross-platform terminal I/O). This combination provides the same interactive feel as Ink without a JavaScript runtime.

**Forces at play:**
- hex-cli is a Rust binary; adding a Node.js/Ink dependency is not acceptable
- The inference gateway is already wired through hex-nexus REST (`POST /api/inference/chat`)
- Users expect parity with the Claude Code experience: streaming, history, code highlighting
- hex has hexagonal architecture — the TUI is a primary adapter, not a domain concern
- Session state must be persisted (conversation history) for multi-turn context
- The chat command must remain aware of the current hex project (ADRs, workplans, agents)
- The model needs the full hex tool surface (40+ tools) without a hardcoded schema list

**Alternatives considered:**
1. **Ship a Node.js chat binary separately** — rejected; adds runtime dependency, splits the UX
2. **Use `cursive`** — higher-level but less flexible; ratatui has stronger ecosystem and is the de facto standard
3. **Plain readline loop** — no streaming, no syntax highlighting, no tool use display; far below target UX
4. **Embed a web view (Tauri)** — overkill for CLI; desktop app is already handled by hex-desktop
5. **ratatui + crossterm** — direct Rust, composable, proven in production (Helix, Gitui, opencode, etc.)

## Decision

We will add a `hex chat` subcommand to `hex-cli` that provides a rich, interactive TUI chat experience backed by the hex inference gateway.

### Architecture

`hex chat` is a **primary adapter** in hexagonal terms — it drives the `IInferencePort` and `ISessionPort` defined in `hex-core`. It must not contain business logic.

```
hex-cli/src/
  commands/
    chat.rs          # Entry point: parses flags, wires deps, launches TUI
  tui/
    chat.rs          # TUI app struct, ratatui event loop (actual implementation)
    mcp_client.rs    # Embedded MCP client — spawns hex mcp, discovers tools dynamically
    session.rs       # Session persistence (~/.hex/sessions/chat-{id}.json)
    skills.rs        # Slash command parser, user skill loader, /hex command
    markdown.rs      # Markdown → ANSI (pulldown-cmark)
```

hex-nexus provides the concrete adapters via its REST API; `hex chat` calls them through the same HTTP client used by all other hex-cli commands.

### Key Features

**Streaming output**: The message rendering widget consumes a `tokio::sync::mpsc::Receiver<StreamEvent>` of tokens. Each token triggers a ratatui re-render of the current assistant message bubble, producing the typewriter effect familiar from Claude Code.

**Multi-line input**: Enter to submit, Shift+Enter for newline, Up/Down for history scroll, Ctrl+C to cancel.

**Markdown rendering**: Assistant output is parsed through `pulldown-cmark` → custom ANSI emitter with syntax highlighting for code blocks.

**Embedded MCP client** (`tui/mcp_client.rs`): On session start, spawns `hex mcp` (same binary) as a child process via stdio JSON-RPC. Calls `tools/list` to discover all 40+ hex tools dynamically. Tool calls in the inference loop are routed through `McpClient::call_tool()`. Falls back to a hardcoded 5-tool schema if spawn fails.

**Tool use display**: When the model emits a tool call, the TUI renders:
```
  ⚙ hex_adr_search("authentication")
  └─ 3 results found
```

**Project context injection**: On session start, concurrent fetch of `/api/status`, `/api/hexflo/swarms`, `/api/adrs`, and `/api/inference/list` builds a system prompt block that makes the model project-aware without manual copy-paste.

**Session persistence**: Conversations are stored in `~/.hex/sessions/chat-{uuid}.json`. `hex chat --resume` lists and resumes prior sessions.

**Lifecycle hooks**: Fires `hex hook session-start` / `session-end` at TUI init/quit, and `hex hook route` before each inference turn (for inbox notification checks). Hook output surfaces as a dim italic system message.

**User skills**: Loads `.claude/skills/*.md` from project and global dirs at startup. Each skill becomes a slash command. `/skills` lists them. Project-local files override global on name collision.

**hex command execution**: `/hex <subcommand>` runs any `hex` CLI subcommand and streams output into the TUI. The model can also invoke `hex_exec` via MCP (routes through `POST /api/exec`).

**Notification badge**: When `hex hook route` detects a priority-2 inbox message, the status bar shows `🔔 N` and inference is held.

### opencode-Inspired Enhancements (Planned)

The following features are planned based on patterns in opencode's TUI:

| Feature | opencode Pattern | hex chat Target |
|---------|-----------------|-----------------|
| File context display | Shows files currently in scope as a sidebar/overlay | Show files referenced in tool results or manually added via `/add <path>` |
| Inline diff viewer | Renders before/after diffs for code changes inline in message stream | Render diffs when `hex_analyze` or `hex_git_diff` tool results contain diff output |
| Provider/model switcher | Interactive model picker with latency/cost indicators | `/model` picker showing available providers from `/api/inference/list` with cost metadata |
| Session sidebar | Left panel listing recent sessions with preview | `hex chat --resume` picker; eventual in-session sidebar toggle |
| Compact mode | `--no-tui` plain output | Already implemented via `--no-tui` flag |

### Constraints

- `hex chat` **must not** import from any `adapters/secondary/` directly — only from `hex-core` ports.
- The ratatui event loop runs in the `hex-cli` binary only; no TUI code in `hex-nexus`.
- Token streaming uses Server-Sent Events (SSE) from the nexus `POST /api/inference/chat/stream` endpoint.
- `hex_exec` tool must use argv-split (never `sh -c`) to prevent shell injection.

## Consequences

**Positive:**
- hex becomes a self-contained development environment — users can interact conversationally without Claude Code
- Consistent UX with Claude Code's streaming + syntax highlighting experience, all in Rust
- Project-aware context injection means the model always knows ADR/workplan state without manual setup
- Sessions are persistent and resumable — long-running exploration work is not lost
- Embedded MCP client gives the model the full 40+ hex tool surface without a hardcoded schema
- Lifecycle hooks integrate hex's enforcement and inbox systems into every conversation
- User skills make `.claude/skills/*.md` available as slash commands — no separate skill runner needed

**Negative:**
- Adds `ratatui`, `crossterm`, `pulldown-cmark`, `syntect`, `dialoguer`, `dirs` to `hex-cli` (~500KB)
- MCP client spawns a child process on every session — minimal overhead but adds startup latency
- Terminal width/height edge cases require testing across macOS, Linux, Windows Terminal

**Mitigations:**
- ratatui and crossterm are proven in production (Helix, Gitui, opencode)
- MCP spawn failure falls back gracefully to hardcoded schemas
- `--no-tui` flag for CI/pipe contexts bypasses all TUI overhead

## Implementation

| Phase | Description | Status |
|-------|------------|--------|
| P1 | Add `POST /api/inference/chat/stream` SSE endpoint to hex-nexus | **Complete** (a30d6499) |
| P2 | Implement `hex chat` command + ratatui TUI (streaming, markdown, flowing layout) | **Complete** (b9b70e83) |
| P3 | Project context injection — fetch hex state on startup, build system prompt | **Complete** (90eb8dd3) |
| P4 | Slash commands (`/help`, `/clear`, `/model`, `/context`, `/adr`, `/plan`, `/save`) | **Complete** (90eb8dd3) |
| P5 | Tool use loop — OpenAI function calling, stream tool_calls, execute via nexus | **Complete** (654d4012) |
| P6 | Session persistence — auto-save to `~/.hex/sessions/`, `hex chat --resume` picker | **Complete** (90eb8dd3) |
| P7 | Hooks — session-start/end, route hook, notification badge | **Complete** (1bdf7cf4) |
| P8 | Embedded MCP client — spawn `hex mcp`, `tools/list`, dynamic tool routing | **Complete** (753d5476) |
| P9 | Dynamic user skills — load `.claude/skills/`, `/skills`, `/hex <cmd>` | **Complete** (c9d1d9ef, 1bdf7cf4) |
| P10 | `hex_exec` MCP tool + `POST /api/exec` nexus endpoint | **Complete** (3f7c77dc) |
| P11 | opencode-parity: superseded — `hex chat` now execs opencode directly (ADR-2603231800) | **Superseded** |

### Slash Commands (current)

| Command | Action |
|---------|--------|
| `/help` | List all available slash commands including user skills |
| `/clear` | Clear conversation history (keeps system context) |
| `/model <name>` | Switch model for the session |
| `/context` | Show the current injected system context |
| `/adr <query>` | Search ADRs via nexus |
| `/plan` | List workplans with status |
| `/save` | Save session to `~/.hex/sessions/chat-{uuid}.json` |
| `/skills` | List user-defined skills from `.claude/skills/` |
| `/hex <cmd>` | Run any hex CLI subcommand, stream output into TUI |
| `/<skill-name>` | Invoke a user skill — injects body as message |

### MCP Client Protocol (P8)

`McpClient` spawns `hex mcp` via `tokio::process::Command`, communicates over stdio JSON-RPC:

1. `initialize` → server responds with capabilities
2. `initialized` notification → server echoes response
3. `tools/list` → returns all tool schemas; converted to OpenAI function-calling format
4. Per tool call: `tools/call {name, arguments}` → `{content: [{type:"text", text:"..."}]}`

All requests serialized through `tokio::sync::Mutex<McpClient>` — one request at a time.

### Context Injection (P3)

On `hex chat` startup, before the first turn, build a system message from live hex state:

```
You are an AI assistant for the hex project. Current state:

Project: {name} ({buildHash})
Active workplans: {workplan summaries}
Recent ADRs: {adr titles + status}
Inference providers: {provider list}

Available commands: hex analyze, hex adr search, hex plan list, hex plan status, ...
```

Fetched concurrently via `tokio::join!` on 4 nexus endpoints. `--no-context` skips injection. `--system` appends to this.

### hex_exec Security (P10)

`POST /api/exec` and `/hex <cmd>` both:
- Split subcommand string on whitespace → explicit `argv` array
- Pass to `tokio::process::Command::new(exe).args(argv)` — **never `sh -c`**
- Shell metacharacters (`;`, `|`, `&&`) become literal arguments, rejected by clap
- 30-second timeout; output capped at 200 lines in TUI

## References

- [opencode](https://github.com/anomalyco/opencode) — Rust-native TUI chat client; reference for file context display, diff viewer, provider switcher patterns
- Claude Code reference: Ink + React TUI (`src/ink/`, `src/vim/`, `src/history.ts`, `src/components/`)
- `hex-nexus/src/orchestration/mod.rs` — existing inference_tx channel wiring
- `hex-cli/src/tui/mcp_client.rs` — embedded MCP client implementation
- `hex-cli/src/tui/skills.rs` — slash command dispatcher + user skill loader
- ADR-2603231800 — hex context injection into opencode (complementary: inject hex into opencode)
- ADR-2604010000 — Unified execution path (inference routing)
- ADR-2604011200 — SpacetimeDB native autonomous dispatch
- ADR-027 — HexFlo swarm coordination (session-level agent context)
- ADR-060 — Inbox notifications (route hook surfaces these in chat)
- [ratatui](https://ratatui.rs) — Rust TUI framework
- [crossterm](https://github.com/crossterm-rs/crossterm) — cross-platform terminal I/O
- [syntect](https://github.com/trishume/syntect) — syntax highlighting for Rust
