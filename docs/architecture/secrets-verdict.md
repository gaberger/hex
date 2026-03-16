# Secrets Packaging Verdict

## 1. Executive Summary

**Approach B (SDK-only) wins decisively**, with one element borrowed from Approach A. hex is an npm package — introducing a Docker dependency fundamentally conflicts with the install experience. Approach B's fetch-based adapter, local vault, and BYO-Infisical model align with hex's existing zero-infrastructure philosophy. The one valuable idea from Approach A — a convenience command to spin up a local Infisical stack for teams that want it — should be offered as an optional `hex secrets serve` command, not as the default path.

## 2. Scoring Matrix

| Criterion | Weight | Approach A (Embedded) | Approach B (SDK-Only) | Notes |
|-----------|--------|----------------------|----------------------|-------|
| Installation friction | 20% | 4 | 9 | A requires Docker + 800MB image pull before secrets work |
| npm packaging complexity | 15% | 3 | 9 | A ships Docker Compose YAMLs and lifecycle code in the npm package |
| Security posture | 20% | 6 | 8 | B uses OS keychain for credentials; A stores all secrets in a plaintext JSON config file |
| Resource overhead | 10% | 3 | 10 | A consumes 230-500MB RAM idle; B consumes zero |
| Offline/air-gapped support | 10% | 7 | 8 | Both work offline. A requires pre-pulled images; B's local vault has zero deps |
| Alignment with hex patterns | 15% | 5 | 9 | B follows composition-root auto-discovery and graceful-null fallback patterns exactly |
| Maintenance burden | 10% | 3 | 8 | A requires tracking Infisical Docker image versions, Postgres migrations, Redis config |

**Weighted scores:**

- **Approach A**: 4.30
- **Approach B**: 8.80

## 3. Detailed Analysis

### 3.1 Installation Friction (20%)

**Approach A: 4/10.** The `hex setup --secrets` command checks for Docker, pulls three images (~800MB), generates config, and runs a first-run wizard that creates an Infisical admin account, machine identity, and project via API calls. This is a 30-60 second operation on a warm cache, longer on first run. A user who just wanted to store an API key must install Docker Desktop first.

**Approach B: 9/10.** `hex secrets init` is an interactive prompt that takes under 30 seconds. The local vault path requires only a password. The Infisical path requires credentials the user already has. The `--skip` flag makes it zero-step. The only reason it's not 10 is that Infisical setup still requires users to create a Machine Identity in Infisical's admin panel — hex can't automate that for an external instance.

### 3.2 npm Packaging Complexity (15%)

**Approach A: 3/10.** hex's npm package would need to include Docker Compose YAML templates, a `DockerSecretsAdapter` that shells out to `docker compose`, health-check polling logic, and a first-run wizard that calls Infisical's admin API. This is ~400 lines of infrastructure code in the npm package that has nothing to do with hex's core value proposition. The generated `.hex/docker-compose.secrets.yml` file (Section 3 of Approach A) is 80 lines of YAML that hex must template, version, and maintain.

**Approach B: 9/10.** The fetch-based `InfisicalAdapter` already exists. The new code is `LocalVaultAdapter` (~80 lines using Node's built-in `crypto`), `CachingSecretsAdapter` (~40 lines), and CLI commands. Total new code: ~300 lines, all using hex's existing patterns and zero new npm dependencies.

### 3.3 Security Posture (20%)

**Approach A: 6/10.** Secrets are encrypted at rest by Infisical (AES-256-GCM), which is good. However, `.hex/secrets-config.json` stores the encryption key, auth secret, Postgres password, AND machine identity credentials all in one plaintext JSON file (Section 9.4). File permissions (0600) are the only protection. If this file leaks, the entire secrets store is compromised — the attacker has the encryption key AND the API credentials. The Docker network exposure (Postgres on 5433, Redis on 6380) also increases the attack surface, even if host-only.

**Approach B: 8/10.** Credentials are stored in the OS keychain (macOS Keychain, libsecret on Linux, Windows Credential Manager) — never in plaintext files. The `.hex/secrets.json` file contains only a `credentialRef` URI, not the actual secret. The local vault uses PBKDF2 with 600K iterations (OWASP 2023 recommendation) and AES-256-GCM. Access tokens exist only in process memory. The attack surface is minimal: no listening ports, no containers, no databases.

### 3.4 Resource Overhead (10%)

**Approach A: 3/10.** 230MB RAM idle, up to 500MB active. 800MB disk for Docker images alone. Three containers running continuously while `hex serve` is active. On an 8GB developer machine already running an IDE, browser, and Claude Code, this is significant. The doc acknowledges this (Section 7, Cons: "Non-trivial on 8GB machines").

**Approach B: 10/10.** Zero background processes, zero RAM overhead, zero disk beyond config files. The local vault is a single JSON file under 10KB.

### 3.5 Offline/Air-Gapped Support (10%)

**Approach A: 7/10.** Works offline after initial Docker image pull. But the initial pull requires internet access to Docker Hub, and Docker images are large. In truly air-gapped environments, you need to pre-stage images via `docker save/load`.

**Approach B: 8/10.** The local vault works with zero network. For Infisical-backed setups, the caching layer provides stale-while-revalidate (Section 6, "Unreachable Infisical — Fallback Behavior"). Env vars work everywhere. The only gap: you can't do the initial `hex secrets init` for Infisical without network, but that's inherent to using any remote service.

### 3.6 Alignment with hex Patterns (15%)

**Approach A: 5/10.** Introduces a new pattern: hex managing Docker containers. hex currently has zero Docker dependencies. The `DockerSecretsAdapter` shells out to `docker compose` — this is hex acting as an infrastructure orchestrator, which is outside its core domain. The composition root changes (Section 5.1) add Docker health-check logic that runs before every `createAppContext()`, adding latency and failure modes to all hex commands, not just secrets-related ones.

**Approach B: 9/10.** Follows hex's existing patterns exactly. Three adapters implement `ISecretsPort`, selected by the composition root based on available config — this is textbook hexagonal architecture. The `CachingSecretsAdapter` is a decorator pattern already used elsewhere in hex. The local vault uses Node built-ins only. The graceful fallback chain (Infisical -> local vault -> env vars) matches hex's existing "try, fall back, warn" pattern.

### 3.7 Maintenance Burden (10%)

**Approach A: 3/10.** hex would need to track Infisical server releases, Postgres compatibility, Redis version requirements, and Docker Compose schema changes. The upgrade command (Section 6.2) is a 5-step process involving image pulls, stack restarts, and implicit database migrations. If Infisical changes its admin API between versions, the first-run wizard breaks. The rollback mechanism stores previous versions in config, adding state management complexity.

**Approach B: 8/10.** The fetch-based adapter talks to a stable REST API. If Infisical changes their API, users update their own Infisical instance, and hex only needs to update the adapter code. The local vault is self-contained crypto with no external dependencies. The optional `@infisical/sdk` peer dependency (Section 4, Option 2) is explicitly deferred to "later if users request dynamic secrets."

## 4. Critical Risks

### Approach A Risks

| Risk | Severity | Likelihood | Detail |
|------|----------|------------|--------|
| Docker not available | HIGH | HIGH | Many corporate machines restrict Docker. Windows Home users don't have Hyper-V. WSL2 setups are fragile. |
| First-run wizard API breakage | HIGH | MEDIUM | The wizard calls Infisical's admin setup API to create accounts, identities, and projects. These APIs change between Infisical releases. |
| Port conflicts | MEDIUM | MEDIUM | Ports 8080, 5433, 6380 may already be in use. The doc doesn't specify conflict resolution. |
| Supply chain attack | MEDIUM | LOW | Pulling `infisical/infisical:v0.86.0` from Docker Hub introduces a dependency on Docker Hub's security. Digest pinning is mentioned but not implemented in the YAML. |
| Config file compromise | HIGH | LOW | `.hex/secrets-config.json` is a single point of failure containing all credentials and the encryption key. |
| Docker Desktop licensing | MEDIUM | MEDIUM | Docker Desktop requires a paid subscription for companies with >250 employees or >$10M revenue. |

### Approach B Risks

| Risk | Severity | Likelihood | Detail |
|------|----------|------------|--------|
| Infisical setup complexity | MEDIUM | MEDIUM | Creating a Machine Identity in Infisical's UI requires multiple steps. New users may struggle. |
| Keychain integration fragility | LOW | MEDIUM | OS keychain APIs differ across platforms and may require elevated permissions. The `security` CLI on macOS works but `libsecret` on Linux varies by distro. |
| Local vault password forgotten | MEDIUM | LOW | No recovery mechanism. Password loss = re-create vault. Acceptable for dev use. |
| Stale cache serving wrong values | LOW | LOW | 5-minute TTL means secrets updated in Infisical take up to 5 minutes to propagate. Acceptable for dev workflows. |

## 5. Hybrid Recommendation

**Take Approach B as the foundation. Add Approach A's Docker option as an opt-in convenience command.**

### What to take from Approach B (the base)

- `InfisicalAdapter` (fetch-based, existing code)
- `LocalVaultAdapter` (new, ~80 lines)
- `CachingSecretsAdapter` (new, ~40 lines)
- `EnvSecretsAdapter` (existing)
- Composition root auto-discovery: config file -> local vault -> env vars
- OS keychain credential storage
- All CLI commands: `init`, `status`, `list`, `run`, `pull`, `set`
- Agent YAML `secrets:` declarations with fallback behavior
- Config inheritance (project -> user -> env)

### What to take from Approach A (optional add-on)

- `hex secrets serve` command: starts an Infisical Docker stack for users who want a local instance but don't want to configure it manually. This is a convenience, not a requirement.
- The command generates a Docker Compose file and runs the first-run wizard, but:
  - It is NEVER invoked automatically by `hex serve`
  - It is documented as "advanced / team use"
  - hex's core secrets path (init wizard) does NOT mention Docker
  - The `DockerSecretsAdapter` is lazy-loaded only when `hex secrets serve` is called

### What to discard

- Approach A's model of Docker as default infrastructure: eliminated
- Approach A's modification of `hex serve` to start containers: eliminated
- Approach A's automatic health checks in composition root startup: eliminated
- Approach A's upgrade/rollback machinery: eliminated (users manage their own Infisical)
- Approach B's optional `@infisical/sdk` peer dependency: deferred (not needed now)

## 6. Implementation Roadmap

### Phase 1: Core (Week 1)

Priority: get secrets working with zero new infrastructure.

| Task | Effort | Files |
|------|--------|-------|
| `LocalVaultAdapter` implementing `ISecretsPort` | 1 day | `src/adapters/secondary/local-vault-adapter.ts` |
| `CachingSecretsAdapter` decorator | 0.5 day | `src/adapters/secondary/caching-secrets-adapter.ts` |
| Composition root auto-discovery chain | 0.5 day | `src/composition-root.ts` |
| `hex secrets init` wizard (interactive + non-interactive) | 1 day | `src/adapters/primary/cli-adapter.ts` |
| `hex secrets status` | 0.5 day | CLI adapter |
| `hex secrets list` | 0.5 day | CLI adapter |
| Unit tests for all three adapters | 1 day | `tests/unit/` |

### Phase 2: Agent Integration (Week 2)

| Task | Effort | Files |
|------|--------|-------|
| Agent YAML `secrets:` declaration schema | 0.5 day | Agent YAML schema, validator |
| Runtime secret injection in agent spawn | 1 day | Swarm port, composition root |
| `hex secrets run -- <command>` | 0.5 day | CLI adapter |
| `hex secrets pull <file>` | 0.5 day | CLI adapter |
| `hex secrets set / delete` (local vault) | 0.5 day | CLI adapter |
| OS keychain integration (macOS first) | 1 day | `src/adapters/secondary/keychain-adapter.ts` |
| Integration tests | 1 day | `tests/integration/` |

### Phase 3: Polish (Week 3, optional)

| Task | Effort | Files |
|------|--------|-------|
| `hex secrets serve` (Docker convenience command) | 2 days | New secondary adapter, CLI command |
| Linux keychain support (`libsecret`) | 1 day | Keychain adapter |
| Windows credential manager support | 1 day | Keychain adapter |
| Config inheritance (project -> user -> env) | 0.5 day | Config loader |
| Documentation and examples | 1 day | Docs |

### Phase 4: Future (deferred)

- Optional `@infisical/sdk` peer dependency for dynamic secrets
- Secret rotation triggers
- Team vault sharing (encrypted vault with shared key)

## 7. Final Verdict

**HYBRID — Approach B foundation + Approach A's Docker as opt-in convenience.**

Rationale: hex is an npm package that targets developers who may or may not have Docker. Approach B's zero-infrastructure model (local vault + BYO Infisical + env var fallback) matches hex's existing architecture perfectly. Every new adapter implements `ISecretsPort`, the composition root picks the best available backend, and agents are oblivious to the underlying implementation. This is hexagonal architecture working as intended.

Approach A's embedded Docker stack is a useful convenience for teams who want a turnkey local Infisical, but it must not be the default path. Gating hex's secrets feature on Docker availability would exclude a large portion of hex's target audience and add significant maintenance burden for a feature most users won't need.

The hybrid preserves Approach B's simplicity as the primary experience while offering Approach A's power as an explicit opt-in for advanced users. This is the right ordering of priorities for an npm-distributed developer tool.

**Winner: Approach B (with `hex secrets serve` borrowed from Approach A as Phase 3).**
