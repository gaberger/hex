import { describe, it, expect } from 'bun:test';
import { createAppContext } from '../../src/composition-root.js';

const PROJECT_ROOT = '/Volumes/ExtendedStorage/PARA/01-Projects/hex-intf';

describe('Composition Root', () => {
  it('createAppContext returns an object with all expected properties', async () => {
    const ctx = await createAppContext(PROJECT_ROOT);
    expect(ctx).toHaveProperty('archAnalyzer');
    expect(ctx).toHaveProperty('ast');
    expect(ctx).toHaveProperty('fs');
    expect(ctx).toHaveProperty('rootPath');
    expect(ctx.rootPath).toBe(PROJECT_ROOT);
  });

  it('archAnalyzer is callable', async () => {
    const ctx = await createAppContext(PROJECT_ROOT);
    expect(typeof ctx.archAnalyzer.analyzeArchitecture).toBe('function');
    expect(typeof ctx.archAnalyzer.findDeadExports).toBe('function');
    expect(typeof ctx.archAnalyzer.detectCircularDeps).toBe('function');
  });

  it('fs is callable', async () => {
    const ctx = await createAppContext(PROJECT_ROOT);
    expect(typeof ctx.fs.read).toBe('function');
    expect(typeof ctx.fs.write).toBe('function');
    expect(typeof ctx.fs.exists).toBe('function');
    expect(typeof ctx.fs.glob).toBe('function');
    // Verify it actually works against real filesystem
    const exists = await ctx.fs.exists('package.json');
    expect(exists).toBe(true);
  });
});
