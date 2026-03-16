# Dashboard Daemon: TypeScript (Bun) vs Rust — Decision Analysis

**Date**: 2025-03-15
**Status**: Recommendation — TypeScript with Bun runtime
**Context**: hex dashboard daemon — long-lived background service for developer machines

---

## Measured Data (this machine, macOS arm64)

| Metric | Node.js | Bun | Rust (estimated) |
|--------|---------|-----|-------------------|
| Idle process RSS | 37 MB | 20 MB | 2-5 MB |
| HTTP server RSS | 40 MB | 23 MB | 5-10 MB |
| Cold start | ~200ms | ~30ms | ~5ms |
| Compiled binary size | N/A | 58 MB | 5-15 MB |

## Criterion-by-Criterion Analysis

### 1. Memory Footprint

- **Bun HTTP server**: 23 MB RSS (measured)
- **Node.js HTTP server**: 40 MB RSS (measured)
- **Rust equivalent**: ~5-10 MB RSS (tokio + hyper + serde, based on published benchmarks)

**Verdict**: Bun is 17 MB heavier than Rust. On a dev machine with 16-64 GB RAM, 23 MB is negligible. This is not a differentiator.

### 2. Startup Time

- **Bun**: ~30ms cold start (measured — `bun build --compile` produces instant-start binaries)
- **Node.js**: ~200ms cold start
- **Rust**: ~5ms cold start

**Verdict**: Bun's 30ms is indistinguishable from Rust's 5ms for a lazy-start daemon. Both are "instant" to a human. Not a differentiator.

### 3. File Watching

- **Node.js/Bun `fs.watch()`**: Uses FSEvents on macOS (native, reliable). The `recursive: true` option works on macOS and Windows. Linux requires manual recursion or `chokidar`. The existing `dashboard-hub.ts` already uses `fs.watch({ recursive: true })` successfully.
- **Rust `notify` crate**: Cross-platform, uses FSEvents/inotify/kqueue. More reliable on Linux for deep trees.

**Verdict**: The daemon already works with `fs.watch()`. Linux recursive watching is the only gap, solvable with `chokidar` if needed. Not worth rewriting for.

### 4. WebSocket/HTTP

- **Bun**: Native `Bun.serve()` with WebSocket upgrade built in. Zero dependencies. The existing code uses `node:http` (compatible with Bun).
- **Rust**: `tokio` + `axum` or `tokio-tungstenite`. Mature but requires ~500 lines of boilerplate that TypeScript doesn't need.

**Verdict**: Bun has a significant DX advantage here. The dashboard hub is already 500+ lines of working TypeScript HTTP/SSE code.

### 5. Build and Distribution

**Current state**: hex is an npm package. The daemon runs as TypeScript via Bun.

**If Rust**: Must cross-compile for 4+ targets and distribute via `optionalDependencies`:
```
@hex/daemon-darwin-arm64
@hex/daemon-darwin-x64
@hex/daemon-linux-x64
@hex/daemon-linux-arm64
```

This is the oxlint/biome pattern (oxlint ships 19 platform packages, biome ships 8). It requires:
- Rust cross-compilation CI (GitHub Actions matrix)
- Separate npm packages per platform
- A JS shim that selects the right binary at install time
- Testing on all 4 platforms per release
- Separate versioning/publishing pipeline

**If Bun compiled binary**: `bun build --compile` produces a 58 MB self-contained binary. Cross-compilation is possible (`--target=bun-linux-x64`, etc.) but produces large binaries. However, this is unnecessary — Bun is already a dependency of hex.

**If TypeScript (no compile)**: The daemon is just TypeScript source shipped in the npm package. `bun run src/daemon.ts` — zero build step, zero platform binaries, zero CI complexity.

**Verdict**: Rust adds massive distribution complexity for zero user-facing benefit. The daemon doesn't need native performance — it handles dozens of connections, not thousands.

### 6. Code Sharing (THE DECISIVE FACTOR)

The daemon needs direct access to:

| Type | Location |
|------|----------|
| `AppContext` | `src/core/ports/app-context.ts` |
| `AppContextFactory` | Creates fully-wired contexts per project |
| `SwarmTask`, `SwarmAgent`, `AgentDBPattern` | `src/core/ports/swarm.ts` |
| `ProjectRegistration`, `ProjectRegistry` | `src/core/domain/value-objects.ts` |
| `ArchAnalysisResult`, `ImportEdge` | `src/core/domain/value-objects.ts` |
| `IRegistryPort` | `src/core/ports/registry.ts` |
| `ISwarmPort` | `src/core/ports/swarm.ts` |
| `IEventBusPort` | `src/core/ports/event-bus.ts` |
| `DomainEvent` | `src/core/domain/entities.ts` |
| All use cases | `ArchAnalyzer`, `SummaryService`, etc. |

The existing `dashboard-hub.ts` imports `AppContext` and `AppContextFactory` and calls use cases directly:
```typescript
import type { AppContext, AppContextFactory } from '../../core/ports/app-context.js';
```

**In Rust**: Every one of these types must be duplicated as Rust structs. Every port interface becomes a Rust trait. The composition root cannot be shared. The daemon would communicate with hex only via JSON over HTTP/stdio, losing type safety at the boundary. Any domain type change requires updating two codebases.

**In TypeScript**: `import { ... } from '../core/ports/index.js'` — done. Type changes propagate automatically. The daemon IS part of the hex architecture.

**Verdict**: This is the decisive factor. The daemon is not a standalone tool (like biome or oxlint). It is an adapter within hex's hexagonal architecture. Rewriting it in Rust would violate the architecture's own principles — it would create a parallel, untyped boundary between the daemon and the domain core.

### 7. Developer Experience

- hex contributors are TypeScript developers
- The dashboard hub is already 500+ lines of working TypeScript
- Rust would require contributors to install `rustup`, learn Rust, and maintain two build systems
- The Rust ecosystem has no equivalent to Bun's zero-config TypeScript execution

**Verdict**: Rust creates an unnecessary contribution barrier.

### 8. IPC with ruflo/MCP Tools

- **TypeScript**: `execFile('npx', ['@claude-flow/cli', ...])` — already working in `RufloAdapter`
- **Rust**: `Command::new("npx").args(["@claude-flow/cli", ...])` — equivalent, but loses type-safe result parsing

Both work. Neither has a meaningful advantage.

### 9. Real-World Precedent Analysis

| Tool | Language | Why |
|------|----------|-----|
| Turborepo daemon | Rust | Turbo is a Rust project; daemon shares Rust codebase |
| Biome | Rust | Standalone tool; no JS ecosystem integration needed |
| oxlint | Rust | Standalone tool; performance-critical (linting millions of lines) |
| eslint_d | JavaScript | Ecosystem tool; shares eslint's JS plugins |
| Prettier daemon | JavaScript | Ecosystem tool; shares prettier's JS formatters |
| Watchman | C++ | Standalone tool; extreme performance requirements |
| tsc --watch | TypeScript | Ecosystem tool; shares TypeScript compiler code |

**Pattern**: Standalone performance tools use Rust/C++. Ecosystem tools that share code with a JS/TS project stay in JS/TS. hex's daemon is firmly in the "ecosystem tool" category.

---

## Recommendation: TypeScript with Bun Runtime

**Use TypeScript.** The daemon should remain a TypeScript adapter within hex's hexagonal architecture.

### Specific Implementation Plan

1. **Runtime**: Bun (already the project runtime; 23 MB RSS, 30ms startup)
2. **HTTP**: `Bun.serve()` for native WebSocket + HTTP (migrate from `node:http`)
3. **File watching**: Keep `fs.watch({ recursive: true })`; add `chokidar` fallback for Linux
4. **Process management**: Daemonize via `bun run src/daemon.ts &` with PID file
5. **Distribution**: Ship as TypeScript source in the npm package — no compilation needed

### What Would Change the Recommendation

Rust would become worth considering if:
- The daemon needed to handle **thousands of concurrent connections** (it handles dozens)
- Memory budget was **under 5 MB** (no such constraint exists)
- The daemon was a **standalone product** separate from hex (it's not)
- The team had **Rust expertise** and wanted to maintain two build systems (they don't)

### Quantified Cost of Rust

| Cost | Estimate |
|------|----------|
| Rewrite 500+ lines of TypeScript to Rust | 2-3 weeks |
| Duplicate ~30 domain types as Rust structs | Ongoing maintenance |
| Set up cross-compilation CI (4 targets) | 1 week |
| Maintain platform-specific npm packages | Ongoing |
| Test on 4 platforms per release | +30min CI per release |
| Memory savings | ~15 MB (23 MB to 8 MB) |
| Startup savings | ~25ms (30ms to 5ms) |

**The Rust rewrite would cost weeks of effort and ongoing maintenance burden to save 15 MB of RAM and 25ms of startup time on a developer machine.**
