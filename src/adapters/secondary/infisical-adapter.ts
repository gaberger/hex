/**
 * Infisical Secrets Adapter
 *
 * Implements ISecretsPort backed by a self-hosted (or cloud) Infisical instance.
 * Uses raw fetch() — no external SDK dependency, matching the LLMAdapter pattern.
 *
 * Authentication: Universal Auth (Machine Identity)
 *   - clientId + clientSecret → short-lived accessToken
 *   - Token auto-refreshes when expired
 *
 * All secrets are fetched via the Infisical REST API v3.
 */

import type {
  ISecretsPort,
  SecretContext,
  SecretMetadata,
  SecretResult,
} from '../../core/ports/secrets.js';

export interface InfisicalAdapterConfig {
  /** Base URL of Infisical instance (e.g. "https://secrets.example.com") */
  siteUrl: string;
  /** Machine Identity client ID */
  clientId: string;
  /** Machine Identity client secret */
  clientSecret: string;
  /** Default Infisical project ID */
  projectId: string;
  /** Default environment slug (default: "dev") */
  defaultEnvironment?: string;
}

interface TokenState {
  accessToken: string;
  expiresAt: number; // epoch ms
}

export class InfisicalAdapter implements ISecretsPort {
  private token: TokenState | null = null;
  private readonly env: string;

  constructor(private readonly config: InfisicalAdapterConfig) {
    this.env = config.defaultEnvironment ?? 'dev';
  }

  // ── ISecretsPort implementation ─────────────────────────

  async resolveSecret(key: string, context?: SecretContext): Promise<SecretResult> {
    try {
      const token = await this.getAccessToken();
      const env = context?.environment ?? this.env;
      const projectId = context?.projectId ?? this.config.projectId;
      const secretPath = context?.path ?? '/';

      const params = new URLSearchParams({
        workspaceId: projectId,
        environment: env,
        secretPath,
      });

      const res = await fetch(
        `${this.config.siteUrl}/api/v3/secrets/raw/${encodeURIComponent(key)}?${params}`,
        {
          headers: {
            Authorization: `Bearer ${token}`,
          },
        },
      );

      if (!res.ok) {
        if (res.status === 404) {
          return { ok: false, error: `Secret "${key}" not found in ${env}${secretPath}` };
        }
        const text = await res.text();
        return { ok: false, error: `Infisical error (${res.status}): ${text}` };
      }

      const json = (await res.json()) as { secret: { secretValue: string } };
      return { ok: true, value: json.secret.secretValue };
    } catch (err) {
      return { ok: false, error: `Infisical fetch failed: ${err instanceof Error ? err.message : String(err)}` };
    }
  }

  async hasSecret(key: string, context?: SecretContext): Promise<boolean> {
    const result = await this.resolveSecret(key, context);
    return result.ok;
  }

  async listSecrets(context?: SecretContext): Promise<SecretMetadata[]> {
    const token = await this.getAccessToken();
    const env = context?.environment ?? this.env;
    const projectId = context?.projectId ?? this.config.projectId;
    const secretPath = context?.path ?? '/';

    const params = new URLSearchParams({
      workspaceId: projectId,
      environment: env,
      secretPath,
    });

    const res = await fetch(
      `${this.config.siteUrl}/api/v3/secrets/raw?${params}`,
      {
        headers: {
          Authorization: `Bearer ${token}`,
        },
      },
    );

    if (!res.ok) {
      throw new Error(`Infisical listSecrets failed (${res.status}): ${await res.text()}`);
    }

    const json = (await res.json()) as {
      secrets: Array<{
        secretKey: string;
        environment: string;
        createdAt: string;
        updatedAt: string;
        version: number;
      }>;
    };

    return json.secrets.map((s) => ({
      key: s.secretKey,
      environment: s.environment,
      createdAt: s.createdAt,
      updatedAt: s.updatedAt,
      version: s.version,
    }));
  }

  // ── Universal Auth token management ────────────────────

  private async getAccessToken(): Promise<string> {
    // Re-use token if it has >30s remaining
    if (this.token && this.token.expiresAt > Date.now() + 30_000) {
      return this.token.accessToken;
    }

    const res = await fetch(
      `${this.config.siteUrl}/api/v1/auth/universal-auth/login`,
      {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          clientId: this.config.clientId,
          clientSecret: this.config.clientSecret,
        }),
      },
    );

    if (!res.ok) {
      throw new Error(`Infisical auth failed (${res.status}): ${await res.text()}`);
    }

    const json = (await res.json()) as {
      accessToken: string;
      expiresIn: number; // seconds
      tokenType: string;
    };

    this.token = {
      accessToken: json.accessToken,
      expiresAt: Date.now() + json.expiresIn * 1000,
    };

    return this.token.accessToken;
  }
}
