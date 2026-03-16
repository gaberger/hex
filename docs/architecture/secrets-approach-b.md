# Approach B: SDK-Only External Infisical Packaging

> hex ships adapter code and CLI commands. Users bring their own Infisical instance (cloud, self-hosted, or skip it entirely).

## 1. Overview

In this approach hex contains **zero bundled infrastructure**. The existing `InfisicalAdapter` (fetch-based, no SDK) and `EnvSecretsAdapter` (env-var fallback) are the only moving parts. Users who want centralized secrets management run their own Infisical instance — however they prefer — and point hex at it via a setup wizard.

Users who do **not** want Infisical get two alternatives:
- Plain environment variables (existing `EnvSecretsAdapter`, zero config)
- An encrypted local vault (AES-256-GCM file, no network dependency)

### Architecture Fit

```
composition-root.ts
  ├─ InfisicalAdapter   (if .hex/secrets.json exists + Infisical reachable)
  ├─ LocalVaultAdapter  (if .hex/vault.enc exists)
  └─ EnvSecretsAdapter  (always-available fallback)
```

All three implement `ISecretsPort`. The composition root picks the highest-priority adapter that is configured and healthy, with env vars as the universal fallback.

---

## 2. Installation Flow

### `hex secrets init` — Interactive Wizard

```
$ hex secrets init

? Secrets backend:
  ❯ Infisical (cloud or self-hosted)
    Local encrypted vault
    Environment variables only (no setup needed)

# ── Infisical path ──────────────────────────────
? Infisical URL: https://app.infisical.com
? Authentication method:
  ❯ Universal Auth (Machine Identity)
    Token-based (service token)
? Client ID: ****
? Client Secret: ****
? Default project: my-project (auto-detected from org)
? Default environment: dev

✓ Connection verified — 12 secrets found in dev
✓ Config saved to .hex/secrets.json
✓ Added .hex/secrets.json to .gitignore

# ── Local vault path ────────────────────────────
? Master password: ****
? Confirm: ****
✓ Vault created at .hex/vault.enc
✓ Added .hex/vault.enc to .gitignore
```

### Validation Steps (Infisical path)

1. POST to `/api/v1/auth/universal-auth/login` — confirms credentials
2. GET `/api/v3/secrets/raw?workspaceId=...&environment=...` — confirms project access
3. If either fails, print specific error and retry prompt

### Graceful Skip

```
$ hex secrets init --skip
✓ Secrets management skipped. Using environment variables only.
  Set secrets via: export ANTHROPIC_API_KEY=sk-...
```

No `.hex/secrets.json` is created. `EnvSecretsAdapter` activates automatically.

---

## 3. Configuration File Format

### `.hex/secrets.json`

```jsonc
{
  "$schema": "https://hex-intf.dev/schemas/secrets.json",
  "version": 1,
  "backend": "infisical",       // "infisical" | "local-vault" | "env"

  // ── Infisical-specific ───────────────────────
  "infisical": {
    "siteUrl": "https://app.infisical.com",
    "projectId": "proj_abc123",
    "defaultEnvironment": "dev",
    "auth": {
      "method": "universal-auth",
      // Credentials are NOT stored here — see credential storage below
      "credentialRef": "keychain://hex/infisical/proj_abc123"
    }
  },

  // ── Environment overrides ────────────────────
  "environments": {
    "staging": {
      "projectId": "proj_abc123",
      "environment": "staging"
    },
    "prod": {
      "projectId": "proj_def456",
      "environment": "prod",
      "siteUrl": "https://secrets.corp.example.com"
    }
  }
}
```

### Config Inheritance

Resolution order (first found wins):

1. **Project-level**: `.hex/secrets.json` in project root
2. **User-level**: `~/.hex/secrets.json` (shared defaults across projects)
3. **Environment**: `HEX_SECRETS_SITE_URL`, `HEX_SECRETS_PROJECT_ID`, etc.

Project config can partially override user config (e.g., project specifies `projectId` but inherits `siteUrl` and credentials from user config).

### `.gitignore` Additions

`hex secrets init` appends to `.gitignore`:

```
# hex secrets
.hex/secrets.json
.hex/vault.enc
.hex/vault.key
```

---

## 4. Dependency Strategy

### Option 1: Keep fetch-based adapter (RECOMMENDED)

The existing `InfisicalAdapter` uses raw `fetch()` — zero npm dependencies. This is the right default because:

- **npm install stays fast**: no native modules, no postinstall scripts
- **Works in Bun, Node, Deno**: fetch is a universal API
- **Covers 95% of use cases**: CRUD on static secrets, Universal Auth, token refresh
- **Matches hex's existing pattern**: `LLMAdapter` also uses raw fetch

What fetch covers:
- Universal Auth login + token refresh
- Read/write/list/delete static secrets
- Folder-scoped operations
- Environment switching

### Option 2: Optional `@infisical/sdk` peer dependency

For advanced features not available via REST API:

| Feature | REST API | SDK Required |
|---------|----------|--------------|
| Static secrets CRUD | Yes | No |
| Dynamic secrets (DB creds, AWS STS) | No | Yes |
| PKI / certificate issuance | No | Yes |
| Secret rotation triggers | No | Yes |
| Automatic lease renewal | No | Yes |

**Declaration in `package.json`:**

```jsonc
{
  "peerDependencies": {
    "@infisical/sdk": "^2.0.0"
  },
  "peerDependenciesMeta": {
    "@infisical/sdk": {
      "optional": true
    }
  }
}
```

**Runtime detection in composition root:**

```typescript
async function resolveSecretsAdapter(config: SecretsConfig): Promise<ISecretsPort> {
  if (config.backend === 'infisical' && config.infisical?.features?.includes('dynamic-secrets')) {
    try {
      const { InfisicalSDKAdapter } = await import('./adapters/secondary/infisical-sdk-adapter.js');
      return new InfisicalSDKAdapter(config.infisical);
    } catch {
      throw new Error(
        'Dynamic secrets require @infisical/sdk. Install it:\n  npm install @infisical/sdk'
      );
    }
  }
  // Default: fetch-based, zero deps
  return new InfisicalAdapter(toAdapterConfig(config));
}
```

### Recommendation

**Ship with fetch-only (Option 1) now.** Add the optional SDK peer dep later if users request dynamic secrets. The REST API covers every use case hex agents need today (reading API keys, DB passwords, and config values at runtime).

---

## 5. CLI Commands

### `hex secrets init`

Interactive setup wizard (see Section 2).

```
hex secrets init                    # Interactive wizard
hex secrets init --local            # Skip to local vault setup
hex secrets init --skip             # Opt out, env-vars only
hex secrets init --site-url URL \
  --client-id ID --client-secret S  # Non-interactive (CI)
```

### `hex secrets status`

```
$ hex secrets status

Backend:     Infisical (https://app.infisical.com)
Project:     my-project (proj_abc123)
Environment: dev
Auth:        Universal Auth (token valid, expires in 47m)
Secrets:     12 keys accessible
Cache:       3 entries, oldest 4m ago
```

Exit codes: `0` = healthy, `1` = unreachable, `2` = auth expired, `3` = not configured.

### `hex secrets list`

```
$ hex secrets list

KEY                     ENVIRONMENT  UPDATED         VERSION
ANTHROPIC_API_KEY       dev          2 hours ago     3
DATABASE_URL            dev          1 day ago       7
REDIS_URL               dev          1 day ago       2
STRIPE_SECRET_KEY       dev          5 days ago      1

$ hex secrets list --env staging
$ hex secrets list --path /api-keys
$ hex secrets list --json          # Machine-readable output
```

Values are **never** printed. Only metadata.

### `hex secrets run -- <command>`

Injects resolved secrets as environment variables into a subprocess:

```
$ hex secrets run -- bun run dev
# Equivalent to:
# ANTHROPIC_API_KEY=sk-... DATABASE_URL=postgres://... bun run dev

$ hex secrets run --env staging -- npm test
$ hex secrets run --only ANTHROPIC_API_KEY,DATABASE_URL -- node server.js
```

Implementation:

```typescript
import { spawn } from 'node:child_process';

async function secretsRun(
  secrets: ISecretsPort,
  args: string[],
  opts: { env?: string; only?: string[] },
): Promise<number> {
  const context: SecretContext = { environment: opts.env };
  const allMeta = await secrets.listSecrets(context);
  const keys = opts.only ?? allMeta.map((m) => m.key);

  const secretEnv: Record<string, string> = {};
  for (const key of keys) {
    const result = await secrets.resolveSecret(key, context);
    if (result.ok) secretEnv[key] = result.value;
  }

  const child = spawn(args[0], args.slice(1), {
    stdio: 'inherit',
    env: { ...process.env, ...secretEnv },
  });

  return new Promise((resolve) => {
    child.on('close', (code) => resolve(code ?? 1));
  });
}
```

### `hex secrets pull <file>`

Exports secrets to a dotenv file for tools that require it:

```
$ hex secrets pull .env.local

✓ Wrote 12 secrets to .env.local
⚠ .env.local added to .gitignore

$ hex secrets pull .env.staging --env staging
$ hex secrets pull --only ANTHROPIC_API_KEY,DATABASE_URL .env.local
```

The command refuses to overwrite without `--force` and always checks `.gitignore`.

### `hex secrets set <key> <value>` (local vault only)

```
$ hex secrets set ANTHROPIC_API_KEY sk-ant-...
✓ Stored ANTHROPIC_API_KEY in local vault

$ hex secrets delete ANTHROPIC_API_KEY
✓ Removed ANTHROPIC_API_KEY from local vault
```

For Infisical-backed setups, direct users to the Infisical UI/CLI for writes.

---

## 6. Agent Integration

### Secret References in Agent YAML

Agents reference secrets by logical key name. No values appear in YAML:

```yaml
# agents/hex-coder.yml
name: hex-coder
secrets:
  - key: ANTHROPIC_API_KEY
    required: true
  - key: DATABASE_URL
    required: false
    fallback_env: DATABASE_URL   # Try env var if secrets backend fails
  - key: GITHUB_TOKEN
    required: false
```

### Runtime Resolution Flow

```
Agent spawns
  → composition-root reads agents/<name>.yml
  → for each secret in secrets[]:
      1. ISecretsPort.resolveSecret(key, { environment: currentEnv })
      2. If ok → inject into agent's process.env
      3. If not ok AND fallback_env set → try process.env[fallback_env]
      4. If not ok AND required: true → abort agent with clear error
      5. If not ok AND required: false → skip, log warning
```

### Caching Strategy

Secrets are cached **per-session** with TTL:

```typescript
interface CachedSecret {
  value: string;
  fetchedAt: number;  // epoch ms
}

class CachingSecretsAdapter implements ISecretsPort {
  private cache = new Map<string, CachedSecret>();
  private readonly ttlMs: number;

  constructor(
    private readonly inner: ISecretsPort,
    ttlMs: number = 5 * 60 * 1000, // 5 minutes default
  ) {
    this.ttlMs = ttlMs;
  }

  async resolveSecret(key: string, context?: SecretContext): Promise<SecretResult> {
    const cacheKey = `${key}:${context?.environment ?? 'default'}:${context?.path ?? '/'}`;
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

  // hasSecret and listSecrets delegate to inner (no caching)
  async hasSecret(key: string, ctx?: SecretContext): Promise<boolean> {
    return this.inner.hasSecret(key, ctx);
  }
  async listSecrets(ctx?: SecretContext): Promise<SecretMetadata[]> {
    return this.inner.listSecrets(ctx);
  }
}
```

Cache configuration in `.hex/secrets.json`:

```jsonc
{
  "cache": {
    "ttlSeconds": 300,    // 5 min default
    "maxEntries": 100     // Evict LRU beyond this
  }
}
```

### Unreachable Infisical — Fallback Behavior

| Scenario | Behavior |
|----------|----------|
| Infisical unreachable, cached value exists | Return cached value (stale-while-revalidate) |
| Infisical unreachable, no cache, `fallback_env` set | Read from `process.env` |
| Infisical unreachable, no cache, `required: true` | Fail with actionable error |
| Infisical unreachable, no cache, `required: false` | Skip, log warning |
| Auth token expired | Auto-refresh via Universal Auth (existing behavior) |
| Auth credentials revoked | Fail with "re-run `hex secrets init`" message |

The composition root wraps `InfisicalAdapter` in `CachingSecretsAdapter`, so stale reads are the first fallback automatically.

---

## 7. Zero-Docker Experience: Local Encrypted Vault

For users who want secrets management without running any external service.

### Setup

```
$ hex secrets init --local

? Master password: ********
? Confirm password: ********

✓ Generated vault at .hex/vault.enc
✓ Key derivation: PBKDF2 (SHA-512, 600,000 iterations)
✓ Encryption: AES-256-GCM
✓ Added .hex/vault.enc to .gitignore
```

### Implementation: `LocalVaultAdapter`

```typescript
import { createCipheriv, createDecipheriv, pbkdf2Sync, randomBytes } from 'node:crypto';
import { readFileSync, writeFileSync } from 'node:fs';
import type { ISecretsPort, SecretMetadata, SecretResult } from '../../core/ports/secrets.js';

interface VaultData {
  version: 1;
  secrets: Record<string, { value: string; createdAt: string; updatedAt: string; version: number }>;
}

interface EncryptedVault {
  salt: string;     // hex, 32 bytes
  iv: string;       // hex, 16 bytes
  tag: string;      // hex, 16 bytes (GCM auth tag)
  data: string;     // hex, AES-256-GCM ciphertext
  kdf: 'pbkdf2';
  kdfIterations: number;
}

export class LocalVaultAdapter implements ISecretsPort {
  private vault: VaultData | null = null;

  constructor(
    private readonly vaultPath: string,
    private readonly password: string,
  ) {}

  async resolveSecret(key: string): Promise<SecretResult> {
    const data = this.getVault();
    const entry = data.secrets[key];
    if (entry) return { ok: true, value: entry.value };
    return { ok: false, error: `Key "${key}" not found in local vault` };
  }

  async hasSecret(key: string): Promise<boolean> {
    const data = this.getVault();
    return key in data.secrets;
  }

  async listSecrets(): Promise<SecretMetadata[]> {
    const data = this.getVault();
    return Object.entries(data.secrets).map(([key, entry]) => ({
      key,
      environment: 'local',
      createdAt: entry.createdAt,
      updatedAt: entry.updatedAt,
      version: entry.version,
    }));
  }

  // ── Encryption ────────────────────────────────

  private getVault(): VaultData {
    if (this.vault) return this.vault;
    const raw = readFileSync(this.vaultPath, 'utf-8');
    const encrypted: EncryptedVault = JSON.parse(raw);
    const key = pbkdf2Sync(
      this.password,
      Buffer.from(encrypted.salt, 'hex'),
      encrypted.kdfIterations,
      32,
      'sha512',
    );
    const decipher = createDecipheriv(
      'aes-256-gcm',
      key,
      Buffer.from(encrypted.iv, 'hex'),
    );
    decipher.setAuthTag(Buffer.from(encrypted.tag, 'hex'));
    const decrypted = Buffer.concat([
      decipher.update(Buffer.from(encrypted.data, 'hex')),
      decipher.final(),
    ]);
    this.vault = JSON.parse(decrypted.toString('utf-8'));
    return this.vault!;
  }

  static createVault(vaultPath: string, password: string): void {
    const salt = randomBytes(32);
    const iv = randomBytes(16);
    const key = pbkdf2Sync(password, salt, 600_000, 32, 'sha512');
    const data: VaultData = { version: 1, secrets: {} };
    const plaintext = Buffer.from(JSON.stringify(data));
    const cipher = createCipheriv('aes-256-gcm', key, iv);
    const encrypted = Buffer.concat([cipher.update(plaintext), cipher.final()]);
    const tag = cipher.getAuthTag();
    const vault: EncryptedVault = {
      salt: salt.toString('hex'),
      iv: iv.toString('hex'),
      tag: tag.toString('hex'),
      data: encrypted.toString('hex'),
      kdf: 'pbkdf2',
      kdfIterations: 600_000,
    };
    writeFileSync(vaultPath, JSON.stringify(vault, null, 2));
  }
}
```

### Password Input

The master password is obtained at runtime via one of (in priority order):

1. **macOS Keychain** (via `security find-generic-password`): stored on first `init`, retrieved automatically
2. **`HEX_VAULT_PASSWORD` env var**: for CI/CD or scripted use
3. **Interactive prompt**: `readline` with masked input (TTY only)

```typescript
async function getVaultPassword(): Promise<string> {
  // 1. Keychain (macOS)
  if (process.platform === 'darwin') {
    try {
      const { execFileSync } = await import('node:child_process');
      const pw = execFileSync('security', [
        'find-generic-password', '-s', 'hex-vault', '-w',
      ], { encoding: 'utf-8' }).trim();
      if (pw) return pw;
    } catch { /* not stored yet */ }
  }

  // 2. Environment variable
  if (process.env.HEX_VAULT_PASSWORD) return process.env.HEX_VAULT_PASSWORD;

  // 3. Interactive prompt
  return promptPassword('Vault password: ');
}
```

### Vault File Location

| Platform | Path |
|----------|------|
| Project-scoped | `.hex/vault.enc` (gitignored) |
| User-scoped | `~/.hex/vault.enc` (shared across projects) |

Project vault takes priority. User vault is fallback.

---

## 8. Tradeoffs Analysis

### Pros

| Advantage | Detail |
|-----------|--------|
| **Zero infrastructure overhead** | No Docker, no background processes, no ports to manage |
| **npm-native** | `npm install hex-intf` — done. No post-install scripts, no native modules |
| **Works with existing deployments** | Teams already running Infisical just point hex at their instance |
| **CI/CD friendly** | Env vars + service tokens work out of the box, no sidecar containers |
| **Smaller attack surface** | No bundled server means no CVEs from Infisical's dependency tree |
| **Offline capable** | Local vault works without network access |
| **Multi-runtime** | fetch-based adapter works in Node, Bun, Deno, Cloudflare Workers |

### Cons

| Disadvantage | Detail |
|--------------|--------|
| **More setup for new users** | Must provision Infisical separately (cloud signup or Docker Compose) |
| **No UI bundled** | No secrets dashboard — users manage secrets via Infisical's own UI or `hex secrets` CLI |
| **Feature ceiling** | Without SDK, no dynamic secrets, PKI, or auto-rotation |
| **Auth complexity** | Users must create Machine Identities in Infisical's admin panel |
| **Split documentation** | hex docs + Infisical docs — user bounces between two sources |

### Who This Is Best For

| User Profile | Why Approach B Fits |
|-------------|---------------------|
| **Solo developers** | Local vault or env vars — zero infra overhead |
| **Existing Infisical users** | Point hex at existing instance, immediate value |
| **CI/CD pipelines** | Env vars or service tokens — standard patterns |
| **Teams with security policies** | BYO infrastructure means secrets never touch hex's supply chain |
| **Open-source project maintainers** | Contributors use env vars, maintainers use Infisical — both work |

### Who Should Consider Approach A Instead

| User Profile | Why |
|-------------|-----|
| **Teams new to secrets management** | Bundled instance removes setup friction |
| **Workshop/training environments** | One command to get everything running |
| **Air-gapped environments** | Bundled Docker image avoids pulling from registries |

---

## 9. Security Considerations

### Credential Storage

| Platform | Storage | Implementation |
|----------|---------|----------------|
| macOS | Keychain Services | `security add-generic-password -s hex-infisical -a <projectId> -w <clientSecret>` |
| Linux | `libsecret` / `pass` | `secret-tool store --label hex-infisical service hex-infisical` |
| Windows | Windows Credential Manager | `cmdkey /add:hex-infisical /user:<clientId> /pass:<clientSecret>` |
| CI/CD | Environment variables | `HEX_INFISICAL_CLIENT_ID`, `HEX_INFISICAL_CLIENT_SECRET` |

The `.hex/secrets.json` file stores a `credentialRef` URI (e.g., `keychain://hex/infisical/proj_abc123`) — never the actual credential.

### Token Lifecycle

```
Client credentials (long-lived, in keychain)
  → Universal Auth login
    → Access token (short-lived, 7200s default)
      → Cached in memory (never written to disk)
        → Auto-refresh 30s before expiry (existing InfisicalAdapter behavior)
```

- Access tokens exist only in process memory — not persisted to disk
- If the process exits, the next run gets a fresh token
- Client credentials can be rotated in Infisical without touching hex config (just re-run `hex secrets init`)

### Local Vault Encryption

| Parameter | Value | Rationale |
|-----------|-------|-----------|
| Algorithm | AES-256-GCM | Authenticated encryption — tamper detection built in |
| KDF | PBKDF2-SHA512 | Widely supported, NIST-approved |
| Iterations | 600,000 | OWASP 2023 recommendation for PBKDF2-SHA512 |
| Salt | 32 bytes, random | Per-vault, prevents rainbow tables |
| IV | 16 bytes, random | Per-encryption, prevents nonce reuse |
| Auth tag | 128-bit | GCM default, detects tampering |

Re-encryption happens on every write (new IV + salt). The vault file is atomic-written (write to `.hex/vault.enc.tmp`, then rename).

### Network Security

- **TLS verification**: Node's default TLS verification is used. `hex secrets init` rejects self-signed certs unless `--insecure` is explicitly passed (with a warning).
- **DNS rebinding**: The adapter validates that `siteUrl` uses HTTPS (except `localhost` for local dev).
- **Request timeouts**: 10-second timeout on all Infisical API calls to prevent hanging agents.

```typescript
const controller = new AbortController();
const timeout = setTimeout(() => controller.abort(), 10_000);
try {
  const res = await fetch(url, {
    headers: { Authorization: `Bearer ${token}` },
    signal: controller.signal,
  });
  // ...
} finally {
  clearTimeout(timeout);
}
```

### Threat Model Summary

| Threat | Mitigation |
|--------|------------|
| Secret leaked in git | `.hex/secrets.json` + `.hex/vault.enc` auto-added to `.gitignore` |
| Credential on disk | Keychain/credential manager — not plaintext files |
| Vault brute force | PBKDF2 600K iterations — ~3 seconds per guess on modern hardware |
| Man-in-the-middle | TLS verification enforced, HTTPS required for non-localhost |
| Process memory dump | Secrets are short-lived strings, not pinned — same risk as any env-var-based app |
| Token theft | Access tokens are memory-only, short-lived (2h), auto-rotated |
| Stale cached secret | TTL-based cache eviction (default 5 min) |

---

## Appendix: Composition Root Wiring

How the composition root selects and wires the secrets adapter:

```typescript
// composition-root.ts (additions)

import type { ISecretsPort } from './core/ports/secrets.js';
import { EnvSecretsAdapter } from './adapters/secondary/env-secrets-adapter.js';

async function buildSecretsAdapter(): Promise<ISecretsPort> {
  const configPath = path.join(projectRoot, '.hex', 'secrets.json');

  // 1. Check for config file
  if (!existsSync(configPath)) {
    return new EnvSecretsAdapter(); // No config = env vars only
  }

  const config = JSON.parse(readFileSync(configPath, 'utf-8'));

  // 2. Local vault
  if (config.backend === 'local-vault') {
    const { LocalVaultAdapter } = await import('./adapters/secondary/local-vault-adapter.js');
    const password = await getVaultPassword();
    const vaultPath = path.join(projectRoot, '.hex', 'vault.enc');
    return new LocalVaultAdapter(vaultPath, password);
  }

  // 3. Infisical
  if (config.backend === 'infisical' && config.infisical) {
    const { InfisicalAdapter } = await import('./adapters/secondary/infisical-adapter.js');
    const credentials = await loadCredentials(config.infisical.auth.credentialRef);
    const adapter = new InfisicalAdapter({
      siteUrl: config.infisical.siteUrl,
      clientId: credentials.clientId,
      clientSecret: credentials.clientSecret,
      projectId: config.infisical.projectId,
      defaultEnvironment: config.infisical.defaultEnvironment,
    });

    // Wrap with caching
    const { CachingSecretsAdapter } = await import('./adapters/secondary/caching-secrets-adapter.js');
    const ttl = (config.cache?.ttlSeconds ?? 300) * 1000;
    return new CachingSecretsAdapter(adapter, ttl);
  }

  // 4. Fallback
  return new EnvSecretsAdapter();
}
```

---

## Appendix: Migration Path Between Approaches

If a team starts with Approach B and later wants to adopt Approach A (embedded), or vice versa:

| From | To | Steps |
|------|----|-------|
| Env vars → Infisical | `hex secrets init`, enter Infisical URL + creds | 2 min |
| Infisical → Env vars | `hex secrets pull .env.local`, delete `.hex/secrets.json` | 1 min |
| Local vault → Infisical | `hex secrets pull .env.tmp`, import into Infisical, `hex secrets init` | 5 min |
| Approach A → B | Export secrets from embedded Infisical, point config at external instance | 10 min |
| Approach B → A | `hex secrets init --embedded` (if Approach A ships later) | 5 min |

The `ISecretsPort` abstraction makes the adapter swap invisible to agents and use cases.
