import { afterEach, describe, expect, it } from 'bun:test';
import { existsSync, readFileSync, unlinkSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { randomBytes } from 'node:crypto';

import { LocalVaultAdapter } from '../../src/adapters/secondary/local-vault-adapter.js';

function tempVaultPath(): string {
  return join(tmpdir(), `hex-vault-test-${randomBytes(8).toString('hex')}.json`);
}

function cleanup(path: string): void {
  for (const p of [path, `${path}.tmp`]) {
    if (existsSync(p)) unlinkSync(p);
  }
}

describe('LocalVaultAdapter', () => {
  const paths: string[] = [];

  function createTempVault(password = 'test-password'): { path: string; adapter: LocalVaultAdapter } {
    const path = tempVaultPath();
    paths.push(path);
    LocalVaultAdapter.createVault(path, password);
    return { path, adapter: new LocalVaultAdapter(path, password) };
  }

  afterEach(() => {
    for (const p of paths) cleanup(p);
    paths.length = 0;
  });

  it('createVault creates a valid encrypted file', () => {
    const { path } = createTempVault();
    expect(existsSync(path)).toBe(true);

    const envelope = JSON.parse(readFileSync(path, 'utf-8'));
    expect(envelope.kdf).toBe('pbkdf2');
    expect(envelope.kdfIterations).toBe(600_000);
    expect(typeof envelope.salt).toBe('string');
    expect(typeof envelope.iv).toBe('string');
    expect(typeof envelope.tag).toBe('string');
    expect(typeof envelope.data).toBe('string');
    // Salt should be 32 bytes = 64 hex chars
    expect(envelope.salt.length).toBe(64);
    // IV should be 16 bytes = 32 hex chars
    expect(envelope.iv.length).toBe(32);
  });

  it('round-trip: addSecret then resolveSecret returns correct value', async () => {
    const { adapter } = createTempVault();
    adapter.addSecret('API_KEY', 'sk-12345');

    const result = await adapter.resolveSecret('API_KEY');
    expect(result).toEqual({ ok: true, value: 'sk-12345' });
  });

  it('wrong password throws (GCM auth tag failure)', () => {
    const { path } = createTempVault('correct-password');
    const badAdapter = new LocalVaultAdapter(path, 'wrong-password');

    expect(() => badAdapter.addSecret('X', 'Y')).toThrow();
  });

  it('missing key returns { ok: false }', async () => {
    const { adapter } = createTempVault();
    const result = await adapter.resolveSecret('NONEXISTENT');

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.error).toContain('NONEXISTENT');
    }
  });

  it('addSecret to existing key updates version and updatedAt', async () => {
    const { adapter } = createTempVault();
    adapter.addSecret('DB_URL', 'postgres://v1');

    const list1 = await adapter.listSecrets();
    const meta1 = list1.find((m) => m.key === 'DB_URL')!;
    expect(meta1.version).toBe(1);

    // Small delay to ensure updatedAt differs
    adapter.addSecret('DB_URL', 'postgres://v2');

    const list2 = await adapter.listSecrets();
    const meta2 = list2.find((m) => m.key === 'DB_URL')!;
    expect(meta2.version).toBe(2);
    // createdAt should stay the same
    expect(meta2.createdAt).toBe(meta1.createdAt);

    const result = await adapter.resolveSecret('DB_URL');
    expect(result).toEqual({ ok: true, value: 'postgres://v2' });
  });

  it('removeSecret makes key unresolvable', async () => {
    const { adapter } = createTempVault();
    adapter.addSecret('TOKEN', 'abc');
    adapter.removeSecret('TOKEN');

    const result = await adapter.resolveSecret('TOKEN');
    expect(result.ok).toBe(false);

    const has = await adapter.hasSecret('TOKEN');
    expect(has).toBe(false);
  });

  it('listSecrets returns metadata without values', async () => {
    const { adapter } = createTempVault();
    adapter.addSecret('A', 'val-a');
    adapter.addSecret('B', 'val-b');

    const list = await adapter.listSecrets();
    expect(list.length).toBe(2);

    const keys = list.map((m) => m.key).sort();
    expect(keys).toEqual(['A', 'B']);

    // Metadata should have these fields
    for (const meta of list) {
      expect(typeof meta.createdAt).toBe('string');
      expect(typeof meta.updatedAt).toBe('string');
      expect(typeof meta.version).toBe('number');
      expect(meta.environment).toBe('local');
      // Value must NOT be exposed in metadata
      expect((meta as Record<string, unknown>)['value']).toBeUndefined();
    }
  });

  it('fresh IV on each write (two writes produce different iv values)', () => {
    const { path, adapter } = createTempVault();

    adapter.addSecret('K1', 'v1');
    const iv1 = JSON.parse(readFileSync(path, 'utf-8')).iv;

    adapter.addSecret('K2', 'v2');
    const iv2 = JSON.parse(readFileSync(path, 'utf-8')).iv;

    expect(iv1).not.toBe(iv2);
  });
});
