/**
 * Tests for import-boundary-checker (Intervention A)
 *
 * Validates that pre-generation boundary checking catches the same
 * violations that arch-analyzer catches post-build, but earlier.
 */
import { describe, test, expect } from 'bun:test';
import {
  checkImport,
  validatePlannedImports,
  allowedImportsFor,
} from '../../src/core/usecases/import-boundary-checker.js';

describe('checkImport', () => {
  test('allows adapter/primary → ports (valid hex import)', () => {
    const result = checkImport(
      'src/adapters/primary/http-adapter.ts',
      'src/core/ports/index.ts',
      ['IHttpPort'],
    );
    expect(result).toBeNull();
  });

  test('blocks adapter/primary → domain (Priority 1 violation)', () => {
    const result = checkImport(
      'src/adapters/primary/http-adapter.ts',
      'src/core/domain/entities.ts',
      ['DomainEvent'],
    );
    expect(result).not.toBeNull();
    expect(result!.rule).toContain('adapters must not import from domain');
  });

  test('blocks adapter/primary → adapter/secondary (cross-adapter)', () => {
    const result = checkImport(
      'src/adapters/primary/cli-adapter.ts',
      'src/adapters/secondary/git-adapter.ts',
      ['GitAdapter'],
    );
    expect(result).not.toBeNull();
    expect(result!.rule).toContain('adapters must not import from other adapters');
  });

  test('allows usecases → ports', () => {
    const result = checkImport(
      'src/core/usecases/scaffold-service.ts',
      'src/core/ports/index.ts',
      ['IFileSystemPort'],
    );
    expect(result).toBeNull();
  });

  test('allows usecases → domain', () => {
    const result = checkImport(
      'src/core/usecases/arch-analyzer.ts',
      'src/core/domain/value-objects.ts',
      ['ASTSummary'],
    );
    expect(result).toBeNull();
  });

  test('blocks usecases → adapters', () => {
    const result = checkImport(
      'src/core/usecases/scaffold-service.ts',
      'src/adapters/secondary/filesystem-adapter.ts',
      ['FileSystemAdapter'],
    );
    expect(result).not.toBeNull();
  });

  test('blocks domain → anything external', () => {
    const result = checkImport(
      'src/core/domain/entities.ts',
      'src/core/ports/index.ts',
      ['IHttpPort'],
    );
    expect(result).not.toBeNull();
    expect(result!.rule).toContain('domain must not import from ports');
  });

  test('allows same-layer imports', () => {
    const result = checkImport(
      'src/core/domain/entities.ts',
      'src/core/domain/value-objects.ts',
      ['Language'],
    );
    expect(result).toBeNull();
  });

  test('returns null for unknown layers (non-blocking)', () => {
    const result = checkImport(
      'scripts/setup.ts',
      'src/core/domain/entities.ts',
      ['DomainEvent'],
    );
    expect(result).toBeNull();
  });
});

describe('validatePlannedImports', () => {
  test('returns valid for clean imports', () => {
    const result = validatePlannedImports('src/adapters/primary/cli-adapter.ts', [
      { fromFile: 'src/adapters/primary/cli-adapter.ts', toFile: 'src/core/ports/index.ts', names: ['ICLIPort'] },
    ]);
    expect(result.valid).toBe(true);
    expect(result.violations).toHaveLength(0);
  });

  test('catches multiple violations in one file', () => {
    const result = validatePlannedImports('src/adapters/primary/http-adapter.ts', [
      { fromFile: 'src/adapters/primary/http-adapter.ts', toFile: 'src/core/domain/entities.ts', names: ['DomainEvent'] },
      { fromFile: 'src/adapters/primary/http-adapter.ts', toFile: 'src/core/domain/value-objects.ts', names: ['Language'] },
      { fromFile: 'src/adapters/primary/http-adapter.ts', toFile: 'src/core/ports/index.ts', names: ['IHttpPort'] },
    ]);
    expect(result.valid).toBe(false);
    expect(result.violations).toHaveLength(2); // domain imports blocked, ports OK
  });

  test('warns about adapter importing domain even if technically caught', () => {
    const result = validatePlannedImports('src/adapters/secondary/storage-adapter.ts', [
      { fromFile: 'src/adapters/secondary/storage-adapter.ts', toFile: 'src/core/domain/value-objects.ts', names: ['CodeUnit'] },
    ]);
    expect(result.valid).toBe(false);
    expect(result.warnings.length).toBeGreaterThan(0);
  });
});

describe('allowedImportsFor', () => {
  test('adapters/primary can only import from ports', () => {
    const allowed = allowedImportsFor('src/adapters/primary/http-adapter.ts');
    expect(allowed).toContain('adapters/primary'); // same-layer
    expect(allowed).toContain('ports');
    expect(allowed).not.toContain('domain');
    expect(allowed).not.toContain('usecases');
    expect(allowed).not.toContain('adapters/secondary');
  });

  test('usecases can import from domain and ports', () => {
    const allowed = allowedImportsFor('src/core/usecases/scaffold-service.ts');
    expect(allowed).toContain('usecases'); // same-layer
    expect(allowed).toContain('domain');
    expect(allowed).toContain('ports');
    expect(allowed).not.toContain('adapters/primary');
  });

  test('domain can only import from itself', () => {
    const allowed = allowedImportsFor('src/core/domain/entities.ts');
    expect(allowed).toEqual(['domain']);
  });

  test('unknown path returns empty', () => {
    const allowed = allowedImportsFor('scripts/random.ts');
    expect(allowed).toEqual([]);
  });
});
