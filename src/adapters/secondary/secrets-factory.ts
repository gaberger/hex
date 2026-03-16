/**
 * Secrets Factory
 *
 * Reads `.hex/secrets.json` from a project root and builds the appropriate
 * ISecretsPort adapter. This is a "factory adapter" — the only file that
 * knows about all three concrete secrets adapters.
 *
 * Placement: adapters/secondary/ (composition-layer peer, not a use case).
 */

import { createRequire } from 'node:module';
import { resolve } from 'node:path';
// Use createRequire to avoid Bun's ESM named-export race under parallel test load
const _require = createRequire(import.meta.url);
const { existsSync, readFileSync } = _require('node:fs') as typeof import('node:fs');

import { CachingSecretsAdapter } from './caching-secrets-adapter.js';
import { EnvSecretsAdapter } from './env-secrets-adapter.js';
import { InfisicalAdapter } from './infisical-adapter.js';
import { LocalVaultAdapter } from './local-vault-adapter.js';

/* ------------------------------------------------------------------ */
/*  Config schema                                                      */
/* ------------------------------------------------------------------ */

interface SecretsConfig {
  version: 1;
  backend: 'infisical' | 'local-vault' | 'env';
  infisical?: {
    siteUrl: string;
    projectId: string;
    defaultEnvironment?: string;
    auth: {
      method: 'universal-auth';
      clientId: string;
      clientSecret: string;
    };
  };
  localVault?: {
    path?: string;
  };
  cache?: {
    ttlSeconds?: number;
  };
}

const DEFAULT_CACHE_TTL_SECONDS = 300;
const CONFIG_FILENAME = '.hex/secrets.json';

/* ------------------------------------------------------------------ */
/*  Factory                                                            */
/* ------------------------------------------------------------------ */

/**
 * Build the correct ISecretsPort from the project's `.hex/secrets.json`.
 *
 * Falls back to EnvSecretsAdapter when no config exists, config is
 * invalid, or the requested backend cannot be initialised.
 */
export interface BuildSecretsOptions {
  /** Override vault password instead of reading from process.env (useful for testing). */
  vaultPassword?: string;
}

export async function buildSecretsAdapter(
  projectRoot: string,
  options?: BuildSecretsOptions,
): Promise<EnvSecretsAdapter | CachingSecretsAdapter | LocalVaultAdapter> {
  const configPath = resolve(projectRoot, CONFIG_FILENAME);

  if (!existsSync(configPath)) {
    return new EnvSecretsAdapter();
  }

  let config: SecretsConfig;
  try {
    const raw = readFileSync(configPath, 'utf-8');
    config = JSON.parse(raw) as SecretsConfig;
  } catch {
    console.warn(`[hex] Warning: invalid JSON in ${configPath} — falling back to env secrets`);
    return new EnvSecretsAdapter();
  }

  switch (config.backend) {
    case 'infisical':
      return buildInfisical(config);

    case 'local-vault':
      return buildLocalVault(config, projectRoot, options?.vaultPassword);

    case 'env':
    default:
      return new EnvSecretsAdapter();
  }
}

/* ------------------------------------------------------------------ */
/*  Backend builders                                                   */
/* ------------------------------------------------------------------ */

function buildInfisical(config: SecretsConfig): CachingSecretsAdapter | EnvSecretsAdapter {
  const inf = config.infisical;
  if (!inf) {
    console.warn('[hex] Warning: backend is "infisical" but no infisical config — falling back to env');
    return new EnvSecretsAdapter();
  }

  const adapter = new InfisicalAdapter({
    siteUrl: inf.siteUrl,
    clientId: inf.auth.clientId,
    clientSecret: inf.auth.clientSecret,
    projectId: inf.projectId,
    defaultEnvironment: inf.defaultEnvironment,
  });

  const ttl = (config.cache?.ttlSeconds ?? DEFAULT_CACHE_TTL_SECONDS) * 1000;
  return new CachingSecretsAdapter(adapter, ttl);
}

function buildLocalVault(config: SecretsConfig, projectRoot: string, passwordOverride?: string): LocalVaultAdapter | EnvSecretsAdapter {
  const vaultRelPath = config.localVault?.path ?? '.hex/vault.enc';
  const vaultPath = resolve(projectRoot, vaultRelPath);

  if (!existsSync(vaultPath)) {
    console.warn(`[hex] Warning: vault file not found at ${vaultPath} — falling back to env secrets`);
    return new EnvSecretsAdapter();
  }

  const password = passwordOverride ?? process.env['HEX_VAULT_PASSWORD'];
  if (!password) {
    // Interactive prompt is Phase 2 — for now, require env var
    console.warn('[hex] Warning: HEX_VAULT_PASSWORD not set — falling back to env secrets');
    return new EnvSecretsAdapter();
  }

  return new LocalVaultAdapter(vaultPath, password);
}
