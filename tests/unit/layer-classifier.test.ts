import { describe, it, expect } from 'bun:test';
import { classifyLayer, isAllowedImport, getViolationRule } from '../../src/core/usecases/layer-classifier.js';

describe('classifyLayer', () => {
  it('maps domain path', () => {
    expect(classifyLayer('src/core/domain/entities.ts')).toBe('domain');
  });

  it('maps ports path', () => {
    expect(classifyLayer('src/core/ports/index.ts')).toBe('ports');
  });

  it('maps usecases path', () => {
    expect(classifyLayer('src/core/usecases/arch-analyzer.ts')).toBe('usecases');
  });

  it('maps adapters/primary path', () => {
    expect(classifyLayer('src/adapters/primary/cli.ts')).toBe('adapters/primary');
  });

  it('maps adapters/secondary path', () => {
    expect(classifyLayer('src/adapters/secondary/git.ts')).toBe('adapters/secondary');
  });

  it('maps infrastructure path', () => {
    expect(classifyLayer('src/infrastructure/treesitter/queries.ts')).toBe('infrastructure');
  });

  it('returns unknown for unrecognized paths', () => {
    expect(classifyLayer('README.md')).toBe('unknown');
    expect(classifyLayer('package.json')).toBe('unknown');
  });
});

describe('isAllowedImport', () => {
  it('domain -> ports is forbidden (domain has no outward deps)', () => {
    expect(isAllowedImport('domain', 'ports')).toBe(false);
  });

  it('ports -> domain is allowed (ports re-export domain value objects)', () => {
    expect(isAllowedImport('ports', 'domain')).toBe(true);
  });

  it('domain -> adapters/secondary is forbidden', () => {
    expect(isAllowedImport('domain', 'adapters/secondary')).toBe(false);
  });

  it('usecases -> domain is allowed', () => {
    expect(isAllowedImport('usecases', 'domain')).toBe(true);
  });

  it('usecases -> ports is allowed', () => {
    expect(isAllowedImport('usecases', 'ports')).toBe(true);
  });

  it('usecases -> adapters/primary is forbidden', () => {
    expect(isAllowedImport('usecases', 'adapters/primary')).toBe(false);
  });

  it('adapters/secondary -> ports is allowed', () => {
    expect(isAllowedImport('adapters/secondary', 'ports')).toBe(true);
  });

  it('adapters/secondary -> domain is forbidden', () => {
    expect(isAllowedImport('adapters/secondary', 'domain')).toBe(false);
  });

  it('cross-adapter import is forbidden', () => {
    expect(isAllowedImport('adapters/secondary', 'adapters/primary')).toBe(false);
  });

  it('infrastructure -> ports is allowed', () => {
    expect(isAllowedImport('infrastructure', 'ports')).toBe(true);
  });

  it('same layer is always allowed', () => {
    expect(isAllowedImport('domain', 'domain')).toBe(true);
    expect(isAllowedImport('usecases', 'usecases')).toBe(true);
  });
});

describe('getViolationRule', () => {
  it('returns null for allowed imports', () => {
    expect(getViolationRule('ports', 'domain')).toBeNull();
    expect(getViolationRule('usecases', 'domain')).toBeNull();
  });

  it('returns violation for domain -> ports', () => {
    const rule = getViolationRule('domain', 'ports');
    expect(rule).toBeTypeOf('string');
    expect(rule).toContain('domain must not import from ports');
  });

  it('returns descriptive string for violations', () => {
    const rule = getViolationRule('domain', 'adapters/secondary');
    expect(rule).toBeTypeOf('string');
    expect(rule!.length).toBeGreaterThan(0);
  });

  it('describes cross-adapter violations', () => {
    const rule = getViolationRule('adapters/secondary', 'adapters/primary');
    expect(rule).toBeTypeOf('string');
    expect(rule).toContain('adapters');
  });
});
