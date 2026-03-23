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
    expect(result).toBe('node:path');
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

// ─── Go module prefix stripping ───────────────────────────

describe('resolveImportPath — Go module prefix', () => {
  it('strips simple module prefix from Go import', () => {
    const result = resolveImportPath(
      'src/composition-root.go',
      'hex-f1/src/core/domain',
      'hex-f1',
    );
    expect(result).toBe('src/core/domain');
  });

  it('strips long module prefix (github.com) to prevent layer misclassification', () => {
    const result = resolveImportPath(
      'src/composition-root.go',
      'github.com/org/domain-service/src/core/ports',
      'github.com/org/domain-service',
    );
    expect(result).toBe('src/core/ports');
  });

  it('keeps relative Go imports unchanged even with modulePrefix', () => {
    const result = resolveImportPath(
      'src/adapters/primary/http.go',
      './handler',
      'hex-f1',
    );
    expect(result).toBe('src/adapters/primary/handler');
  });

  it('keeps stdlib imports as-is when modulePrefix is set', () => {
    const result = resolveImportPath(
      'src/main.go',
      'fmt',
      'hex-f1',
    );
    expect(result).toBe('fmt');
  });

  it('keeps external imports as-is when they do not match modulePrefix', () => {
    const result = resolveImportPath(
      'src/main.go',
      'github.com/other/pkg',
      'hex-f1',
    );
    expect(result).toBe('github.com/other/pkg');
  });

  it('works without modulePrefix (backwards compatible)', () => {
    const result = resolveImportPath(
      'src/main.go',
      'hex-f1/src/core/domain',
    );
    // Without prefix, returned as-is (existing behavior)
    expect(result).toBe('hex-f1/src/core/domain');
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
