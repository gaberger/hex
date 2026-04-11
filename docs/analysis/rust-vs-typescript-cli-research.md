# Rust vs TypeScript for hex CLI Framework: Research Findings

Date: 2026-03-17

---

## 1. Binary Distribution

### The npm optionalDependencies Pattern (Industry Standard)

Rust-based JS tools distribute via npm using platform-specific optional dependencies. A thin JS shim resolves the correct native binary from `node_modules`.

| Tool | npm Wrapper Package | Platform Packages (examples) |
|------|-------------------|------------------------------|
| **Biome** | `@biomejs/biome` | `@biomejs/cli-darwin-arm64`, `@biomejs/cli-linux-x64`, `@biomejs/cli-linux-x64-musl`, `@biomejs/cli-win32-x64` (8 targets) |
| **SWC** | `@swc/core` | `@swc/core-darwin-arm64`, `@swc/core-linux-x64-gnu`, `@swc/core-win32-x64-msvc` (13 targets) |
| **Turborepo** | `turbo` | `turbo-darwin-arm64`, `turbo-linux-64`, `turbo-windows-64` (6 targets) |

**How it works**: npm resolves `optionalDependencies` with `os` and `cpu` fields. Only the matching platform binary (~15 MB) is downloaded. A JS postinstall fallback downloads the binary if optional dep resolution fails.

**Alternative channels**: `cargo install`, `brew`, standalone binary downloads from GitHub releases. These complement npm but do not replace it for JS ecosystem users.

**Key concern**: npm `optionalDependencies` has a known bug — regenerating `package-lock.json` with existing `node_modules` only includes the current platform's dependency, breaking CI/CD reproducibility across platforms.

### NAPI-RS (for Node.js native addons, not standalone CLIs)

NAPI-RS v3 generates TypeScript definitions and JS bindings automatically. It now also supports compiling to WebAssembly with minimal code changes, enabling a single codebase to target both native and WASM.

**Performance**: Native Rust via NAPI-RS is ~61% faster than pure JS; WASM is ~44% faster than JS. Native Rust is ~45% faster than WASM on average.

---

## 2. Startup Time

| Runtime | Cold Start (hello world) | Source |
|---------|------------------------|--------|
| **Rust native binary** | 0.5 ms (x86 desktop), 4.4 ms (Raspberry Pi 3) | [bdrung/startup-time](https://github.com/bdrung/startup-time) (1000-iteration average) |
| **Bun** | 8-15 ms | Strapi benchmark, 2025 |
| **Node.js 24** | 40-120 ms | Strapi benchmark, 2025 |

**Real-world CLI startup** (with imports/initialization):

| Scenario | Bun | Node.js | Rust (estimated) |
|----------|-----|---------|-------------------|
| Simple CLI dispatch | 15-30 ms | 80-200 ms | 1-5 ms |
| CLI with heavy module loading | 40-80 ms | 150-300 ms | 5-15 ms |

**Note**: Bun uses JavaScriptCore (optimized for fast startup) vs Node.js V8 (optimized for long-running). Rust has no VM or module resolution overhead.

---

## 3. Tree-sitter Integration

### Rust (native, first-class)

- **Crate**: `tree-sitter` (latest 0.26.x on crates.io)
- Tree-sitter's core is written in C; the Rust crate provides safe bindings with zero overhead
- Grammar crates exist per language: `tree-sitter-typescript`, `tree-sitter-go`, `tree-sitter-rust`, etc.
- Queries, pattern matching, and incremental parsing are all native API
- No grammar compilation step at install time — grammars compile with `cargo build`
- Can also compile to WASM via `wasmtime-c-api` if needed

### Node.js — Two Options

| | `node-tree-sitter` (native) | `web-tree-sitter` (WASM) |
|---|---|---|
| **Binding type** | N-API native addon (C++) | WebAssembly (.wasm) |
| **Performance** | Fastest on Node.js | "Considerably slower" (per official docs) |
| **Distribution** | Requires native compilation or prebuild per platform/arch | Universal .wasm files, no recompilation |
| **Grammar format** | Compiled `.node` addons | `.wasm` files |
| **DX pain point** | Rebuild on Node/Electron version change; platform-specific binaries | Async initialization; API differences from native |

**hex currently uses**: `web-tree-sitter` (WASM) — confirmed by the build script copying `tree-sitter.wasm` to dist. This trades performance for distribution simplicity.

### Quantitative Gap

No published head-to-head benchmarks with ms-level measurements exist. The official tree-sitter README states: "Executing .wasm files in Node.js is considerably slower than running Node.js bindings." Pulsar editor reports that WASM performance penalty "can only decrease" over time but remains measurable today.

---

## 4. LLM/AI SDK Ecosystem

### TypeScript (Anthropic Official)

| Package | Maintainer | Weekly Downloads | Notes |
|---------|-----------|-----------------|-------|
| `@anthropic-ai/sdk` | Anthropic (official) | ~7.3M/week | Full API coverage, streaming, tool use, batches |
| `@anthropic-ai/bedrock-sdk` | Anthropic (official) | Available | AWS Bedrock integration |
| `@anthropic-ai/vertex-sdk` | Anthropic (official) | Available | GCP Vertex integration |
| `@modelcontextprotocol/sdk` | Anthropic (official) | — | Official MCP TypeScript SDK |

### Rust

| Crate | Maintainer | Downloads | Notes |
|-------|-----------|-----------|-------|
| `anthropic-sdk-rust` | Community (unofficial) | ~37K all-time | Claims full feature parity with TS SDK |
| `anthropic-agent-sdk` | Community | ~2.4K all-time | Mirrors TS Agent SDK; has `rmcp` feature flag |
| `rmcp` | Official (modelcontextprotocol org) | — | **Official MCP Rust SDK** at [github.com/modelcontextprotocol/rust-sdk](https://github.com/modelcontextprotocol/rust-sdk) |
| `rust-mcp-sdk` | Community | — | Full MCP 2025-11-25 spec implementation |
| `cc-sdk` | Community | — | Rust SDK for Claude Code CLI |

**Key difference**: Anthropic publishes **no official Rust API SDK**. The TypeScript SDK is first-party with 7M+ weekly downloads. All Rust API SDKs are community-maintained. However, the **MCP Rust SDK (`rmcp`) is official**, maintained under the `modelcontextprotocol` GitHub org.

---

## 5. Developer Contribution Barrier

### Quantitative Indicators

| Metric | TypeScript | Rust |
|--------|-----------|------|
| Stack Overflow 2024 survey: "most used" | ~38% of developers | ~13% of developers |
| GitHub users with language experience | Very high (JS/TS dominant) | Growing but niche |
| Time to productive contribution (estimated) | Days (familiar syntax) | Weeks-months (ownership, lifetimes, traits) |
| Compile-time feedback | Runtime errors, type errors | Borrow checker, lifetime errors, trait bounds |
| Package ecosystem size | ~2.5M npm packages | ~160K crates |

### Qualitative Factors

**TypeScript advantages for contributors**:
- Near-zero barrier for JS/TS developers (hex's target audience)
- Hot reload via `bun --watch`
- Flexible prototyping (dynamic types available when needed)
- Test ecosystem maturity (vitest, jest, bun:test all work)

**Rust advantages for contributors**:
- Compiler catches entire classes of bugs before runtime
- No null/undefined runtime surprises
- Excellent documentation culture (docs.rs, The Book)
- `cargo` is a unified build/test/bench/publish tool

**Rust barriers**:
- Ownership and borrowing model requires conceptual shift
- Async Rust has known complexity (Pin, lifetime in futures)
- Longer compile times (incremental helps but still slower than TS transpilation)
- Trait bounds and generics can produce opaque error messages

---

## Summary Table

| Dimension | TypeScript (current) | Rust (potential) | Winner |
|-----------|---------------------|------------------|--------|
| npm distribution | Native (just JS) | Requires optionalDeps shim pattern | TypeScript (simpler) |
| Startup time | 8-15ms (Bun), 40-120ms (Node) | 0.5-5ms | Rust |
| Tree-sitter DX | WASM (slower, easier dist) or native (rebuild pain) | Native, zero-overhead, compiles with cargo | Rust |
| Anthropic SDK | Official, 7M+ weekly downloads | Community-only, ~37K total downloads | TypeScript |
| MCP SDK | Official TS SDK | Official Rust SDK (`rmcp`) | Tie |
| Contributor pool | Large (JS/TS developers) | Small (Rust developers) | TypeScript |
| Existing codebase | 100% TypeScript | hex-hub already in Rust | TypeScript (migration cost) |

---

## Sources

- [Biome package.json (GitHub)](https://github.com/biomejs/biome/blob/main/packages/@biomejs/biome/package.json)
- [SWC platform-specific deps (GitHub issue)](https://github.com/swc-project/swc/issues/2898)
- [Turborepo CLI Architecture (DeepWiki)](https://deepwiki.com/vercel/turborepo/2.4-cli-architecture)
- [Packaging Rust for npm (Orhun's Blog)](https://blog.orhun.dev/packaging-rust-for-npm/)
- [Publishing binaries on npm (Sentry Engineering)](https://sentry.engineering/blog/publishing-binaries-on-npm)
- [bdrung/startup-time benchmark](https://github.com/bdrung/startup-time)
- [Bun vs Node.js 2025 (Strapi)](https://strapi.io/blog/bun-vs-nodejs-performance-comparison-guide)
- [tree-sitter Rust crate (crates.io)](https://crates.io/crates/tree-sitter)
- [Modern Tree-sitter part 7 (Pulsar)](https://blog.pulsar-edit.dev/posts/20240902-savetheclocktower-modern-tree-sitter-part-7/)
- [NAPI-RS v3 announcement](https://napi.rs/blog/announce-v3)
- [Native Rust vs WASM benchmark (yieldcode.blog)](https://yieldcode.blog/post/native-rust-wasm/)
- [@anthropic-ai/sdk (npm)](https://www.npmjs.com/package/@anthropic-ai/sdk)
- [anthropic-sdk-rust (crates.io)](https://crates.io/crates/anthropic-sdk-rust)
- [Official MCP Rust SDK (GitHub)](https://github.com/modelcontextprotocol/rust-sdk)
- [anthropic-agent-sdk (crates.io)](https://crates.io/crates/anthropic-agent-sdk)
- [npm platform-specific deps bug (GitHub)](https://github.com/npm/cli/issues/4828)
