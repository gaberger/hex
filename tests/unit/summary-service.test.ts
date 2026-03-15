import { describe, it, expect } from 'bun:test';
import type { IASTPort, IFileSystemPort, ASTSummary } from '../../src/core/ports/index.js';
import { SummaryService } from '../../src/core/usecases/summary-service.js';

// ─── Mock Factories ─────────────────────────────────────

function mockAST(results?: Record<string, ASTSummary>): IASTPort {
  return {
    extractSummary: async (filePath: string, level: ASTSummary['level']): Promise<ASTSummary> =>
      results?.[filePath] ?? {
        filePath, language: 'typescript', level,
        exports: [], imports: [], dependencies: [],
        lineCount: 10, tokenEstimate: 50,
      },
    diffStructural: () => ({ added: [], removed: [], modified: [] }),
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

// ─── Tests ──────────────────────────────────────────────

describe('SummaryService.summarizeFile', () => {
  it('delegates to ast.extractSummary', async () => {
    const expected: ASTSummary = {
      filePath: 'src/foo.ts', language: 'typescript', level: 'L2',
      exports: [{ name: 'Foo', kind: 'class' }], imports: [],
      dependencies: [], lineCount: 30, tokenEstimate: 150,
    };
    const svc = new SummaryService(mockAST({ 'src/foo.ts': expected }), mockFS([]));
    const result = await svc.summarizeFile('src/foo.ts', 'L2');
    expect(result).toEqual(expected);
  });
});

describe('SummaryService.summarizeProject', () => {
  it('returns summaries for all files from fs.glob', async () => {
    const svc = new SummaryService(mockAST(), mockFS(['src/a.ts', 'src/b.ts', 'src/c.ts']));
    const results = await svc.summarizeProject('/root', 'L0');
    expect(results).toHaveLength(3);
    expect(results.map((r) => r.filePath)).toContain('src/b.ts');
  });

  it('uses the requested level for all files', async () => {
    const svc = new SummaryService(mockAST(), mockFS(['src/a.ts', 'src/b.ts']));
    const results = await svc.summarizeProject('/root', 'L3');
    for (const r of results) expect(r.level).toBe('L3');
  });

  it('passes rootPath-based glob pattern to fs.glob', async () => {
    let captured = '';
    const fs: IFileSystemPort = {
      read: async () => '', write: async () => {}, exists: async () => true,
      glob: async (p) => { captured = p; return []; },
    };
    const svc = new SummaryService(mockAST(), fs);
    await svc.summarizeProject('/myproject', 'L1');
    expect(captured).toContain('/myproject');
  });
});
