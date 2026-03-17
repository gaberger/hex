/**
 * Property Tests — Path Normalizer
 *
 * Verifies algebraic properties of path normalization:
 * - Idempotency: normalizing twice yields the same result
 * - Extension mapping: .js → .ts, .jsx → .tsx
 * - Language isolation: Go/Rust paths are unaffected by TS rules
 */

import { describe, it, expect } from 'bun:test';
import {
  normalizePath,
  resolveImportPath,
  rustModuleCandidates,
} from '../../src/core/usecases/path-normalizer.js';

// ── Property: Idempotency ───────────────────────────────────

describe('Property: normalizePath is idempotent', () => {
  const paths = [
    'src/core/domain/entities.ts',
    './src/adapters/primary/cli.ts',
    'src/ports/index.js',
    'src/foo.jsx',
    'src/bar/',
    'src/adapters/secondary/git',
    'internal/domain/model.go',
    'src/core/ports.rs',
    'crate::core::ports',
  ];

  for (const path of paths) {
    it(`normalize(normalize("${path}")) === normalize("${path}")`, () => {
      const once = normalizePath(path);
      const twice = normalizePath(once);
      expect(twice).toBe(once);
    });
  }
});

// ── Property: .js → .ts extension mapping ───────────────────

describe('Property: .js extensions map to .ts', () => {
  const tsPaths = [
    'src/foo.js',
    'src/core/domain/entities.js',
    'src/adapters/primary/cli-adapter.js',
    './src/ports/index.js',
  ];

  for (const path of tsPaths) {
    it(`"${path}" normalizes to .ts`, () => {
      const result = normalizePath(path);
      expect(result).toEndWith('.ts');
      expect(result).not.toEndWith('.js');
    });
  }
});

// ── Property: .jsx → .tsx extension mapping ─────────────────

describe('Property: .jsx extensions map to .tsx', () => {
  const jsxPaths = [
    'src/components/App.jsx',
    './src/ui/Button.jsx',
  ];

  for (const jsxPath of jsxPaths) {
    it(`"${jsxPath}" normalizes to .tsx`, () => {
      const result = normalizePath(jsxPath);
      expect(result).toEndWith('.tsx');
      expect(result).not.toEndWith('.jsx');
    });
  }
});

// ── Property: Go paths preserve .go extension ───────────────

describe('Property: Go paths are not affected by TS rules', () => {
  const goPaths = [
    'internal/domain/model.go',
    'cmd/server/main.go',
    'pkg/api.go',
    './internal/ports/storage.go',
  ];

  for (const goPath of goPaths) {
    it(`"${goPath}" keeps .go extension`, () => {
      const result = normalizePath(goPath);
      expect(result).toEndWith('.go');
    });
  }
});

// ── Property: Rust paths preserve .rs extension ─────────────

describe('Property: Rust paths are not affected by TS rules', () => {
  const rsPaths = [
    'src/main.rs',
    'src/lib.rs',
    'src/routes/api.rs',
  ];

  for (const rsPath of rsPaths) {
    it(`"${rsPath}" keeps .rs extension`, () => {
      const result = normalizePath(rsPath);
      expect(result).toEndWith('.rs');
    });
  }
});

// ── Property: Leading ./ is always stripped ──────────────────

describe('Property: leading ./ is stripped', () => {
  const paths = [
    './src/foo.ts',
    '././src/bar.ts',
    './internal/domain.go',
    './src/main.rs',
  ];

  for (const path of paths) {
    it(`"${path}" has no leading ./`, () => {
      const result = normalizePath(path);
      expect(result).not.toMatch(/^\.\//);
    });
  }
});

// ── Property: resolveImportPath produces normalizable paths ─

describe('Property: resolveImportPath results are normalizable', () => {
  const cases: Array<{ from: string; imp: string }> = [
    { from: 'src/adapters/primary/cli.ts', imp: '../../core/ports/index.js' },
    { from: 'src/core/usecases/foo.ts', imp: '../domain/entities.js' },
    { from: 'src/core/usecases/foo.ts', imp: './bar.js' },
  ];

  for (const { from, imp } of cases) {
    it(`resolve("${from}", "${imp}") is idempotent after normalization`, () => {
      const resolved = resolveImportPath(from, imp);
      const normalized = normalizePath(resolved);
      const twice = normalizePath(normalized);
      expect(twice).toBe(normalized);
    });
  }
});

// ── Property: Rust module candidates always produce two paths ─

describe('Property: rustModuleCandidates returns exactly 2 candidates', () => {
  const basePaths = [
    'src/core/ports',
    'src/adapters/primary/http_adapter',
    'src/domain/model',
  ];

  for (const basePath of basePaths) {
    it(`candidates for "${basePath}" has length 2`, () => {
      const candidates = rustModuleCandidates(basePath);
      expect(candidates).toHaveLength(2);
      expect(candidates[0]).toEndWith('.rs');
      expect(candidates[1]).toEndWith('/mod.rs');
    });
  }
});
