/**
 * Secrets Port
 *
 * Contract for resolving secrets at runtime. Adapters may back this with
 * environment variables (local dev), Infisical (self-hosted), or any
 * other secrets manager.
 *
 * Dependency: domain/secrets-types.ts only.
 */

import type { SecretContext, SecretMetadata, SecretResult } from '../domain/secrets-types.js';

export type { SecretContext, SecretMetadata, SecretResult };

export interface ISecretsPort {
  /** Resolve a single secret by key. Returns Result to avoid throwing on missing keys. */
  resolveSecret(key: string, context?: SecretContext): Promise<SecretResult>;

  /** Check whether a secret exists without retrieving its value. */
  hasSecret(key: string, context?: SecretContext): Promise<boolean>;

  /** List secret metadata (keys + versions, never values) for audit/discovery. */
  listSecrets(context?: SecretContext): Promise<SecretMetadata[]>;
}
