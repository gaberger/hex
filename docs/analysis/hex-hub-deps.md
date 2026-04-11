# hex-hub Dependency Analysis & Tech Stack Report

**Date**: 2026-03-16
**Problem**: Dashboard hub binary for hex — replaces TypeScript hub with Rust/Axum

---

## 1. Component Decomposition

| Component | Requirement | Language | Rationale |
|-----------|------------|----------|-----------|
| HTTP API (14 routes) | Low latency, type-safe routing | Rust | Axum's extractor pattern is ideal for typed JSON APIs |
| SSE Streaming | Long-lived connections, backpressure | Rust | tokio broadcast + axum SSE — zero-copy, no GC pauses |
| WebSocket Broker | Bidirectional, topic fan-out | Rust | axum::ws + tokio tasks — lightweight per-connection |
| Static File Serving | Compile-time embedding | Rust | rust-embed eliminates runtime file resolution |
| Daemon Lifecycle | Process management, signal handling | Rust | Direct OS API access, no runtime overhead |
| Auth Middleware | Request interception | Rust | tower Layer pattern — composable and type-safe |
| Dashboard UI | Browser rendering | Vanilla JS | Stays as-is — no framework needed for 1,498 lines |
| Push Clients | HTTP POST to hub | TypeScript | DashboardAdapter stays TS — same ecosystem as hex CLI |
| MCP Tools | Tool definitions | TypeScript | Stays TS — hex CLI is the host process |

**Communication pattern**: HTTP/JSON between TypeScript clients and Rust hub. No FFI, no gRPC, no shared memory. Clean process boundary.

---

## 2. Direct Dependencies (17)

### Core Framework

| Crate | Version | Purpose | Alternatives Considered | Verdict |
|-------|---------|---------|------------------------|---------|
| **axum** | 0.8 | HTTP framework | actix-web, warp, rocket | Best: tower ecosystem, extractors, ws+sse built-in |
| **tokio** | 1 | Async runtime | async-std, smol | Only choice: axum requires tokio |
| **tower-http** | 0.6 | CORS middleware | custom impl | Best: maintained by same team as axum |
| **http** | 1 | HTTP types | — | Required by axum/tower-http |

### Serialization

| Crate | Version | Purpose | Alternatives | Verdict |
|-------|---------|---------|-------------|---------|
| **serde** | 1 | Derive serialization | — | De facto standard, no alternative |
| **serde_json** | 1 | JSON parsing | simd-json | serde_json is sufficient; simd-json overkill for this workload |

### Real-Time

| Crate | Version | Purpose | Alternatives | Verdict |
|-------|---------|---------|-------------|---------|
| **async-stream** | 0.3 | SSE stream construction | futures::stream | Better ergonomics for yield-based streams |
| **futures** | 0.3 | Stream/Sink traits | tokio-stream | Required for WebSocket split pattern |

### Embedding & Serving

| Crate | Version | Purpose | Alternatives | Verdict |
|-------|---------|---------|-------------|---------|
| **rust-embed** | 8 | Compile-time asset embedding | include_bytes!, tower-http ServeDir | Best: debug-mode disk reads, release embedding |
| **mime_guess** | 2 | Content-type detection | — | Required by rust-embed |

### Utilities

| Crate | Version | Purpose | Alternatives | Verdict |
|-------|---------|---------|-------------|---------|
| **chrono** | 0.4 | Timestamps (RFC3339, millis) | time | Either works; chrono more ergonomic for JSON |
| **rand** | 0.8 | Token generation | getrandom | rand wraps getrandom with better API |
| **url** | 2 | Origin parsing for CORS | manual parsing | Safer than regex/string matching |
| **urlencoding** | 2 | Decode URL-encoded file paths | percent-encoding | Simpler API for our use case |
| **uuid** | 1 | WebSocket client IDs | nanoid, ulid | Standard, widely recognized |
| **tracing** | 0.1 | Structured logging | log, slog | Best: async-aware, span-based, tower integration |
| **tracing-subscriber** | 0.3 | Log formatting | env_logger | Required companion to tracing |

---

## 3. Dependency Health Assessment

### Supply Chain

| Metric | Value | Assessment |
|--------|-------|------------|
| Direct deps | 17 | Moderate — could trim 2-3 |
| Transitive deps | 336 | Typical for axum+tokio project |
| Binary size (release) | 1.4 MB | Excellent — below 5 MB target |
| Clean build time | ~47s | Acceptable for CI |
| Incremental build | ~1-3s | Good for development |
| Unsafe code in direct deps | tokio, axum (audited) | Low risk — tier-1 ecosystem |

### Candidates for Removal

| Crate | Can Remove? | How |
|-------|------------|-----|
| **mime_guess** | Yes | Only used by rust-embed; could use `include_bytes!` + hardcoded Content-Type for single HTML file |
| **url** | Maybe | Replace with manual localhost check (3 lines) — but url is safer |
| **futures** | No | Required for `SinkExt`/`StreamExt` on WebSocket |
| **chrono** | Maybe | Replace with `std::time` + manual formatting — saves ~50 transitive deps |

**Recommendation**: Keep all 17. The binary is 1.4 MB — removing chrono would save ~100KB but add manual date formatting complexity. Not worth it.

---

## 4. Cross-Language Communication

```
┌─────────────────────┐                    ┌──────────────────┐
│   hex CLI (TypeScript)│                    │  hex-hub (Rust)  │
│                     │                    │  Port 5555       │
│ DashboardAdapter ───┼── HTTP POST ──────→│  Axum routes     │
│ MCP Tools ──────────┼── HTTP GET/POST ──→│                  │
│ DaemonManager ──────┼── spawn() ────────→│  (child process) │
│                     │                    │                  │
│ Browser ────────────┼── EventSource ────→│  SSE /api/events │
│ Browser ────────────┼── WebSocket ──────→│  WS /ws          │
│                     │                    │                  │
└─────────────────────┘                    └──────────────────┘
```

### Contract Definition

| Layer | Format | Defined In |
|-------|--------|-----------|
| HTTP API | JSON over REST | dashboard-hub.ts routes (source of truth) |
| SSE | `event: X\ndata: JSON\n\n` | SSE protocol standard |
| WebSocket | JSON messages with `type` discriminator | ws-broker.ts (3 message types) |
| Lock file | JSON at `~/.hex/daemon/hub.lock` | daemon-manager.ts |
| Process | `spawn(binaryPath, args)` → stdout/stderr | daemon-manager.ts |

### Cross-Language Risks

| Risk | Severity | Mitigation |
|------|----------|------------|
| DJB2 hash divergence | **High** | Must add cross-language test with hardcoded values |
| JSON field casing mismatch | Medium | All Rust structs use `#[serde(rename_all = "camelCase")]` |
| Timestamp format mismatch | Low | Both use millisecond epoch via `Utc::now().timestamp_millis()` |
| Body size limit mismatch | Low | Rust enforces 256KB/16KB matching TS `readBody()` limits |

---

## 5. Future Dependency Recommendations

### If adding health history (Phase 5)

| Need | Recommended Crate | Why |
|------|------------------|-----|
| Embedded DB | **rusqlite** | Zero-config, single file, ships in binary |
| NOT | sled, redb | Overkill for append-only health scores |

### If adding webhook notifications

| Need | Recommended Crate | Why |
|------|------------------|-----|
| HTTP client | **reqwest** | De facto Rust HTTP client, tokio-native |
| NOT | hyper (raw) | Too low-level for simple POST requests |

### If adding Mermaid graph export

| Need | Recommended Crate | Why |
|------|------------------|-----|
| Template | **None** — use `format!()` | Mermaid is just text; no template engine needed |

---

## 6. Comparison with TypeScript Hub Dependencies

| TypeScript Hub | Rust Hub | Notes |
|---------------|----------|-------|
| node:http (built-in) | axum + tokio | More deps but better type safety |
| node:fs (readFileSync) | rust-embed | Eliminated runtime file lookup |
| node:crypto (randomBytes) | rand | Equivalent |
| ws (optional npm) | axum ws feature | Built-in, no optional dep |
| No CORS library | tower-http cors | Proper implementation vs manual headers |
| **Total: 1 optional dep** | **Total: 17 deps** | Rust has more deps but zero runtime deps |

The TypeScript hub has fewer library dependencies because Node.js bundles HTTP/FS/crypto. But it requires a **90MB runtime** (Bun) to function. The Rust hub has 17 crate dependencies but produces a **1.4MB self-contained binary**.

---

## 7. Verdict

**The dependency choices are sound.** Every crate is tier-1 Rust ecosystem, well-maintained, and necessary. The 17-dep / 336-transitive count is typical and healthy for an axum project. Binary size at 1.4MB confirms no bloat.

**One action item**: Add a cross-language DJB2 hash compatibility test before shipping. This is the highest-risk cross-language contract point.
