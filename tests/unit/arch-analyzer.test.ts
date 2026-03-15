import { describe, it, expect } from 'bun:test';
import type { IASTPort, IFileSystemPort, ASTSummary } from '../../src/core/ports/index.js';
import { ArchAnalyzer } from '../../src/core/usecases/arch-analyzer.js';

// ─── Mock Factories ─────────────────────────────────────

function makeSummary(filePath: string, opts: {
  exports?: ASTSummary['exports'];
  imports?: ASTSummary['imports'];
} = {}): ASTSummary {
  return {
    filePath,
    language: 'typescript',
    level: 'L1',
    exports: opts.exports ?? [],
    imports: opts.imports ?? [],
    dependencies: (opts.imports ?? []).map((i) => i.from),
    lineCount: 20,
    tokenEstimate: 100,
  };
}

function mockFS(files: string[]): IFileSystemPort {
  return {
    read: async () => '',
    write: async () => {},
    exists: async () => true,
    glob: async () => files,
  };
}

function mockAST(summaries: Record<string, ASTSummary>): IASTPort {
  return {
    extractSummary: async (filePath: string) => summaries[filePath] ?? makeSummary(filePath),
    diffStructural: () => ({ added: [], removed: [], modified: [] }),
  };
}

// ─── findDeadExports ────────────────────────────────────

describe('ArchAnalyzer.findDeadExports', () => {
  it('detects an export used by no other file', async () => {
    const files = ['src/core/domain/foo.ts'];
    const summaries = {
      'src/core/domain/foo.ts': makeSummary('src/core/domain/foo.ts', {
        exports: [{ name: 'unusedFn', kind: 'function' }],
      }),
    };
    const analyzer = new ArchAnalyzer(mockAST(summaries), mockFS(files));
    const dead = await analyzer.findDeadExports('/root');
    expect(dead).toHaveLength(1);
    expect(dead[0].exportName).toBe('unusedFn');
  });

  it('does not flag exports used by at least one file', async () => {
    const files = ['src/core/domain/foo.ts', 'src/core/usecases/bar.ts'];
    const summaries = {
      'src/core/domain/foo.ts': makeSummary('src/core/domain/foo.ts', {
        exports: [{ name: 'usedFn', kind: 'function' }],
      }),
      'src/core/usecases/bar.ts': makeSummary('src/core/usecases/bar.ts', {
        imports: [{ names: ['usedFn'], from: '../domain/foo.js' }],
      }),
    };
    const analyzer = new ArchAnalyzer(mockAST(summaries), mockFS(files));
    const dead = await analyzer.findDeadExports('/root');
    expect(dead).toHaveLength(0);
  });

  it('excludes index.ts entry points from dead code', async () => {
    const files = ['src/core/ports/index.ts'];
    const summaries = {
      'src/core/ports/index.ts': makeSummary('src/core/ports/index.ts', {
        exports: [{ name: 'SomePort', kind: 'interface' }],
      }),
    };
    const analyzer = new ArchAnalyzer(mockAST(summaries), mockFS(files));
    const dead = await analyzer.findDeadExports('/root');
    expect(dead).toHaveLength(0);
  });
});

// ─── validateHexBoundaries ──────────────────────────────

describe('ArchAnalyzer.validateHexBoundaries', () => {
  it('passes for correct hex imports (adapter -> port) with relative .js path', async () => {
    const files = ['src/adapters/secondary/db.ts'];
    const summaries = {
      'src/adapters/secondary/db.ts': makeSummary('src/adapters/secondary/db.ts', {
        imports: [{ names: ['IFSPort'], from: '../../core/ports/index.js' }],
      }),
      'src/core/ports/index.ts': makeSummary('src/core/ports/index.ts'),
    };
    const analyzer = new ArchAnalyzer(mockAST(summaries), mockFS(files));
    const violations = await analyzer.validateHexBoundaries('/root');
    expect(violations).toHaveLength(0);
  });

  it('catches domain importing from adapter with relative .js path', async () => {
    const files = ['src/core/domain/entity.ts'];
    const summaries = {
      'src/core/domain/entity.ts': makeSummary('src/core/domain/entity.ts', {
        imports: [{ names: ['DB'], from: '../../adapters/secondary/db.js' }],
      }),
      'src/adapters/secondary/db.ts': makeSummary('src/adapters/secondary/db.ts'),
    };
    const analyzer = new ArchAnalyzer(mockAST(summaries), mockFS(files));
    const violations = await analyzer.validateHexBoundaries('/root');
    expect(violations.length).toBeGreaterThan(0);
    expect(violations[0].fromLayer).toBe('domain');
  });

  it('catches cross-adapter imports with relative path', async () => {
    const files = ['src/adapters/secondary/db.ts'];
    const summaries = {
      'src/adapters/secondary/db.ts': makeSummary('src/adapters/secondary/db.ts', {
        imports: [{ names: ['CLI'], from: '../primary/cli.js' }],
      }),
      'src/adapters/primary/cli.ts': makeSummary('src/adapters/primary/cli.ts'),
    };
    const analyzer = new ArchAnalyzer(mockAST(summaries), mockFS(files));
    const violations = await analyzer.validateHexBoundaries('/root');
    expect(violations.length).toBeGreaterThan(0);
    expect(violations[0].rule).toContain('adapters');
  });
});

// ─── detectCircularDeps ─────────────────────────────────

describe('ArchAnalyzer.detectCircularDeps', () => {
  it('detects A->B->A cycle with relative .js imports', async () => {
    const files = ['src/core/usecases/a.ts', 'src/core/usecases/b.ts'];
    const summaries = {
      'src/core/usecases/a.ts': makeSummary('src/core/usecases/a.ts', {
        imports: [{ names: ['B'], from: './b.js' }],
      }),
      'src/core/usecases/b.ts': makeSummary('src/core/usecases/b.ts', {
        imports: [{ names: ['A'], from: './a.js' }],
      }),
    };
    const analyzer = new ArchAnalyzer(mockAST(summaries), mockFS(files));
    const cycles = await analyzer.detectCircularDeps('/root');
    expect(cycles.length).toBeGreaterThan(0);
  });

  it('detects A->B->C->A cycle with relative .js imports', async () => {
    const files = ['src/core/usecases/a.ts', 'src/core/usecases/b.ts', 'src/core/domain/c.ts'];
    const summaries = {
      'src/core/usecases/a.ts': makeSummary('src/core/usecases/a.ts', {
        imports: [{ names: ['B'], from: './b.js' }],
      }),
      'src/core/usecases/b.ts': makeSummary('src/core/usecases/b.ts', {
        imports: [{ names: ['C'], from: '../domain/c.js' }],
      }),
      'src/core/domain/c.ts': makeSummary('src/core/domain/c.ts', {
        imports: [{ names: ['A'], from: '../usecases/a.js' }],
      }),
    };
    const analyzer = new ArchAnalyzer(mockAST(summaries), mockFS(files));
    const cycles = await analyzer.detectCircularDeps('/root');
    expect(cycles.length).toBeGreaterThan(0);
  });

  it('returns empty for acyclic graph with relative imports', async () => {
    const files = ['src/core/usecases/a.ts', 'src/core/usecases/b.ts'];
    const summaries = {
      'src/core/usecases/a.ts': makeSummary('src/core/usecases/a.ts', {
        imports: [{ names: ['B'], from: './b.js' }],
      }),
      'src/core/usecases/b.ts': makeSummary('src/core/usecases/b.ts'),
    };
    const analyzer = new ArchAnalyzer(mockAST(summaries), mockFS(files));
    const cycles = await analyzer.detectCircularDeps('/root');
    expect(cycles).toHaveLength(0);
  });
});

// ─── analyzeArchitecture ────────────────────────────────

describe('ArchAnalyzer.analyzeArchitecture', () => {
  it('computes healthScore 100 for clean project with relative .js imports', async () => {
    const files = ['src/core/usecases/uc.ts', 'src/core/ports/index.ts', 'src/adapters/secondary/impl.ts'];
    const summaries = {
      'src/core/usecases/uc.ts': makeSummary('src/core/usecases/uc.ts', {
        imports: [{ names: ['IPort'], from: '../ports/index.js' }],
      }),
      'src/core/ports/index.ts': makeSummary('src/core/ports/index.ts', {
        exports: [{ name: 'IPort', kind: 'interface' }],
      }),
      'src/adapters/secondary/impl.ts': makeSummary('src/adapters/secondary/impl.ts', {
        imports: [{ names: ['IPort'], from: '../../core/ports/index.js' }],
      }),
    };
    const analyzer = new ArchAnalyzer(mockAST(summaries), mockFS(files));
    const result = await analyzer.analyzeArchitecture('/root');
    expect(result.summary.healthScore).toBe(100);
  });

  it('penalizes violations in healthScore with relative .js imports', async () => {
    const files = ['src/core/domain/e.ts', 'src/adapters/secondary/db.ts'];
    const summaries = {
      'src/core/domain/e.ts': makeSummary('src/core/domain/e.ts', {
        imports: [{ names: ['DB'], from: '../../adapters/secondary/db.js' }],
      }),
      'src/adapters/secondary/db.ts': makeSummary('src/adapters/secondary/db.ts', {
        exports: [{ name: 'DB', kind: 'class' }],
      }),
    };
    const analyzer = new ArchAnalyzer(mockAST(summaries), mockFS(files));
    const result = await analyzer.analyzeArchitecture('/root');
    expect(result.summary.healthScore).toBeLessThan(100);
    expect(result.summary.violationCount).toBeGreaterThan(0);
  });
});
