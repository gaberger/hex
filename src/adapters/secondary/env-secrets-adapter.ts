/**
 * Environment Variable Secrets Adapter
 *
 * Fallback ISecretsPort implementation that reads from process.env.
 * Zero dependencies, zero config — works everywhere.
 *
 * Key mapping: Infisical-style keys like "ANTHROPIC_API_KEY" map directly
 * to process.env["ANTHROPIC_API_KEY"]. The optional SecretContext is ignored
 * since env vars have no concept of environments or folders.
 */

import type { ISecretsPort, SecretMetadata, SecretResult } from '../../core/ports/secrets.js';

export class EnvSecretsAdapter implements ISecretsPort {
  async resolveSecret(key: string): Promise<SecretResult> {
    const value = process.env[key];
    if (value !== undefined && value !== '') {
      return { ok: true, value };
    }
    return { ok: false, error: `Environment variable "${key}" is not set` };
  }

  async hasSecret(key: string): Promise<boolean> {
    const value = process.env[key];
    return value !== undefined && value !== '';
  }

  async listSecrets(): Promise<SecretMetadata[]> {
    // Env vars have no metadata — return empty. Use Infisical for audit trails.
    return [];
  }
}
