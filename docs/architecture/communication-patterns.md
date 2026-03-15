# Cross-Language Communication Patterns for Hexagonal Architecture

> Reference guide for the `dependency-analyst` agent when recommending IPC/API patterns between hex-intf components across TypeScript, Go, and Rust boundaries.

---

## Overview

In a multi-language hexagonal architecture, the **port interface** defines the contract and the **adapter** implements the transport. Domain logic never knows whether `ILLMPort.prompt()` crosses a REST boundary, a gRPC channel, or a WASM bridge. This document catalogs seven communication patterns, their hex-intf mappings, and when to recommend each.

---

## 1. REST API

### When to Use

- Frontend (TypeScript/React/Svelte) talks to backend (Go/Rust) for CRUD operations.
- Stateless request-response workflows: slide generation, document conversion, batch processing.
- External API integration where the provider only offers HTTP (e.g., LLM providers, cloud storage).
- Low-frequency operations where latency tolerance is > 50ms.

### Hex-intf Mapping

- **Port**: Any output port that represents an external service call. `ILLMPort`, `IStoragePort`, `IBuildPort` (remote build servers).
- **Primary adapter**: `HTTPAdapter` (driving) exposes domain use cases as REST endpoints.
- **Secondary adapter**: `RestClientAdapter` (driven) implements output ports by calling external REST APIs.
- **Structure**: The adapter owns serialization (JSON/msgpack). The port interface uses domain types only.

```
Primary:  HTTPAdapter  -->  ICodeGenerationPort  -->  Domain
Secondary: Domain  -->  ILLMPort  -->  RestLLMAdapter  -->  api.anthropic.com
```

### Language Pairings

| Client | Server | Notes |
|--------|--------|-------|
| TypeScript | Go | Excellent. Go's `net/http` + `chi`/`echo` are mature. TS has `fetch`/`axios`. |
| TypeScript | Rust | Good. Rust's `axum`/`actix-web` are performant but longer compile times. |
| Go | Rust | Viable but unusual. Prefer gRPC for service-to-service. |

### Token Efficiency

High. REST endpoints summarize well at L2:

```
POST /api/v1/generate { spec: Specification, lang: Language } -> CodeUnit
GET  /api/v1/summary/:fileId -> ASTSummary
```

An LLM can reason about REST contracts from OpenAPI specs or L2 route summaries with minimal tokens.

### Testability (London School)

Straightforward. Mock the HTTP client at the adapter boundary:

```typescript
// Unit test: mock the port, not the HTTP layer
const mockLLM: ILLMPort = {
  prompt: vi.fn().mockResolvedValue({ code: "function hello() {}" }),
  streamPrompt: vi.fn(),
};
const useCase = new GenerateCode(mockLLM, mockAST);
```

For integration tests, use `msw` (TypeScript), `httptest` (Go), or `wiremock` (Rust) to stub HTTP responses.

### Example

**TS React frontend communicating with Go workplan service:**

```
React UI  --[POST /api/workplan]--> Go HTTPAdapter --> IWorkplanPort --> Domain
```

The Go adapter deserializes JSON into domain types, calls the use case, and returns the result. The React app never knows the backend language.

### Tradeoffs

| Dimension | Rating | Notes |
|-----------|--------|-------|
| Latency | Medium | 1-50ms local, 50-500ms remote. HTTP overhead per request. |
| Complexity | Low | Every language has excellent HTTP libraries. |
| Debugging | Easy | `curl`, browser devtools, structured logs. |
| Deployment | Low | Standard load balancers, reverse proxies, CDNs. |

---

## 2. WebSocket

### When to Use

- Real-time bidirectional communication: multiplayer games, collaborative editors, live dashboards.
- Server-push scenarios: build progress streaming, LLM token streaming, live test results.
- Persistent connections where HTTP polling would waste resources.
- Latency-sensitive interactions requiring < 16ms frame times (games).

### Hex-intf Mapping

- **Port**: `IStreamPort` or event-based ports. `ILLMPort.streamPrompt()` naturally maps to WebSocket.
- **Primary adapter**: `WebSocketAdapter` (driving) receives commands and pushes events.
- **Secondary adapter**: Domain events are published through a `IEventPort` that the WebSocket adapter subscribes to.
- **Structure**: The adapter manages connection lifecycle (connect, reconnect, heartbeat). The port sees `AsyncGenerator<T>` or event callbacks.

```
Browser  <--[ws://]--> WebSocketAdapter --> ICodeGenerationPort --> Domain
                                                                      |
                                                    Domain Events --> IEventPort --> WebSocketAdapter --> Browser
```

### Language Pairings

| Client | Server | Notes |
|--------|--------|-------|
| TypeScript (browser) | Go | Best choice. Go's goroutine-per-connection model handles thousands of WS connections efficiently. `gorilla/websocket` or `nhooyr/websocket`. |
| TypeScript (browser) | Rust | Excellent for high-connection-count servers. `tokio-tungstenite` with async runtime. |
| TypeScript (Node) | Go/Rust | For tool backends. `ws` library in Node. |

### Token Efficiency

Moderate. WebSocket message schemas summarize well, but the connection lifecycle (reconnection, heartbeat, error recovery) adds complexity that bloats L2 summaries. Recommend documenting the message protocol separately:

```
WS Message Protocol:
  -> { type: "generate", payload: Specification }
  <- { type: "progress", payload: { step: number, total: number } }
  <- { type: "result", payload: CodeUnit }
  <- { type: "error", payload: { code: string, message: string } }
```

### Testability (London School)

Mock the WebSocket as a port interface. Never test WebSocket protocol details in unit tests:

```typescript
// The port hides the transport
const mockStream: IStreamPort = {
  subscribe: vi.fn().mockReturnValue(asyncGenerator([
    { type: "progress", step: 1 },
    { type: "result", code: "..." },
  ])),
};
```

Integration tests use in-process WebSocket servers (`ws` in Node, `httptest` + upgrade in Go).

### Example

**TS React frontend communicating with Go game server for real-time multiplayer:**

```
React Game UI  <--[ws://game.example.com/play]--> Go WebSocketAdapter
  -> { type: "move", position: { x: 3, y: 7 } }
  <- { type: "state", board: [...], turn: "player2" }
  <- { type: "opponent_move", position: { x: 1, y: 2 } }
```

### Tradeoffs

| Dimension | Rating | Notes |
|-----------|--------|-------|
| Latency | Low | Sub-millisecond after connection established. No HTTP overhead per message. |
| Complexity | Medium | Connection management, reconnection logic, state synchronization. |
| Debugging | Medium | Browser WS inspector helps. Harder to replay than HTTP. |
| Deployment | Medium | Sticky sessions or connection-aware load balancing required. |

---

## 3. gRPC

### When to Use

- Typed service mesh between Go and Rust microservices.
- High-throughput internal communication (> 10k requests/second between services).
- Bidirectional streaming between backend services (e.g., build orchestration streaming results).
- When you need code-generated clients/servers with guaranteed type compatibility across languages.
- NOT for browser-to-server (requires grpc-web proxy).

### Hex-intf Mapping

- **Port**: Any output port calling another hex-intf service. `IBuildPort` calling a remote build farm, `IASTPort` calling a tree-sitter service.
- **Adapter**: `GrpcClientAdapter` (driven) wraps generated stubs. `GrpcServerAdapter` (driving) exposes use cases.
- **Structure**: `.proto` files define the contract. Generated code implements the serialization. The adapter translates between protobuf types and domain types.

```
Go Service --> IBuildPort --> GrpcBuildAdapter --> [protobuf/HTTP2] --> Rust Build Service
                                                                          |
                                                          GrpcServerAdapter --> IBuildPort --> Domain
```

### Language Pairings

| Client | Server | Notes |
|--------|--------|-------|
| Go | Rust | Ideal. Both have excellent gRPC support (`tonic` for Rust, `google.golang.org/grpc`). |
| Go | Go | Common for Go microservice meshes. |
| Rust | Rust | Works but may be overkill for same-language. |
| TypeScript | Go/Rust | Only via `grpc-web` or `connect-es`. Adds proxy complexity. Prefer REST for browser clients. |

### Token Efficiency

High. Protobuf definitions are extremely L2-friendly:

```protobuf
service BuildService {
  rpc Compile(CompileRequest) returns (BuildResult);
  rpc StreamTest(TestRequest) returns (stream TestEvent);
}
```

LLMs reason well about `.proto` files. The generated code is boilerplate and should not be included in L2 summaries.

### Testability (London School)

Mock at the port boundary, not the gRPC layer:

```go
// Go: mock the port interface
type MockBuildPort struct {
    CompileFunc func(ctx context.Context, project Project) (BuildResult, error)
}
func (m *MockBuildPort) Compile(ctx context.Context, p Project) (BuildResult, error) {
    return m.CompileFunc(ctx, p)
}
```

For integration tests, use `bufconn` (Go) for in-process gRPC or start a test server on a random port.

### Example

**Go orchestrator calling Rust build service for cross-compilation:**

```
Go Orchestrator --> IBuildPort --> GrpcBuildClient --> [h2] --> Rust GrpcBuildServer --> Cargo
  CompileRequest { project_id: "abc", target: "wasm32" }
  <- BuildResult { success: true, artifacts: ["output.wasm"], duration_ms: 4200 }
```

### Tradeoffs

| Dimension | Rating | Notes |
|-----------|--------|-------|
| Latency | Low | HTTP/2 multiplexing, binary protobuf. Typically < 5ms local. |
| Complexity | Medium-High | Proto compilation step, generated code management, versioning. |
| Debugging | Hard | Binary protocol. Need `grpcurl` or `grpc-cli`. Not browser-inspectable. |
| Deployment | Medium | Requires HTTP/2-aware infrastructure. Service mesh (Istio/Linkerd) helps. |

---

## 4. NATS/JetStream

### When to Use

- Event-driven architectures where multiple consumers react to the same event.
- Decoupled adapters that should not know about each other (e.g., `CodeGenerated` event triggers both testing and documentation).
- Workflow orchestration with durable message delivery (JetStream).
- Fan-out patterns: one event, many handlers.
- Cross-service domain events in a distributed hex-intf deployment.

### Hex-intf Mapping

- **Port**: `IEventPort` for publishing domain events. `IEventSubscriptionPort` for consuming.
- **Adapter**: `NatsPublisherAdapter` publishes domain events as NATS messages. `NatsSubscriberAdapter` receives and dispatches to use cases.
- **Structure**: Domain events are the messages. The adapter handles serialization (JSON or protobuf) and subject mapping.

```
Domain --> CodeGenerated event --> IEventPort --> NatsPublisherAdapter --> NATS
                                                                           |
  NATS --> NatsSubscriberAdapter --> ITestTriggerPort --> RunTestsUseCase
  NATS --> NatsSubscriberAdapter --> IDocGenPort --> GenerateDocsUseCase
```

### Language Pairings

| Publisher | Subscriber | Notes |
|-----------|------------|-------|
| Go | Go/Rust/TS | NATS is written in Go. Go client is first-class. |
| Rust | Go/TS | `async-nats` crate is well-maintained. |
| TypeScript | Go/Rust | `nats.js` works in Node and Deno. |
| Any | Any | NATS is language-agnostic by design. All three languages have mature clients. |

### Token Efficiency

Moderate. Subject hierarchies and message schemas summarize well, but the subscription/consumer group configuration adds boilerplate:

```
NATS Subjects:
  hexintf.events.code.generated    -> { projectId, codeUnit, language }
  hexintf.events.build.completed   -> { projectId, result, duration }
  hexintf.events.test.failed       -> { projectId, failures[] }
```

### Testability (London School)

Mock the event port. Never bring NATS into unit tests:

```typescript
const mockEventPort: IEventPort = {
  publish: vi.fn().mockResolvedValue(undefined),
};
const useCase = new GenerateCode(mockLLM, mockAST, mockEventPort);
await useCase.execute(spec);
expect(mockEventPort.publish).toHaveBeenCalledWith(
  expect.objectContaining({ type: "CodeGenerated" })
);
```

Integration tests use NATS test server (`nats-server -p 0` for random port).

### Example

**Decoupled build pipeline with event-driven triggers:**

```
Go CodeGen Service publishes:  hexintf.events.code.generated
  -> Rust Build Service subscribes, compiles, publishes: hexintf.events.build.completed
  -> TS Dashboard subscribes to build.completed, updates UI
  -> Go Test Service subscribes to code.generated, runs tests
```

### Tradeoffs

| Dimension | Rating | Notes |
|-----------|--------|-------|
| Latency | Low-Medium | ~1ms for pub/sub. JetStream adds persistence overhead. |
| Complexity | Medium | Requires NATS server infrastructure. Subject design matters. |
| Debugging | Medium | NATS CLI tools help. Harder to trace causal chains across services. |
| Deployment | Medium | NATS cluster needed for production. JetStream for durability. |

---

## 5. WASM Bridge

### When to Use

- Running Rust or Go logic inside a TypeScript runtime (browser or Node.js).
- Client-side computation: AST parsing, code validation, encryption, image processing.
- Sharing domain logic across server (native) and client (WASM) without duplication.
- When network latency to a backend service is unacceptable for interactive features.
- Sandboxed execution of untrusted code.

### Hex-intf Mapping

- **Port**: Any port whose implementation can be compiled to WASM. `IASTPort` (tree-sitter runs as WASM in browser), `IValidationPort`.
- **Adapter**: `WasmASTAdapter` loads the `.wasm` module and exposes it through the port interface. The TypeScript caller sees a normal async interface.
- **Structure**: The Rust/Go code compiles to WASM with `wasm-bindgen` (Rust) or `GOOS=js GOARCH=wasm` (Go). The TS adapter wraps the WASM instantiation.

```
TS Application --> IASTPort --> WasmASTAdapter --> [wasm-bindgen] --> Rust tree-sitter (compiled to WASM)
```

### Language Pairings

| WASM Source | Host | Notes |
|-------------|------|-------|
| Rust | TypeScript (browser) | Best. `wasm-bindgen` + `wasm-pack` provide excellent DX. Small binaries. |
| Rust | TypeScript (Node) | Good. Use `@aspect-build/rules_js` or direct `wasm-pack`. |
| Go | TypeScript (browser) | Works but larger binaries (~2MB+ minimum). Improving with TinyGo. |
| Rust | Go | Possible via `wasmtime` or `wasmer` as Go host. Niche use case. |

### Token Efficiency

Low-Moderate. The WASM boundary introduces binding code that does not summarize well at L2. The port interface itself is clean, but the adapter internals (`wasm-bindgen` annotations, memory management) are verbose:

```
// L2 summary is clean:
WasmASTAdapter implements IASTPort
  + extractSummary(filePath: string): Promise<ASTSummary>

// But the adapter internals require understanding WASM memory model
// Recommend: keep adapter thin, push logic into Rust, summarize Rust at L2 separately
```

### Testability (London School)

Mock at the port boundary. The WASM module is an implementation detail:

```typescript
// Unit test: mock the port
const mockAST: IASTPort = {
  extractSummary: vi.fn().mockResolvedValue({
    exports: [{ name: "GitAdapter", kind: "class" }],
    imports: ["IGitPort"],
    lines: 187,
  }),
  diffStructural: vi.fn(),
};
```

Integration tests load the actual WASM module. Test the Rust code natively with `cargo test` (faster than WASM roundtrip).

### Example

**Browser-side AST parsing for live code preview:**

```
React Editor  --> IASTPort --> WasmASTAdapter --> tree-sitter.wasm (compiled from Rust)
  User types code -> extractSummary() runs in-browser -> instant L2 preview
  No network round-trip. ~5ms for a 500-line file.
```

### Tradeoffs

| Dimension | Rating | Notes |
|-----------|--------|-------|
| Latency | Very Low | No network. ~1-10ms function calls. Memory copy overhead for large data. |
| Complexity | High | WASM build toolchain, memory management, async bridging. |
| Debugging | Hard | Source maps partially work. `console.log` from WASM is limited. |
| Deployment | Low | Ship `.wasm` file as static asset. No server infrastructure. |

---

## 6. FFI (C ABI)

### When to Use

- Calling a Rust library from Go or Node.js for performance-critical operations.
- Reusing an existing Rust crate without running a separate service.
- Zero-copy data sharing between languages in the same process.
- When network overhead (even localhost) is unacceptable: real-time audio, video processing, cryptographic operations.

### Hex-intf Mapping

- **Port**: Performance-critical output ports. `ICryptoPort`, `IParserPort`, `ICompressionPort`.
- **Adapter**: `FFIParserAdapter` uses `node-ffi-napi` (Node.js) or `cgo` (Go) to call Rust's C-ABI exports.
- **Structure**: Rust exposes `#[no_mangle] extern "C"` functions. The adapter in Go/Node translates between C types and domain types.

```
Go Application --> IParserPort --> FFIParserAdapter --> [C ABI] --> Rust libparser.so
Node Application --> IParserPort --> NativeParserAdapter --> [N-API] --> Rust libparser.node
```

### Language Pairings

| Caller | Callee | Notes |
|--------|--------|-------|
| Go | Rust (via C ABI) | Works via `cgo`. Rust compiles to `cdylib`. Beware cgo overhead per call. |
| Node.js | Rust (via N-API) | Use `napi-rs` for ergonomic bindings. Better than raw `node-ffi-napi`. |
| Node.js | Rust (via C ABI) | Use `node-ffi-napi` or `koffi`. More fragile than N-API. |
| Go | Go | Not applicable (same language). |
| Rust | C libraries | Straightforward with `bindgen`. Common for system libraries. |

### Token Efficiency

Low. FFI boundaries are noisy. C ABI signatures, unsafe blocks, memory management, and type conversion code do not summarize cleanly:

```
// Rust side: hard to summarize at L2
#[no_mangle]
pub extern "C" fn parse_file(path_ptr: *const c_char, path_len: usize) -> *mut ParseResult { ... }

// Recommendation: summarize the PORT interface at L2, not the FFI adapter
// The FFI adapter is inherently L3-only for editing purposes
```

### Testability (London School)

Always mock at the port boundary. Never test FFI mechanics in unit tests:

```go
// Go: mock the port
type MockParser struct{}
func (m *MockParser) ParseFile(path string) (*ParseResult, error) {
    return &ParseResult{Exports: []string{"main"}}, nil
}
```

Test the Rust library natively with `cargo test`. Test the FFI bridge in integration tests with the actual shared library loaded.

### Example

**Node.js CLI tool using Rust for fast AST parsing:**

```
Node CLI --> IASTPort --> NapiASTAdapter --> [N-API] --> Rust tree-sitter (native speed)
  Parse 1000 files: ~200ms (native) vs ~2000ms (pure JS tree-sitter WASM)
```

### Tradeoffs

| Dimension | Rating | Notes |
|-----------|--------|-------|
| Latency | Very Low | In-process calls. Nanosecond overhead per call (minus cgo overhead for Go). |
| Complexity | Very High | Unsafe code, memory ownership across boundaries, build system integration. |
| Debugging | Very Hard | Crashes at FFI boundary are hard to diagnose. ASAN/Valgrind needed. |
| Deployment | High | Must ship platform-specific binaries. Cross-compilation required. |

---

## 7. Unix Domain Sockets

### When to Use

- Same-machine IPC between colocated services that need higher throughput than REST.
- Sidecar pattern: a Rust service running alongside a Go service on the same host.
- Development mode: connect local services without network configuration.
- When you want filesystem-level access control (socket file permissions).
- Process isolation is needed but network overhead is not acceptable.

### Hex-intf Mapping

- **Port**: Any output port calling a colocated service. `IBuildPort` calling a local compiler daemon, `IASTPort` calling a local tree-sitter server.
- **Adapter**: `UnixSocketBuildAdapter` connects to `/var/run/hex-intf/build.sock`. Protocol on top can be JSON-RPC, newline-delimited JSON, or protobuf.
- **Structure**: The adapter manages the socket connection. The port interface is identical to a REST or gRPC version -- only the adapter changes.

```
Go Orchestrator --> IBuildPort --> UnixSocketBuildAdapter --> /var/run/build.sock --> Rust Build Daemon
```

### Language Pairings

| Client | Server | Notes |
|--------|--------|-------|
| Go | Rust | Natural. Both handle Unix sockets natively (`net.Dial("unix", ...)` / `tokio::net::UnixListener`). |
| TypeScript (Node) | Rust/Go | `net.createConnection({ path: ... })` in Node. Works well. |
| Any | Any | Unix sockets are language-agnostic. Protocol on top determines compatibility. |

### Token Efficiency

Moderate. The socket path and protocol are simple, but the connection management code adds some noise. Recommend using a standard protocol (JSON-RPC 2.0) on top for L2-friendly summaries:

```
Unix Socket: /var/run/hex-intf/build.sock
Protocol: JSON-RPC 2.0
Methods:
  compile({ projectId, target }) -> { success, artifacts[], duration }
  lint({ projectId }) -> { errors[], warnings[] }
```

### Testability (London School)

Mock at the port boundary. For integration tests, create a temporary socket:

```typescript
const mockBuild: IBuildPort = {
  compile: vi.fn().mockResolvedValue({ success: true, artifacts: ["out.js"] }),
  lint: vi.fn().mockResolvedValue({ errors: [], warnings: [] }),
};
```

```go
// Integration test: create temp socket
tmpDir := t.TempDir()
sockPath := filepath.Join(tmpDir, "test.sock")
listener, _ := net.Listen("unix", sockPath)
defer listener.Close()
```

### Example

**Local development: Go orchestrator with Rust build daemon as sidecar:**

```
Go Dev Server --> IBuildPort --> UnixSocketAdapter --> /tmp/hex-build.sock --> Rust cargo-daemon
  Saves ~5ms per request vs localhost TCP.
  Socket permissions restrict access to current user.
```

### Tradeoffs

| Dimension | Rating | Notes |
|-----------|--------|-------|
| Latency | Very Low | No TCP/IP stack. ~0.1ms per message. |
| Complexity | Low-Medium | Simple socket API. Need to choose a wire protocol. |
| Debugging | Medium | `socat` for debugging. No browser tools. |
| Deployment | Low | No network config. But single-machine only. |

---

## Decision Matrix

| Pattern | Latency | Complexity | LLM-Friendly (L2) | Best For |
|---------|---------|------------|-------------------|----------|
| **REST API** | Medium (1-500ms) | Low | High | Frontend-backend CRUD, external APIs, simple integrations |
| **WebSocket** | Low (<1ms msg) | Medium | Moderate | Real-time games, live collaboration, streaming LLM output |
| **gRPC** | Low (<5ms) | Medium-High | High (proto files) | Go-Rust service mesh, high-throughput internal APIs |
| **NATS/JetStream** | Low-Medium | Medium | Moderate | Event-driven workflows, fan-out, decoupled adapters |
| **WASM Bridge** | Very Low (<10ms) | High | Low-Moderate | Client-side computation, shared domain logic, sandboxing |
| **FFI (C ABI)** | Very Low (ns) | Very High | Low | Performance-critical single-process, native library reuse |
| **Unix Socket** | Very Low (<1ms) | Low-Medium | Moderate | Same-machine sidecar, local dev, high-throughput IPC |

### Quick Selection Guide

```
Need browser support?
  Yes -> REST (CRUD) or WebSocket (real-time)
  No  -> Continue

Same machine?
  Yes -> Unix Socket (simple) or FFI (max perf) or WASM (sandboxed)
  No  -> Continue

Need streaming?
  Yes -> gRPC (typed) or WebSocket (browser-compatible) or NATS (fan-out)
  No  -> REST (simple) or gRPC (typed)

Event-driven / multiple consumers?
  Yes -> NATS/JetStream
  No  -> gRPC (point-to-point)
```

---

## Hybrid Approaches

Real projects rarely use a single communication pattern. Here are common combinations for hex-intf projects:

### Pattern 1: REST + WebSocket + WASM (Interactive Application)

**Scenario**: A code editor with live preview.

```
Browser:
  React UI --[REST]--> Go API Server          (file CRUD, project management)
  React UI <--[WS]---> Go API Server          (live build status, error streaming)
  React UI --[WASM]--> Rust tree-sitter       (client-side AST parsing, instant feedback)
```

- REST handles stateless CRUD operations.
- WebSocket pushes real-time build results and collaborative editing events.
- WASM runs tree-sitter in the browser for sub-10ms AST extraction without server round-trips.

**Hex-intf port mapping**: All three transports implement different adapters for the same ports. `IASTPort` has both a `WasmASTAdapter` (client-side) and a `RestASTAdapter` (server-side fallback).

### Pattern 2: REST + gRPC + NATS (Microservice Backend)

**Scenario**: A distributed build and test orchestration platform.

```
External:
  TS Frontend --[REST]--> Go API Gateway      (user-facing API)

Internal:
  Go API Gateway --[gRPC]--> Rust Build Service    (compile, lint)
  Go API Gateway --[gRPC]--> Go Test Service       (test execution)

Events:
  Rust Build Service --[NATS]--> hexintf.build.completed
  Go Test Service subscribes to hexintf.build.completed --> runs tests
  TS Dashboard subscribes to hexintf.*.completed --> updates UI via WS
```

- REST for the external boundary (simple, cacheable, browser-friendly).
- gRPC for the internal service mesh (typed, fast, streaming).
- NATS for event-driven coordination (decoupled, fan-out).

### Pattern 3: FFI + Unix Socket (High-Performance Local)

**Scenario**: A CLI tool that needs both native-speed parsing and a long-running daemon.

```
Node CLI --[N-API/FFI]--> Rust parser library     (in-process AST extraction)
Node CLI --[Unix Socket]--> Rust build daemon     (persistent compilation cache)
```

- FFI for the hot path (parsing every file, needs nanosecond overhead).
- Unix socket for the warm path (build requests that benefit from a persistent daemon with caching).

### Pattern 4: gRPC + WASM (Isomorphic Domain Logic)

**Scenario**: Validation logic that runs both server-side and client-side.

```
Server:
  Go Service --[gRPC]--> Rust Validation Service  (authoritative validation)

Client:
  TS Browser --[WASM]--> Rust Validation (same code, compiled to WASM)  (instant feedback)
```

The same Rust validation code compiles to both a native gRPC server and a WASM module. The hex-intf port `IValidationPort` has three adapters:

1. `NativeValidationAdapter` -- direct Rust call (server-side)
2. `GrpcValidationAdapter` -- remote call from Go (server-to-server)
3. `WasmValidationAdapter` -- in-browser call from TypeScript (client-side)

---

## Port Abstraction Across Transports

The core value of hexagonal architecture is that **domain logic never knows about transport**. Here is how each port adapts to different patterns:

### ILLMPort: Multiple Transport Adapters

```typescript
// The port interface -- transport-agnostic
interface ILLMPort {
  prompt(context: TokenBudget, messages: Message[]): Promise<LLMResponse>;
  streamPrompt(context: TokenBudget, messages: Message[]): AsyncGenerator<string>;
}
```

| Adapter | Transport | When |
|---------|-----------|------|
| `RestLLMAdapter` | REST (HTTPS) | Calling external API (Anthropic, OpenAI) |
| `GrpcLLMAdapter` | gRPC | Calling internal LLM gateway service |
| `NatsLLMAdapter` | NATS | Queuing prompts for batch processing |
| `WasmLLMAdapter` | WASM | Running a local ONNX model in-browser |

### IASTPort: Transport Selection by Environment

```typescript
interface IASTPort {
  extractSummary(filePath: string): Promise<ASTSummary>;
  diffStructural(before: ASTSummary, after: ASTSummary): StructuralDiff;
}
```

| Adapter | Transport | When |
|---------|-----------|------|
| `WasmASTAdapter` | WASM Bridge | Browser environment, instant feedback |
| `NativeASTAdapter` | FFI (N-API) | Node.js CLI, maximum performance |
| `SocketASTAdapter` | Unix Socket | Talking to a persistent tree-sitter daemon |
| `GrpcASTAdapter` | gRPC | Remote AST service in a distributed setup |

### IBuildPort: Transport Depends on Deployment

```typescript
interface IBuildPort {
  compile(project: Project): Promise<BuildResult>;
  lint(project: Project): Promise<LintResult>;
  test(project: Project, suite: TestSuite): Promise<TestResult>;
}
```

| Adapter | Transport | When |
|---------|-----------|------|
| `LocalBuildAdapter` | Direct call | Same-process, development mode |
| `SocketBuildAdapter` | Unix Socket | Local build daemon with caching |
| `GrpcBuildAdapter` | gRPC | Remote build farm (CI/CD) |
| `NatsBuildAdapter` | NATS | Distributed build queue with workers |

### The Pattern

```
Domain Use Case
    |
    v
Port Interface  (typed contract, transport-invisible)
    |
    v
Adapter Factory  (selects adapter based on config/environment)
    |
    +---> RestAdapter      (config: { transport: "rest", url: "..." })
    +---> GrpcAdapter      (config: { transport: "grpc", endpoint: "..." })
    +---> WasmAdapter      (config: { transport: "wasm", module: "..." })
    +---> UnixSocketAdapter (config: { transport: "unix", path: "..." })
    +---> NatsAdapter      (config: { transport: "nats", subject: "..." })
```

The adapter factory reads configuration (environment variables, config files) and instantiates the correct adapter. The domain use case receives the port interface via dependency injection and never imports any adapter directly.

```typescript
// In composition root (infrastructure/bootstrap.ts)
function createLLMPort(config: Config): ILLMPort {
  switch (config.llm.transport) {
    case "rest":   return new RestLLMAdapter(config.llm.url, config.llm.apiKey);
    case "grpc":   return new GrpcLLMAdapter(config.llm.endpoint);
    case "nats":   return new NatsLLMAdapter(config.nats.url, config.llm.subject);
    case "wasm":   return new WasmLLMAdapter(config.llm.wasmPath);
    default:       throw new Error(`Unknown LLM transport: ${config.llm.transport}`);
  }
}
```

This composition root is the only place that knows about transports. Everything else speaks the port language.

---

## Recommendations for the Dependency Analyst

When the `dependency-analyst` agent evaluates a problem statement, apply this decision process:

1. **Identify components** and their language assignments (from phase 2: language fit).
2. **Map each boundary** between components to a communication pattern using the decision matrix.
3. **Check for hybrid needs**: most real projects need 2-3 patterns.
4. **Verify testability**: every boundary must be mockable at the port level for London-school TDD.
5. **Assess LLM-friendliness**: prefer patterns whose contracts summarize well at L2. Avoid FFI unless performance demands it.
6. **Document the recommendation** in the `communication` output field, referencing specific patterns from this guide.

### Red Flags

- **FFI in a project with no performance requirements** -- over-engineering. Use REST or gRPC.
- **WebSocket for CRUD** -- wrong tool. Use REST.
- **gRPC for browser clients** -- adds proxy complexity. Use REST + WebSocket.
- **No event system in a multi-service project** -- tight coupling. Add NATS.
- **Single pattern for everything** -- usually a sign that the architecture has not been thought through.

### Green Flags

- **Port interface identical regardless of transport** -- correct hexagonal design.
- **Adapter factory in composition root** -- clean dependency injection.
- **Each pattern chosen for a specific boundary** -- intentional architecture.
- **Mock at port level in all unit tests** -- proper London-school TDD.
