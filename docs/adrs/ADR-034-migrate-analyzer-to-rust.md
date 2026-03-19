# ADR-034: Migrate Hex Analyzer from TypeScript to Rust

## Status
Proposed — 2025-07-19

## Context

The hex analyzer currently lives entirely in TypeScript:

| File | LOC | Responsibility |
|------|-----|----------------|
| `src/core/usecases/arch-analyzer.ts` | 627 | Core analysis orchestrator (boundaries, dead exports, cycles, orphans, health score) |
| `src/core/usecases/layer-classifier.ts` | 147 | Hex layer classification (directory + filename pattern matching) |
| `src/core/usecases/path-normalizer.ts` | 175 | Import path resolution (TS, Go, Rust) |
| `src/core/usecases/import-boundary-checker.ts` | 114 | Pre-generation boundary validation (shift-left) |
| `src/core/ports/index.ts` | ~15 | `IArchAnalysisPort` interface |
| `src/adapters/secondary/treesitter-adapter.ts` | ~800 | Tree-sitter WASM/NAPI parser + L0–L3 summaries |
| `src/adapters/primary/cli-adapter.ts` | ~50 | `hex analyze` CLI command |
| `src/adapters/primary/mcp-adapter.ts` | ~40 | `hex_analyze` / `hex_analyze_json` MCP tools |

We have already moved the agent runtime (hex-agent) and orchestration hub (hex-nexus) to Rust. The analyzer is one of the last major TS subsystems. Moving it to Rust:

1. **Eliminates the push model** — hex-nexus can analyze projects directly instead of depending on the TS CLI to compute and push results
2. **Faster analysis** — tree-sitter native Rust bindings are 3-10x faster than WASM
3. **Single binary** — `hex-nexus` becomes fully self-contained for analysis
4. **Reduces TS surface** — moves toward eventual full deprecation of the TS codebase

### Current Data Flow (Push Model)
```
TS CLI (hex analyze) → tree-sitter WASM → arch-analyzer.ts → POST /api/push → hex-nexus stores
```

### Target Data Flow (Native)
```
hex-nexus (Rust) → tree-sitter native → analysis module → in-process → state
          ↑
     hex-cli (Rust thin CLI) calls GET /api/{project}/analyze (on-demand)
```

## Decision

Migrate the hex analyzer to Rust inside `hex-nexus`, following a phased approach:

### Phase 1: Domain + Ports (Tier 0)

Create analysis domain types and port traits in hex-nexus:

```
hex-nexus/src/
  analysis/
    mod.rs              # Module root, re-exports
    domain.rs           # ImportEdge, DeadExport, DependencyViolation, ArchAnalysisResult, HexLayer, HealthScore
    ports.rs            # ArchAnalysisPort trait, AstPort trait (for tree-sitter)
    layer_classifier.rs # Hex layer classification (directory + filename patterns)
    path_normalizer.rs  # Import path resolution (TS, Go, Rust)
```

**Key types:**
```rust
pub enum HexLayer {
    Domain, Ports, Usecases,
    AdaptersPrimary, AdaptersSecondary,
    Infrastructure, CompositionRoot, EntryPoint, Unknown,
}

pub struct ImportEdge {
    pub from_file: String,
    pub to_file: String,
    pub from_layer: HexLayer,
    pub to_layer: HexLayer,
    pub import_path: String,
}

pub struct DependencyViolation {
    pub edge: ImportEdge,
    pub rule: String,       // e.g. "adapters/primary may only import from ports"
}

pub struct DeadExport {
    pub file: String,
    pub export_name: String,
    pub line: usize,
}

pub struct ArchAnalysisResult {
    pub violations: Vec<DependencyViolation>,
    pub dead_exports: Vec<DeadExport>,
    pub circular_deps: Vec<Vec<String>>,
    pub orphan_files: Vec<String>,
    pub unused_ports: Vec<String>,
    pub health_score: u8,       // 0–100
    pub file_count: usize,
    pub edge_count: usize,
}

#[async_trait]
pub trait ArchAnalysisPort: Send + Sync {
    async fn analyze(&self, root_path: &Path) -> Result<ArchAnalysisResult>;
    async fn validate_boundaries(&self, root_path: &Path) -> Result<Vec<DependencyViolation>>;
    async fn find_dead_exports(&self, root_path: &Path) -> Result<Vec<DeadExport>>;
    async fn detect_circular_deps(&self, root_path: &Path) -> Result<Vec<Vec<String>>>;
}
```

### Phase 2: Tree-Sitter Native Adapter (Tier 1)

Add tree-sitter Rust crates for import extraction:

```toml
# hex-nexus/Cargo.toml additions
tree-sitter = "0.24"
tree-sitter-typescript = "0.24"
tree-sitter-go = "0.23"
tree-sitter-rust = "0.23"
```

```
hex-nexus/src/
  analysis/
    treesitter_adapter.rs  # Native tree-sitter parser — extract imports/exports per file
```

This adapter implements a new `AstPort` trait:
```rust
#[async_trait]
pub trait AstPort: Send + Sync {
    /// Extract all import statements from a source file
    fn extract_imports(&self, path: &Path, source: &str, lang: Language) -> Result<Vec<ImportStatement>>;
    /// Extract all export declarations from a source file
    fn extract_exports(&self, path: &Path, source: &str, lang: Language) -> Result<Vec<ExportDeclaration>>;
}
```

### Phase 3: Analysis Use Cases (Tier 3)

Port the core analysis logic:

```
hex-nexus/src/
  analysis/
    boundary_checker.rs    # Validate hex dependency direction rules
    dead_export_finder.rs  # Find exports with zero consumers
    cycle_detector.rs      # DFS-based circular dependency detection
    analyzer.rs            # Orchestrator: combines all checks, computes health score
```

The `ArchAnalyzer` struct composes `AstPort` and filesystem access:
```rust
pub struct ArchAnalyzer {
    ast: Arc<dyn AstPort>,
}

impl ArchAnalysisPort for ArchAnalyzer {
    async fn analyze(&self, root_path: &Path) -> Result<ArchAnalysisResult> { ... }
}
```

### Phase 4: REST Routes + MCP Integration (Tier 2)

Add analysis routes to hex-nexus:

```
hex-nexus/src/
  routes/
    analysis.rs   # GET /api/{project_id}/analyze, GET /api/{project_id}/analyze/json
```

Update MCP tools in the TS bridge to call hex-nexus instead of running local analysis.

### Phase 5: hex-cli Thin Client

The new Rust CLI (`hex-cli/`) provides `hex analyze` by calling hex-nexus REST:

```
hex-cli/src/
  commands/
    analyze.rs   # hex analyze <path> → GET /api/{project}/analyze
```

### Phase 6: Deprecate TypeScript Analyzer

1. Mark TS analyzer files with `@deprecated` JSDoc tags
2. Add deprecation warning to `hex analyze` CLI (TS version)
3. Update MCP adapter to proxy to hex-nexus when available, fall back to TS
4. Remove TS analyzer after 2 release cycles

## Migration Order

```
Phase 1 (domain+ports)     → no breaking changes, additive
Phase 2 (tree-sitter)      → no breaking changes, additive
Phase 3 (analysis logic)   → no breaking changes, additive
Phase 4 (REST routes)      → hex-nexus gains /analyze endpoints
Phase 5 (hex-cli)          → new binary, TS CLI still works
Phase 6 (deprecate TS)     → remove TS analyzer files
```

## Consequences

### Positive
- hex-nexus becomes self-sufficient for architecture analysis
- 3-10x faster parsing with native tree-sitter (no WASM overhead)
- Single binary deployment — no Node.js runtime needed for analysis
- Enables server-side analysis triggers (webhooks, CI integration)
- Push model eliminated — analysis is on-demand

### Negative
- Duplicated logic during migration window (TS + Rust coexist)
- tree-sitter Rust crates add ~2MB to binary size
- Need to maintain parity with TS feature set until deprecation

### Risks
- Tree-sitter query differences between WASM and native may cause subtle divergence
- `@hex:public` annotation parsing needs careful porting
- Go structural interface matching is complex to re-implement

## Files Affected

### New (Rust)
- `hex-nexus/src/analysis/mod.rs`
- `hex-nexus/src/analysis/domain.rs`
- `hex-nexus/src/analysis/ports.rs`
- `hex-nexus/src/analysis/layer_classifier.rs`
- `hex-nexus/src/analysis/path_normalizer.rs`
- `hex-nexus/src/analysis/treesitter_adapter.rs`
- `hex-nexus/src/analysis/boundary_checker.rs`
- `hex-nexus/src/analysis/dead_export_finder.rs`
- `hex-nexus/src/analysis/cycle_detector.rs`
- `hex-nexus/src/analysis/analyzer.rs`
- `hex-nexus/src/routes/analysis.rs`
- `hex-cli/src/commands/analyze.rs`

### Modified
- `hex-nexus/Cargo.toml` (tree-sitter deps)
- `hex-nexus/src/lib.rs` (register analysis module)
- `hex-nexus/src/routes/mod.rs` (register analysis routes)

### Deprecated (TypeScript — Phase 6)
- `src/core/usecases/arch-analyzer.ts`
- `src/core/usecases/layer-classifier.ts`
- `src/core/usecases/path-normalizer.ts`
- `src/core/usecases/import-boundary-checker.ts`
- Related test files in `tests/unit/`
