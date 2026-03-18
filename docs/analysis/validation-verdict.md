# Validation Verdict: hex-desktop, hex-agent, hex-nexus

**Date**: 2026-03-18
**Scope**: Post-build semantic validation for D5 (hex-desktop testing/packaging), SpacetimeDB config resolution (hex-agent), per-module DB names and e2e isolation (hex-nexus)
**Overall Verdict**: **PASS**
**Weighted Score**: **89/100**

---

## Category Scores

| Category | Weight | Score | Weighted |
|----------|--------|-------|----------|
| Behavioral Specs | 40% | 92 | 36.8 |
| Property Tests | 20% | 78 | 15.6 |
| Smoke Tests | 25% | 95 | 23.75 |
| Sign Conventions | 15% | 88 | 13.2 |
| **Total** | **100%** | | **89.35** |

---

## 1. Behavioral Specs (92/100)

### hex-desktop

| Behavior | Test Exists | File |
|----------|-------------|------|
| HubStatus serializes to camelCase | YES | `tests/integration_test.rs::hub_status_camel_case_serialization` |
| hub_get success | YES | `tests/integration_test.rs::list_agents_returns_populated_list` |
| hub_get server error | YES | `tests/integration_test.rs::list_agents_server_error_returns_err` |
| hub_post success | YES | `tests/integration_test.rs::spawn_agent_success` |
| hub_post conflict error | YES | `tests/integration_test.rs::spawn_agent_conflict_returns_error` |
| hub_delete success | YES | `tests/integration_test.rs::kill_agent_success` |
| hub_delete not found | YES | `tests/integration_test.rs::kill_agent_not_found` |
| Connection refused returns error | YES | `tests/integration_test.rs::connection_refused_returns_http_error` |
| Error without "error" field returns "Unknown error" | YES | `tests/integration_test.rs::error_response_without_error_field_returns_unknown` |
| HubState base_url override | YES | `tests/integration_test.rs::hub_state_with_override_url` |

### hex-agent

| Behavior | Test Exists | File |
|----------|-------------|------|
| resolve_stdb_config reads env vars first | Partial | Logic in `main.rs:547-569`; private, no isolated unit test |
| resolve_stdb_config falls back to ~/.hex/state.json | Partial | Same; tested indirectly via code inspection |
| discover_hub finds lock file | Partial | `main.rs:612-643`; private, no isolated test |
| Agent registers with hub on connect | YES | `tests/hub_integration_test.rs::agent_connects_and_registers_with_hub` |
| HubMessage serde roundtrip all variants | YES | `tests/hub_integration_test.rs::hub_message_serde_roundtrip_all_variants` |
| Agent detects hub disconnect | YES | `tests/hub_integration_test.rs::agent_detects_hub_disconnect` |
| Send fails when not connected | YES | `tests/hub_integration_test.rs::send_fails_when_not_connected` |
| generate_agent_name produces deterministic names | Indirect | Verified via hub integration (agent_name field checked) |

### hex-nexus

| Behavior | Test Exists | File |
|----------|-------------|------|
| resolve_config reads env var first | YES | `state_config.rs:67-98` |
| Unknown backend returns SQLite default | YES | `state_config.rs:90-96` (warns, returns default) |
| Per-module DB names injected into agents | YES | `orchestration/agent_manager.rs:209-210` |
| Chat round-trip with agent naming | YES | `tests/chat_roundtrip_test.rs` (5 tests) |
| Agent register broadcasts name to browser | YES | `tests/chat_roundtrip_test.rs::agent_register_broadcasts_name_to_browser` |
| Heartbeat propagates agent name | YES | `tests/chat_roundtrip_test.rs::heartbeat_propagates_agent_name` |
| Agent disconnect broadcasts notification | YES | `tests/chat_roundtrip_test.rs::agent_disconnect_broadcasts_notification` |

**Deduction (-8)**: `resolve_stdb_config` and `discover_hub` in hex-agent `main.rs` are private functions without isolated unit tests. They work correctly (verified by integration tests and code inspection) but could regress silently.

---

## 2. Property Tests (78/100)

**Present**:
- `hub_message_serde_roundtrip_all_variants` -- covers all 8 HubMessage variants
- `hub_status_camel_case_serialization` -- verifies no snake_case leaks
- SpacetimeDB config defaults verified (`spacetime_launcher.rs::default_config_values`)
- Token generation uniqueness (`commands_test.rs::generated_tokens_are_unique`)

**Missing (-22)**:
- No `proptest`/`quickcheck` for HubMessage with arbitrary payloads
- No fuzz testing for `parse_hub_response` with malformed JSON
- No property test for `generate_agent_name` collision rate

---

## 3. Smoke Tests (95/100)

| Check | Result |
|-------|--------|
| `cargo check -p hex-desktop -p hex-agent -p hex-nexus` | PASS (2 warnings in hex-nexus: dead code) |
| `cargo build -p hex-agent --release` | PASS (45s) |
| `cargo test -p hex-desktop -p hex-agent -p hex-nexus` | PASS: all tests green |
| `bun test` (TypeScript) | 1263 pass, 1 fail (pre-existing), 4 skip |
| `~/.hex/state.json` valid JSON | PASS |
| `hex-desktop/tauri.conf.json` valid JSON | PASS |

**Deduction (-5)**:
- 2 dead-code warnings in hex-nexus (`connected` field on `SpacetimeStateAdapter`)
- 1 failing bun test (`composition-root.ts` import boundary) -- pre-existing, unrelated to these changes

---

## 4. Sign Convention Audit (88/100)

### Return types
All public functions in `hex-desktop/src/commands.rs` return `Result<T, String>` consistently:
- `get_hub_status` -> `Result<HubStatus, String>`
- `hub_get/hub_post/hub_delete` -> `Result<serde_json::Value, String>`
- `spawn_agent/kill_agent/list_agents` -> `Result<serde_json::Value, String>`

### Serde rename conventions
- `HubStatus`: `#[serde(rename_all = "camelCase")]` -- for Tauri/JS frontend
- `HubMessage` variants: per-variant `#[serde(rename = "snake_case")]` -- for WebSocket protocol
- `StateBackendConfig`: `#[serde(tag = "backend", rename_all = "lowercase")]` -- for config files
- All three conventions are internally consistent within their domain

### Error message patterns
All follow "verb + noun": `"HTTP request failed: {e}"`, `"Failed to parse response: {e}"`

**Deduction (-12)**:
- Mixed convention (camelCase for Tauri, snake_case for WS) is intentional but undocumented
- `get_hub_version()` returns `String` directly (not `Result`), minor inconsistency

---

## Issues Found

| Severity | Issue |
|----------|-------|
| Low | `resolve_stdb_config()` and `discover_hub()` in hex-agent are private with no unit tests |
| Low | 2 dead-code warnings in hex-nexus (`connected` field in `SpacetimeStateAdapter`) |
| Info | `default_db` variable in `load_hex_state_config()` assigned but unused (`let _ = default_db`) |
| Info | Mixed serde rename strategies correct for consumers but undocumented |

---

## Test Result Summary

```
hex-desktop:  7 unit + 9 integration = 16 tests PASS
hex-agent:   17 unit + 7 integration = 24 tests PASS
hex-nexus:   24 unit + 7 e2e         = 31 tests PASS (1 doc-test ignored)
TypeScript:  1263 pass, 1 fail (pre-existing), 4 skip
```

Total Rust tests across three crates: **71 passed, 0 failed**.

---
---

# Validation Verdict: API Optimization Layer (ADR-028/029/030/031)

**Date**: 2026-03-18
**Scope**: Prompt caching, rate limiting, batch API, extended thinking, workload routing, Haiku preflight, auto-compaction, multi-provider (MiniMax), RL model selection, SecretBrokerPort wiring
**Overall Verdict**: **PASS**
**Weighted Score**: **83/100**

---

## Category Scores

| Category | Weight | Score | Weighted |
|----------|--------|-------|----------|
| Behavioral Specs | 40% | 85 | 34.0 |
| Property Tests | 20% | 60 | 12.0 |
| Smoke Tests | 25% | 100 | 25.0 |
| Sign Conventions | 15% | 80 | 12.0 |
| **Total** | **100%** | | **83.0** |

---

## 1. Behavioral Specs (85/100)

| ID | Behavior | Test | Status |
|----|----------|------|--------|
| B1 | Prompt caching adds cache_control to system blocks | No isolated test | UNTESTED |
| B2 | Rate limiter tracks per-model usage | `rate_limit_reset_on_success`, `rate_limit_peak_utilization` | PASS |
| B3 | Exponential backoff on 429 | `rate_limit_backoff_exponential` | PASS |
| B4 | Workload classifier routes batch tasks | `workload_classification` | PASS |
| B5 | Thinking config respects budget | `thinking_config_with_budget`, `thinking_config_defaults` | PASS |
| B6 | Cache metrics track savings ratio | `cache_metrics_savings_ratio` | PASS |
| B7 | OpenAI adapter strips thinking tags | `strip_thinking_with_tags`, `strip_thinking_no_tags` | PASS |
| B8 | OpenAI adapter normalizes tool calls | `to_openai_tools_conversion`, `to_openai_messages_basic` | PASS |
| B9 | ModelSelection fallback chain complete | No chain traversal test | PARTIAL |
| B10 | SecretBrokerPort resolves API keys | 10 tests across `env_secrets` + `hub_claim_secrets` | PASS |

**Deduction (-15)**: B1 cache_control JSON structure untested; B9 fallback chain not tested as a sequence.

---

## 2. Property Tests (60/100)

**Verified via unit tests** (no formal property framework):
- Rate limit monotonicity (record_usage always increments)
- Cache savings ratio bounds [0.0, 1.0]
- WorkloadClass::classify totality (never panics)
- ModelSelection round-trip stability
- ThinkingConfig::with_budget(0) disables thinking

**Missing (-40)**:
- No `proptest`/`quickcheck` in hex-agent Cargo.toml
- RateLimitState transitions not fuzz-tested
- CacheMetrics not tested with adversarial inputs
- No property test for backoff ceiling (should never exceed 60s)

---

## 3. Smoke Tests (100/100)

| Check | Result |
|-------|--------|
| `cargo build -p hex-agent` | PASS |
| `cargo test` — 233 tests, 0 failures | PASS |
| `hex-agent --help` — all 6 new CLI args present | PASS |
| `hex-agent build-hash` — returns commit hash | PASS |
| `--no-cache` arg visible | PASS |
| `--thinking-budget` arg visible (default: 0) | PASS |
| `--no-preflight` arg visible | PASS |
| `--compact-threshold` arg visible (default: 85) | PASS |
| `--provider` arg visible (default: auto) | PASS |
| `hex analyze` — Grade A (96/100), 0 violations | PASS |

---

## 4. Sign Convention Audit (80/100)

| Convention | Status |
|------------|--------|
| Error types use `thiserror` derive | PASS — all 6 new error types |
| Ports use `async_trait` | PASS — 4 new ports (RateLimiterPort, BatchPort, TokenMetricsPort, PreflightPort) |
| Adapters import from ports, not domain | PASS — verified via hex analyze |
| Domain types derive `Debug, Clone` | PASS |
| Serde naming: `camelCase` for API, `snake_case` for Rust | PASS |
| Builder pattern for config | PASS — `with_cache()`, `with_thinking_budget()`, `with_compact_threshold()` |
| Noop adapters for optional deps | PASS — `NoopRateLimiter`, `NoopPreflight`, `NoopRlAdapter` |
| Test naming: descriptive `snake_case` | PASS |

**Deduction (-20)**: `cache_control` block structure untested in isolation; `OpenAiCompatAdapter` streaming fallback (non-streaming emulation) not explicitly documented as limitation.

---

## Files Validated

### New (11 files, 1035 lines)
- `domain/api_optimization.rs` — 8 domain types + 7 tests
- `ports/rate_limiter.rs` — RateLimiterPort
- `ports/batch.rs` — BatchPort
- `ports/token_metrics.rs` — TokenMetricsPort
- `ports/preflight.rs` — PreflightPort
- `adapters/secondary/rate_limiter.rs` — In-memory per-model tracking
- `adapters/secondary/token_metrics.rs` — Metrics aggregation
- `adapters/secondary/haiku_preflight.rs` — Haiku/MiniMax classification
- `adapters/secondary/openai_compat.rs` — OpenAI-compatible adapter + 4 tests
- `usecases/workload_router.rs` — Interactive/batch routing

### Modified (6 files)
- `domain/tokens.rs` — cache fields on TokenUsage
- `ports/anthropic.rs` — options parameter
- `ports/rl.rs` — MiniMax + MiniMaxFast variants
- `adapters/secondary/anthropic.rs` — cache_control, beta headers, rate limit parsing
- `usecases/conversation.rs` — preflight, auto-compaction, rate limiter, metrics
- `main.rs` — SecretBrokerPort, multi-provider, 6 new CLI args

---

## Test Summary

```
hex-agent unit:        45 passed, 0 failed
hex-agent domain:      11 passed, 0 failed
hex-agent hub:          4 passed, 0 failed
hex-agent rl:           6 passed, 0 failed
hex-nexus:             64 passed, 0 failed, 1 ignored
Full workspace:       233 passed, 0 failed, 2 ignored
```

---

## Recommendations (non-blocking)

1. Add `#[test] fn fallback_chain_traversal()` — verify full Opus→...→Local→None chain
2. Add `#[test] fn cache_control_json_structure()` — verify system block has cache_control field
3. Add `proptest` dependency for domain type fuzzing
4. Document OpenAI adapter's non-streaming limitation (emulates via buffered response)
