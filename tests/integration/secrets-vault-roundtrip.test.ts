import { describe, it, expect, beforeEach, afterEach } from 'bun:test';
import { mkdtempSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';

import { LocalVaultAdapter } from '../../src/adapters/secondary/local-vault-adapter.js';

/* ------------------------------------------------------------------ */
/*  Helpers                                                            */
/* ------------------------------------------------------------------ */

function makeTmpDir(): string {
  return mkdtempSync(join(tmpdir(), 'hex-vault-rt-test-'));
}

// Low iteration count for fast tests
const FAST_KDF_ITERATIONS = 1000;

/* ================================================================== */
/*  LocalVault round-trip                                              */
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
    // The crypto error surfaces when we actually try to read the vault
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
