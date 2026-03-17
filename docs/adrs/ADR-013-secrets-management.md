# ADR-013: Secrets Management

## Status

Accepted

## Date

2026-03-17

## Context

hex agents need API keys at runtime -- LLM provider tokens, webhook URLs, Infisical credentials, and service mesh authentication. These secrets must never appear in source code, git history, or log output. The challenge is that hex runs in multiple modes (interactive CLI, background swarm agents, CI pipelines), each with different security constraints and secret injection mechanisms.

Requirements:
- Support local development (env vars), team development (encrypted vault), and production (secrets manager).
- Never require a specific secrets backend -- projects must be able to start with env vars and graduate to Infisical without code changes.
- Cache resolved secrets to avoid repeated network round-trips during a single session.
- Provide a consistent `ISecretsPort` interface so use cases and adapters never know which backend is active.

## Decision

Implement a three-backend secrets system behind a single `ISecretsPort` interface, with a `CachingSecretsAdapter` decorator and a factory that selects the backend from project configuration.

### Port Interface

```typescript
interface ISecretsPort {
  resolveSecret(key: string, context?: SecretContext): Promise<SecretResult>;
  hasSecret(key: string, context?: SecretContext): Promise<boolean>;
  listSecrets(context?: SecretContext): Promise<SecretMetadata[]>;
}
```

`SecretResult` is a discriminated union (`{ ok: true, value: string } | { ok: false, error: string }`) to avoid throwing on missing keys. `SecretContext` carries optional `environment` and `path` fields for backends that support scoping.

### Backend Adapters

| Backend | Adapter | Use Case | Auth |
|---------|---------|----------|------|
| **Environment variables** | `EnvSecretsAdapter` | Local dev, CI | `process.env` lookup |
| **Local encrypted vault** | `LocalVaultAdapter` | Team dev, offline | AES-256-GCM encrypted JSON file at `.hex/vault.enc`; password from `HEX_VAULT_PASSWORD` env var |
| **Infisical** | `InfisicalAdapter` | Production, hosted teams | Universal Auth (client ID + secret) via REST API |

### Backend Selection

The `buildSecretsAdapter` factory reads `.hex/secrets.json` from the project root:

```json
{
  "version": 1,
  "backend": "infisical",
  "infisical": {
    "siteUrl": "https://secrets.example.com",
    "projectId": "proj_abc123",
    "defaultEnvironment": "dev",
    "auth": {
      "method": "universal-auth",
      "clientId": "...",
      "clientSecret": "..."
    }
  },
  "cache": { "ttlSeconds": 300 }
}
```

When no config file exists, the factory falls back to `EnvSecretsAdapter`. When the configured backend cannot be initialized (missing vault file, invalid credentials), it also falls back to env vars with a warning.

### Caching Decorator

`CachingSecretsAdapter` wraps any `ISecretsPort` with TTL-based in-memory caching:

- Only `resolveSecret` results are cached (not `hasSecret` or `listSecrets`).
- Cache key is `key:environment:path` to avoid cross-environment collisions.
- Default TTL is 5 minutes (300 seconds), configurable in `.hex/secrets.json`.
- `clearCache()` method enables manual invalidation.

The decorator pattern means caching is composable -- it works with any backend without modifying the backend adapter.

### Security Properties

| Property | Implementation |
|----------|---------------|
| **No secrets in source** | `.hex/secrets.json` contains config, not secret values. Infisical credentials come from env vars. |
| **No secrets in logs** | `resolveSecret` returns a `SecretResult` union, not a raw string. Adapters never log resolved values. |
| **No secrets in git** | `.hex/vault.enc` is encrypted. `.env` files are gitignored. |
| **Composition root only** | Secrets are resolved in `composition-root.ts` and injected into adapters via constructor parameters. |
| **Path traversal safe** | `LocalVaultAdapter` receives an absolute resolved path from the factory, not user input. |

## Consequences

### Positive

- Zero-config start: projects work immediately with env vars, no setup required.
- Gradual adoption: teams can migrate from env vars to local vault to Infisical without changing application code.
- Caching reduces latency for secrets-heavy workflows (LLM calls that need API keys on every request).
- The factory pattern centralizes backend selection in one file, keeping adapters simple and single-purpose.
- `SecretResult` union type makes missing-key handling explicit -- no silent `undefined` values.

### Negative

- Three backend adapters increase implementation surface area.
- Local vault encryption uses a password-derived key; password strength is the user's responsibility.
- Infisical adapter requires network access; offline development must use env vars or local vault.
- Cache invalidation is time-based only -- no event-driven refresh when secrets rotate.

## Alternatives Considered

1. **HashiCorp Vault** -- Industry standard, but requires a running server and has significant operational overhead for a development tool. Rejected as too heavy for the primary use case (single-developer AI-assisted development).
2. **AWS Secrets Manager / GCP Secret Manager** -- Cloud-provider-specific. Rejected because hex must be cloud-agnostic.
3. **dotenv files only** -- Simple and widely understood, but no encryption, no team sharing, and no environment scoping. Retained as the fallback backend but not sufficient as the only option.
4. **1Password CLI (`op`)** -- Good developer UX, but requires a 1Password subscription and the `op` CLI installed. Too vendor-specific for a framework default.
5. **SOPS (Mozilla)** -- Encrypted files with cloud KMS. Good for GitOps but requires KMS setup. Could be added as a future backend behind the same `ISecretsPort`.
