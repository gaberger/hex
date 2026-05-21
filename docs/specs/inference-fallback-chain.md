# Inference Fallback Chain (3 Provider Families)

*status*: proposed  ·  *date*: 2026-05-21

Inference Fallback Chain (3 Provider Families)

**Status:** Proposed  
**Owner:** hex-nexus inference subsystem  
**Last Updated:** 2025-01-20

---

## Overview

This specification defines the behavioral contract for hex's inference fallback chain across three provider families: **Ollama**, **OpenRouter**, and **OpenAI-compatible**. Each provider family has distinct endpoint shapes, authentication patterns, and tool-calling response formats.

The inference subsystem MUST correctly route requests to the appropriate provider based on configuration hints (`provider=` field or URL patterns) and handle tool-calling semantics per family.

---

## Provider Families

### 1. Ollama

**Behavioral Spec ID:** `ollama_endpoint_used_for_tools`

**Recognition:**
- `provider='ollama'` in config, OR
- URL contains `localhost:11434` or recognizable Ollama endpoint pattern

**Request Shape:**
```http
POST {url}/api/chat
Content-Type: application/json

{
  "model": "qwen2.5-coder:7b",
  "messages": [...],
  "tools": [
    {
      "type": "function",
      "function": {
        "name": "repo_grep",
        "description": "Search repository...",
        "parameters": { ... }
      }
    }
  ],
  "stream": false
}
```

**Response Shape:**
```json
{
  "model": "qwen2.5-coder:7b",
  "created_at": "2025-01-20T...",
  "message": {
    "role": "assistant",
    "content": "",
    "tool_calls": [
      {
        "function": {
          "name": "repo_grep",
          "arguments": "{\"pattern\":\"foo\"}"
        }
      }
    ]
  },
  "done": true
}
```

**Key Properties:**
- No `/v1` prefix in path
- `tools` array at top level of request body
- `message.tool_calls` in response (NOT `choices[0].message.tool_calls`)
- No authentication header required for local installs

---

### 2. OpenRouter

**Behavioral Spec ID:** `openrouter_endpoint_used_for_tools`

**Recognition:**
- `provider='openrouter'` in config, OR
- URL contains `openrouter.ai`

**Request Shape:**
```http
POST https://openrouter.ai/api/v1/chat/completions
Content-Type: application/json
Authorization: Bearer sk-or-v1-...

{
  "model": "anthropic/claude-3.5-sonnet",
  "messages": [...],
  "tools": [
    {
      "type": "function",
      "function": {
        "name": "repo_grep",
        "description": "Search repository...",
        "parameters": { ... }
      }
    }
  ]
}
```

**Response Shape (OpenAI-compatible):**
```json
{
  "id": "gen-...",
  "model": "anthropic/claude-3.5-sonnet",
  "choices": [
    {
      "message": {
        "role": "assistant",
        "content": null,
        "tool_calls": [
          {
            "id": "call_...",
            "type": "function",
            "function": {
              "name": "repo_grep",
              "arguments": "{\"pattern\":\"foo\"}"
            }
          }
        ]
      },
      "finish_reason": "tool_calls"
    }
  ]
}
```

**Key Properties:**
- OpenAI-compatible `/chat/completions` endpoint
- **Requires** `Authorization: Bearer` header
- `choices[0].message.tool_calls` response shape
- Supports unified model namespace (e.g., `anthropic/`, `openai/`, `google/`)

---

### 3. OpenAI-Compatible

**Behavioral Spec ID:** `openai_compat_endpoint_used_for_tools`

**Recognition:**
- `provider='openai-compat'` OR `provider='openai'` in config
- Base URL recognition: endpoint may include `/v1`, `/openai`, or `/v1beta/openai` suffixes

**Request Shape:**
```http
POST {base}/chat/completions
Content-Type: application/json
Authorization: Bearer <optional-api-key>

{
  "model": "gpt-4",
  "messages": [...],
  "tools": [
    {
      "type": "function",
      "function": {
        "name": "repo_grep",
        "description": "Search repository...",
        "parameters": { ... }
      }
    }
  ]
}
```

**Base URL Suffix Rules:**
- If `base` ends with `/v1` → `POST {base}/chat/completions`
- If `base` ends with `/openai` → `POST {base}/v1/chat/completions`
- If `base` ends with `/v1beta/openai` → `POST {base}/v1/chat/completions`
- Otherwise → `POST {base}/v1/chat/completions`

**Response Shape:**
```json
{
  "id": "chatcmpl-...",
  "model": "gpt-4",
  "choices": [
    {
      "message": {
        "role": "assistant",
        "content": null,
        "tool_calls": [
          {
            "id": "call_...",
            "type": "function",
            "function": {
              "name": "repo_grep",
              "arguments": "{\"pattern\":\"foo\"}"
            }
          }
        ]
      },
      "finish_reason": "tool_calls"
    }
  ]
}
```

**Key Properties:**
- OpenAI `/chat/completions` API standard
- Optional authentication (depends on provider/local setup)
- `choices[0].message.tool_calls` response shape
- Covers: OpenAI API, local vLLM, Azure OpenAI Service, LiteLLM, etc.

---

## Fallback Chain Logic

1. **Provider Selection:**
   - If `provider` field is set → use that family's logic
   - Else if URL contains `openrouter.ai` → OpenRouter
   - Else if URL contains `localhost:11434` or `/api/chat` → Ollama
   - Else → OpenAI-compatible (default)

2. **Tool-Call Parsing:**
   - Ollama: extract from `response.message.tool_calls`
   - OpenRouter / OpenAI-compat: extract from `response.choices[0].message.tool_calls`

3. **Error Handling:**
   - 401/403 → log auth failure, try next provider in chain (if configured)
   - 404 on `/chat/completions` → retry with alternate base suffix
   - 5xx / timeout → log transient failure, fallback to next provider

---

## Success Criteria

- [ ] Unit tests cover all three provider request/response shapes
- [ ] Integration test validates Ollama local endpoint with tool calls
- [ ] Integration test validates OpenRouter with mock bearer token
- [ ] Integration test validates OpenAI-compat with `/v1`, `/openai`, `/v1beta/openai` suffix variants
- [ ] `hex-nexus/src/inference/` contains provider-specific adapters implementing this spec
- [ ] Dashboard "Inference Config" UI allows operator to set `provider=` and base URL per model

---

## Implementation Files

- `hex-nexus/src/inference/providers/ollama.rs`
- `hex-nexus/src/inference/providers/openrouter.rs`
- `hex-nexus/src/inference/providers/openai_compat.rs`
- `hex-nexus/src/inference/fallback.rs` (chain orchestration)
- `hex-nexus/tests/inference_providers_test.rs` (integration suite)

---

## References

- ADR-2026-05-08-2500 (Typed Tool Library and SOP Execution)
- Ollama API docs: https://github.com/ollama/ollama/blob/main/docs/api.md
- OpenRouter API docs: https://openrouter.ai/docs
- OpenAI API reference: https://platform.openai.com/docs/api-reference/chat

---

**End of Spec**