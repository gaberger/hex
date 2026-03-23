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

  // Skipped: these usecases depend on native tree-sitter bindings not available in test env.
  // See workplan: feat-test-suite-cleanup.json
  it.todo('core/usecases/layer-classifier.ts imports cleanly');
  it.todo('core/usecases/path-normalizer.ts imports cleanly');
  it.todo('core/usecases/import-boundary-checker.ts imports cleanly');
  it.todo('core/usecases/arch-analyzer.ts imports cleanly');

  it('core/domain/report-formatter.ts imports cleanly', async () => {
    const mod = await import('../../src/core/domain/report-formatter.js');
    expect(typeof mod.formatArchReport).toBe('function');
  });

  it('core/domain/action-items.ts imports cleanly', async () => {
    const mod = await import('../../src/core/domain/action-items.js');
    expect(typeof mod.extractArchActions).toBe('function');
    expect(typeof mod.buildActionItemReport).toBe('function');
  });

  // Skipped: legacy TS CLI adapter replaced by Rust hex-cli (ADR-010, ADR-2603222050)
  it.todo('adapters/primary/cli-adapter.ts imports cleanly');

  it('index.ts (public API) imports cleanly', async () => {
    const mod = await import('../../src/index.js');
    expect(mod).toBeDefined();
  });
});
