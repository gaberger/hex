# ADR-010: TypeScript-to-Rust Migration Cost and Risk Analysis

## Status: Accepted
## Date: 2026-03-15
**Decision:** Recommend hybrid architecture (Option C) over full rewrite

---

## 1. Current TypeScript Surface Area

### Port Interfaces (1,532 lines across 13 files)

| Port Interface              | Methods | Rust Trait Feasibility |
|-----------------------------|---------|------------------------|
| IASTPort                    | 2       | Native advantage (tree-sitter is Rust-native) |
| ILLMPort                    | 2       | Straightforward (reqwest + async) |
| IBuildPort                  | 3       | Shell-out, same as TS |
| IWorktreePort               | 4       | git2 crate is mature |
| IGitPort                    | 6       | git2 crate is mature |
| IFileSystemPort             | 5       | std::fs, trivial |
| IArchAnalysisPort           | 5       | Complex -- reimplements graph logic |
| ISwarmPort                  | 21      | Largest port; wraps HexFlo (formerly ruflo) CLI |
| ISwarmOrchestrationPort     | 2       | Composes swarm + worktree |
| IValidationPort             | 5       | LLM-dependent, moderate |
| IScaffoldPort               | 7       | Template generation, moderate |
| ICodeGenerationPort         | 2       | LLM-dependent |
| IWorkplanPort               | 2       | LLM-dependent, uses AsyncGenerator |
| INotificationEmitPort       | 5       | Terminal I/O, straightforward |
| INotificationQueryPort      | 6       | In-memory state, straightforward |
| IEventBusPort               | 6       | tokio::broadcast, straightforward |
| IRegistryPort               | 7       | File-based JSON, trivial |
| ISecretsPort                | 3       | HTTP client (Infisical) + env |
| ICheckpointPort             | 4       | File-based JSON, trivial |
| ISerializationPort          | 4       | serde, native advantage |
| IWASMBridgePort             | 5       | wasmtime crate, native advantage |
| IFFIPort                    | 5       | libloading crate, native advantage |
| IServiceMeshPort            | 6       | reqwest/tonic, moderate |
| ISchemaPort                 | 6       | jsonschema crate, moderate |
| IAgentExecutorPort          | 3       | HTTP client + streaming |
| IHubCommandReceiverPort     | 5       | WebSocket client |
| IHubCommandSenderPort       | 4       | HTTP client |
| **TOTAL**                   | **~140 methods** | **27 traits** |

### Codebase Size

| Layer            | Lines of TypeScript |
|------------------|---------------------|
| Ports            | 1,532               |
| Adapters         | 10,613              |
| Use cases        | 3,857               |
| Domain + other   | ~2,337              |
| **Total src/**   | **18,339**          |

### Existing Rust Code (hex-hub)

The `hex-hub/` directory already contains a Rust binary (axum + tokio + rust-embed) serving the dashboard. It has routes, state management, WebSocket support, and command dispatch. This is approximately 800-1,000 lines of Rust and proves the team can ship Rust in production.

---

## 2. Adapter Migration Complexity

### Adapters Where Rust Has a Native Advantage

| Adapter               | TS Dependency         | Rust Equivalent         | Verdict |
|-----------------------|-----------------------|-------------------------|---------|
| TreeSitterAdapter     | web-tree-sitter (WASM)| tree-sitter (native C)  | **Much simpler** -- no WASM loading, 5-10x faster parsing |
| WASMBridgeAdapter     | WebAssembly API       | wasmtime                | **Simpler** -- wasmtime is the reference impl |
| FFIAdapter            | execFile JSON-stdio   | libloading / dlopen     | **Simpler** -- real FFI vs JSON-over-stdio |
| SerializationAdapter  | JSON.parse/stringify  | serde                   | **Simpler** -- serde is best-in-class |
| FileSystemAdapter     | node:fs/promises      | std::fs + tokio::fs     | **Equivalent** |
| GitAdapter            | execFile (git CLI)    | git2 (libgit2 bindings) | **Simpler** -- type-safe, no string parsing |
| WorktreeAdapter       | execFile (git CLI)    | git2                    | **Simpler** |

### Adapters Where Rust Is Equivalent or Harder

| Adapter                  | TS Dependency            | Rust Equivalent           | Verdict |
|--------------------------|--------------------------|---------------------------|---------|
| LLMAdapter               | fetch (HTTP + SSE)       | reqwest + eventsource     | **Equivalent** |
| HexFloAdapter            | execFile (npx)           | Command::new              | **Equivalent** -- still shells out |
| InfisicalAdapter         | fetch (REST API)         | reqwest                   | **Equivalent** |
| BuildAdapter             | execFile (bun/npm)       | Command::new              | **Equivalent** |
| DashboardAdapter         | fetch + EventSource      | reqwest + tokio           | **Equivalent** |
| MCP Adapter              | MCP TS SDK               | See section 3             | **Harder** |
| CLI Adapter              | Commander.js (1,200 LOC) | clap                      | **Equivalent** -- clap is excellent |
| AnthropicAgentAdapter    | Anthropic TS SDK         | No official Rust SDK      | **Harder** -- must use raw HTTP |
| ClaudeCodeExecutorAdapter| execFile                 | Command::new              | **Equivalent** |
| TerminalNotifier         | process.stderr.write     | eprintln! / crossterm     | **Equivalent** |
| ValidationAdapter        | Template strings         | format! macros            | **Equivalent** |

### Adapters Unique to the TS Ecosystem

| Adapter             | Issue in Rust |
|---------------------|---------------|
| InMemoryEventBus    | Trivial to rewrite with tokio::broadcast |
| RegistryAdapter     | Trivial (serde_json + file I/O) |
| CachingSecretsAdapter | Trivial (HashMap + Instant) |
| FileCheckpointAdapter | Trivial (serde_json + file I/O) |

---

## 3. MCP Server Compatibility

The MCP (Model Context Protocol) adapter is the primary integration surface for LLM agents. Current status:

- **TypeScript**: Official `@modelcontextprotocol/sdk` package. The hex MCP adapter (mcp-adapter.ts) is ~700 lines defining 20+ tools with JSON Schema input validation.
- **Rust**: There is a community crate `mcp-server` (also known as `rmcp`) on crates.io. As of mid-2025, it supports tool definitions, stdio transport, and SSE transport. However, it is **not an official Anthropic SDK** and has less ecosystem testing than the TS version.
- **Risk**: MCP is a moving protocol. The TS SDK tracks spec changes within days. A Rust crate may lag by weeks or months. This is the single highest-risk adapter for migration.

**Recommendation**: Keep the MCP adapter in TypeScript. It is the LLM-facing surface and benefits from same-day SDK updates.

---

## 4. Hybrid Architecture Assessment

The project already has hybrid infrastructure in place.

### Existing Cross-Language Ports

The codebase defines three bridging mechanisms in `src/core/ports/cross-lang.ts`:

1. **IWASMBridgePort** -- Load and call WASM modules. Adapter exists at `wasm-bridge-adapter.ts`.
2. **IFFIPort** -- Call native binaries via execFile with JSON-over-stdio. Adapter exists at `ffi-adapter.ts`.
3. **IServiceMeshPort** -- HTTP service mesh for cross-language service discovery. Adapter exists at `service-mesh-adapter.ts`.

### Option A: NAPI-RS (Rust as Node Native Addon)

| Aspect | Assessment |
|--------|------------|
| Integration | Rust compiled to .node addon, called synchronously from TS |
| Latency | Microsecond-level calls, no serialization overhead for simple types |
| Build complexity | Requires napi-rs toolchain, platform-specific binaries (darwin-arm64, linux-x64, etc.) |
| Distribution | Must ship prebuilt binaries or require Rust toolchain on user machines |
| Best for | tree-sitter parsing, AST diffing, dependency graph analysis |
| Maturity | napi-rs is production-grade (used by SWC, Biome, Rspack) |

### Option B: WASM (Rust compiled to WebAssembly)

| Aspect | Assessment |
|--------|------------|
| Integration | Rust compiled to .wasm, loaded via existing WASMBridgeAdapter |
| Latency | ~10-100us overhead per call (WASM instantiation amortized) |
| Build complexity | Simple: `cargo build --target wasm32-wasi` |
| Distribution | Single .wasm file, platform-independent |
| Best for | Pure computation (parsing, analysis, serialization) |
| Limitation | No filesystem or network access without WASI; no direct git2 |

### Option C: Process Boundary (Rust binary, JSON-over-stdio/HTTP)

| Aspect | Assessment |
|--------|------------|
| Integration | Already proven by hex-hub (Rust binary, axum HTTP server) |
| Latency | ~1-5ms per call (HTTP round-trip) |
| Build complexity | Separate cargo workspace, independent release cycle |
| Distribution | Ship a single binary alongside the npm package |
| Best for | Long-running services (dashboard, analysis server, WASM host) |
| Already working | hex-hub proves this pattern at `hex-hub/src/main.rs` |

### Recommended Hybrid Architecture

```
                    TypeScript (keep)                     Rust (new/expand)
               +---------------------------+        +------------------------+
  LLM Agents ->| MCP Adapter (mcp-adapter) |        |                        |
               | CLI Adapter (commander)    |        |  hex-core (new crate)  |
               | Dashboard Adapter (SSE)    |        |  - tree-sitter parse   |
               +---------------------------+        |  - dep graph analysis  |
                          |                          |  - dead export detect  |
                    Ports (TS interfaces)             |  - hex boundary check  |
                          |                          |  - circular dep detect |
               +---------------------------+        |  - serde serialization |
               | Use Cases (TS)            |------->|                        |
               | - CodeGenerator           | NAPI   +------------------------+
               | - WorkplanExecutor        |  or         |
               | - SwarmOrchestrator       | FFI    +------------------------+
               | - NotificationOrchestrator|        |  hex-hub (existing)    |
               +---------------------------+        |  - dashboard server    |
                          |                          |  - project registry    |
               +---------------------------+        |  - command dispatch    |
               | Secondary Adapters (TS)   |        +------------------------+
               | - LLM, Ruflo, Secrets     |
               | - Git (keep for now)      |
               +---------------------------+
```

**Phase 1** (Low risk, high reward): Move tree-sitter parsing to native Rust via NAPI-RS. The TreeSitterAdapter currently loads WASM grammars through web-tree-sitter. Native tree-sitter in Rust would eliminate WASM overhead, simplify grammar loading, and provide 5-10x faster parsing. This is the single biggest performance win.

**Phase 2** (Medium risk, medium reward): Move the ArchAnalyzer logic (dependency graph, dead exports, boundary validation, circular detection) into the same Rust crate. These are pure graph algorithms operating on AST summaries -- ideal for Rust.

**Phase 3** (Low risk, incremental): Expand hex-hub to subsume more server-side responsibilities (project registry, checkpoint persistence) since it already runs as a daemon.

---

## 5. Risk Matrix

### Full Rewrite to Rust

| # | Risk | Likelihood | Impact | Mitigation |
|---|------|------------|--------|------------|
| 1 | **MCP SDK lag**: Rust MCP crate falls behind protocol spec changes | High | Critical | Must maintain a fork or contribute upstream |
| 2 | **LLM SDK gap**: No official Anthropic Rust SDK; streaming SSE parsing is error-prone | High | High | Use raw reqwest + manual SSE parsing (brittle) |
| 3 | **Development velocity halved**: 18K LOC rewrite estimated at 8-12 weeks for experienced Rust developer | High | High | No mitigation -- this is inherent |
| 4 | **Ecosystem mismatch**: hex targets JS/TS projects primarily; Rust CLI for JS projects feels foreign | Medium | Medium | Ship as npm postinstall binary (like esbuild) |
| 5 | **ruflo dependency** (now HexFlo): ruflo is a Node CLI tool; Rust must still shell out to it | High | Medium | No change in architecture -- still subprocess |

**Estimated effort**: 8-12 engineer-weeks for a competent Rust developer.
**Feature parity risk**: High. AsyncGenerator patterns, dynamic imports, and TS SDK integrations do not translate cleanly.

### Stay Pure TypeScript

| # | Risk | Likelihood | Impact | Mitigation |
|---|------|------------|--------|------------|
| 1 | **tree-sitter WASM overhead**: 5-10x slower than native for large codebases | High | Medium | Acceptable for most projects; NAPI escape hatch exists |
| 2 | **No native FFI**: FFIAdapter uses JSON-over-stdio, not real FFI | Medium | Low | Acceptable for current use cases |
| 3 | **Single-threaded analysis**: Graph algorithms blocked by Node event loop | Medium | Medium | Worker threads exist but are awkward |
| 4 | **Bundle size**: tree-sitter WASM grammars are ~2-5MB each | Low | Low | Lazy-load only needed grammars |
| 5 | **Perceived performance ceiling**: Cannot match native tools (Biome, Oxc) | Low | Low | hex is not a compiler; analysis runs are infrequent |

**Estimated effort**: 0 (status quo).
**Risk**: Low. Current architecture works. Performance is adequate.

### Hybrid Architecture (Recommended)

| # | Risk | Likelihood | Impact | Mitigation |
|---|------|------------|--------|------------|
| 1 | **NAPI-RS build complexity**: Platform-specific binaries, CI matrix | Medium | Medium | Use napi-rs GitHub Actions template (proven by SWC) |
| 2 | **Two build systems**: cargo + bun/npm | Medium | Low | Already the case (hex-hub exists) |
| 3 | **Debugging across boundary**: Stack traces stop at NAPI/FFI call | Medium | Low | Structured error types with context |
| 4 | **Version coupling**: Rust crate version must match TS expectations | Low | Medium | Semantic versioning + integration tests |
| 5 | **Graceful fallback**: Must work when Rust binary is not available | Low | Medium | Already handled -- TreeSitterAdapter falls back to stub |

**Estimated effort**: 3-4 engineer-weeks for Phase 1 (NAPI tree-sitter).
**Risk**: Low. Proven pattern (SWC, Biome, Rspack all use NAPI-RS). Existing fallback architecture means Rust is optional, not required.

---

## 6. Decision

**Reject** full Rust rewrite. The 8-12 week cost, MCP SDK risk, and LLM SDK gap make it unjustifiable given that the current TypeScript architecture is functional and well-structured.

**Reject** status quo only if tree-sitter performance becomes a bottleneck on large codebases (>100K LOC projects).

**Recommend** hybrid architecture (Option C / NAPI-RS) with phased rollout:

1. **Phase 1**: `hex-core` Rust crate with native tree-sitter parsing, exposed via NAPI-RS. TreeSitterAdapter gains a fast path that falls back to WASM when the native addon is not available.
2. **Phase 2**: Move graph algorithms (dead exports, boundary validation, circular deps) into `hex-core`.
3. **Phase 3**: Expand hex-hub to own more server-side state.

The hexagonal architecture makes this migration safe: adapters are swappable by design. The `IASTPort` and `IArchAnalysisPort` interfaces do not change -- only the adapter implementation behind them.

---

## 7. Technology Evaluation Matrix

| Criterion (weight)         | Full Rust | Stay TS | Hybrid NAPI | Hybrid WASM | Hybrid FFI |
|---------------------------|-----------|---------|-------------|-------------|------------|
| Performance (0.2)         | 5         | 2       | 5           | 4           | 3          |
| Migration effort (0.25)   | 1         | 5       | 4           | 4           | 4          |
| MCP compatibility (0.2)   | 2         | 5       | 5           | 5           | 5          |
| Distribution ease (0.15)  | 3         | 5       | 3           | 5           | 3          |
| Maintainability (0.2)     | 3         | 4       | 4           | 3           | 3          |
| **Weighted total**        | **2.60**  | **4.15**| **4.30**    | **4.15**    | **3.70**   |

Hybrid NAPI-RS scores highest by combining native performance on the hot path with TypeScript ecosystem advantages for LLM integration.
