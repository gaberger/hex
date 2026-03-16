# hex-hub Rust Service Audit Report

## 1. Architecture

**Verdict: Clean, but not hexagonal.**

The Rust service is a well-structured axum application with clear module separation:
- `state.rs` -- shared types and state (domain-like)
- `routes/` -- HTTP handlers (primary adapter)
- `middleware/` -- cross-cutting concerns
- `daemon.rs` -- process lifecycle
- `embed.rs` -- static asset serving

However, it does NOT follow hex architecture internally. There are no port traits, no secondary adapters, and no composition root. Routes directly manipulate `AppState` via `Arc<RwLock<...>>`. This is acceptable for a small infrastructure service -- hex architecture would be over-engineering here. The service is itself an adapter in the larger hex system.

## 2. Route Completeness (vs. Node dashboard-hub.ts)

| Endpoint | Node (TS) | Rust | Notes |
|----------|-----------|------|-------|
| `GET /` | Yes | Yes | Static HTML. Rust uses rust-embed; TS reads from filesystem |
| `GET /api/projects` | Yes | Yes | Identical semantics |
| `POST /api/projects/register` | Yes | Yes | Identical semantics |
| `DELETE /api/projects/:id` | Yes | Yes | Identical semantics |
| `POST /api/push` | Yes | Yes | Identical state types (health, tokens, tokenFile, swarm, graph, project) |
| `POST /api/event` | Yes | Yes | Identical semantics |
| `GET /api/events` (SSE) | Yes | Yes | Both support `?project=` filter |
| `GET /api/:id/health` | Yes | Yes | Same default fallback JSON |
| `GET /api/:id/tokens/overview` | Yes | Yes | Same default fallback |
| `GET /api/:id/tokens/:file` | Yes | Yes | Both URL-decode the file path |
| `GET /api/:id/swarm` | Yes | Yes | Same default fallback |
| `GET /api/:id/graph` | Yes | Yes | Same default fallback |
| `GET /api/:id/project` | Yes | Yes | Same fallback to entry metadata |
| `POST /api/:id/decisions/:id` | Yes | Yes | Identical broadcast logic |
| `GET /ws` (WebSocket) | No | Yes | **Rust adds WebSocket support not in Node** |

**Result: 100% route parity with Node, plus WebSocket as a bonus.**

## 3. State Management

- **Storage**: In-memory `HashMap<String, ProjectEntry>` behind `tokio::sync::RwLock`. No persistence.
- **Thread safety**: Correct. `RwLock` allows concurrent reads, exclusive writes. Wrapped in `Arc` for shared ownership across handlers.
- **Memory leaks**: Potential issue -- `token_files` HashMap grows unboundedly per project. Each `tokenFile` push inserts a new key with no eviction. Over a long session with many files, this could consume significant memory. The Node version has the same issue.
- **Broadcast channels**: SSE uses `broadcast::channel(256)`, WS uses `broadcast::channel(256)`. If a slow consumer lags behind 256 messages, it gets `RecvError::Lagged` -- correctly handled with `continue` (skip missed events). This is a reasonable trade-off.

## 4. SSE Implementation

**Verdict: Well implemented.**

- Sends initial `connected` event with project list on connection -- matches Node behavior.
- Project filter via `?project=` query param works correctly.
- Global events (project_id = None) always pass the filter -- correct.
- 15-second keepalive heartbeat -- matches Node's 15s interval.
- Uses `async_stream` crate for ergonomic stream construction.
- Lagged receivers skip missed messages rather than disconnecting -- good resilience.
- No reconnection ID (`Last-Event-ID`) support -- same as Node, acceptable.

## 5. WebSocket Implementation

**Verdict: Fully functional, well-designed.**

This is NEW functionality not present in the Node version. Features:

- Token-based auth via `?token=` query parameter.
- Welcome message with client ID and auth status.
- Topic-based pub/sub with subscribe/unsubscribe commands.
- Wildcard topic matching (`project:abc:*` matches `project:abc:file-change`).
- Publish command with auth gating (unauthenticated clients cannot publish when auth is configured).
- Clean two-task architecture: one task for sending (broadcast -> client), one for receiving (client -> broadcast). `tokio::select!` ensures both tasks are cleaned up.
- Good test coverage for topic matching logic.

**Issue**: Error messages cannot be sent back to the client on invalid JSON or unauthorized publish -- the sender half is moved into the send task. Comment at line 100-101 acknowledges this. Not a bug, but limits debuggability for clients.

## 6. Auth Middleware

**Verdict: Correct and consistent with Node.**

- GET and OPTIONS bypass auth -- matches Node behavior.
- Bearer token comparison via exact string match.
- Token sourced from `--token` CLI arg or `HEX_DASHBOARD_TOKEN` env var.
- When no token is configured, all requests pass -- matches Node.
- Applied as axum middleware layer, runs before route handlers.
- WebSocket auth is separate (query param `?token=`) since WS upgrade is a GET.

**Note**: Token comparison uses `==` (not constant-time comparison). This is acceptable for a localhost-only service but would be a timing side-channel in a public-facing service.

## 7. Error Handling

**Verdict: Good -- no panics in request handlers.**

- All route handlers return `(StatusCode, Json<...>)` tuples -- no unwrap/panic in hot paths.
- `main.rs` uses `expect()` on bind and serve -- acceptable for startup failures.
- Serde deserialization failures on push/event bodies return appropriate error responses.
- Unknown push types return 400 Bad Request.
- Unregistered project IDs return 404.
- Broadcast send errors (`let _ = state.sse_tx.send(...)`) are silently ignored -- correct, since no subscribers is not an error.

## 8. Static File Serving

- Uses `rust-embed` crate to embed `assets/index.html` at compile time into the binary.
- Single file: `assets/index.html`.
- Only serves the index -- no CSS/JS/image assets served. If the dashboard HTML references external assets, they will 404. The Node version also only serves `index.html` (the dashboard is a single self-contained HTML file).
- Falls back to 500 "Dashboard HTML not found" if embedding fails.

## 9. Daemon Mode

**Verdict: Partial -- flag is parsed but no actual daemonization.**

- `--daemon` flag is parsed in `main.rs` (line 22) but only affects the log message ("daemon started" vs "running").
- No `fork()`, `setsid()`, or `nohup` behavior. The process does NOT detach from the terminal.
- Lock file written to `~/.hex/daemon/hub.lock` with PID, port, token, timestamp, version.
- Lock file removed on graceful shutdown (SIGINT/SIGTERM).
- `generate_token()` creates a random 32-hex-char token when none is provided.

**Gap**: The Node version's `daemon-manager.ts` handles spawning hex-hub as a background process. The Rust binary itself does not self-daemonize. This is likely by design -- the caller (daemon-manager) handles backgrounding.

## 10. Gaps and Issues

### Missing vs. Node

1. **No actual daemonization** -- `--daemon` is cosmetic. The Node daemon-manager handles this externally, so this may be intentional.
2. **No request body size validation on register** -- Uses `SMALL_BODY_LIMIT` (4KB) via `DefaultBodyLimit` layer, which is correct. No issue here.

### Potential Issues

1. **DJB2 hash cross-language compatibility**: The `make_project_id` function has a thorough test suite with cross-language vectors (line 169-185). This is critical -- if Rust and TypeScript disagree on project IDs, registration breaks. The test vectors look correct.

2. **Unbounded token_files growth**: Each `tokenFile` push adds to a HashMap that never evicts. In a long-running session analyzing hundreds of files, memory grows monotonically. Consider adding an LRU eviction policy or max entry count.

3. **No HTTPS/TLS**: Binds to `127.0.0.1` only (localhost), so this is acceptable. Traffic never leaves the machine.

4. **No graceful WebSocket shutdown**: On server shutdown, SSE clients get their connections closed, but WebSocket clients are not explicitly notified. The broadcast channel closing will cause the send task to break, which will abort the recv task -- so cleanup does happen, just without a close frame.

5. **No request logging middleware**: Only tracing for startup/shutdown. Consider adding tower-http `TraceLayer` for request-level logging.

6. **Release profile is aggressive**: `opt-level = "z"` (size optimization), `lto = "fat"`, `codegen-units = 1`, `strip = true`, `panic = "abort"`. This maximizes binary size reduction but makes debugging release builds impossible and increases compile times significantly. Good for distribution, bad for development.

### Additions vs. Node

1. **WebSocket support** -- Full pub/sub with topic matching. Not in Node version.
2. **Body size limits per route** -- 256KB for push, 16KB for events, 4KB for register/decisions. Node only had per-handler `readBody` limits.
3. **Embedded assets** -- Dashboard HTML compiled into the binary. No filesystem dependency at runtime.

## Summary

The hex-hub Rust service is a well-implemented, production-quality replacement for the Node dashboard-hub. It achieves 100% API parity and adds WebSocket support. Code quality is high: no panics in handlers, correct concurrency primitives, good test coverage for critical cross-language compatibility. The main gaps are cosmetic (no request logging) or by-design (daemonization handled externally). The unbounded token_files growth is the only architectural concern worth addressing.

**Recommendation: Ship it.** Address token_files eviction before long-running production use.
