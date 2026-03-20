# OpenCode Research Analysis

**Date**: 2026-03-20
**Purpose**: Understand OpenCode UX patterns for hex-nexus adoption

---

## 1. What Is OpenCode?

OpenCode is an open-source AI coding agent built for the terminal. Originally created by SST (sst/opencode), now maintained under anomalyco/opencode. It has 93K+ GitHub stars and 2.5M+ monthly developers. It is free and open-source — users only pay for their chosen LLM provider.

**Core philosophy**: IDE-level intelligence in the terminal, with provider independence (75+ LLM providers supported).

**Repository**: https://github.com/opencode-ai/opencode

---

## 2. Architecture — Three Frontends, One Backend

### Backend (Go)

- Core backend written in **Go**
- Event-driven architecture centered around session management, tool execution, and a REST API
- `opencode serve` runs a headless HTTP server exposing an OpenAPI 3.1 spec
- Server-Sent Events (SSE) for real-time streaming to clients
- SQLite for state persistence
- MCP (Model Context Protocol) integration — MCP servers defined in config, tools auto-discovered on startup

### SDKs

- Auto-generated from OpenAPI spec
- Available in **Go**, **TypeScript**, and **Python**
- TypeScript SDK used by all frontend clients

### Frontend 1: TUI (Terminal)

- Built on **OpenTUI** — a native terminal UI framework written in **Zig** with TypeScript bindings
- **@opentui/react** — React reconciler for OpenTUI (JSX-based, familiar React patterns)
- **@opentui/solid** — SolidJS reconciler also available
- 60 FPS rendering with dirty-rectangle optimization
- Flexbox-like layout system
- Keyboard, mouse, and paste event handling

### Frontend 2: Desktop App (Tauri + Electron)

- Primary: **Tauri** (Rust wrapper) — lighter, native
- Alternative: **Electron** — broader plugin compatibility
- Both share the same **SolidJS**-based UI layer
- Build chain: tsgo -b → Vite → Tauri CLI → platform packaging
- Native features: file system access, notifications, auto-updates, deep linking

### Frontend 3: Web Console (SolidStart)

- **SolidStart**-based admin web app
- Account management, subscription billing, usage analytics
- Three-tier: presentation → application logic → data persistence

### Shared UI Package

- **@opencode-ai/ui** — SolidJS component library shared between Desktop + Console
- Includes: session rendering, message display, theme system, code diffs, tool execution display
- Diff viewer: **Pierre** (diff component)
- Code viewer: **Shiki** (syntax highlighting)

---

## 3. Key UX Patterns

### Session / Conversation View

- **Session view** is the core — handles message display, scrolling, user input
- **Sticky headers** remain visible during scroll
- **Auto-scroll** to bottom during active AI work
- Multi-panel layout: conversation history, file changes, terminal output, code reviews
- File tree shows change indicators derived from diffs

### Two-Agent Model

- **Build agent**: Full-access development (read + write + execute)
- **Plan agent**: Read-only analysis, denies file edits, asks permission before commands
- **@general subagent**: Complex multi-step searches and tasks
- Users switch between agents depending on task

### Command System (Three Access Methods)

1. **Slash commands**: Type `/` in prompt — quick actions
2. **Command palette**: `Ctrl+P` — searchable list of all 60+ commands
3. **Keybindings**: Leader key system (`Ctrl+X` default, then shortcut) — avoids terminal conflicts

### Multi-Session / Parallel Agents

- Run multiple agents in parallel on the same project
- One refactoring while another writes tests
- Session sharing via links

### LSP Integration

- Auto-detects and loads correct Language Server for the project
- Provides type checking, cross-file dependency awareness
- Gives the AI architectural consistency information

### State Persistence

- UI preferences in `~/.opencode/state/kv.json`
- KVProvider with typed signal accessors
- Changes immediately written to disk
- Settings persist across TUI sessions

---

## 4. What Makes OpenCode Distinctive

| Feature | OpenCode | Claude Code | Cursor | Aider |
|---------|----------|-------------|--------|-------|
| Provider independence | 75+ providers | Anthropic only | OpenAI-focused | Multi-provider |
| Interface | TUI + Desktop + Web | TUI only | IDE (VS Code fork) | TUI only |
| Auth options | GitHub Copilot, ChatGPT Plus, API keys | API key | Subscription | API key |
| Multi-session | Yes, parallel agents | No | No | No |
| LSP integration | Native | No | Yes (IDE) | No |
| Open source | Yes (93K stars) | No | No | Yes |
| Cost | Free (pay LLM only) | $20/mo or API | $20/mo | Free (pay LLM) |
| MCP support | Yes, auto-discovery | Yes | Limited | No |
| Agent modes | Build + Plan + subagents | Single agent | Single agent | Single agent |

### Key differentiators:
1. **Provider agnosticism** — single tool works with any LLM provider
2. **Three frontend surfaces** from one backend (TUI, Desktop, Web)
3. **OpenTUI** — custom Zig-based terminal rendering at 60fps
4. **Build/Plan agent duality** — explicit separation of read vs write modes
5. **Parallel multi-session** — multiple agents on same project without conflicts
6. **Native LSP** — language server integration gives AI real type information

---

## 5. Web Technologies Summary

| Layer | Technology |
|-------|-----------|
| Backend | Go |
| TUI rendering | Zig (OpenTUI) |
| TUI bindings | TypeScript via FFI |
| TUI framework | React (via @opentui/react) or SolidJS |
| Desktop (primary) | Tauri (Rust) |
| Desktop (alt) | Electron |
| Desktop UI | SolidJS + Vite |
| Web console | SolidStart |
| Shared UI lib | SolidJS (@opencode-ai/ui) |
| Diff viewer | Pierre |
| Code highlighting | Shiki |
| API spec | OpenAPI 3.1 |
| Streaming | Server-Sent Events (SSE) |
| State | SQLite |
| Build | tsgo, Vite |

---

## 6. Multi-Model Support & Inference Routing

### Provider Configuration

- Supports 75+ providers via Models.dev integration
- Built-in providers: OpenAI, Anthropic, Google Gemini, AWS Bedrock, Groq, Azure OpenAI, OpenRouter
- Local models via Ollama
- Auth via: direct API keys, GitHub Copilot subscription, ChatGPT Plus/Pro subscription

### Routing Architecture

- Provider transformations normalize messages/options/schemas across all 75+ providers
- Handles provider-specific requirements before LLM invocation
- Vercel AI Gateway support — single endpoint, one API key, model specified as `provider/model-name`

### Per-Agent Model Assignment

- oh-my-opencode plugin: assign different models per agent and per task category
- Config in `~/.config/opencode/oh-my-opencode.jsonc`
- Specialized agents (planner, researcher, debugger) can each use optimal models

### Model Switching

- Models can be switched per-session in the TUI
- No restart required — hot-swap models mid-conversation

---

## 7. Relevance to hex-nexus

### Patterns worth adopting:

1. **Go/Rust backend + OpenAPI + SSE** — hex-nexus already has Rust backend; add OpenAPI spec generation and SSE streaming for real-time UI updates

2. **Shared UI component library** — create `@hex/ui` with SolidJS components for both the nexus dashboard and potential desktop/TUI surfaces

3. **Session-centric conversation view** — sticky headers, auto-scroll, file change indicators in tree

4. **Command palette + slash commands + keybindings** — three access methods for the same 60+ actions

5. **Build/Plan agent duality** — map to hex's existing agent roles (hex-coder = build, planner = plan)

6. **Pierre + Shiki** for diff viewing and code highlighting in the browser

7. **Provider transformation layer** — normalize multi-model routing similar to hex-nexus model selector

8. **Multi-session parallel agents** — align with hex swarm coordination (HexFlo)

9. **KV state persistence** — simple, immediate, typed — good pattern for UI preferences

10. **LSP integration** feeding type info to AI — could enhance hex analyze accuracy
