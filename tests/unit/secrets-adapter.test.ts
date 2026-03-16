import { describe, it, expect, beforeEach, afterEach } from 'bun:test';
import { EnvSecretsAdapter } from '../../src/adapters/secondary/env-secrets-adapter.js';
import { InfisicalAdapter } from '../../src/adapters/secondary/infisical-adapter.js';
import type { ISecretsPort } from '../../src/core/ports/secrets.js';

describe('EnvSecretsAdapter', () => {
  let adapter: ISecretsPort;
  const originalEnv = { ...process.env };

  beforeEach(() => {
    adapter = new EnvSecretsAdapter();
  });

  afterEach(() => {
    // Restore env
    for (const key of Object.keys(process.env)) {
      if (!(key in originalEnv)) delete process.env[key];
    }
    Object.assign(process.env, originalEnv);
  });

  it('resolves an existing env var', async () => {
    process.env['TEST_SECRET_KEY'] = 'test-value-123';
    const result = await adapter.resolveSecret('TEST_SECRET_KEY');
    expect(result).toEqual({ ok: true, value: 'test-value-123' });
  });

  it('returns error for missing env var', async () => {
    delete process.env['NONEXISTENT_KEY'];
    const result = await adapter.resolveSecret('NONEXISTENT_KEY');
    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.error).toContain('NONEXISTENT_KEY');
    }
  });

  it('returns error for empty env var', async () => {
    process.env['EMPTY_VAR'] = '';
    const result = await adapter.resolveSecret('EMPTY_VAR');
    expect(result.ok).toBe(false);
  });

  it('hasSecret returns true for set vars', async () => {
    process.env['HAS_SECRET_TEST'] = 'yes';
    expect(await adapter.hasSecret('HAS_SECRET_TEST')).toBe(true);
  });

  it('hasSecret returns false for unset vars', async () => {
    delete process.env['NO_SECRET_TEST'];
    expect(await adapter.hasSecret('NO_SECRET_TEST')).toBe(false);
  });

  it('listSecrets returns empty array (env has no metadata)', async () => {
    const list = await adapter.listSecrets();
    expect(list).toEqual([]);
  });
});

describe('InfisicalAdapter', () => {
  it('implements ISecretsPort interface', () => {
    const adapter = new InfisicalAdapter({
      siteUrl: 'https://localhost:8080',
      clientId: 'test-id',
      clientSecret: 'test-secret',
      projectId: 'test-project',
    });
    // Type check — these methods must exist
    expect(typeof adapter.resolveSecret).toBe('function');
    expect(typeof adapter.hasSecret).toBe('function');
    expect(typeof adapter.listSecrets).toBe('function');
  });

  it('defaults environment to dev', () => {
    const adapter = new InfisicalAdapter({
      siteUrl: 'https://localhost:8080',
      clientId: 'test-id',
      clientSecret: 'test-secret',
      projectId: 'test-project',
    });
    // Access private field via any — we verify the default was set
    expect((adapter as any).env).toBe('dev');
  });

  it('uses custom environment when provided', () => {
    const adapter = new InfisicalAdapter({
      siteUrl: 'https://localhost:8080',
      clientId: 'test-id',
      clientSecret: 'test-secret',
      projectId: 'test-project',
      defaultEnvironment: 'production',
    });
    expect((adapter as any).env).toBe('production');
  });
});
