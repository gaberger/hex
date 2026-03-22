import { describe, it, expect, beforeEach, afterEach } from 'bun:test';
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';

import { createAppContext } from '../../src/composition-root.js';

/* ------------------------------------------------------------------ */
/*  Helpers                                                            */
/* ------------------------------------------------------------------ */

function makeTmpDir(): string {
  return mkdtempSync(join(tmpdir(), 'hex-secrets-comp-test-'));
}

function writeSecretsConfig(projectRoot: string, config: object): void {
  const hexDir = join(projectRoot, '.hex');
  mkdirSync(hexDir, { recursive: true });
  writeFileSync(join(hexDir, 'secrets.json'), JSON.stringify(config), 'utf-8');
}

/* ================================================================== */
/*  Composition root secrets field                                     */
/* ================================================================== */

describe('composition root secrets field', () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = makeTmpDir();
  });

  afterEach(() => {
    rmSync(tmpDir, { recursive: true, force: true });
  });

  it('secrets field is populated with EnvSecretsAdapter when no config exists', async () => {
    const ctx = await createAppContext(tmpDir);
    expect(ctx.secrets).toBeDefined();
    expect(typeof ctx.secrets.resolveSecret).toBe('function');
    expect(typeof ctx.secrets.hasSecret).toBe('function');
    expect(typeof ctx.secrets.listSecrets).toBe('function');
  });

  it('secrets field works with env-only config', async () => {
    writeSecretsConfig(tmpDir, { version: 1, backend: 'env' });

    const ctx = await createAppContext(tmpDir);
    expect(ctx.secrets).toBeDefined();
    expect(typeof ctx.secrets.resolveSecret).toBe('function');

    // Set a test env var and verify resolution works end-to-end
    const testKey = `HEX_INT_TEST_${Date.now()}`;
    process.env[testKey] = 'integration-value';
    try {
      const result = await ctx.secrets.resolveSecret(testKey);
      expect(result).toEqual({ ok: true, value: 'integration-value' });
    } finally {
      delete process.env[testKey];
    }
  });

  it('secrets field resolves missing keys gracefully', async () => {
    const ctx = await createAppContext(tmpDir);
    const result = await ctx.secrets.resolveSecret('DEFINITELY_NOT_SET_KEY_XYZ');
    expect(result.ok).toBe(false);
  });
});
