import { describe, it, expect } from 'bun:test';
import { runCLI, type AppContext } from '../../src/adapters/primary/cli-adapter.js';
import type {
  IArchAnalysisPort,
  IASTPort,
  IFileSystemPort,
  ArchAnalysisResult,
  ASTSummary,
} from '../../src/core/ports/index.js';

// ─── Mock Factories ─────────────────────────────────────

function mockArchResult(overrides: Partial<ArchAnalysisResult['summary']> = {}): ArchAnalysisResult {
  return {
    deadExports: [], orphanFiles: [], dependencyViolations: [],
    circularDeps: [], unusedPorts: [], unusedAdapters: [],
    summary: {
      totalFiles: 10, totalExports: 25, deadExportCount: 0,
      violationCount: 0, circularCount: 0, healthScore: 100,
      ...overrides,
    },
  };
}

function mockSummary(filePath: string, level: ASTSummary['level'] = 'L1'): ASTSummary {
  return {
    filePath, language: 'typescript', level,
    exports: [{ name: 'Foo', kind: 'function' }],
    imports: [], dependencies: [], lineCount: 42, tokenEstimate: 200,
  };
}

function mockContext(overrides: Partial<{
  analyzeResult: ArchAnalysisResult;
  summaryResult: ASTSummary;
  analyzeThrows: Error;
}> = {}): AppContext {
  const archAnalyzer: IArchAnalysisPort = {
    buildDependencyGraph: async () => [],
    findDeadExports: async () => [],
    validateHexBoundaries: async () => [],
    detectCircularDeps: async () => [],
    analyzeArchitecture: overrides.analyzeThrows
      ? async () => { throw overrides.analyzeThrows!; }
      : async () => overrides.analyzeResult ?? mockArchResult(),
  };
  const ast: IASTPort = {
    extractSummary: async (fp, lvl) => overrides.summaryResult ?? mockSummary(fp, lvl),
    diffStructural: () => ({ added: [], removed: [], modified: [] }),
  };
  const fs: IFileSystemPort = {
    read: async () => '', write: async () => {},
    exists: async () => true, glob: async () => [],
  };
  return {
    archAnalyzer, ast, fs, rootPath: '/test',
    astIsStub: false, codeGenerator: null, workplanExecutor: null,
    summaryService: {
      summarizeFile: async (fp, lvl) => mockSummary(fp, lvl),
      summarizeProject: async () => [],
    },
  };
}

// ─── Tests ──────────────────────────────────────────────

describe('CLI Adapter', () => {
  it('analyze command calls archAnalyzer and prints results', async () => {
    const captured: string[] = [];
    const ctx = mockContext({ analyzeResult: mockArchResult({ healthScore: 85 }) });
    const result = await runCLI(['analyze'], ctx, (m) => captured.push(m));
    expect(result.exitCode).toBe(0);
    expect(result.output).toContain('85/100');
  });

  it('summarize command calls ast.extractSummary with correct level', async () => {
    let calledLevel: string | undefined;
    const ctx = mockContext();
    ctx.ast.extractSummary = async (_fp, lvl) => {
      calledLevel = lvl;
      return mockSummary('test.ts', lvl);
    };
    const result = await runCLI(['summarize', 'test.ts', '--level', 'L2'], ctx);
    expect(result.exitCode).toBe(0);
    expect(calledLevel).toBe('L2');
    expect(result.output).toContain('test.ts');
  });

  it('help prints usage without error', async () => {
    const ctx = mockContext();
    const result = await runCLI(['help'], ctx);
    expect(result.exitCode).toBe(0);
    expect(result.output).toContain('Usage:');
    expect(result.output).toContain('analyze');
  });

  it('unknown command returns exit code 1', async () => {
    const ctx = mockContext();
    const result = await runCLI(['foobar'], ctx);
    expect(result.exitCode).toBe(1);
    expect(result.output).toContain('Unknown command');
  });

  it('handles errors gracefully', async () => {
    const ctx = mockContext({ analyzeThrows: new Error('boom') });
    const result = await runCLI(['analyze'], ctx);
    expect(result.exitCode).toBe(1);
    expect(result.output).toContain('Error: boom');
  });
});
