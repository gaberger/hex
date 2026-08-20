# secret-grant

> TTL-based secret distribution + inference endpoint registry + audit log.

Two roles in one module:

1. **Secret grant** — short-lived (TTL) per-agent credentials issued by an operator. Agents `claim_grant` to receive the value once. Expired grants are pruned by hex-nexus on a tick.
2. **Inference endpoint** — service-discovery table for inference providers (Ollama, vLLM, OpenAI-compatible). Registers URL + auth requirement + health.
3. **Secret vault** — long-lived encrypted secret storage (e.g. API keys) keyed by `secret_key`.
4. **Audit log** — append-only record of secret operations (private — readable only by module owner).

## Tables

| Table | Visibility | Key | Purpose |
|---|---|---|---|
| `secret_grant` | **private** | composite `agent_id + secret_key` | Active grants — TTL'd reference to a secret |
| `inference_endpoint` | public | `id` (unique) | Provider registry — URL, provider type, auth flag, health |
| `secret_vault` | public | `secret_key` (unique) | Stored secret values (encrypted at rest by SpacetimeDB) |
| `secret_audit_log` | **private** | `id` (auto_inc) | Audit trail — `actor`, `action`, `secret_key`, `agent_id`, `timestamp` |

`secret_grant` is private — only module-internal reducers see the row. Clients claim via `claim_grant` which returns the value through the reducer return path, never via subscription.

## Reducers

### Grants

| Reducer | Args | Effect |
|---|---|---|
| `grant_secret` | `agent_id, secret_key, expires_at` | Insert grant for agent (`make_grant_id` composes the row key) |
| `claim_grant` | `agent_id, secret_key, now` | Return the secret value if valid + not expired; errors otherwise |
| `revoke_secret` | `agent_id, secret_key` | Delete a single grant |
| `revoke_all_for_agent` | `agent_id` | Delete every grant for an agent |
| `prune_expired` | `now` (RFC3339) | Delete every grant where `is_expired(expires_at, now)` |

### Inference endpoints

| Reducer | Args | Effect |
|---|---|---|
| `register_endpoint` | `id, name, provider, url, requires_auth, secret_key` | Insert/upsert endpoint; validates provider + auth config |
| `update_health` | `id, health_status, last_check` | Update health (`healthy`/`degraded`/`unhealthy`) |
| `remove_endpoint` | `id` | Delete endpoint |

### Vault

| Reducer | Args | Effect |
|---|---|---|
| `store_secret` | `secret_key, value, owner` | Upsert secret value |
| `delete_secret` | `secret_key` | Delete secret |

### Audit

| Reducer | Args | Effect |
|---|---|---|
| `audit_log` | `actor, action, secret_key, agent_id, timestamp` | Append audit row |

## Helpers (lib API)

- `make_grant_id(agent_id, secret_key) -> String` — composite key for `secret_grant`.
- `is_expired(expires_at, now) -> bool` — lexicographic ISO 8601 compare.
- `validate_provider(s)` — accepts `ollama`, `vllm`, `openai`, `anthropic`, etc.
- `validate_health_status(s)` — `healthy`/`degraded`/`unhealthy`/`unknown`.
- `validate_auth_config(requires_auth, secret_key)` — if `requires_auth`, `secret_key` must be non-empty.

## Subscriptions

```sql
SELECT * FROM inference_endpoint WHERE health_status = 'healthy'
-- secret_grant is private; clients use claim_grant() to retrieve values
```

## Example flow

```
register_endpoint("ollama-local", "Ollama (local)", "ollama", "http://localhost:11434", false, "")
store_secret("openrouter_key", "sk-or-...", "owner@example.com")
grant_secret("agent-uuid", "openrouter_key", "2026-05-04T13:00:00Z")
claim_grant("agent-uuid", "openrouter_key", "2026-05-04T12:30:00Z") // returns value
prune_expired("2026-05-04T13:00:01Z")                                // sweeps expired grants
```
