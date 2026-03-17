import { describe, it, expect } from 'bun:test';
import { InitGuard, INIT_LIMITS } from '../../src/core/usecases/init-guard.js';
import type { IFileSystemPort, StreamOptions } from '../../src/core/ports/index.js';

/**
 * Build a fake IFileSystemPort for dependency injection (ADR-014).
 * Only the methods used by InitGuard need real implementations.
 */
function createFakeFS(opts: {
  existingPaths?: Set<string>;
  streamFileCount?: number;
  streamShouldThrow?: boolean;
}): IFileSystemPort {
  const existingPaths = opts.existingPaths ?? new Set<string>();

  return {
    read: async () => '',
    write: async () => {},
    exists: async (p: string) => existingPaths.has(p),
    glob: async () => [],
    mtime: async () => 0,
    async *streamFiles(
      _pattern: string,
      _options?: StreamOptions,
    ): AsyncGenerator<string> {
      if (opts.streamShouldThrow) {
        throw new Error('Permission denied');
      }
      const count = opts.streamFileCount ?? 0;
      for (let i = 0; i < count; i++) {
        yield `file-${i}.ts`;
      }
    },
  };
}

describe('INIT_LIMITS', () => {
  it('has expected constant values', () => {
    expect(INIT_LIMITS.MAX_FILES).toBe(1_000_000);
    expect(INIT_LIMITS.MAX_IGNORE_RULES).toBe(1_000);
    expect(INIT_LIMITS.LARGE_PROJECT_FILE_THRESHOLD).toBe(10_000);
    expect(INIT_LIMITS.LARGE_PROJECT_SIZE_BYTES).toBe(1_000_000_000);
  });
});

describe('InitGuard', () => {
  describe('assessProject', () => {
    it('returns isLargeProject: false for small projects', async () => {
      // 10 files * 5 extrapolation = 50 estimated, well under 10,000 threshold
      const fs = createFakeFS({ streamFileCount: 10 });
      const guard = new InitGuard(fs);

      const result = await guard.assessProject('/fake/root');

      expect(result.projectStats.isLargeProject).toBe(false);
      expect(result.canProceed).toBe(true);
      expect(result.errors).toHaveLength(0);
    });

    it('returns isLargeProject: true when file count exceeds threshold', async () => {
      // 2001 files * 5 extrapolation = 10,005 > LARGE_PROJECT_FILE_THRESHOLD (10,000)
      const fs = createFakeFS({ streamFileCount: 2001 });
      const guard = new InitGuard(fs);

      const result = await guard.assessProject('/fake/root');

      expect(result.projectStats.isLargeProject).toBe(true);
      expect(result.projectStats.estimatedFiles).toBe(2001 * 5);
      expect(result.warnings.length).toBeGreaterThan(0);
      expect(result.warnings.some((w) => w.includes('Large project'))).toBe(true);
    });

    it('returns canProceed: false when files exceed MAX_FILES', async () => {
      // 200_001 files * 5 = 1_000_005 > MAX_FILES (1_000_000)
      const fs = createFakeFS({ streamFileCount: 200_001 });
      const guard = new InitGuard(fs);

      const result = await guard.assessProject('/fake/root');

      expect(result.canProceed).toBe(false);
      expect(result.errors.length).toBeGreaterThan(0);
      expect(result.errors.some((e) => e.includes('too large'))).toBe(true);
    });

    it('reports errors when streamFiles throws', async () => {
      const fs = createFakeFS({ streamShouldThrow: true });
      const guard = new InitGuard(fs);

      const result = await guard.assessProject('/fake/root');

      expect(result.errors.some((e) => e.includes('Unable to scan'))).toBe(true);
    });

    it('warns about known large directories when they exist', async () => {
      const fs = createFakeFS({
        streamFileCount: 5,
        existingPaths: new Set(['/project/node_modules', '/project/dist']),
      });
      const guard = new InitGuard(fs);

      const result = await guard.assessProject('/project');

      const nmWarning = result.warnings.some((w) => w.includes('node_modules'));
      const distWarning = result.warnings.some((w) => w.includes('dist'));
      expect(nmWarning).toBe(true);
      expect(distWarning).toBe(true);
    });

    it('computes estimatedSizeBytes from file count', async () => {
      const fs = createFakeFS({ streamFileCount: 100 });
      const guard = new InitGuard(fs);

      const result = await guard.assessProject('/fake/root');

      // 100 * 5 extrapolation * 4096 bytes per file
      expect(result.projectStats.estimatedSizeBytes).toBe(100 * 5 * 4096);
    });
  });

  describe('validateDependencies', () => {
    it('returns empty missing array when deps exist', async () => {
      const fs = createFakeFS({
        existingPaths: new Set([
          '/project/node_modules/@anthropic-ai',
          '/project/node_modules/typescript',
        ]),
      });
      const guard = new InitGuard(fs);

      const result = await guard.validateDependencies('/project');

      expect(result.missing).toEqual([]);
      expect(result.warnings).toEqual([]);
    });

    it('returns missing items when deps do not exist', async () => {
      const fs = createFakeFS({ existingPaths: new Set() });
      const guard = new InitGuard(fs);

      const result = await guard.validateDependencies('/project');

      expect(result.missing).toContain('node_modules/@anthropic-ai');
      expect(result.missing).toContain('node_modules/typescript');
      expect(result.warnings).toHaveLength(2);
    });

    it('returns only the deps that are missing', async () => {
      const fs = createFakeFS({
        existingPaths: new Set(['/project/node_modules/typescript']),
      });
      const guard = new InitGuard(fs);

      const result = await guard.validateDependencies('/project');

      expect(result.missing).toEqual(['node_modules/@anthropic-ai']);
      expect(result.warnings).toHaveLength(1);
    });
  });
});
