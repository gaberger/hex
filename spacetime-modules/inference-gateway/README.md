# inference-gateway

> Routes ALL LLM inference through SpacetimeDB (ADR-035 / ADR-2026-04-05-0900 P2).

Agents — including sandboxed Docker agents — write a request via `request_inference`. SpacetimeDB schedules the `execute_inference` procedure (immediate tick) which makes the outbound HTTP call to the LLM API directly. The response lands in `inference_response` and the agent picks it up via subscription. hex-nexus is only on the fallback path (calls `complete_inference` if the in-WASM HTTP procedure is unavailable).

## Supported provider types

| Type | Protocol |
|---|---|
| `anthropic` | Anthropic Messages API (`POST /v1/messages`) |
| `openai_compat` | OpenAI Chat Completions |
| `openrouter` | OpenAI-compat + cost tracking |
| `ollama` | Local Ollama (OpenAI-compat) |
| `vllm` | vLLM (OpenAI-compat) |

## Tables

| Table | Visibility | Key | Purpose |
|---|---|---|---|
| `inference_request` | public | `request_id` (auto_inc) | Agent-written request — provider, model, messages_json, max_tokens, status |
| `inference_response` | public | `response_id` (auto_inc) | Final response — content_json, tokens, latency_ms, cost_usd |
| `inference_provider` | public | `provider_id` (PK) | Registered LLM endpoint — base_url, models_json, rate-limit counters, health, `quality_score` |
| `agent_budget` | public | `agent_id` (PK) | Per-agent token + USD budget enforcement |
| `inference_stream_chunk` | public | `chunk_id` (auto_inc) | Streaming chunks (`text_delta`, `tool_use_start`, `input_json_delta`, `message_stop`) |
| `inference_api_key` | **private** | `provider_id` (PK) | Resolved API key — populated only by hex-nexus via `set_api_key` |
| `inference_execute_schedule` | scheduled | `scheduled_id` (auto_inc) | One-shot row that triggers `execute_inference` via `ScheduleAt::Interval(0)` |

## Reducers

### Request lifecycle

| Reducer | Args | Effect |
|---|---|---|
| `request_inference` | `agent_id, provider, model, messages_json, tools_json, max_tokens, temperature, thinking_budget, cache_control, priority` | Insert request (status=`queued`), schedule `execute_inference` immediately |
| `complete_inference` | `request_id, content_json, model_used, input/output/cache tokens, latency_ms, cost_usd, openrouter_cost_usd, created_at` | Mark request `completed`, append `inference_response`, debit `agent_budget` |
| `fail_inference` | `request_id, status, error_message, latency_ms` | Mark `failed`/`rate_limited`/`budget_exceeded` + audit response |
| `cancel_inference` | `request_id, agent_id` | Cancel a `queued` request (ownership-checked) |
| `append_stream_chunk` | `request_id, agent_id, chunk_type, content, sequence` | Append a streaming chunk |

### Provider management

| Reducer | Args | Effect |
|---|---|---|
| `register_provider` | `provider_id, provider_type, base_url, api_key_ref, models_json, rate_limit_rpm, rate_limit_tpm, quantization_level, context_window, quality_score` | Insert/upsert provider |
| `remove_provider` | `provider_id` | Delete provider |
| `update_provider_health` | `provider_id, healthy, last_health_check, avg_latency_ms` | Update health state |
| `set_api_key` | `provider_id, api_key` | hex-nexus only — write resolved key into private table |

### Budget + rate-limit

| Reducer | Args | Effect |
|---|---|---|
| `set_agent_budget` | `agent_id, total_budget_tokens, total_budget_usd, max_single_request_tokens` | Initialize/update budget |
| `reset_rate_counters` | — | Reset per-provider RPM/TPM windows (called periodically) |

### Procedures

| Procedure | Args | Effect |
|---|---|---|
| `execute_inference` | `request_id` (via schedule) | Reads provider + key, makes HTTP POST, calls `complete_inference` or `fail_inference` |

## Helpers (lib API)

`validate_provider_type`, `validate_chunk_type`, `validate_request_status`, `validate_response_status`, `validate_priority`, `is_within_budget`, `ema_latency`, `serde_json_escape`, `add_cost_usd`.

## Subscriptions

```sql
-- Sandboxed agent — receive its own responses
SELECT * FROM inference_response WHERE agent_id = ?
SELECT * FROM inference_stream_chunk WHERE agent_id = ? AND request_id = ? ORDER BY sequence

-- Dashboard / dispatcher
SELECT * FROM inference_provider WHERE healthy = 1
SELECT * FROM inference_request WHERE status = 'queued' ORDER BY priority DESC, created_at ASC
SELECT * FROM agent_budget
```

`inference_api_key` is **private** — never subscribe.

## Status values

- Request: `queued` · `processing` · `completed` · `failed`
- Response: `completed` · `failed` · `rate_limited` · `budget_exceeded`
- Stream chunk: `text_delta` · `tool_use_start` · `input_json_delta` · `message_stop`
- Priority (u8): `0=low` · `1=normal` · `2=high` · `3=critical`

## Notes

- `temperature`, `cost_usd`, `openrouter_cost_usd` are stored as strings to avoid float-precision drift.
- `cache_control` is `0/1` (SpacetimeDB bool workaround).
- `quantization_level` follows ADR-2026-03-27-1000: `q2`/`q3`/`q4`/`q8`/`fp16`/`cloud`.
