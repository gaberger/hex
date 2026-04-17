# Swarm Coordination (HexFlo)

Source ADR: 027. Skill: `/hex-swarm`.

HexFlo is the native Rust coordination layer in hex-nexus — zero external deps, replaces ruflo. State persists in SpacetimeDB via the `hexflo-coordination` WASM module with SQLite fallback for offline use.

## Module layout

```
hex-nexus/src/coordination/
  mod.rs           # HexFlo struct — unified swarm/task/agent API
  memory.rs        # Key-value persistent memory (scopes: global, per-swarm, per-agent)
  cleanup.rs       # Heartbeat timeout + dead-agent task reclamation

spacetime-modules/hexflo-coordination/
  src/lib.rs       # Tables: swarm, swarm_task, swarm_agent, hexflo_memory
                   # Reducers: swarm_init, task_create, task_assign, task_complete,
                   #           agent_register, agent_heartbeat, memory_store
```

## API surface

| Operation | HexFlo API | REST |
|-----------|-----------|------|
| Init swarm | `HexFlo::swarm_init(name, topology)` | `POST /api/swarms` |
| Swarm status | `HexFlo::swarm_status()` | `GET /api/swarms` |
| Create task | `HexFlo::task_create(swarm_id, title)` | `POST /api/swarms/:id/tasks` |
| Complete task | `HexFlo::task_complete(id, result)` | `PATCH /api/swarms/tasks/:id` |
| Store memory | `HexFlo::memory_store(key, value, scope)` | `POST /api/hexflo/memory` |
| Retrieve memory | `HexFlo::memory_retrieve(key)` | `GET /api/hexflo/memory/:key` |
| Search memory | `HexFlo::memory_search(query)` | `GET /api/hexflo/memory/search` |
| Cleanup stale | `HexFlo::cleanup_stale_agents()` | `POST /api/hexflo/cleanup` |

## MCP tools (Claude Code integration)

Served by `hex mcp` (Rust binary). Tool names map 1:1 to CLI commands.

```
mcp__hex__hex_analyze          → hex analyze [path]
mcp__hex__hex_status           → hex status
mcp__hex__hex_swarm_init       → hex swarm init
mcp__hex__hex_swarm_status     → hex swarm status
mcp__hex__hex_task_create      → hex task create
mcp__hex__hex_task_list        → hex task list
mcp__hex__hex_task_complete    → hex task complete
mcp__hex__hex_memory_store     → hex memory store
mcp__hex__hex_memory_retrieve  → hex memory get
mcp__hex__hex_memory_search    → hex memory search
mcp__hex__hex_inbox_notify     → hex inbox notify (ADR-060)
mcp__hex__hex_inbox_query      → hex inbox list  (ADR-060)
mcp__hex__hex_inbox_ack        → hex inbox ack   (ADR-060)
mcp__hex__hex_adr_list         → hex adr list
mcp__hex__hex_adr_search       → hex adr search
mcp__hex__hex_adr_status       → hex adr status
mcp__hex__hex_adr_abandoned    → hex adr abandoned
mcp__hex__hex_nexus_status     → hex nexus status
mcp__hex__hex_nexus_start      → hex nexus start
mcp__hex__hex_secrets_status   → hex secrets status
mcp__hex__hex_secrets_has      → hex secrets has
```

All MCP tools delegate to the hex-nexus REST API.

## CLI

```bash
hex swarm init <name> [topology]    # initialise
hex swarm status                    # active swarms
hex task create <swarm-id> <title>  # create task
hex task list                       # list
hex task complete <id> [result]     # mark done
hex memory store <key> <value>      # store KV
hex memory get <key>                # retrieve
hex memory search <query>           # search
```

## Heartbeat protocol

- Agents heartbeat on every `UserPromptSubmit` (via `hex hook route`).
- Stale after 45s without heartbeat.
- Dead after 120s — tasks reclaimed.

Always use background agents with bypassPermissions for file writes:

```
Agent tool: { subagent_type: "coder", mode: "bypassPermissions", run_in_background: true }
```

## Task state sync (ADR-048)

Include `HEXFLO_TASK:{task_id}` in subagent prompts. Hooks (`hex hook subagent-start` / `subagent-stop`) read stdin and:

1. **SubagentStart**: extract task ID → `PATCH /api/hexflo/tasks/{id}` with `agent_id` → status `in_progress`.
2. **SubagentStop**: read `current_task_id` from session state → PATCH with `status: "completed"` + first 200 chars of output.
3. Persist tracking in `~/.hex/sessions/agent-{CLAUDE_SESSION_ID}.json`.

```
Agent tool: {
  prompt: "HEXFLO_TASK:88bb424c-...\nImplement the port interface for...",
  subagent_type: "coder",
  mode: "bypassPermissions",
  run_in_background: true
}
```

`agent_id` auto-resolves from `~/.hex/sessions/agent-{CLAUDE_SESSION_ID}.json` (written by `hex hook session-start`). MCP tool `hex_hexflo_task_assign` also auto-resolves when not given explicitly.
