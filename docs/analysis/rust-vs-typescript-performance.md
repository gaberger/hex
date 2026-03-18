# Rust vs TypeScript Performance Analysis for hex Framework

Based on reading the actual source code of all performance-sensitive components.

---

## 1. AST Parsing via Tree-Sitter (`treesitter-adapter.ts`)

**Bottleneck: File I/O, not CPU parsing.**

The adapter already uses `web-tree-sitter` (WASM) -- the same C library Rust would use via `tree-sitter` crate. The parse itself is native-speed WASM, not interpreted JS. The actual work is:

- `fs.read(filePath)` -- I/O bound
- `parser.parse(source)` -- delegated to WASM (C code), not JS
- `extractExports/extractImports` -- shallow tree walks of top-level AST nodes only (single `for` loop over `root.childCount`)
- Mtime-based cache (`summaryCache`) already skips re-parsing unchanged files

**Rust advantage: Negligible.** The CPU-hot path (parsing) is already native code via WASM. The JS wrapper does shallow node iteration. Switching to Rust's `tree-sitter` crate would save maybe 1-2ms of JS overhead per file on the tree walk, but the `fs.read()` dominates. For a 100-file project, total parse time is likely <500ms either way, and caching eliminates repeat costs.

## 2. Import Graph / Architecture Analysis (`arch-analyzer.ts`)

**Bottleneck: N file reads + N tree-sitter parses (I/O bound).**

`collectSummaries()` does:
1. `fs.glob('**/*.ts')` + `fs.glob('**/*.go')` + `fs.glob('**/*.rs')` -- filesystem scan
2. `Promise.all(sourceFiles.map(f => ast.extractSummary(f, 'L1')))` -- parallel file reads + parses

Then pure in-memory graph operations:
- `buildEdgesFromSummaries` -- single loop, Map lookups
- `findDeadFromSummaries` -- single loop, Set operations
- `findCyclesFromEdges` -- DFS cycle detection
- `detectUnusedPorts` -- Set intersection

**Rust advantage: Negligible.** The graph algorithms are O(V+E) where V is file count (typically 20-200 files in a hex project). The in-memory graph work takes microseconds. The bottleneck is the N filesystem reads that feed `collectSummaries`. Rust's `walkdir` is faster than Node's glob, but we're talking about scanning <200 files -- the difference is imperceptible.

## 3. Code Generation (`code-generator.ts`)

**Bottleneck: LLM API latency. Completely network-bound.**

The code generator does:
1. `loadPortSummaries()` -- reads port files (I/O, <10 files)
2. `loadAdjacentContext()` -- reads sibling adapters (I/O, <5 files)
3. `buildSystemPrompt()` -- string concatenation (microseconds)
4. `this.llm.prompt(budget, messages)` -- **LLM API call: 2-30 seconds**
5. `stripCodeFences()` -- regex on response (microseconds)
6. `this.build.compile()` -- spawns `tsc`/`go build` (external process)
7. Refinement loop: up to 2 more LLM calls if arch violations found

**Rust advantage: Zero.** A single LLM API call takes 2-30 seconds. The string manipulation around it takes <1ms. Even the refinement loop makes at most 2 additional LLM calls. The language of the orchestrator is irrelevant when 99.99% of wall time is waiting for HTTP responses.

## 4. Swarm Orchestration (`swarm-orchestrator.ts`)

**Bottleneck: Process spawning + LLM API latency.**

The orchestrator:
1. Calls `swarm.init()` -- ruflo CLI subprocess
2. Creates tasks via `swarm.createTask()` -- ruflo subprocess per task
3. Executes steps in waves with `Promise.allSettled()` -- each step spawns a Claude agent
4. Each agent does its own LLM calls (minutes per agent)

**Rust advantage: Zero.** Each "step" spawns an external process (Claude Code agent) that runs for minutes. The orchestrator is purely a coordinator waiting on subprocess completion. The overhead of Node.js vs Rust for spawning and awaiting processes is unmeasurable against multi-minute agent execution times.

## 5. MCP Server (`mcp-adapter.ts`)

**Bottleneck: LLM/analysis latency behind each tool call.**

The MCP adapter is a JSON-RPC dispatch layer:
- Receives a tool call (JSON parse -- microseconds)
- Routes to the appropriate use case (switch statement)
- Awaits the use case result (seconds: LLM calls, file I/O, or analysis)
- Returns JSON response (serialization -- microseconds)

**Rust advantage: Negligible.** JSON-RPC parsing/serialization is not a bottleneck when every tool handler awaits multi-second operations. The MCP server handles maybe 1-10 requests per second at peak. Even Node.js handles thousands of JSON-RPC messages per second.

## 6. hex-hub Rust Binary Analysis

The hex-hub is a good case study. It is:
- An HTTP server (axum) with WebSocket support
- In-memory state with `RwLock<HashMap<...>>`
- Background eviction task (every 60s)
- Static asset serving via `rust-embed`

**Why Rust works here:** hex-hub is a long-running daemon that benefits from:
- Low memory footprint (no GC, no V8 heap)
- `rust-embed` bakes HTML/CSS/JS into the binary -- single-file deployment
- Predictable latency (no GC pauses) for WebSocket broadcasts

**But these benefits don't extend to the framework.** hex-hub is a simple state-holding server. The framework is an orchestration tool that spawns processes and waits on LLM APIs.

---

## Summary Table

| Operation | Bottleneck | Rust Speedup | Verdict |
|-----------|-----------|-------------|---------|
| Tree-sitter parsing | File I/O + WASM (already native) | ~0% | **No benefit** |
| Import graph analysis | N file reads, then O(V+E) graph | ~0% | **No benefit** |
| Code generation | LLM API: 2-30s per call | 0% | **No benefit** |
| Swarm orchestration | Agent subprocess: minutes each | 0% | **No benefit** |
| MCP server | Use case latency behind each call | ~0% | **No benefit** |
| CLI startup | V8 init (~200ms) vs native (~5ms) | ~195ms once | **Minor benefit** |

## Conclusion

**Rust provides no meaningful performance improvement for hex's actual workloads.**

Every performance-sensitive operation is bottlenecked on:
1. **File I/O** (reading source files for analysis)
2. **LLM API latency** (seconds to tens of seconds per call)
3. **Subprocess execution** (agent processes running for minutes)
4. **External tool compilation** (`tsc`, `go build`, `cargo build`)

The CPU-intensive part of AST parsing is already native code via web-tree-sitter WASM. The graph algorithms operate on tiny datasets (<200 nodes). The MCP server handles trivial request volumes.

The one measurable win -- CLI startup time (~200ms faster) -- is a one-time cost per invocation and irrelevant for interactive or daemon usage.

### Where Rust IS justified

- **hex-hub daemon**: Long-running, low-memory, single-binary deployment. Already in Rust. Correct choice.
- **If hex ever does batch analysis of 10,000+ files**: Rust's parallelism and memory efficiency would matter. Current project sizes (20-200 files) do not reach this threshold.

### Cost of Rust rewrite

- Loss of `web-tree-sitter` ecosystem (though `tree-sitter` Rust crate exists)
- Loss of direct npm distribution (`npx hex`)
- Increased build complexity (cross-compilation for macOS/Linux/Windows)
- Longer development cycles for adapter changes
- Team must maintain two language ecosystems (Rust framework + TypeScript/Go/Rust target projects)
