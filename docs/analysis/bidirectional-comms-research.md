# Bidirectional Communication Research: WebSocket vs NATS vs Socket.io

## Context

hex's dashboard currently uses SSE (Server-Sent Events) for one-way server-to-client push. The system needs bidirectional communication so CLI agents and background worktrees can PUSH events TO the hub, not just receive them.

**Current architecture:**
- `DashboardHub` (multi-project broker) and `DashboardAdapter` (single-project) both use `node:http` with zero external deps
- SSE broadcasts via `Set<ServerResponse>` with project-scoped filtering
- `IEventBusPort` already defines a publish/subscribe domain event bus (in-process only)
- Only dependency: `@claude-flow/cli` and `tree-sitter-wasms`
- Agent-to-hub communication today: HTTP POST (request/response only, no streaming)

**Requirements:**
- CLI agents push events (task progress, file changes, decisions) to hub
- Multiple dashboards view the same project simultaneously
- Background worktree agents report progress back
- macOS + Linux, lightweight dev tool (not cloud infra)
- Must fit hexagonal architecture (adapter boundary)

---

## Option 1: WebSocket (`ws` library)

### How it works
Upgrade the existing `node:http` server to accept WebSocket connections alongside HTTP. Clients (both browser dashboards and CLI agents) connect via `ws://localhost:3847/ws`.

### Dependency weight
- `ws` package: ~73KB unpacked, zero native deps (pure JS fallback available)
- Already the most popular WebSocket library in Node.js (40M+ weekly downloads)
- Types: `@types/ws` available, excellent TypeScript support

### Operational complexity
- **No separate process** -- attaches to the existing HTTP server via `server.on('upgrade')`
- Zero configuration, zero external services
- Works immediately on any machine with Node.js

### Pub/sub patterns
- Must implement topic routing manually (trivial: `Map<topic, Set<WebSocket>>`)
- Project-scoped channels: `project:{id}:events`, `project:{id}:progress`
- Global channels: `hub:projects`, `hub:health`
- Message format: `{ topic: string, event: string, data: unknown }`

### Reconnection/reliability
- `ws` has no built-in reconnect -- client must implement (simple: exponential backoff, ~20 lines)
- No message persistence/replay (same as current SSE)
- Connection state is visible via `ws.readyState`

### Multi-client fan-out
- Iterate over subscriber set per topic, same pattern as current SSE broadcast
- `wss.clients` gives all connected clients
- Per-project filtering already proven in `DashboardHub.broadcastToProject()`

### Hex architecture mapping
```
Port:     IRealtimePort (new port in core/ports/)
            publish(topic, event, data): Promise<void>
            subscribe(topic, handler): Subscription

Adapter:  WebSocketAdapter (adapters/primary/)
            Implements IRealtimePort using `ws` library
            Attaches to existing HTTP server

            OR extends DashboardHub directly (co-located,
            since it already owns the HTTP server)
```

### Pros
- Minimal new dependency (1 package)
- Reuses existing HTTP server -- no new ports to open
- Browser-native WebSocket API on client side (no client library needed)
- Battle-tested, extremely mature
- Perfect fit for the "lightweight dev tool" requirement
- Bidirectional by nature -- agents connect and push, hub pushes back

### Cons
- Manual topic routing (but simple)
- No built-in message persistence
- No built-in clustering (irrelevant for dev tool)

---

## Option 2: NATS (`nats.js` + external `nats-server`)

### How it works
Run NATS server as a separate process. Hub, CLI agents, and dashboards all connect as NATS clients. Publish/subscribe on subjects like `hex.{projectId}.events`.

### Dependency weight
- `nats` package: ~320KB unpacked
- `nats-server` binary: ~20MB (must be installed separately via `brew install nats-server` or downloaded)
- No native Node.js deps in the client

### Operational complexity
- **Requires a separate process** -- `nats-server` must be running before hub starts
- Must manage lifecycle: start on `hex dashboard`, stop on shutdown
- Configuration file needed for auth, ports, clustering
- Developer must install `nats-server` or project must bundle/download it
- Significant onboarding friction for a dev tool

### Pub/sub patterns
- First-class subject-based routing: `hex.projectId.events.file-change`
- Wildcard subscriptions: `hex.*.events.>` (all events, all projects)
- Queue groups for load balancing (irrelevant here)
- Request/reply pattern built-in
- JetStream for persistence (overkill for this use case)

### Reconnection/reliability
- Excellent built-in reconnection with configurable backoff
- JetStream provides at-least-once delivery and replay
- Connection monitoring and health checks built-in

### Multi-client fan-out
- Native pub/sub fan-out -- all subscribers receive published messages
- No manual iteration needed
- Subject-based filtering is elegant

### Hex architecture mapping
```
Port:     IRealtimePort (same interface as WebSocket option)

Adapter:  NatsRealtimeAdapter (adapters/secondary/)
            Connects to nats-server as a client
            Translates DomainEvents to NATS subjects

Note:     NATS is a secondary (driven) adapter because it's
          an external service the app connects to, not a
          driving input.
```

### Pros
- Best-in-class pub/sub with wildcards and subject hierarchy
- Built-in reconnection and reliability
- Clean separation of concerns (message broker is external)
- Scales to many agents without connection management

### Cons
- **Requires separate process** -- major friction for a dev tool
- Must install `nats-server` binary (platform-specific)
- 320KB client + 20MB server binary
- Overkill for local-machine dev tool use case
- Browser cannot connect directly to NATS (needs WebSocket bridge anyway)
- Adds operational complexity that contradicts "lightweight" requirement

---

## Option 3: Socket.io

### How it works
Replace SSE with Socket.io server attached to the HTTP server. Provides WebSocket with automatic fallback to long-polling, rooms, and namespaces.

### Dependency weight
- `socket.io` server: ~1.8MB unpacked (pulls in `engine.io`, `socket.io-parser`, etc.)
- `socket.io-client`: ~720KB unpacked (needed by CLI agents)
- Total: ~2.5MB of dependencies
- Types included (written in TypeScript)

### Operational complexity
- **No separate process** -- attaches to existing HTTP server
- Zero configuration for basic use
- Slightly more complex than raw `ws` due to protocol layer

### Pub/sub patterns
- **Rooms**: `socket.join('project:abc')` then `io.to('project:abc').emit('event', data)`
- **Namespaces**: `/dashboard`, `/agent` for different client types
- Built-in broadcast, room-based fan-out, and acknowledgements
- Much richer than raw WebSocket, less powerful than NATS subjects

### Reconnection/reliability
- Excellent built-in reconnection with exponential backoff
- Connection state recovery (missed events replayed on reconnect)
- Heartbeat and timeout detection built-in
- Long-polling fallback for restrictive networks (irrelevant on localhost)

### Multi-client fan-out
- `io.to('room').emit()` -- zero manual iteration
- Rooms map perfectly to project IDs
- Adapter acknowledgement (`callback`) for reliable delivery

### Hex architecture mapping
```
Port:     IRealtimePort (same interface)

Adapter:  SocketIoAdapter (adapters/primary/)
            Co-located with DashboardHub
            Uses rooms for project scoping

Note:     Primary adapter because it drives the system
          (handles incoming connections and events)
```

### Pros
- Rooms/namespaces map perfectly to multi-project dashboard
- Built-in reconnection and state recovery
- TypeScript-native
- No manual topic routing needed
- Well-documented, large ecosystem

### Cons
- **2.5MB dependency footprint** -- heaviest option by far
- Requires Socket.io client library on both sides (browser + CLI agent)
- Long-polling fallback is wasted on localhost
- Abstraction layer over WebSocket adds complexity without benefit for this use case
- Non-standard protocol (not raw WebSocket -- harder to debug with standard tools)
- Proprietary framing means `wscat` and browser DevTools WS inspector show encoded frames

---

## Option 4: NATS Embedded (nats-server in-process)

### How it works
Embed a NATS server directly in the Node.js process using the NATS server's WebSocket transport, or spawn `nats-server` as a child process managed by the hub.

### Dependency weight
- NATS server is written in Go -- cannot embed in Node.js process
- Must spawn as child process or use WASM (no production WASM build exists)
- Same as Option 2 but with lifecycle management

### Operational complexity
- Must download/bundle `nats-server` binary per platform (macOS arm64, macOS x64, Linux x64, Linux arm64)
- Binary management adds significant packaging complexity
- Child process lifecycle: start, health check, graceful shutdown, crash recovery

### Assessment
**Not viable.** NATS cannot be embedded in a Node.js process. "Embedded" means spawning a child process, which gives all the downsides of Option 2 plus additional complexity of binary management. This contradicts the lightweight dev tool requirement.

---

## Option 5: Node.js Built-in Alternatives

### BroadcastChannel (node:worker_threads)
- Available since Node.js 18
- **Limitation**: Only works between threads in the SAME process
- Cannot communicate between the hub process and separate CLI agent processes
- Useless for the cross-process requirement

### worker_threads MessagePort
- Same limitation: intra-process only
- Would require hub and agents to run in the same Node.js process as worker threads
- Breaks the current architecture where CLI agents are separate processes

### node:net (TCP sockets)
- Raw TCP -- would need to implement framing, serialization, and protocol from scratch
- No browser support (dashboard can't use raw TCP)
- Worse than WebSocket in every way for this use case

### node:dgram (UDP)
- Unreliable transport -- messages can be lost
- No browser support
- Not suitable for event delivery

### Named Pipes / Unix Domain Sockets
- Agent-to-hub communication only (no browser support)
- Could supplement WebSocket for CLI-to-hub path (lower latency)
- Platform differences (Unix sockets vs Windows named pipes)
- Adds complexity for marginal benefit over WebSocket on localhost

### Assessment
**None of the built-in options solve the full problem.** The browser dashboard needs WebSocket (or SSE). Cross-process agent communication needs a network protocol. Built-in primitives are either intra-process or lack browser support.

---

## Recommendation: WebSocket (`ws` library)

### Rationale

| Criterion | ws | NATS | Socket.io | NATS embedded |
|-----------|-----|------|-----------|---------------|
| Dependency weight | 73KB | 320KB + 20MB server | 2.5MB | 320KB + 20MB + binary mgmt |
| Separate process | No | Yes | No | Yes (child) |
| Pub/sub built-in | Manual (trivial) | Excellent | Rooms (good) | Excellent |
| Reconnection | Manual (simple) | Built-in | Built-in | Built-in |
| Multi-client fan-out | Manual (trivial) | Native | Native | Native |
| TypeScript | @types/ws | Native | Native | Native |
| Browser support | Native WebSocket | Needs bridge | Needs client lib | Needs bridge |
| Hex fit | Primary adapter | Secondary adapter | Primary adapter | Secondary adapter |
| Operational burden | None | High | None | Very high |

**`ws` wins because:**

1. **Zero operational overhead**: Attaches to the existing HTTP server. No new processes, no new binaries, no new ports. This is a dev tool -- every extra process is friction.

2. **Minimal dependency**: 73KB, no native modules, no transitive deps that matter. The project currently has exactly 2 dependencies. Adding `ws` keeps it lean.

3. **Browser-native client**: The dashboard uses the browser's built-in `WebSocket` API. No client library needed. Socket.io requires its own client; NATS requires a WebSocket bridge.

4. **Proven pattern in DashboardHub**: The hub already implements topic-based fan-out with `broadcastToProject()`. Replacing `SSEClient.res.write()` with `ws.send()` is nearly mechanical. The `projectFilter` field on `SSEClient` becomes a topic subscription.

5. **Hex architecture fit**: WebSocket is a primary adapter (it drives the system by receiving incoming connections and events). It sits alongside the existing HTTP server in `adapters/primary/`. The `IEventBusPort` already defines the domain-side pub/sub contract -- the WebSocket adapter bridges external clients to internal domain events.

6. **CLI agent integration**: Agents connect via `new WebSocket('ws://localhost:3847/ws')` and push events with `ws.send(JSON.stringify({ topic, event, data }))`. Reconnection is ~20 lines of code with exponential backoff.

### Proposed Architecture

```
                    Browser Dashboard
                         |
                    WebSocket (native)
                         |
    ┌────────────────────┴────────────────────┐
    │           DashboardHub                   │
    │  ┌──────────────────────────────────┐   │
    │  │  HTTP Server (node:http)         │   │
    │  │    ├── REST API routes           │   │
    │  │    ├── SSE (deprecated, remove)  │   │
    │  │    └── WS upgrade handler        │   │
    │  │         └── WebSocketBroker      │   │
    │  │              ├── topic registry  │   │
    │  │              ├── client tracking │   │
    │  │              └── fan-out logic   │   │
    │  └──────────────────────────────────┘   │
    └────────────────────┬────────────────────┘
                         |
                    WebSocket (ws)
                         |
              ┌──────────┼──────────┐
              │          │          │
          CLI Agent  Worktree   Another
          (push)     Agent      Dashboard
                     (push)     (subscribe)
```

### New Port Interface

```typescript
// core/ports/realtime.ts
export interface IRealtimePort {
  /** Publish an event to a topic */
  publish(topic: string, event: string, data: unknown): Promise<void>;

  /** Subscribe to a topic pattern */
  subscribe(topic: string, handler: (event: string, data: unknown) => void): Subscription;

  /** Subscribe to all events on a topic pattern (wildcard) */
  subscribePattern(pattern: string, handler: (topic: string, event: string, data: unknown) => void): Subscription;
}

// Topic convention:
//   project:{id}:file-change
//   project:{id}:task-progress
//   project:{id}:agent-status
//   project:{id}:decision-request
//   hub:project-registered
//   hub:project-unregistered
```

### Migration Path

1. Add `ws` dependency (~73KB)
2. Create `IRealtimePort` in `core/ports/realtime.ts`
3. Create `WebSocketBroker` class (topic registry + fan-out) in `adapters/primary/`
4. Integrate into `DashboardHub.start()` via `server.on('upgrade', ...)`
5. Bridge `IEventBusPort` events to WebSocket topics (domain events auto-broadcast)
6. Update dashboard HTML to use `new WebSocket()` instead of `EventSource`
7. Add `connectToHub(url)` helper for CLI agents (with reconnection)
8. Deprecate and eventually remove SSE endpoint

### What NOT to do

- Do not add NATS for a single-machine dev tool. It's infrastructure for distributed systems.
- Do not add Socket.io. The abstraction layer and 2.5MB footprint are not justified when raw WebSocket covers every requirement.
- Do not try to use `BroadcastChannel` or `worker_threads`. They are intra-process only and cannot solve cross-process communication.
- Do not implement a custom TCP protocol. WebSocket already is that, with browser support.
