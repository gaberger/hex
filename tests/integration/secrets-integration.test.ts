import { describe, it, expect, beforeEach, afterEach } from 'bun:test';
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';

import { buildSecretsAdapter } from '../../src/composition-root.js';
import { EnvSecretsAdapter } from '../../src/adapters/secondary/env-secrets-adapter.js';
import { CachingSecretsAdapter } from '../../src/adapters/secondary/caching-secrets-adapter.js';
import { LocalVaultAdapter } from '../../src/adapters/secondary/local-vault-adapter.js';
import { createAppContext } from '../../src/composition-root.js';

/* ------------------------------------------------------------------ */
/*  Helpers                                                            */
/* ------------------------------------------------------------------ */

function makeTmpDir(): string {
  return mkdtempSync(join(tmpdir(), 'hex-secrets-test-'));
}

function writeSecretsConfig(projectRoot: string, config: object): void {
  const hexDir = join(projectRoot, '.hex');
  mkdirSync(hexDir, { recursive: true });
  writeFileSync(join(hexDir, 'secrets.json'), JSON.stringify(config), 'utf-8');
}

// Low iteration count for fast tests
const FAST_KDF_ITERATIONS = 1000;

/* ================================================================== */
/*  1. Config-based adapter selection                                  */
/* ================================================================== */

describe('config-based adapter selection', () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = makeTmpDir();
  });

  afterEach(() => {
    rmSync(tmpDir, { recursive: true, force: true });
  });

  it('returns EnvSecretsAdapter when no config file exists', async () => {
    const adapter = await buildSecretsAdapter(tmpDir);
    expect(adapter).toBeInstanceOf(EnvSecretsAdapter);
  });

  it('returns EnvSecretsAdapter for backend "env"', async () => {
    writeSecretsConfig(tmpDir, { version: 1, backend: 'env' });
    const adapter = await buildSecretsAdapter(tmpDir);
    expect(adapter).toBeInstanceOf(EnvSecretsAdapter);
  });

  it('returns CachingSecretsAdapter wrapping InfisicalAdapter for backend "infisical"', async () => {
    writeSecretsConfig(tmpDir, {
      version: 1,
      backend: 'infisical',
      infisical: {
        siteUrl: 'https://localhost:8080',
        projectId: 'test-proj',
        auth: {
          method: 'universal-auth',
          clientId: 'fake-id',
          clientSecret: 'fake-secret',
        },
      },
    });
    const adapter = await buildSecretsAdapter(tmpDir);
    expect(adapter).toBeInstanceOf(CachingSecretsAdapter);
  });

  it('returns LocalVaultAdapter for backend "local-vault" with valid vault', async () => {
    const hexDir = join(tmpDir, '.hex');
    mkdirSync(hexDir, { recursive: true });
    const vaultPath = join(hexDir, 'vault.enc');
    const password = 'integration-test-pw';
    LocalVaultAdapter.createVault(vaultPath, password, FAST_KDF_ITERATIONS);

    writeSecretsConfig(tmpDir, {
      version: 1,
      backend: 'local-vault',
    });

    const adapter = await buildSecretsAdapter(tmpDir, { vaultPassword: password });
    expect(adapter).toBeInstanceOf(LocalVaultAdapter);
  });

  it('falls back to EnvSecretsAdapter when vault file is missing for local-vault backend', async () => {
    writeSecretsConfig(tmpDir, {
      version: 1,
      backend: 'local-vault',
    });
    // No vault file created — should fall back
    const adapter = await buildSecretsAdapter(tmpDir, { vaultPassword: 'irrelevant' });
    expect(adapter).toBeInstanceOf(EnvSecretsAdapter);
  });

  it('falls back to EnvSecretsAdapter when infisical config block is missing', async () => {
    writeSecretsConfig(tmpDir, {
      version: 1,
      backend: 'infisical',
      // no infisical key
    });
    const adapter = await buildSecretsAdapter(tmpDir);
    expect(adapter).toBeInstanceOf(EnvSecretsAdapter);
  });

  it('falls back to EnvSecretsAdapter for invalid JSON in config', async () => {
    const hexDir = join(tmpDir, '.hex');
    mkdirSync(hexDir, { recursive: true });
    writeFileSync(join(hexDir, 'secrets.json'), '{ broken json !!!', 'utf-8');
    const adapter = await buildSecretsAdapter(tmpDir);
    expect(adapter).toBeInstanceOf(EnvSecretsAdapter);
  });
});

/* ================================================================== */
/*  2. LocalVault round-trip                                           */
/* ================================================================== */

describe('LocalVault round-trip', () => {
  let tmpDir: string;
  let vaultPath: string;
  const password = 'round-trip-test-pw';

  beforeEach(() => {
    tmpDir = makeTmpDir();
    vaultPath = join(tmpDir, 'vault.enc');
    LocalVaultAdapter.createVault(vaultPath, password, FAST_KDF_ITERATIONS);
  });

  afterEach(() => {
    rmSync(tmpDir, { recursive: true, force: true });
  });

  it('adds 3 secrets and resolves each correctly', async () => {
    const vault = new LocalVaultAdapter(vaultPath, password, FAST_KDF_ITERATIONS);

    vault.addSecret('API_KEY', 'sk-abc123');
    vault.addSecret('DB_PASSWORD', 'pg-secret');
    vault.addSecret('JWT_SECRET', 'jwt-tok-42');

    const r1 = await vault.resolveSecret('API_KEY');
    expect(r1).toEqual({ ok: true, value: 'sk-abc123' });

    const r2 = await vault.resolveSecret('DB_PASSWORD');
    expect(r2).toEqual({ ok: true, value: 'pg-secret' });

    const r3 = await vault.resolveSecret('JWT_SECRET');
    expect(r3).toEqual({ ok: true, value: 'jwt-tok-42' });
  });

  it('removes a secret and verifies it is gone', async () => {
    const vault = new LocalVaultAdapter(vaultPath, password, FAST_KDF_ITERATIONS);

    vault.addSecret('KEEP_ME', 'kept');
    vault.addSecret('DELETE_ME', 'gone');
    vault.addSecret('ALSO_KEEP', 'also-kept');

    vault.removeSecret('DELETE_ME');

    expect(await vault.hasSecret('KEEP_ME')).toBe(true);
    expect(await vault.hasSecret('DELETE_ME')).toBe(false);
    expect(await vault.hasSecret('ALSO_KEEP')).toBe(true);

    const result = await vault.resolveSecret('DELETE_ME');
    expect(result.ok).toBe(false);
  });

  it('listSecrets returns metadata for stored secrets', async () => {
    const vault = new LocalVaultAdapter(vaultPath, password, FAST_KDF_ITERATIONS);

    vault.addSecret('S1', 'v1');
    vault.addSecret('S2', 'v2');

    const list = await vault.listSecrets();
    expect(list).toHaveLength(2);

    const keys = list.map((m) => m.key).sort();
    expect(keys).toEqual(['S1', 'S2']);

    for (const entry of list) {
      expect(entry.version).toBe(1);
      expect(entry.environment).toBe('local');
      expect(entry.createdAt).toBeTruthy();
      expect(entry.updatedAt).toBeTruthy();
    }
  });

  it('rejects wrong password with a decryption error', () => {
    const badVault = new LocalVaultAdapter(vaultPath, 'wrong-password', FAST_KDF_ITERATIONS);
    expect(() => {
      // readVault is called internally by resolveSecret, but it is sync —
      // the async wrapper won't help us catch the crypto error synchronously.
      // Instead, call hasSecret which also triggers readVault.
      // We need to await the promise and catch the rejection.
    }).not.toThrow(); // no-op — real test below

    // The crypto error surfaces when we actually try to read
    expect(badVault.resolveSecret('anything')).rejects.toThrow();
  });

  it('updates an existing secret and increments version', async () => {
    const vault = new LocalVaultAdapter(vaultPath, password, FAST_KDF_ITERATIONS);

    vault.addSecret('VERSIONED', 'v1');
    vault.addSecret('VERSIONED', 'v2');

    const result = await vault.resolveSecret('VERSIONED');
    expect(result).toEqual({ ok: true, value: 'v2' });

    const list = await vault.listSecrets();
    const entry = list.find((m) => m.key === 'VERSIONED');
    expect(entry?.version).toBe(2);
  });

  it('persists secrets across adapter instances', async () => {
    const vault1 = new LocalVaultAdapter(vaultPath, password, FAST_KDF_ITERATIONS);
    vault1.addSecret('PERSIST_KEY', 'persist-value');

    // Create a fresh adapter pointing to the same vault file
    const vault2 = new LocalVaultAdapter(vaultPath, password, FAST_KDF_ITERATIONS);
    const result = await vault2.resolveSecret('PERSIST_KEY');
    expect(result).toEqual({ ok: true, value: 'persist-value' });
  });
});

/* ================================================================== */
/*  3. Composition root secrets field                                  */
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
