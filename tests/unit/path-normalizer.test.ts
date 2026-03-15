import { describe, it, expect } from 'bun:test';
import { resolveImportPath, normalizePath } from '../../src/core/usecases/path-normalizer.js';

// ─── resolveImportPath ─────────────────────────────────

describe('resolveImportPath', () => {
  it('resolves ../ relative imports with .js to .ts', () => {
    const result = resolveImportPath(
      'src/adapters/secondary/git.ts',
      '../../core/ports/index.js',
    );
    expect(result).toBe('src/core/ports/index.ts');
  });

  it('resolves ./ same-directory imports', () => {
    const result = resolveImportPath(
      'src/core/usecases/analyzer.ts',
      './layer-classifier.js',
    );
    expect(result).toBe('src/core/usecases/layer-classifier.ts');
  });

  it('resolves deeply nested ../../../ paths', () => {
    const result = resolveImportPath(
      'src/adapters/primary/cli/commands/analyze.ts',
      '../../../../core/domain/entity.js',
    );
    expect(result).toBe('src/core/domain/entity.ts');
  });

  it('returns bare specifiers normalized', () => {
    const result = resolveImportPath(
      'src/core/usecases/foo.ts',
      'node:path',
    );
    expect(result).toBe('node:path.ts');
  });

  it('handles index imports via ../', () => {
    const result = resolveImportPath(
      'src/core/usecases/foo.ts',
      '../ports/index.js',
    );
    expect(result).toBe('src/core/ports/index.ts');
  });

  it('handles .ts extension imports unchanged', () => {
    const result = resolveImportPath(
      'src/core/usecases/foo.ts',
      './bar.ts',
    );
    expect(result).toBe('src/core/usecases/bar.ts');
  });
});

// ─── normalizePath ─────────────────────────────────────

describe('normalizePath', () => {
  it('strips leading ./', () => {
    expect(normalizePath('./src/core/foo.ts')).toBe('src/core/foo.ts');
  });

  it('strips multiple leading ./', () => {
    expect(normalizePath('././src/foo.ts')).toBe('src/foo.ts');
  });

  it('replaces .js with .ts', () => {
    expect(normalizePath('src/core/foo.js')).toBe('src/core/foo.ts');
  });

  it('replaces .jsx with .tsx', () => {
    expect(normalizePath('src/ui/App.jsx')).toBe('src/ui/App.tsx');
  });

  it('adds .ts if no extension', () => {
    expect(normalizePath('src/core/foo')).toBe('src/core/foo.ts');
  });

  it('leaves .ts paths unchanged', () => {
    expect(normalizePath('src/core/foo.ts')).toBe('src/core/foo.ts');
  });
});
