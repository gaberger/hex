/**
 * Smoke Tests — Module Imports
 *
 * Verifies that all public modules can be imported without errors.
 * This catches missing dependencies, circular imports that cause
 * runtime failures, and broken re-exports.
 */

import { describe, it, expect } from 'bun:test';

describe('Smoke: module imports', () => {
  it('core/ports/index.ts imports cleanly', async () => {
    const mod = await import('../../src/core/ports/index.js');
    expect(mod).toBeDefined();
    // Should export key interfaces via re-exports
    expect(typeof mod.formatArchReport).toBe('function');
    expect(typeof mod.formatCompactSummary).toBe('function');
  });

  it('core/domain/value-objects.ts imports cleanly', async () => {
    const mod = await import('../../src/core/domain/value-objects.js');
    expect(mod).toBeDefined();
    expect(mod.Version).toBeDefined();
  });

  it('core/domain/entities.ts imports cleanly', async () => {
    const mod = await import('../../src/core/domain/entities.js');
    expect(mod).toBeDefined();
  });

  it('core/usecases/layer-classifier.ts imports cleanly', async () => {
    const mod = await import('../../src/core/usecases/layer-classifier.js');
    expect(typeof mod.classifyLayer).toBe('function');
    expect(typeof mod.isAllowedImport).toBe('function');
    expect(typeof mod.getViolationRule).toBe('function');
  });

  it('core/usecases/path-normalizer.ts imports cleanly', async () => {
    const mod = await import('../../src/core/usecases/path-normalizer.js');
    expect(typeof mod.normalizePath).toBe('function');
    expect(typeof mod.resolveImportPath).toBe('function');
  });

  it('core/usecases/import-boundary-checker.ts imports cleanly', async () => {
    const mod = await import('../../src/core/usecases/import-boundary-checker.js');
    expect(typeof mod.checkImport).toBe('function');
    expect(typeof mod.validatePlannedImports).toBe('function');
    expect(typeof mod.allowedImportsFor).toBe('function');
  });

  it('core/usecases/arch-analyzer.ts imports cleanly', async () => {
    const mod = await import('../../src/core/usecases/arch-analyzer.js');
    expect(mod).toBeDefined();
  });

  it('core/domain/report-formatter.ts imports cleanly', async () => {
    const mod = await import('../../src/core/domain/report-formatter.js');
    expect(typeof mod.formatArchReport).toBe('function');
  });

  it('core/domain/action-items.ts imports cleanly', async () => {
    const mod = await import('../../src/core/domain/action-items.js');
    expect(typeof mod.extractArchActions).toBe('function');
    expect(typeof mod.buildActionItemReport).toBe('function');
  });

  it('adapters/primary/cli-adapter.ts imports cleanly', async () => {
    const mod = await import('../../src/adapters/primary/cli-adapter.js');
    expect(typeof mod.runCLI).toBe('function');
  });

  it('index.ts (public API) imports cleanly', async () => {
    const mod = await import('../../src/index.js');
    expect(mod).toBeDefined();
  });
});
