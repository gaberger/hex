# ADR-2603261000: Secure Inference Provider Registry and Encrypted Secrets Vault

**Status:** Accepted
**Date:** 2026-03-26
**Drivers:** Security gap analysis comparing hex to OpenFang — secrets stored plaintext in SQLite, API keys embedded in inference provider records, no memory zeroization, frontier providers (Anthropic, OpenAI) not first-class inference participants, no provider fallback chains.

## Context

hex's current secrets and inference systems have three independent but related problems:

### 1. Secrets are stored plaintext

`hex secrets set` stores values via HexFlo memory → SQLite with no encryption at rest. `hex secrets get` returns values over localhost HTTP (no TLS). There is no equivalent of `Zeroizing<String>` — key material lives in heap memory indefinitely. The SQLite file at `~/.hex/hub.db` contains raw API key values readable by any process with filesystem access.

### 2. Inference provider records embed raw API keys

`hex inference add --key <value>` sends `"secret_key": key` inline in the provider registration JSON body. This value is stored in the inference provider record in SpacetimeDB/SQLite. A key intended for runtime use is now a persisted credential.

### 3. Frontier providers are not first-class inference providers

Anthropic and OpenAI are only usable via environment variables — they cannot be registered with `hex inference add`, cannot participate in fallback chains, and are invisible to the inference registry. This means the system that routes model requests (hex-nexus, agent YAML model selection) cannot treat Anthropic as just another registered provider.

### Why SpacetimeDB is an asset here, not a liability

SpacetimeDB's real-time subscription model gives hex something OpenFang lacks entirely: **live visibility into secret grants across all connected clients**. When an agent claims a grant, every connected dashboard and CLI session sees it instantly. The grant system — TTL, agent scoping, purpose tags — is architecturally superior to OpenFang's env-var-only approach. The goal is not to remove SpacetimeDB from this path but to fix what's stored in it.

The fix is to store **references to secrets** (env var names, vault key IDs) in SpacetimeDB and SQLite — never raw values. Raw values are encrypted at rest in the vault; in-process, they are held in `Zeroizing<String>` and dropped immediately after use.

### Alternatives considered

- **Move to HashiCorp Vault**: Too heavy a dependency for a developer tool. The threat model is local-process isolation, not enterprise secrets management.
- **OS keychain (keyring crate)**: Good for user-interactive scenarios; doesn't work well for background agents or CI. Added as an optional backend, not the default.
- **Keep env-var-only (OpenFang model)**: Loses the grant/TTL/audit capabilities that make hex's system better. Rejected.

## Decision

### Secrets vault

1. **Encrypt at rest.** The vault backend (SQLite via HexFlo memory) stores values encrypted with AES-256-GCM. The encryption key is derived from a machine secret (`~/.hex/vault.key`) generated on first use with `OsRng`. The vault key file is mode `0600`.

2. **Zeroize in-process key material.** All structs holding secret values implement `ZeroizeOnDrop` (via the `zeroize` crate). This includes the vault decryption result, the `SecretGrant` claim payload, and inference provider API keys loaded for a request.

3. **SpacetimeDB stores references, not values.** The `hexflo_memory` table and SpacetimeDB inference tables store vault key IDs (e.g., `vault:OPENROUTER_API_KEY`) or env var names (e.g., `env:ANTHROPIC_API_KEY`). Resolution to raw values happens only in hex-nexus at request time, never in WASM modules.

4. **Grant claims are time-scoped and single-use by default.** An agent that claims a grant receives the decrypted value once over a short-lived authenticated channel (Unix socket or localhost with HMAC token). The grant is then marked `claimed` in SpacetimeDB — visible to all connected clients in real time.

### Inference provider registry

5. **Anthropic, OpenAI, and Gemini become first-class provider types.** `hex inference add anthropic --model claude-sonnet-4-6 --key-ref ANTHROPIC_API_KEY` registers them like any other provider. `--key-ref` takes an env var name or vault key ID — never a raw value.

6. **Provider records store `key_ref`, not `key_value`.** The inference provider schema adds a `key_ref: Option<String>` field (the reference) and removes acceptance of raw key values. hex-nexus resolves the reference at call time.

7. **Fallback chains.** Provider registration accepts `fallback: [provider_id, ...]`. When a request to the primary provider fails (rate limit, timeout, HTTP 5xx), hex-nexus retries against the fallback list in order. Fallback is transparent to the caller.

8. **Model aliases.** A `~/.hex/models.toml` (or SpacetimeDB table) maps short aliases to full model IDs: `sonnet` → `claude-sonnet-4-6`, `opus` → `claude-opus-4-6`. Agent YAMLs can reference aliases; the inference gateway resolves them.

### What changes where

| Component | Change |
|-----------|--------|
| `hex-nexus/src/routes/` | New `/api/secrets/vault` routes: encrypt on set, decrypt on get, HMAC-authenticate claim |
| `hex-core/` | Add `SecretRef` type (enum: `EnvVar(String)` / `VaultKey(String)`); `SecretGrant` holds `SecretRef` not value |
| `hex-cli/src/commands/secrets.rs` | `set` sends to vault endpoint; `get` masked display only (no raw value in CLI output) |
| `hex-cli/src/commands/inference.rs` | `--key` renamed `--key-ref`; raw values rejected; `--fallback` added; anthropic/openai/gemini added as provider types |
| `spacetime-modules/hexflo-coordination/` | `hexflo_memory` stores `SecretRef` string; new `inference_provider` table with `key_ref` column |
| `Cargo.toml` | Add `aes-gcm`, `zeroize` crates |

## Consequences

**Positive:**
- Secrets no longer readable by inspecting `~/.hex/hub.db`
- API key material zeroed from heap on drop — reduces window for memory-scraping attacks
- Anthropic/OpenAI participate in fallback chains and model alias resolution
- SpacetimeDB grant audit log gains value — it now records *what was accessed* not *what the value is*
- All inference providers manageable through one interface (`hex inference list` shows everything including frontier models)

**Negative:**
- Vault key loss (`~/.hex/vault.key` deleted) means stored secrets are unrecoverable — must re-`set`
- Adds `aes-gcm` + `zeroize` crates to the dependency tree
- Existing `hex secrets set` stored values become unreadable (format migration needed)

**Mitigations:**
- Vault key loss: warn prominently in `hex secrets status`; suggest `hex secrets export` before destructive operations
- Migration: `hex secrets migrate` command that re-reads env vars and re-encrypts; old plaintext vault entries detected and flagged
- Dependency surface: both `aes-gcm` and `zeroize` are `RustCrypto` crates — same ecosystem, well-audited, no novel deps

## Implementation

| Phase | Description | Status |
|-------|-------------|--------|
| P1 | Add `aes-gcm` + `zeroize`; implement vault encrypt/decrypt in hex-nexus; migrate routes | Pending |
| P2 | Add `SecretRef` type to hex-core; update `SecretGrant` schema; update SpacetimeDB table | Pending |
| P3 | Update `hex secrets set/get` CLI — vault path only, no raw transmission | Pending |
| P4 | Update inference provider schema: `key_ref` replaces `key_value`; add anthropic/openai/gemini types | Pending |
| P5 | Add fallback chain execution in hex-nexus inference routing | Pending |
| P6 | Add model alias table; update agent YAML model resolution to go through alias lookup | Pending |
| P7 | Migration command + integration tests | Pending |

## References

- ADR-026: Secret management (original design)
- ADR-025: SQLite fallback for offline operation
- ADR-044: Config sync repo → SpacetimeDB
- ADR-060: Inbox notifications
- OpenFang comparison: `Zeroizing<String>` on key fields, `api_key_env` reference pattern
