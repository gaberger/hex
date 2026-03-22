import { describe, it, expect, beforeEach, afterEach } from 'bun:test';
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';

import { buildSecretsAdapter } from '../../src/composition-root.js';
import { EnvSecretsAdapter } from '../../src/adapters/secondary/env-secrets-adapter.js';
import { CachingSecretsAdapter } from '../../src/adapters/secondary/caching-secrets-adapter.js';
import { LocalVaultAdapter } from '../../src/adapters/secondary/local-vault-adapter.js';

/* ------------------------------------------------------------------ */
/*  Helpers                                                            */
/* ------------------------------------------------------------------ */

function makeTmpDir(): string {
  return mkdtempSync(join(tmpdir(), 'hex-secrets-cfg-test-'));
}

function writeSecretsConfig(projectRoot: string, config: object): void {
  const hexDir = join(projectRoot, '.hex');
  mkdirSync(hexDir, { recursive: true });
  writeFileSync(join(hexDir, 'secrets.json'), JSON.stringify(config), 'utf-8');
}

// Low iteration count for fast tests
const FAST_KDF_ITERATIONS = 1000;

/* ================================================================== */
/*  Config-based adapter selection                                     */
/* ================================================================== */

describe('secrets config-based adapter selection', () => {
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
