# ADR-026: Secure Secret Distribution via SpacetimeDB Coordination

## Status

Accepted

## Date

2026-03-18

## Context

ADR-024 (Hex-Nexus) introduced autonomous agent orchestration and ADR-025 added SpacetimeDB as the distributed state backend. Agents need API keys to communicate with LLM inference servers — both cloud providers (Anthropic, OpenAI) and local inference (ollama, vllm, llama.cpp). The existing secrets system (ADR-013) resolves secrets only in the TypeScript composition root. hex-agent (Rust) and independently started agents have no secure way to obtain secrets.

### Requirements

1. hex-hub must distribute secrets to agents it spawns without storing plaintext in SpacetimeDB.
2. Independently started agents (debugging, remote nodes) must also obtain secrets securely.
3. Local inference endpoints (ollama, vllm) must be discoverable by all agents via SpacetimeDB.
4. Secret grants must be scoped per-agent and time-limited.
5. The system must work across all three ISecretsPort backends (env, vault, Infisical).

## Decision

Implement a **Secret Broker** pattern where SpacetimeDB stores only grant metadata (key names, not values) and hex-hub resolves and injects actual secret values at agent spawn time.

### Architecture

```
SpacetimeDB (coordination only — NO secret values)
├── secret_grant table (private)
│   agent_id, secret_key, purpose, granted_at, expires_at, claimed
├── inference_endpoint table (public)
│   id, url, provider, model, status, health_checked_at
└── Reducers: grant_secret, revoke_secret, claim_grant,
    register_endpoint, remove_endpoint, update_health

hex-hub (Secret Broker)
├── Subscribes to secret_grant table
├── On agent spawn: resolves granted keys via ISecretsPort
├── Injects resolved values as env vars into child process
├── Serves /secrets/claim endpoint for independent agents
└── Periodically prunes expired grants

hex-agent (Secret Consumer)
├── EnvSecretsAdapter: reads injected env vars (spawned agents)
├── HubClaimAdapter: one-shot HTTP claim (independent agents)
└── Uses resolved keys to authenticate with inference servers
```

### Secret Grant Lifecycle

```
1. hex-hub creates grant:  grant_secret(agent_id, "ANTHROPIC_API_KEY", "llm", ttl=3600)
2. SpacetimeDB stores:     { agent_id, secret_key, purpose, granted_at, expires_at, claimed: false }
3. hex-hub spawns agent:   resolves key via ISecretsPort → injects as env var
4. Agent reads env var:    std::env::var("ANTHROPIC_API_KEY")
5. Grant marked claimed:   claim_grant(agent_id, secret_key) → claimed = true
6. On expiry or revoke:    revoke_secret(agent_id, secret_key) → row deleted
```

### One-Shot Claim Endpoint (Independent Agents)

For agents not spawned by hex-hub (debugging, remote nodes):

```
1. Agent calls: POST http://127.0.0.1:{hub_port}/secrets/claim
   Body: { "agent_id": "a-007", "nonce": "<random-32-bytes>" }
2. hex-hub verifies: grant exists, not expired, not already claimed
3. hex-hub resolves: ISecretsPort.resolveSecret(key) for each granted key
4. hex-hub responds: { "secrets": { "ANTHROPIC_API_KEY": "sk-..." }, "expires_in": 30 }
5. hex-hub marks:    claim_grant(agent_id, key) → claimed = true
6. Claim is single-use: subsequent calls return 409 Conflict
```

### Inference Endpoint Discovery

The `inference_endpoint` table is public — all agents can subscribe:

```rust
#[table(name = inference_endpoint, public)]
pub struct InferenceEndpoint {
    #[unique]
    pub id: String,
    pub url: String,              // "http://127.0.0.1:11434"
    pub provider: String,         // "ollama" | "openai-compatible" | "vllm"
    pub model: String,            // "llama3.1:70b"
    pub status: String,           // "healthy" | "unhealthy" | "unknown"
    pub requires_auth: bool,      // false for ollama, true for vllm with auth
    pub secret_key: String,       // key name in ISecretsPort (empty if no auth)
    pub health_checked_at: String,
}
```

hex-hub periodically health-checks endpoints and updates status. Agents subscribe and route requests to healthy endpoints.

### LLM Provider Extension

Extend the provider enum in both TypeScript and Rust:

```typescript
type LLMProvider = 'anthropic' | 'openai' | 'ollama' | 'openai-compatible';
```

- `ollama`: OpenAI-compatible API, default `http://127.0.0.1:11434/v1`
- `openai-compatible`: any server at configurable baseUrl exposing `/v1/chat/completions`

### SpacetimeDB Module: `secret-grant`

```rust
#[table(name = secret_grant, private)]  // ← PRIVATE: only reducers can read
pub struct SecretGrant {
    #[unique]
    pub id: String,               // "{agent_id}:{secret_key}"
    pub agent_id: String,
    pub secret_key: String,       // key name, e.g. "ANTHROPIC_API_KEY"
    pub purpose: String,          // "llm" | "webhook" | "auth"
    pub granted_at: String,       // ISO 8601
    pub expires_at: String,       // ISO 8601
    pub claimed: bool,
}
```

## Threat Model

| Threat | Impact | Mitigation |
|--------|--------|-----------|
| SpacetimeDB compromise (disk or memory) | Attacker sees grant metadata | No secret values stored — only key names |
| Rogue agent reads another's secrets | Cross-agent secret theft | Grants scoped by agent_id; broker validates identity at spawn |
| Network sniffing on claim endpoint | Plaintext secrets intercepted | Claim endpoint on 127.0.0.1 only; HTTPS in production |
| Replay attack on claim | Reuse of claimed secrets | Single-use nonces; `claimed: true` prevents re-claim |
| Env var leakage in logs/core dumps | Secrets exposed via process state | hex-agent redacts env vars matching `*KEY*`, `*SECRET*`, `*TOKEN*` |
| Stale grants accumulate | Zombie grants for dead agents | `expires_at` enforced; hex-hub prunes on heartbeat timeout |
| Man-in-the-middle on local inference | Altered inference responses | localhost binding; optional mTLS for remote inference |

## Consequences

### Positive

- Secrets never enter SpacetimeDB — entire grant table can leak with zero secret exposure.
- Works with all three ISecretsPort backends (env, vault, Infisical) without changes.
- Local inference endpoints discoverable by all agents via real-time subscription.
- One-shot claim pattern supports debugging and remote agent scenarios.
- Time-limited grants with automatic pruning prevent secret sprawl.

### Negative

- hex-hub becomes a required intermediary for secret distribution (single point of failure for secrets).
- Claim endpoint adds HTTP surface area to hex-hub (must be secured).
- Two secret resolution paths (spawn injection vs. claim endpoint) increase testing surface.
- Health-check polling for inference endpoints adds background load.

### Mitigations for Negatives

- hex-hub unavailability: agents fall back to direct env var lookup (same as today).
- Claim endpoint security: localhost-only binding, nonce validation, rate limiting.
- Testing: property tests covering grant lifecycle (create → claim → expire → prune).

## Dependencies

- ADR-013 (Secrets Management) — ISecretsPort interface and backend adapters
- ADR-024 (Hex-Nexus) — agent spawn lifecycle in hex-hub
- ADR-025 (SpacetimeDB State Backend) — module infrastructure and IStatePort
