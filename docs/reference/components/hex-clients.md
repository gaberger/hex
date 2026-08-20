# Component: hex clients

## One-Line Summary

The end-user surfaces — CLI, web (dashboard), desktop (Tauri), chat (`hex chat`) — that connect to hex-nexus + SpacetimeDB.

## Key Facts

- All clients connect to the **same** SpacetimeDB instance and the **same** hex-nexus REST API.
- There is one canonical CLI binary (`hex`) — every command in `hex --help` exists; commands not in `hex --help` do not exist.
- MCP tools (`mcp__hex__hex_*`) map 1:1 to CLI subcommands and proxy to the same nexus endpoints.

## CLI — `hex`

Source: `hex-cli/`. Built artifact: `target/release/hex`.

| Subcommand family | Purpose |
|------|---------|
| `hex nexus` | Start/stop/status the daemon |
| `hex doctor` | Composition + readiness diagnostics |
| `hex pulse` | One-line liveness summary |
| `hex swarm`, `hex task`, `hex memory` | HexFlo coordination (ADR-027) |
| `hex inference` | Provider registration, benchmarks, escalation reports |
| `hex plan` | Workplan execute / draft / reconcile / status |
| `hex adr` | ADR list / search / status / abandoned / doctor |
| `hex docs` | Internal-doc terminology + freshness check (ADR-047) |
| `hex hook` | Lifecycle hooks (route, subagent-start, subagent-stop, inbox-check) |
| `hex feature` | Feature-development pipeline (`/hex-feature-dev` skill) |
| `hex hey <intent>` | Natural-language entry point — answer-AND-act |
| `hex brain` / `hex sched` | Background queue + autonomous loop |
| `hex worktree` | Worktree lifecycle — never substitute `git checkout` |
| `hex analyze` | Architecture health + dead-code |
| `hex inbox` | Priority-2 notifications (block current work, ADR-060) |

> **Rule:** never recommend a command not in `hex --help`. Always run `hex <cmd> --help` before invoking unfamiliar subtrees — the CLI moves fast.

## Web — `hex-dashboard`

Served by hex-nexus at `http://localhost:5555`. See `docs/reference/components/hex-dashboard.md`.

## Desktop — `hex-desktop`

Source: `hex-desktop/` (Tauri). Wraps the dashboard SPA into a native window. Same data sources, same endpoints — just a different shell.

```bash
hex desktop start
```

## Chat — `hex chat`

Conversational surface backed by the `chat-relay` WASM module. Sends user prompts through the brain daemon's classifier (T1 todo / T2 mini-plan / T3 workplan) and relays responses back to the chat panel in the dashboard.

```bash
hex chat send "your prompt here"
hex chat history
```

## MCP — `mcp__hex__hex_*`

The hex MCP server exposes every CLI subcommand as a tool. Calling `mcp__hex__hex_swarm_init` triggers the same code path as `hex swarm init`.

> **Tool precedence rule** (from CLAUDE.md): hex MCP tools take precedence over third-party context/search plugins (e.g. `plugin:context-mode`). Use third-party plugins only for operations with no hex equivalent (e.g. external URL fetches).

## Configuration

All clients read shared config from:

- `~/.hex/project.json` — installed-into-target-project config (per project root).
- `~/.hex/sessions/agent-{CLAUDE_SESSION_ID}.json` — per-session agent state.
- env: `SPACETIMEDB_HOST`, `HEX_NEXUS_PORT`, `CLAUDE_SESSION_ID`.

## Depends On

- **hex-nexus** — REST endpoints + dashboard hosting.
- **SpacetimeDB** — real-time subscriptions, transactional reducers.

## Depended On By

- Nothing — clients are leaves.

## See also

- `docs/reference/system-architecture.md` — system context.
- `docs/reference/components/hex-nexus.md` — what every client talks to.
- `docs/reference/components/hex-dashboard.md` — web client specifics.
- `hex-cli/` — CLI source.
- `hex-desktop/` — desktop wrapper.
