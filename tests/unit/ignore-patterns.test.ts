import { describe, it, expect } from 'bun:test';
import {
  IgnoreEngine,
  DEFAULT_IGNORE_PATTERNS,
} from '../../src/core/domain/ignore-patterns.js';

describe('DEFAULT_IGNORE_PATTERNS', () => {
  it('contains expected entries', () => {
    expect(DEFAULT_IGNORE_PATTERNS).toContain('node_modules/');
    expect(DEFAULT_IGNORE_PATTERNS).toContain('.git/');
    expect(DEFAULT_IGNORE_PATTERNS).toContain('dist/');
    expect(DEFAULT_IGNORE_PATTERNS).toContain('*.log');
    expect(DEFAULT_IGNORE_PATTERNS).toContain('.DS_Store');
  });
});

describe('IgnoreEngine', () => {
  describe('directory patterns', () => {
    const engine = new IgnoreEngine(['node_modules/', 'dist/']);

    it('ignores paths containing the directory', () => {
      expect(engine.isIgnored('node_modules/foo/bar.js')).toBe(true);
      expect(engine.isIgnored('src/node_modules/pkg/index.js')).toBe(true);
      expect(engine.isIgnored('dist/cli.js')).toBe(true);
    });

    it('does not ignore unrelated paths', () => {
      expect(engine.isIgnored('src/main.ts')).toBe(false);
      expect(engine.isIgnored('lib/utils.ts')).toBe(false);
    });
  });

  describe('glob patterns', () => {
    const engine = new IgnoreEngine(['*.log', '*.swp', '*.tmp']);

    it('matches files by extension', () => {
      expect(engine.isIgnored('app.log')).toBe(true);
      expect(engine.isIgnored('deep/nested/file.swp')).toBe(true);
      expect(engine.isIgnored('scratch.tmp')).toBe(true);
    });

    it('does not match other extensions', () => {
      expect(engine.isIgnored('main.ts')).toBe(false);
      expect(engine.isIgnored('README.md')).toBe(false);
    });
  });

  describe('exact name patterns', () => {
    const engine = new IgnoreEngine(['.DS_Store']);

    it('matches exact basename', () => {
      expect(engine.isIgnored('.DS_Store')).toBe(true);
      expect(engine.isIgnored('src/.DS_Store')).toBe(true);
    });

    it('does not match partial names', () => {
      expect(engine.isIgnored('.DS_Store_extra')).toBe(false);
    });
  });

  describe('comments and blanks', () => {
    const engine = new IgnoreEngine([
      '# This is a comment',
      '',
      '  ',
      'node_modules/',
    ]);

    it('skips comments and blanks', () => {
      expect(engine.isIgnored('node_modules/foo.js')).toBe(true);
      expect(engine.isIgnored('src/main.ts')).toBe(false);
    });
  });

  describe('leading ./ normalisation', () => {
    const engine = new IgnoreEngine(['dist/']);

    it('strips leading ./ before matching', () => {
      expect(engine.isIgnored('./dist/cli.js')).toBe(true);
    });
  });

  describe('backslash normalisation', () => {
    const engine = new IgnoreEngine(['node_modules/']);

    it('handles Windows-style separators', () => {
      expect(engine.isIgnored('node_modules\\pkg\\index.js')).toBe(true);
    });
  });
});

describe('IgnoreEngine.fromProject', () => {
  it('loads .hexignore when present', async () => {
    const fs = {
      exists: async (p: string) => p.endsWith('.hexignore'),
      read: async (_p: string) => 'custom-dir/\n*.dat',
    };

    const engine = await IgnoreEngine.fromProject('/project', fs);

    expect(engine.isIgnored('custom-dir/foo.txt')).toBe(true);
    expect(engine.isIgnored('data.dat')).toBe(true);
    // Defaults are still merged
    expect(engine.isIgnored('node_modules/pkg/i.js')).toBe(true);
  });

  it('falls back to .gitignore when no .hexignore', async () => {
    const fs = {
      exists: async (p: string) => p.endsWith('.gitignore'),
      read: async (_p: string) => 'vendor-custom/',
    };

    const engine = await IgnoreEngine.fromProject('/project', fs);

    expect(engine.isIgnored('vendor-custom/lib.so')).toBe(true);
    expect(engine.isIgnored('node_modules/x')).toBe(true);
  });

  it('uses only defaults when no ignore files exist', async () => {
    const fs = {
      exists: async (_p: string) => false,
      read: async (_p: string) => '',
    };

    const engine = await IgnoreEngine.fromProject('/project', fs);

    expect(engine.isIgnored('node_modules/x')).toBe(true);
    expect(engine.isIgnored('src/main.ts')).toBe(false);
  });
});
