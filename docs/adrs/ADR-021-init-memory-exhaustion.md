# ADR-021: Hex Initialization Memory Exhaustion in Existing Large Projects

**Status**: Accepted
**Date**: 2026-03-17
**Deciders**: Core Team
**Related**: ADR-001 (Hexagonal Architecture), ADR-009 (Ruflo Required)

## Context

When running `hex init` on an existing large project (103K+ LOC Rust codebase), Node.js exhausted its JavaScript heap (~4GB) and crashed during filesystem scanning. The initialization process did not complete, preventing hex adoption in established codebases.

### Observed Failure

```
FATAL ERROR: Reached heap limit Allocation failed - JavaScript heap out of memory
```

**Crash location**: `node::fs::AfterScanDir` — occurred during recursive directory scanning before any user-facing progress indication.

### Project Characteristics (rust-hsa)

- **Size**: 103,361 lines Rust across 120 files
- **Test suite**: 1,698 tests with extensive fixtures
- **Artifacts**:
  - Build cache (`target/` — typically 1-5GB for Rust projects)
  - Snapshot data directories (enterprise network topologies)
  - Generated HTML reports (40+ files, untracked)
  - Test output directories
  - `node_modules/` (already present from other tools)

### Contributing Factors

1. **No default ignore patterns**: Hex scanned build artifacts, test outputs, and cached data
2. **No memory ceiling**: Default Node.js heap limits (~4GB on 64-bit systems) insufficient for large codebases
3. **No progress indication**: Silent failure after ~35 seconds of scanning
4. **Missing dependency**: Repeated AgentDB controller warnings suggest incomplete installation state

```
[AgentDB Patch] Controller index not found:
.../node_modules/agentdb/dist/controllers/index.js
```

## Decision

Implement a layered fix across six areas, prioritized by impact and effort.

### 1. Smart Default Exclusions (Priority: HIGH)

Add `.hexignore` defaults that mirror `.gitignore` conventions:

```gitignore
# Build artifacts
target/          # Rust
build/           # Generic
dist/            # JavaScript
out/             # Java/Kotlin
bin/             # General
obj/             # C#/.NET

# Dependencies
node_modules/
vendor/
.venv/
venv/

# Test outputs
coverage/
test-results/
.pytest_cache/
__pycache__/

# IDE
.vscode/
.idea/
*.swp
.DS_Store

# Generated files
*.html (if not tracked)
*.log
*.tmp
```

**Implementation**: Check for `.hexignore` first, fall back to built-in defaults, respect `.gitignore` as third option.

### 2. Streaming Filesystem Scanner with Backpressure (Priority: HIGH)

Replace recursive in-memory directory accumulation with streaming:

```javascript
import { Readable } from 'stream';
import { pipeline } from 'stream/promises';

async function* walkDirectory(root, ignore) {
  const queue = [root];
  while (queue.length > 0) {
    const dir = queue.shift();
    const entries = await fs.readdir(dir, { withFileTypes: true });

    for (const entry of entries) {
      if (ignore.test(entry.name)) continue;

      const fullPath = path.join(dir, entry.name);
      if (entry.isDirectory()) {
        queue.push(fullPath);
      } else {
        yield { path: fullPath, size: entry.size };
      }
    }
  }
}

// Process with bounded concurrency
await pipeline(
  Readable.from(walkDirectory(projectRoot, ignorePatterns)),
  new BatchProcessor({ highWaterMark: 1000 }),  // Limit in-flight items
  indexer
);
```

### 3. Memory-Aware Initialization Modes (Priority: MEDIUM)

Add initialization modes:

```bash
# Default: full indexing with auto-detection
hex init

# Fast: skip indexing, on-demand file discovery
hex init --fast

# Selective: index only specified directories
hex init --include src/ tests/

# Memory-constrained: increase Node heap automatically
hex init --large-project  # Sets NODE_OPTIONS internally
```

Auto-detect large projects:

```javascript
const quickScan = await countFilesShallow(projectRoot, { maxDepth: 2 });
if (quickScan.totalSize > 1_000_000_000) {  // 1GB threshold
  console.warn('Large project detected. Using streaming mode...');
  useStreamingIndexer = true;
}
```

### 4. Progress Indication and Cancellation (Priority: MEDIUM)

```
Initializing hex project...
├─ Scanning filesystem: 15,432 files found (3.2 GB)
├─ Applying ignore patterns: 12,108 files excluded
├─ Indexing 3,324 source files...
│  └─ [████████░░░░░░░░░░░░] 45% (1,496/3,324) — 2.1s elapsed
└─ Building project graph...

Press Ctrl+C to cancel and save partial index
```

### 5. Incremental Initialization (Priority: LOW)

Phase initialization:

```bash
# Phase 1: Minimal bootstrap
hex init --minimal
# Creates config, registers with hub, NO indexing

# Phase 2: Selective indexing (user-driven)
hex index add src/ --recursive
hex index add tests/ --filter '**/*.test.ts'

# Phase 3: Background completion
hex index complete --background
```

Store indexed state in `.hex/index.db` (SQLite) for incremental updates.

### 6. Dependency Integrity Check (Priority: MEDIUM)

Before initialization, verify critical dependencies:

```javascript
function validateDependencies() {
  const required = [
    'agentdb/dist/controllers/index.js',
    'agentdb/dist/store/index.js',
  ];

  const missing = required.filter(p => !fs.existsSync(path.join('node_modules', p)));

  if (missing.length > 0) {
    console.error('Missing dependencies:', missing);
    console.log('Run: npm install --force');
    process.exit(1);
  }
}
```

## Implementation Order

1. **Week 1**: Default exclusions + dependency validation (quick wins)
2. **Week 2**: Streaming scanner + progress indication (core fix)
3. **Week 3**: Auto-detection + memory modes (user experience)
4. **Week 4**: Incremental initialization (advanced use case)

## Success Metrics

- **Memory**: Initialization completes with <2GB heap on 100K LOC projects
- **Time**: Indexing completes in <10s for typical projects (<10K files)
- **Reliability**: Zero OOM crashes on projects with standard `.gitignore` patterns
- **Adoption**: 95% of `hex init` runs succeed without manual intervention

## Testing Strategy

Create synthetic large projects:

```bash
# Rust project with build artifacts
cargo new large-rust-project
cd large-rust-project
cargo build --release  # Generates target/
# Add 50K LOC of code + 1K test files

# JavaScript monorepo
mkdir monorepo && cd monorepo
for i in {1..100}; do
  mkdir -p packages/pkg-$i/node_modules
done
```

Measure:
- Peak memory usage (`process.memoryUsage().heapUsed`)
- Time to complete initialization
- Number of files indexed vs excluded
- Success rate across 20 diverse project types

## Security Considerations

- **Path traversal**: Validate all paths are within project root
- **Symlink loops**: Track visited inodes to prevent infinite recursion
- **Malicious ignores**: Limit ignore pattern complexity (max 1000 rules)
- **Resource exhaustion**: Hard cap on total indexed files (default: 1M files)

## Alternatives Considered

### A. Require explicit opt-in for large projects
**Rejected**: Poor UX — users shouldn't need to diagnose their project size first.

### B. Offload indexing to Rust/Go native binary
**Deferred**: High implementation cost, complex cross-platform builds.

### C. Cloud-based indexing service
**Rejected**: Privacy concerns, network dependency, latency.

## Open Questions

1. Should hex respect `.gitignore` by default, or require explicit `.hexignore`?
2. What's the right balance between index completeness and initialization speed?
3. Should incremental indexing be the default for projects >10K files?
4. How to handle polyglot monorepos (e.g., Rust + TypeScript + Python)?

## References

- [Node.js Heap Limits](https://nodejs.org/api/cli.html#--max-old-space-sizesize-in-megabytes)
- [ignore npm package](https://www.npmjs.com/package/ignore) — `.gitignore` parser
- [fast-glob](https://www.npmjs.com/package/fast-glob) — Performant glob matcher
- [Streaming Filesystem Operations](https://nodejs.org/api/stream.html)

## Appendix: Reproduction Steps

```bash
git clone https://github.com/example/rust-hsa
cd rust-hsa
cargo build --release  # Populate target/
hex init               # Triggers OOM after ~35s
```

**Expected**: Completes initialization, skips `target/` and `snapshot/`
**Actual**: FATAL ERROR — heap exhaustion at ~4GB
