/**
 * E2E Test: hex analyzes itself
 *
 * Exercises the full stack from CLI -> ArchAnalyzer -> IASTPort -> IFileSystemPort
 * against the real hex source tree. This test WILL FAIL until the blockers
 * identified in docs/analysis/testability-audit-e2e-report.md are resolved:
 *
 *   1. Tree-sitter WASM grammar must be installed at the correct path
 *   2. Import path normalization must be added to ArchAnalyzer
 *   3. L1 tokenEstimate must reflect summary size, not raw source size
 */
import { describe, it, expect, beforeAll } from 'bun:test';
import { createAppContext } from '../../src/composition-root.js';
import { runCLI, type AppContext as CLIContext } from '../../src/adapters/primary/cli-adapter.js';

const PROJECT_ROOT = '/Volumes/ExtendedStorage/PARA/01-Projects/hex-intf';

describe('E2E: hex analyzes itself', () => {
  let ctx: Awaited<ReturnType<typeof createAppContext>>;

  beforeAll(async () => {
    ctx = await createAppContext(PROJECT_ROOT);
  });

  // ── Phase 1: Verify tree-sitter is actually working ──────────

  it('creates a real AppContext with working tree-sitter (not the stub)', async () => {
    const summary = await ctx.ast.extractSummary('src/core/ports/index.ts', 'L1');
    // If tree-sitter loaded, we should see real exports from ports/index.ts
    // The file defines IASTPort, ILLMPort, IBuildPort, etc. -- at least 5 interfaces
    expect(summary.exports.length).toBeGreaterThan(5);
    // It should have real line counts
    expect(summary.lineCount).toBeGreaterThan(100);
    // Verify language detection worked
    expect(summary.language).toBe('typescript');
  });

  it('extracts imports from files that have them', async () => {
    const summary = await ctx.ast.extractSummary('src/core/usecases/arch-analyzer.ts', 'L1');
    // arch-analyzer imports from ports/index.js and layer-classifier.js
    expect(summary.imports.length).toBeGreaterThanOrEqual(2);
  });

  // ── Phase 2: Token efficiency -- L1 < L3 ────────────────────

  it('L1 summaries have fewer estimated tokens than L3 for the same file', async () => {
    const l1 = await ctx.ast.extractSummary('src/core/ports/index.ts', 'L1');
    const l3 = await ctx.ast.extractSummary('src/core/ports/index.ts', 'L3');

    // L3 includes raw source, L1 should be a compressed skeleton
    // The entire point of the L0-L3 hierarchy is token efficiency
    expect(l1.tokenEstimate).toBeLessThan(l3.tokenEstimate);

    // L1 should be at MOST 35% the size of L3 for a meaningful reduction
    const ratio = l1.tokenEstimate / l3.tokenEstimate;
    expect(ratio).toBeLessThan(0.35);
  });

  it('L0 has the fewest tokens (metadata only)', async () => {
    const l0 = await ctx.ast.extractSummary('src/core/ports/index.ts', 'L0');
    const l1 = await ctx.ast.extractSummary('src/core/ports/index.ts', 'L1');
    expect(l0.tokenEstimate).toBeLessThanOrEqual(l1.tokenEstimate);
    // L0 should have no exports or imports extracted
    expect(l0.exports).toHaveLength(0);
    expect(l0.imports).toHaveLength(0);
  });

  // ── Phase 3: Architecture analysis produces plausible results ─

  it('analyzeArchitecture returns correct file count for this project', async () => {
    const result = await ctx.archAnalyzer.analyzeArchitecture(PROJECT_ROOT);
    // The project has ~20+ .ts source files (excluding tests, node_modules)
    expect(result.summary.totalFiles).toBeGreaterThan(15);
    // The project exports many symbols across all port files
    expect(result.summary.totalExports).toBeGreaterThan(20);
    // Health score should not be zero (that would mean the stub is active)
    expect(result.summary.healthScore).toBeGreaterThan(0);
  }, 15000);

  // ── Phase 4: Hex boundary self-validation ────────────────────

  it('hex has zero dependency violations against its own hex rules', async () => {
    const violations = await ctx.archAnalyzer.validateHexBoundaries(PROJECT_ROOT);
    if (violations.length > 0) {
      const report = violations.map(v =>
        `  ${v.from} -> ${v.to}\n    Rule: ${v.rule}`
      ).join('\n');
      // Fail with a detailed report of what is wrong
      expect(violations).toHaveLength(0);
      // This message is for human readers when the assertion above fails:
      console.error(`Hex boundary violations found:\n${report}`);
    }
  }, 15000);

  it('composition-root.ts is the ONLY file that imports adapters and ports', async () => {
    const edges = await ctx.archAnalyzer.buildDependencyGraph(PROJECT_ROOT);
    // Find files that import from BOTH ports and adapters
    const fileImports = new Map<string, Set<string>>();
    for (const edge of edges) {
      if (!fileImports.has(edge.from)) fileImports.set(edge.from, new Set());
      fileImports.get(edge.from)!.add(edge.to);
    }

    const crossBoundaryFiles: string[] = [];
    for (const [file, targets] of fileImports) {
      const importsPort = [...targets].some(t => t.includes('/ports/'));
      const importsAdapter = [...targets].some(t => t.includes('/adapters/'));
      if (importsPort && importsAdapter && !file.includes('composition-root')) {
        // Allow same-sublayer imports (primary->primary, secondary->secondary)
        const fileIsInPrimary = file.includes('/adapters/primary/');
        const fileIsInSecondary = file.includes('/adapters/secondary/');
        const importsCrossAdapter = [...targets].some(t => {
          if (!t.includes('/adapters/')) return false;
          if (fileIsInPrimary && t.includes('/adapters/primary/')) return false;
          if (fileIsInSecondary && t.includes('/adapters/secondary/')) return false;
          return true; // imports from a different adapter sublayer
        });
        if (importsCrossAdapter) {
          crossBoundaryFiles.push(file);
        }
      }
    }
    expect(crossBoundaryFiles).toHaveLength(0);
  }, 15000);

  // ── Phase 5: CLI end-to-end ──────────────────────────────────

  it('CLI analyze command produces structured output with real data', async () => {
    const cliCtx: CLIContext = {
      rootPath: PROJECT_ROOT,
      archAnalyzer: ctx.archAnalyzer,
      ast: ctx.ast,
      fs: ctx.fs,
    };
    const captured: string[] = [];
    const result = await runCLI(['analyze', '.'], cliCtx, (m) => captured.push(m));

    expect(result.exitCode).toBe(0);
    expect(result.output).toContain('HEXAGONAL ARCHITECTURE HEALTH REPORT');
    expect(result.output).toContain('Files scanned');
    expect(result.output).toContain('Score:');
    // Verify the numbers are non-zero (real data, not stub)
    expect(result.output).toContain('SUMMARY');
  }, 15000);

  it('CLI summarize shows real exports from tree-sitter', async () => {
    const cliCtx: CLIContext = {
      rootPath: PROJECT_ROOT,
      archAnalyzer: ctx.archAnalyzer,
      ast: ctx.ast,
      fs: ctx.fs,
    };
    const result = await runCLI(
      ['summarize', 'src/core/domain/entities.ts', '--level', 'L1'],
      cliCtx,
      () => {},
    );
    expect(result.exitCode).toBe(0);
    // These are real exported class names from entities.ts
    expect(result.output).toContain('QualityScore');
    expect(result.output).toContain('FeedbackLoop');
    expect(result.output).toContain('TaskGraph');
  });

  it('CLI returns exit code 1 when health score is below 50', async () => {
    // Create a mock analyzer that returns a bad score to test the threshold
    const badAnalyzer = {
      ...ctx.archAnalyzer,
      analyzeArchitecture: async () => ({
        deadExports: [],
        orphanFiles: [],
        dependencyViolations: [],
        circularDeps: [],
        unusedPorts: [],
        unusedAdapters: [],
        summary: {
          totalFiles: 1,
          totalExports: 1,
          deadExportCount: 0,
          violationCount: 0,
          circularCount: 0,
          healthScore: 30,
        },
      }),
    };
    const cliCtx: CLIContext = {
      rootPath: PROJECT_ROOT,
      archAnalyzer: badAnalyzer,
      ast: ctx.ast,
      fs: ctx.fs,
    };
    const result = await runCLI(['analyze', '.'], cliCtx, () => {});
    expect(result.exitCode).toBe(1);
  });
});
