/**
 * Caching Secrets Adapter
 *
 * Decorator that wraps any ISecretsPort with TTL-based in-memory caching.
 * Only resolveSecret results are cached; hasSecret and listSecrets delegate directly.
 */

import type { ISecretsPort, SecretContext, SecretMetadata, SecretResult } from '../../core/ports/secrets.js';

interface CachedSecret {
  value: string;
  fetchedAt: number;
}

export class CachingSecretsAdapter implements ISecretsPort {
  private cache = new Map<string, CachedSecret>();

  constructor(
    private readonly inner: ISecretsPort,
    private readonly ttlMs: number = 5 * 60 * 1000,
  ) {}

  async resolveSecret(key: string, context?: SecretContext): Promise<SecretResult> {
    const cacheKey = this.buildCacheKey(key, context);
    const cached = this.cache.get(cacheKey);

    if (cached && Date.now() - cached.fetchedAt < this.ttlMs) {
      return { ok: true, value: cached.value };
    }

    const result = await this.inner.resolveSecret(key, context);

    if (result.ok) {
      this.cache.set(cacheKey, { value: result.value, fetchedAt: Date.now() });
    }

    return result;
  }

  async hasSecret(key: string, context?: SecretContext): Promise<boolean> {
    return this.inner.hasSecret(key, context);
  }

  async listSecrets(context?: SecretContext): Promise<SecretMetadata[]> {
    return this.inner.listSecrets(context);
  }

  clearCache(): void {
    this.cache.clear();
  }

  private buildCacheKey(key: string, context?: SecretContext): string {
    const env = context?.environment ?? 'default';
    const path = context?.path ?? '/';
    return `${key}:${env}:${path}`;
  }
}
