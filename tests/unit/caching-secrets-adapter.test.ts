import { describe, it, expect, beforeEach } from 'bun:test';
import { CachingSecretsAdapter } from '../../src/adapters/secondary/caching-secrets-adapter.js';
import type { ISecretsPort } from '../../src/core/ports/secrets.js';
import type { SecretContext, SecretMetadata, SecretResult } from '../../src/core/domain/secrets-types.js';

/** Mock ISecretsPort that counts calls and returns configurable values. */
class MockSecretsPort implements ISecretsPort {
  resolveCallCount = 0;
  hasSecretCallCount = 0;
  listSecretsCallCount = 0;

  resolveValue: SecretResult = { ok: true, value: 'secret-value' };
  hasSecretValue = true;
  listSecretsValue: SecretMetadata[] = [];

  async resolveSecret(_key: string, _context?: SecretContext): Promise<SecretResult> {
    this.resolveCallCount++;
    return this.resolveValue;
  }

  async hasSecret(_key: string, _context?: SecretContext): Promise<boolean> {
    this.hasSecretCallCount++;
    return this.hasSecretValue;
  }

  async listSecrets(_context?: SecretContext): Promise<SecretMetadata[]> {
    this.listSecretsCallCount++;
    return this.listSecretsValue;
  }
}

describe('CachingSecretsAdapter', () => {
  let mock: MockSecretsPort;
  let adapter: CachingSecretsAdapter;

  beforeEach(() => {
    mock = new MockSecretsPort();
    adapter = new CachingSecretsAdapter(mock, 200); // 200ms TTL for fast tests
  });

  it('returns cached value within TTL (inner called once)', async () => {
    const r1 = await adapter.resolveSecret('API_KEY');
    const r2 = await adapter.resolveSecret('API_KEY');

    expect(r1).toEqual({ ok: true, value: 'secret-value' });
    expect(r2).toEqual({ ok: true, value: 'secret-value' });
    expect(mock.resolveCallCount).toBe(1);
  });

  it('re-fetches after TTL expires', async () => {
    const shortAdapter = new CachingSecretsAdapter(mock, 50);

    await shortAdapter.resolveSecret('API_KEY');
    expect(mock.resolveCallCount).toBe(1);

    await new Promise((r) => setTimeout(r, 60));

    mock.resolveValue = { ok: true, value: 'new-value' };
    const result = await shortAdapter.resolveSecret('API_KEY');

    expect(mock.resolveCallCount).toBe(2);
    expect(result).toEqual({ ok: true, value: 'new-value' });
  });

  it('different environments produce different cache keys', async () => {
    await adapter.resolveSecret('DB_PASS', { environment: 'dev' });
    await adapter.resolveSecret('DB_PASS', { environment: 'prod' });

    expect(mock.resolveCallCount).toBe(2);
  });

  it('failed lookups are NOT cached (inner retried on next call)', async () => {
    mock.resolveValue = { ok: false, error: 'not found' };

    const r1 = await adapter.resolveSecret('MISSING');
    expect(r1).toEqual({ ok: false, error: 'not found' });

    mock.resolveValue = { ok: true, value: 'found-now' };
    const r2 = await adapter.resolveSecret('MISSING');
    expect(r2).toEqual({ ok: true, value: 'found-now' });

    expect(mock.resolveCallCount).toBe(2);
  });

  it('clearCache() empties all entries', async () => {
    await adapter.resolveSecret('KEY_A');
    await adapter.resolveSecret('KEY_B');
    expect(mock.resolveCallCount).toBe(2);

    adapter.clearCache();

    await adapter.resolveSecret('KEY_A');
    await adapter.resolveSecret('KEY_B');
    expect(mock.resolveCallCount).toBe(4);
  });

  it('hasSecret delegates without caching', async () => {
    await adapter.hasSecret('X');
    await adapter.hasSecret('X');

    expect(mock.hasSecretCallCount).toBe(2);
  });

  it('listSecrets delegates without caching', async () => {
    await adapter.listSecrets();
    await adapter.listSecrets({ environment: 'prod' });

    expect(mock.listSecretsCallCount).toBe(2);
  });
});
