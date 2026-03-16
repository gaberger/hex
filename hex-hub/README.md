# hex-hub

Rust HTTP service that provides project registration, real-time event streaming (SSE + WebSocket), and an embedded dashboard UI for the hex framework. Replaces the Node.js `dashboard-hub.ts`.

## Quick Start

```bash
cargo build --release
./target/release/hex-hub --port 5555 --token mysecret
# Open http://localhost:5555 in a browser
```

## CLI Options

| Flag | Default | Description |
|------|---------|-------------|
| `--port <N>` | `5555` | Listen port |
| `--token <T>` | random (printed in lock file) | Bearer token for mutation endpoints |
| `--daemon` | off | Log as daemon (no behavioral difference yet) |

The token can also be set via the `HEX_DASHBOARD_TOKEN` environment variable. The `--token` flag takes precedence.

On startup, hex-hub writes a lock file to `~/.hex/daemon/hub.lock` containing PID, port, token, and version. The lock file is removed on graceful shutdown (SIGINT/SIGTERM).

## API Reference

All endpoints return JSON. Mutation endpoints (POST, DELETE) require `Authorization: Bearer <token>` when a token is configured. GET and OPTIONS requests bypass auth.

### Project Management

```bash
# List registered projects
curl http://localhost:5555/api/projects

# Register a project
curl -X POST http://localhost:5555/api/projects/register \
  -H "Authorization: Bearer mysecret" \
  -H "Content-Type: application/json" \
  -d '{"rootPath": "/home/user/my-app", "name": "my-app", "astIsStub": false}'

# Unregister a project
curl -X DELETE http://localhost:5555/api/projects/my-app-1v7n98d \
  -H "Authorization: Bearer mysecret"
```

Project IDs are deterministic: `basename-djb2hash` (base-36). The same `rootPath` always produces the same ID, matching the TypeScript implementation.

### Push State (project adapters -> hub)

```bash
# Push health data
curl -X POST http://localhost:5555/api/push \
  -H "Authorization: Bearer mysecret" \
  -H "Content-Type: application/json" \
  -d '{"projectId": "my-app-1v7n98d", "type": "health", "data": {"score": 85}}'

# Push a single event
curl -X POST http://localhost:5555/api/event \
  -H "Authorization: Bearer mysecret" \
  -H "Content-Type: application/json" \
  -d '{"projectId": "my-app-1v7n98d", "event": "file-change", "data": {"path": "src/main.rs"}}'
```

Supported push types: `health`, `tokens`, `tokenFile` (requires `filePath`), `swarm`, `graph`, `project`.

Body limits: `/api/push` accepts up to 256KB, `/api/event` up to 16KB.

### Per-Project Queries (browser reads)

| Endpoint | Description |
|----------|-------------|
| `GET /api/{pid}/health` | Architecture health summary |
| `GET /api/{pid}/tokens/overview` | Token usage overview |
| `GET /api/{pid}/tokens/{file}` | Token data for a specific file (URL-encoded path) |
| `GET /api/{pid}/swarm` | Swarm status, tasks, and agents |
| `GET /api/{pid}/graph` | Dependency graph (nodes + edges) |
| `GET /api/{pid}/project` | Project metadata (rootPath, name, astIsStub) |

```bash
curl http://localhost:5555/api/my-app-1v7n98d/health
curl http://localhost:5555/api/my-app-1v7n98d/swarm
```

### Decisions

```bash
# Submit a decision response (e.g., from the dashboard UI)
curl -X POST http://localhost:5555/api/my-app-1v7n98d/decisions/dec-123 \
  -H "Authorization: Bearer mysecret" \
  -H "Content-Type: application/json" \
  -d '{"selectedOption": "approve"}'
```

This broadcasts a `decision-response` SSE event to all connected clients.

## SSE Events

Connect to the SSE stream at `/api/events`. Optionally filter by project:

```bash
# All events
curl -N http://localhost:5555/api/events

# Events for one project
curl -N http://localhost:5555/api/events?project=my-app-1v7n98d
```

On connect, the server sends a `connected` event with the current project list. Subsequent events include:

| Event Type | Trigger |
|------------|---------|
| `connected` | Initial connection (contains `{projects: [...]}`) |
| `state-update` | Any `/api/push` call (contains `{projectId, type, timestamp}`) |
| `project-registered` | New project registered |
| `project-unregistered` | Project removed |
| `decision-response` | Decision submitted from dashboard |
| Custom events | Any event name pushed via `/api/event` |

Heartbeat: the server sends a keep-alive comment every 15 seconds.

## WebSocket

Connect to `ws://localhost:5555/ws`. If auth is configured, pass the token as a query parameter:

```
ws://localhost:5555/ws?token=mysecret
```

### Inbound Messages (client -> hub)

Subscribe to topics:
```json
{"type": "subscribe", "topic": "project:my-app-1v7n98d:*"}
```

Unsubscribe:
```json
{"type": "unsubscribe", "topic": "project:my-app-1v7n98d:*"}
```

Publish (requires auth):
```json
{"type": "publish", "topic": "project:abc:task-update", "event": "progress", "data": {"percent": 50}}
```

### Topic Wildcards

Topics use `:` as a separator. A trailing `*` matches any suffix:

- `project:abc:*` matches `project:abc:file-change`, `project:abc:task-progress`, etc.
- `hub:*` matches all hub-level topics.
- Exact match: `hub:health` matches only `hub:health`.

### Welcome Message

On connect, the server sends:
```json
{"topic": "hub:health", "event": "connected", "data": {"clientId": "uuid", "authenticated": true}}
```

## Auth

When a token is configured (via `--token` or `HEX_DASHBOARD_TOKEN`):

- **GET** and **OPTIONS** requests are always allowed (no token needed).
- **POST** and **DELETE** require `Authorization: Bearer <token>` in the header.
- **WebSocket** publish requires `?token=<token>` in the connection URL. Subscribe/unsubscribe work without auth.
- If no token is configured, all requests are allowed.

## Architecture

```
src/
  main.rs              # CLI args, server setup, graceful shutdown
  state.rs             # AppState (projects, broadcast channels), ProjectEntry, request/response types
  daemon.rs            # Lock file write/remove, token generation
  embed.rs             # rust-embed for serving assets/index.html
  middleware/
    auth.rs            # Bearer token middleware (GET/OPTIONS bypass)
  routes/
    mod.rs             # Router construction, CORS, body limits
    projects.rs        # Register, unregister, list projects
    push.rs            # Push state updates and events from project adapters
    query.rs           # Per-project GET endpoints (health, tokens, swarm, graph)
    sse.rs             # SSE stream with project filtering and heartbeat
    ws.rs              # WebSocket handler with topic subscriptions and wildcards
    decisions.rs       # Decision response handler
```

State is held in-memory using `Arc<AppState>` with `RwLock<HashMap>` for projects. SSE and WebSocket fan-out use `tokio::sync::broadcast` channels (buffer size 256).

CORS is restricted to `localhost` and `127.0.0.1` origins.

## Dashboard UI

The HTML dashboard is embedded at compile time from the `assets/` directory using `rust-embed`. The root route (`GET /`) serves `assets/index.html`.

To update the dashboard:

1. Edit files in `hex-hub/assets/`
2. Rebuild: `cargo build --release`

The HTML is baked into the binary -- no external files needed at runtime.

## Replacing the Node Dashboard

hex-hub is a drop-in replacement for `src/adapters/primary/dashboard-hub.ts`. Migration notes:

| Aspect | Node (dashboard-hub.ts) | Rust (hex-hub) |
|--------|------------------------|----------------|
| Port | 5555 | 5555 (same default) |
| Token env var | `HEX_DASHBOARD_TOKEN` | `HEX_DASHBOARD_TOKEN` (same) |
| Project ID | `makeProjectId()` DJB2 | Identical algorithm, cross-tested |
| SSE | EventSource at `/api/events` | Same endpoint, same event format |
| WebSocket | Not supported | New addition |
| Binary size | ~50MB (Node + deps) | ~1.5MB (static binary) |
| Lock file | `~/.hex/daemon/hub.lock` | Same path, same JSON format |

The `DashboardAdapter` in hex's TypeScript code talks to hex-hub over HTTP. No changes are needed in the adapter -- the API contract is identical.

## Development

### Run tests

```bash
cargo test
```

Tests cover WebSocket topic matching and project ID parity with the TypeScript implementation.

### Add a new route

1. Create a handler in the appropriate file under `src/routes/` (or add a new file and declare it in `routes/mod.rs`).
2. Add the route in `routes/mod.rs` `build_router()`.
3. If the route accepts POST/DELETE, it is automatically covered by auth middleware.
4. If the route needs a body limit, wrap it with `.layer(DefaultBodyLimit::max(N))`.

### Modify state

Add fields to `ProjectState` in `state.rs`. Add a new match arm in `push.rs` `push_state()` to handle the new push type. Add a query handler in `query.rs` if the browser needs to read it.

### Logging

Set the `RUST_LOG` environment variable:

```bash
RUST_LOG=debug cargo run
RUST_LOG=hex_hub=trace cargo run
```
