/**
 * Tests for secrets-factory.ts — buildSecretsAdapter
 */

import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { describe, it, expect, beforeEach, afterEach, spyOn } from 'bun:test';

import { buildSecretsAdapter } from '../../src/adapters/secondary/secrets-factory.js';
import { EnvSecretsAdapter } from '../../src/adapters/secondary/env-secrets-adapter.js';
import { CachingSecretsAdapter } from '../../src/adapters/secondary/caching-secrets-adapter.js';
import { LocalVaultAdapter } from '../../src/adapters/secondary/local-vault-adapter.js';

describe('buildSecretsAdapter', () => {
  let tmpDir: string;
  let warnSpy: ReturnType<typeof spyOn>;

  beforeEach(() => {
    tmpDir = mkdtempSync(join(tmpdir(), 'hex-secrets-test-'));
    warnSpy = spyOn(console, 'warn').mockImplementation(() => {});
  });

  afterEach(() => {
    warnSpy.mockRestore();
    rmSync(tmpDir, { recursive: true, force: true });
    // Clean up env var in case a test set it
    delete process.env['HEX_VAULT_PASSWORD'];
  });

  function writeConfig(config: unknown): void {
    const hexDir = join(tmpDir, '.hex');
    mkdirSync(hexDir, { recursive: true });
    writeFileSync(join(hexDir, 'secrets.json'), JSON.stringify(config), 'utf-8');
  }

  function writeRawConfig(content: string): void {
    const hexDir = join(tmpDir, '.hex');
    mkdirSync(hexDir, { recursive: true });
    writeFileSync(join(hexDir, 'secrets.json'), content, 'utf-8');
  }

  // 1. Missing config file -> returns EnvSecretsAdapter
  it('returns EnvSecretsAdapter when config file is missing', async () => {
    const adapter = await buildSecretsAdapter(tmpDir);
    expect(adapter).toBeInstanceOf(EnvSecretsAdapter);
  });

  // 2. Invalid JSON -> returns EnvSecretsAdapter (doesn't throw)
  it('returns EnvSecretsAdapter on invalid JSON without throwing', async () => {
    writeRawConfig('{ not valid json !!!');
    const adapter = await buildSecretsAdapter(tmpDir);
    expect(adapter).toBeInstanceOf(EnvSecretsAdapter);
    expect(warnSpy).toHaveBeenCalledWith(expect.stringContaining('invalid JSON'));
  });

  // 3. backend='env' -> returns EnvSecretsAdapter
  it('returns EnvSecretsAdapter when backend is "env"', async () => {
    writeConfig({ version: 1, backend: 'env' });
    const adapter = await buildSecretsAdapter(tmpDir);
    expect(adapter).toBeInstanceOf(EnvSecretsAdapter);
  });

  // 4. backend='infisical' with valid config -> returns CachingSecretsAdapter
  it('returns CachingSecretsAdapter wrapping InfisicalAdapter', async () => {
    writeConfig({
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

    const adapter = await buildSecretsAdapter(tmpDir);
    expect(adapter).toBeInstanceOf(CachingSecretsAdapter);
  });

  // 4b. backend='infisical' without infisical config -> fallback
  it('returns EnvSecretsAdapter when infisical config is missing', async () => {
    writeConfig({ version: 1, backend: 'infisical' });
    const adapter = await buildSecretsAdapter(tmpDir);
    expect(adapter).toBeInstanceOf(EnvSecretsAdapter);
    expect(warnSpy).toHaveBeenCalledWith(expect.stringContaining('no infisical config'));
  });

  // 5. backend='local-vault' with existing vault + HEX_VAULT_PASSWORD -> returns LocalVaultAdapter
  it('returns LocalVaultAdapter when vault exists and password is set', async () => {
    // Create a real encrypted vault
    const vaultPath = join(tmpDir, '.hex', 'vault.enc');
    mkdirSync(join(tmpDir, '.hex'), { recursive: true });
    LocalVaultAdapter.createVault(vaultPath, 'test-password');

    writeConfig({ version: 1, backend: 'local-vault' });
    process.env['HEX_VAULT_PASSWORD'] = 'test-password';

    const adapter = await buildSecretsAdapter(tmpDir);
    expect(adapter).toBeInstanceOf(LocalVaultAdapter);
  });

  // 6. backend='local-vault' with no vault file -> returns EnvSecretsAdapter with warning
  it('returns EnvSecretsAdapter when vault file does not exist', async () => {
    writeConfig({ version: 1, backend: 'local-vault' });
    process.env['HEX_VAULT_PASSWORD'] = 'test-password';

    const adapter = await buildSecretsAdapter(tmpDir);
    expect(adapter).toBeInstanceOf(EnvSecretsAdapter);
    expect(warnSpy).toHaveBeenCalledWith(expect.stringContaining('vault file not found'));
  });

  // 7. backend='local-vault' with vault but no password -> returns EnvSecretsAdapter with warning
  it('returns EnvSecretsAdapter when HEX_VAULT_PASSWORD is not set', async () => {
    const vaultPath = join(tmpDir, '.hex', 'vault.enc');
    mkdirSync(join(tmpDir, '.hex'), { recursive: true });
    LocalVaultAdapter.createVault(vaultPath, 'test-password');

    writeConfig({ version: 1, backend: 'local-vault' });
    delete process.env['HEX_VAULT_PASSWORD'];

    const adapter = await buildSecretsAdapter(tmpDir);
    expect(adapter).toBeInstanceOf(EnvSecretsAdapter);
    expect(warnSpy).toHaveBeenCalledWith(expect.stringContaining('HEX_VAULT_PASSWORD not set'));
  });

  // 8. Custom vault path
  it('resolves custom vault path relative to project root', async () => {
    const customDir = join(tmpDir, 'secrets');
    mkdirSync(customDir, { recursive: true });
    const vaultPath = join(customDir, 'my-vault.enc');
    LocalVaultAdapter.createVault(vaultPath, 'custom-pw');

    writeConfig({
      version: 1,
      backend: 'local-vault',
      localVault: { path: 'secrets/my-vault.enc' },
    });
    process.env['HEX_VAULT_PASSWORD'] = 'custom-pw';

    const adapter = await buildSecretsAdapter(tmpDir);
    expect(adapter).toBeInstanceOf(LocalVaultAdapter);
  });

  // 9. Unknown backend falls back to env
  it('returns EnvSecretsAdapter for unknown backend value', async () => {
    writeConfig({ version: 1, backend: 'unknown-thing' });
    const adapter = await buildSecretsAdapter(tmpDir);
    expect(adapter).toBeInstanceOf(EnvSecretsAdapter);
  });
});
