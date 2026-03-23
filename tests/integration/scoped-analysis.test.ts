/**
 * Integration Tests: Scoped Architecture Analysis
 *
 * Verifies that `analyzeArchitecture(path)` scopes file discovery to the
 * given target path, so `hex analyze hex-hub/` only sees Rust files and
 * `hex analyze examples/weather/backend/` only sees Go files.
 *
 * These tests assume the scoping fix is in place: analyzeArchitecture
 * passes its path argument through to collectSummaries.
 */
import { describe, it, expect, beforeAll } from 'bun:test';
import { createAppContext } from '../../src/composition-root.js';

const PROJECT_ROOT = '/Volumes/ExtendedStorage/PARA/01-Projects/hex-intf';

// Skipped: depends on tree-sitter + createAppContext which fails without native bindings.
// See workplan: feat-test-suite-cleanup.json
describe.skip('Scoped architecture analysis', () => {
  let ctx: Awaited<ReturnType<typeof createAppContext>>;

  beforeAll(async () => {
    ctx = await createAppContext(PROJECT_ROOT);
  });

  // ── Test 1: hex-hub scoped analysis finds only Rust files ──────

  it('scoped analysis of hex-hub finds only Rust files', async () => {
    const result = await ctx.archAnalyzer.analyzeArchitecture('hex-hub');

    // hex-hub is now a thin binary (main.rs only) — logic lives in hex-hub-core.
    // Should still find at least 1 Rust source file.
    expect(result.summary.totalFiles).toBeGreaterThanOrEqual(1);

    // Verify NO TypeScript or Go files leaked in
    const edges = await ctx.archAnalyzer.buildDependencyGraph('hex-hub');
    const allFiles = new Set<string>();
    for (const edge of edges) {
      allFiles.add(edge.from);
      allFiles.add(edge.to);
    }

    for (const file of allFiles) {
      expect(file).not.toMatch(/\.ts$/);
      expect(file).not.toMatch(/\.go$/);
    }
  }, 15000);

  // ── Test 2: weather backend scoped analysis finds only Go files ─

  it('scoped analysis of weather backend finds only Go files', async () => {
    const result = await ctx.archAnalyzer.analyzeArchitecture(
      'examples/weather/backend',
    );

    // Non-test Go files: composition-root.go, core/domain/index.go,
    // core/ports/index.go, core/usecases/f1_service.go,
    // adapters/primary/http_adapter.go,
    // adapters/secondary/cache_adapter.go, jolpica_adapter.go — ~7-8 files
    // (test files *_test.go are excluded by default)
    expect(result.summary.totalFiles).toBeGreaterThanOrEqual(5);

    // Verify NO TypeScript or Rust files leaked in
    const edges = await ctx.archAnalyzer.buildDependencyGraph(
      'examples/weather/backend',
    );
    const allFiles = new Set<string>();
    for (const edge of edges) {
      allFiles.add(edge.from);
      allFiles.add(edge.to);
    }

    for (const file of allFiles) {
      expect(file).not.toMatch(/\.ts$/);
      expect(file).not.toMatch(/\.rs$/);
    }
  }, 15000);

  // ── Test 3: Root analysis backward compatibility ────────────────

  it('root analysis still finds TypeScript files from src/', async () => {
    const result = await ctx.archAnalyzer.analyzeArchitecture('.');

    // The hex project itself has 15+ TypeScript source files
    expect(result.summary.totalFiles).toBeGreaterThan(15);
    expect(result.summary.totalExports).toBeGreaterThan(20);
    expect(result.summary.healthScore).toBeGreaterThan(0);

    // Build edges and check that src/ TypeScript files are present
    const edges = await ctx.archAnalyzer.buildDependencyGraph('.');
    const fromFiles = edges.map((e) => e.from);
    const hasTsFiles = fromFiles.some((f) => f.endsWith('.ts'));
    expect(hasTsFiles).toBe(true);

    // Root analysis excludes examples/ by default, so no Go files
    const hasGoFiles = fromFiles.some((f) => f.endsWith('.go'));
    expect(hasGoFiles).toBe(false);
  }, 15000);

  // ── Test 4: Scoped analysis detects Go module prefix ────────────

  it('scoped analysis of weather backend detects Go module prefix', async () => {
    // The weather backend has go.mod with "module hex-f1"
    // When scoped to examples/weather/backend, the analyzer should detect
    // this and resolve Go import paths correctly (stripping the module prefix)
    const result = await ctx.archAnalyzer.analyzeArchitecture(
      'examples/weather/backend',
    );

    // If Go module detection works, internal imports resolve to local paths
    // rather than staying as "hex-f1/src/core/ports" etc.
    // This means the dependency graph should have edges between local files
    const edges = await ctx.archAnalyzer.buildDependencyGraph(
      'examples/weather/backend',
    );

    // There should be at least some edges — Go files import each other
    // If module prefix detection fails, imports won't resolve to local files
    // and the graph will have zero or near-zero edges
    if (result.summary.totalFiles >= 5) {
      // Only assert edges if we found enough files (grammar might be missing)
      expect(edges.length).toBeGreaterThanOrEqual(1);

      // Verify resolved import targets don't contain the raw module prefix
      // (they should be resolved to relative paths like "examples/weather/...")
      for (const edge of edges) {
        expect(edge.to).not.toStartWith('hex-f1/');
      }
    }
  }, 15000);

  // ── Test 5: Scoped analysis excludes Rust target/ directory ─────

  it('scoped analysis of hex-hub excludes target/ build artifacts', async () => {
    const result = await ctx.archAnalyzer.analyzeArchitecture('hex-hub');

    // hex-hub/target/ exists with build artifacts — these must be excluded
    // Build the dependency graph to inspect all discovered files
    const edges = await ctx.archAnalyzer.buildDependencyGraph('hex-hub');
    const allFiles = new Set<string>();
    for (const edge of edges) {
      allFiles.add(edge.from);
      allFiles.add(edge.to);
    }

    // No file path should contain /target/
    for (const file of allFiles) {
      expect(file).not.toContain('/target/');
    }

    // The file count should match only src/ files (not target/ artifacts)
    // hex-hub is now a thin binary (main.rs) — logic moved to hex-hub-core
    // A reasonable upper bound proves target/ was excluded
    expect(result.summary.totalFiles).toBeLessThan(10);
  }, 15000);
});
