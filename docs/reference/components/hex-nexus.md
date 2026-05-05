# Component: hex-nexus

## One-Line Summary

Filesystem-bridge daemon — the only unit allowed to touch the OS (FS, git, spawning, outbound HTTP). Runs the dashboard at `:5555` and exposes the REST API.

## Key Facts

- Required service. Starts via `hex nexus start`.
- HTTP server: axum on port **5555** by default.
- Bridges SpacetimeDB WASM sandbox ↔ local OS (filesystem, git, process spawn, LLM HTTP calls).
- Syncs repo config → SpacetimeDB on startup (ADR-044).
- Serves the Solid.js dashboard frontend (bundled via `rust-embed`).
- Hosts HexFlo swarm coordination (ADR-027) — tasks, agents, memory.
- Multi-instance safe via `ICoordinationPort` + filesystem locks + heartbeats (ADR-011).
- Editing `hex-nexus/assets/*` requires `cargo build -p hex-nexus --release` + daemon restart + browser hard-refresh.

## API Surface (REST)

Base URL: `http://localhost:5555`. Selected endpoints:

| Verb + Path | Purpose |
|-------------|---------|
| `GET  /api/health` | Liveness probe |
| `GET  /api/swarms` | List swarms (HexFlo) |
| `POST /api/swarms` | Create a swarm |
| `GET  /api/hexflo/tasks` | List tasks |
| `POST /api/hexflo/tasks` | Create a task |
| `PATCH /api/hexflo/tasks/{id}` | Update task state (used by `hex hook subagent-{start,stop}`) |
| `POST /api/plans/execute` | Execute a workplan (`hex plan execute`) |
| `GET  /api/inference/escalation-report` | Tier-routing report (ADR-2604120202) |
| `GET  /api/git/{repo}/status` | Git status for a project |
| `POST /api/analyze` | Run tree-sitter boundary analysis |
| `GET  /api/projects` | Project list (multi-project dashboard) |

`hex` MCP tools (`mcp__hex__hex_*`) and `hex` CLI subcommands map 1:1 to these endpoints. Real-time push for the dashboard is via SpacetimeDB subscriptions, not nexus SSE.

## API Surface (background services)

Inside the daemon, several services run on schedules:

| Service | Module | Purpose |
|---------|--------|---------|
| `config_sync` | `src/config_sync.rs` | Push repo config files into SpacetimeDB on startup (ADR-044) |
| `coordination` | `src/coordination/` | HexFlo swarm + task + memory + cleanup loops |
| `git` | `src/git/` | git ops invoked by reducers — clone, branch, worktree, merge |
| `analysis` | `src/analysis/` | Tree-sitter parsers, boundary check, dead-code scan |
| `orchestration` | `src/orchestration/` | Workplan execution, agent supervision |
| `inference router` | `src/adapters/inference_router/` | Outbound HTTP to Anthropic / OpenAI / Ollama / vLLM |
| `sched_service` | scheduled background tasks | Brain loop, idle research, daemon cleanup |

## Configuration

| Var | Default | Purpose |
|-----|---------|---------|
| `HEX_NEXUS_PORT` | `5555` | HTTP listen port |
| `SPACETIMEDB_HOST` | `localhost:3000` | Coordination backend |
| `CLAUDE_SESSION_ID` | unset | If set → Claude CLI composition; if unset → standalone composition (ADR-2604112000) |
| `HEX_AUTO_PLAN` | `1` | Set `0` to disable auto T3-workplan invocation |
| `OPENROUTER_MANAGEMENT_KEY` | unset | Required only for `hex inference openrouter ...` admin actions |
| `ANTHROPIC_API_KEY`, `OPENAI_API_KEY` | unset | Provider keys; loaded only in composition root |

Repo-level config: `.hex/project.json`. Synced into SpacetimeDB on every nexus startup so all clients see the same view.

## Composition variants (ADR-2604112000)

Selected at startup based on env:

| Variant | Trigger | Inference |
|---------|---------|-----------|
| Claude-CLI mode | `CLAUDE_SESSION_ID` set | Routes through Claude CLI |
| Standalone mode | `CLAUDE_SESSION_ID` unset | `AgentManager` + `OllamaInferenceAdapter` |

Diagnose with `hex doctor composition`. Validate the standalone path with `hex ci --standalone-gate`.

## Depends On

- **SpacetimeDB** — required state backend.
- **hex-core** — shared port traits + domain types.
- Filesystem (read/write under the project root + `~/.hex/`).
- Optional outbound HTTP to LLM providers.

## Depended On By

- **hex-cli** — most subcommands proxy through nexus REST.
- **hex-dashboard** — served by the same binary; subscribes to SpacetimeDB for data.
- **hex-agent** — uses nexus for FS / git operations the WASM sandbox can't do.
- **MCP server** (`mcp__hex__hex_*`) — every MCP tool is a thin wrapper over the same REST endpoints.

## Operations

```bash
hex nexus start              # foreground; --background to daemonize
hex nexus status             # PID, uptime, port
hex doctor                   # composition + assets-generic + readiness checks
hex doctor composition       # which composition variant is active
hex pulse                    # one-line liveness summary
```

Logs land in `~/.hex/logs/nexus-<pid>.log`. Crash diagnostics: `hex doctor crash`.

## See also

- `docs/adrs/ADR-024-hex-nexus-autonomous-hub.md` — original design.
- `docs/adrs/ADR-044-nexus-git-integration.md` — config sync.
- `docs/adrs/ADR-027-hexflo-swarm-coordination.md` — coordination layer.
- `docs/adrs/ADR-011-multi-instance-coordination.md` — filesystem locks.
- `docs/adrs/ADR-2604112000-hex-standalone-dispatch.md` — composition variants.
- `docs/reference/system-architecture.md` — system-wide context.
- `docs/reference/components/spacetimedb.md` — what nexus syncs into.
