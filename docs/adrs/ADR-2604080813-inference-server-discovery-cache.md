# ADR-2604080813: Inference Server Discovery Cache

**Status:** Accepted
**Date:** 2026-04-08
**Drivers:** Gary Berger

## Context

When hex-nexus starts, it connects to SpacetimeDB and loads inference endpoints from
the `inference_provider` table. However, this means that every time a new machine is
set up (e.g., after an `install.sh` run) or nexus restarts on a fresh SpacetimeDB
instance, all inference providers must be re-discovered and re-registered manually
via `hex inference add` or `hex inference discover`.

On Bazzite (AMD Ryzen AI MAX+, 128GB unified RAM), we have 6 code-focused Ollama
models registered. Every reinstall loses this configuration. The same problem exists
for OpenRouter endpoints with API keys, vLLM servers, and any custom Ollama instances
on LAN hosts.

Additionally, the existing `hex inference discover` LAN scan finds Ollama servers but
doesn't persist findings anywhere — they must be manually added.

**Decision type:** `add`

## Decision

Persist the full set of registered inference endpoints to `~/.hex/inference-servers.json`
whenever any endpoint is added, removed, or calibrated. On nexus startup, read this
file and pre-register any endpoints not already in SpacetimeDB.

### Cache File Location

```
~/.hex/inference-servers.json
```

### Cache File Format

```json
{
  "version": 1,
  "updated_at": "2026-04-08T08:13:00Z",
  "endpoints": [
    {
      "id": "bazzite-qwen2-5-coder-32b",
      "provider": "ollama",
      "url": "http://bazzite:11434",
      "model": "qwen2.5-coder:32b",
      "models_json": "[\"qwen2.5-coder:32b\"]",
      "quantization_level": "q4",
      "rate_limit_rpm": 60,
      "context_window": 131072,
      "quality_score": 0.85,
      "requires_auth": false
    }
  ]
}
```

### Write Triggers

The cache is written (full overwrite) after any of:
- `hex inference add` — new endpoint registered
- `hex inference remove` — endpoint removed
- `hex inference test` / calibration PATCH — quality_score updated

The write is performed by `NexusClient` in `hex-cli` after a successful nexus API call,
reading back the full endpoint list and persisting it.

### Startup Preload (hex-nexus)

During nexus startup (after SpacetimeDB connection is established), if
`~/.hex/inference-servers.json` exists:

1. Read the cache file
2. Fetch the current SpacetimeDB endpoint list
3. For each cached endpoint NOT already in SpacetimeDB (by `id`): call
   `register_provider` reducer to re-register it
4. Log how many endpoints were preloaded

This runs in `config_sync.rs` alongside the existing ADR-044 config sync.

### Source of Truth

SpacetimeDB remains the source of truth at runtime. The cache file is:
- A **bootstrap mechanism** for new installs and nexus restarts
- A **backup** for when SpacetimeDB is unavailable
- NOT used for real-time routing (the live SpacetimeDB table is used for that)

### API Key Handling

Endpoints with `requires_auth: true` store only the secret key **reference name**
(e.g., `"OPENROUTER_API_KEY"`) in the cache, never the actual key value. The key
is resolved from the vault at registration time.

## Impact Analysis

### Affected Files

| File | Change | Impact |
|------|--------|--------|
| `hex-cli/src/commands/inference.rs` | Write cache after add/remove/calibrate | LOW — additive |
| `hex-nexus/src/config_sync.rs` | Preload endpoints on startup | LOW — additive |
| `hex-nexus/src/routes/secrets.rs` | Expose endpoint list for cache write | NONE — already exists |
| `~/.hex/inference-servers.json` | New file created on first `hex inference add` | N/A |

### Consumer Dependency Map

No existing code is modified in a breaking way. All changes are additive:
- `inference.rs` gains a `write_inference_cache()` call after existing API calls
- `config_sync.rs` gains a `preload_inference_cache()` call in the startup sequence

### Build Verification Gates

| Gate | Command |
|------|---------|
| Workspace compile | `cargo check --workspace` |
| Unit tests | `cargo test -p hex-cli -p hex-nexus` |

## Consequences

**Positive:**
- Zero-configuration re-registration after reinstall or SpacetimeDB reset
- LAN Ollama servers discovered once and cached permanently
- Consistent inference state across nexus restarts

**Negative:**
- Cache file can become stale if servers move (different IPs/ports)
- Stale endpoints will fail calibration silently at startup (logged, not fatal)

**Mitigations:**
- Startup preload logs warnings for endpoints that fail connectivity check
- `hex inference prune` (future) can remove stale/unreachable entries
- Cache is always a supplement to SpacetimeDB, never a replacement

## Implementation

| Phase | Description | Validation Gate |
|-------|-------------|-----------------|
| P0 | Add `write_inference_cache()` to `hex-cli/src/commands/inference.rs` — called after add/remove/calibrate | `cargo check -p hex-cli` |
| P1 | Add `preload_inference_cache()` to `hex-nexus/src/config_sync.rs` — reads cache, registers missing endpoints on startup | `cargo check -p hex-nexus` |
| P2 | Integration test: register endpoint → check cache file written → restart nexus → verify endpoint restored | `cargo test --workspace` |

## References

- ADR-044: Config sync (repo files → SpacetimeDB on startup) — same pattern
- ADR-2604052125: Provider templates — templates should also be persisted via this mechanism
- `hex-nexus/src/config_sync.rs` — existing startup sync hook
- `hex-cli/src/commands/inference.rs` — write trigger location
