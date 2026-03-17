/**
 * Tests for secrets-factory.ts — buildSecretsAdapter
 *
 * Each test creates its own isolated temp directory inline rather than
 * relying on beforeEach/afterEach, making them safe under Bun's parallel runner.
 */

import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { describe, it, expect } from 'bun:test';

import { buildSecretsAdapter } from '../../src/composition-root.js';
import { LocalVaultAdapter } from '../../src/adapters/secondary/local-vault-adapter.js';

function makeTmpDir(): string {
  return mkdtempSync(join(tmpdir(), 'hex-secrets-test-'));
}

function writeConfig(dir: string, config: unknown): void {
  const hexDir = join(dir, '.hex');
  mkdirSync(hexDir, { recursive: true });
  writeFileSync(join(hexDir, 'secrets.json'), JSON.stringify(config), 'utf-8');
}

function writeRawConfig(dir: string, content: string): void {
  const hexDir = join(dir, '.hex');
  mkdirSync(hexDir, { recursive: true });
  writeFileSync(join(hexDir, 'secrets.json'), content, 'utf-8');
}

describe('buildSecretsAdapter', () => {
  it('returns EnvSecretsAdapter when config file is missing', async () => {
    const d = makeTmpDir();
    try {
      const adapter = await buildSecretsAdapter(d);
      expect(adapter.constructor.name).toBe('EnvSecretsAdapter');
    } finally { rmSync(d, { recursive: true, force: true }); }
  });

  it('returns EnvSecretsAdapter on invalid JSON without throwing', async () => {
    const d = makeTmpDir();
    try {
      writeRawConfig(d, '{ not valid json !!!');
      const adapter = await buildSecretsAdapter(d);
      expect(adapter.constructor.name).toBe('EnvSecretsAdapter');
    } finally { rmSync(d, { recursive: true, force: true }); }
  });

  it('returns EnvSecretsAdapter when backend is "env"', async () => {
    const d = makeTmpDir();
    try {
      writeConfig(d, { version: 1, backend: 'env' });
      const adapter = await buildSecretsAdapter(d);
      expect(adapter.constructor.name).toBe('EnvSecretsAdapter');
    } finally { rmSync(d, { recursive: true, force: true }); }
  });

  it('returns CachingSecretsAdapter wrapping InfisicalAdapter', async () => {
    const d = makeTmpDir();
    try {
      writeConfig(d, {
        version: 1,
        backend: 'infisical',
        infisical: {
          siteUrl: 'https://secrets.example.com',
          projectId: 'proj-123',
          defaultEnvironment: 'staging',
          auth: {
            method: 'universal-auth',
            clientId: 'cid-abc',
            clientSecret: 'csec-xyz',
          },
        },
        cache: { ttlSeconds: 60 },
      });
      const adapter = await buildSecretsAdapter(d);
      expect(adapter.constructor.name).toBe('CachingSecretsAdapter');
    } finally { rmSync(d, { recursive: true, force: true }); }
  });

  it('returns EnvSecretsAdapter when infisical config is missing', async () => {
    const d = makeTmpDir();
    try {
      writeConfig(d, { version: 1, backend: 'infisical' });
      const adapter = await buildSecretsAdapter(d);
      expect(adapter.constructor.name).toBe('EnvSecretsAdapter');
    } finally { rmSync(d, { recursive: true, force: true }); }
  });

  it('returns LocalVaultAdapter when vault exists and password is set', async () => {
    const d = makeTmpDir();
    try {
      const vaultPath = join(d, '.hex', 'vault.enc');
      mkdirSync(join(d, '.hex'), { recursive: true });
      LocalVaultAdapter.createVault(vaultPath, 'test-password');
      writeConfig(d, { version: 1, backend: 'local-vault' });
      const adapter = await buildSecretsAdapter(d, { vaultPassword: 'test-password' });
      expect(adapter.constructor.name).toBe('LocalVaultAdapter');
    } finally { rmSync(d, { recursive: true, force: true }); }
  });

  it('returns EnvSecretsAdapter when vault file does not exist', async () => {
    const d = makeTmpDir();
    try {
      writeConfig(d, { version: 1, backend: 'local-vault' });
      const adapter = await buildSecretsAdapter(d, { vaultPassword: 'test-password' });
      expect(adapter.constructor.name).toBe('EnvSecretsAdapter');
    } finally { rmSync(d, { recursive: true, force: true }); }
  });

  it('returns EnvSecretsAdapter when no password provided', async () => {
    const d = makeTmpDir();
    try {
      const vaultPath = join(d, '.hex', 'vault.enc');
      mkdirSync(join(d, '.hex'), { recursive: true });
      LocalVaultAdapter.createVault(vaultPath, 'test-password');
      writeConfig(d, { version: 1, backend: 'local-vault' });
      delete process.env['HEX_VAULT_PASSWORD'];
      const adapter = await buildSecretsAdapter(d);
      expect(adapter.constructor.name).toBe('EnvSecretsAdapter');
    } finally { rmSync(d, { recursive: true, force: true }); }
  });

  it('resolves custom vault path relative to project root', async () => {
    const d = makeTmpDir();
    try {
      const customDir = join(d, 'secrets');
      mkdirSync(customDir, { recursive: true });
      LocalVaultAdapter.createVault(join(customDir, 'my-vault.enc'), 'custom-pw');
      writeConfig(d, {
        version: 1,
        backend: 'local-vault',
        localVault: { path: 'secrets/my-vault.enc' },
      });
      const adapter = await buildSecretsAdapter(d, { vaultPassword: 'custom-pw' });
      expect(adapter.constructor.name).toBe('LocalVaultAdapter');
    } finally { rmSync(d, { recursive: true, force: true }); }
  });

  it('returns EnvSecretsAdapter for unknown backend value', async () => {
    const d = makeTmpDir();
    try {
      writeConfig(d, { version: 1, backend: 'unknown-thing' });
      const adapter = await buildSecretsAdapter(d);
      expect(adapter.constructor.name).toBe('EnvSecretsAdapter');
    } finally { rmSync(d, { recursive: true, force: true }); }
  });
});
