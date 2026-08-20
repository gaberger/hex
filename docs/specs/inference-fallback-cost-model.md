# Inference Fallback Cost Model

*status*: accepted  ·  *date*: 2026-05-21

Inference Fallback Cost Model

**Status:** Accepted  
**Date:** 2026-05-12  
**Commit:** 9fadbf42  
**Related:** ADR-2026-05-12-0900-cost-and-token-efficiency, wp-cost-and-token-efficiency

## Overview

The hex inference system implements a **tier-based fallback chain** to minimize per-request cost while maintaining availability. When a user sends a message or an agent invokes a tool-calling LLM, hex selects the **lowest-cost healthy provider** via `priority_for_tools` in `hex-nexus/src/routes/chat.rs:779-793`.

This spec describes the observable behavior of that cost model, including tier ranking, fallback logic, and cost tracking for OpenRouter requests.

---

## Behavioral Specification

### 1. `local_ollama_chosen_over_openrouter_when_both_registered`

**Scenario:** Operator registers both a local Ollama instance and an OpenRouter API key.

**Preconditions:**
- `hex inference add ollama http://localhost:11434 qwen2.5-coder:14b`
- `hex secrets set OPENROUTER_API_KEY sk-or-...`
- `hex inference add openrouter https://openrouter.ai/api/v1 anthropic/claude-3.5-sonnet`
- Both endpoints are `healthy` (no recent 5xx or timeout)

**Expected behavior:**
1. When a tool-calling message arrives (chat or agent tool invocation), hex calls `priority_for_tools` for all registered inference endpoints.
2. Ollama gets `provider_tier=0`, OpenRouter gets `provider_tier=3`.
3. Both have `health=0` (healthy), so final priorities are `0` (Ollama) vs `30` (OpenRouter).
4. Hex picks Ollama → **zero per-request cost**.
5. The response does NOT include an `openrouter_cost_usd` field.

**Observable artifact:**  
Check inference logs: `model="qwen2.5-coder:14b"`, no `openrouter_cost_usd` field.

**Implementation reference:**  
`hex-nexus/src/routes/chat.rs:779-793` (`priority_for_tools`)

---

### 2. `openrouter_fallback_logged_with_cost`

**Scenario:** Local Ollama is unavailable (stopped, network unreachable, or 5xx), and hex falls back to OpenRouter.

**Preconditions:**
- Ollama registered but marked `unhealthy` (recent failure) or not running
- OpenRouter registered and `healthy`

**Expected behavior:**
1. `priority_for_tools` ranks Ollama `provider_tier=0` but `health=2` (unhealthy) → priority `2`.
2. OpenRouter `provider_tier=3`, `health=0` → priority `30`.
3. Hex sorts ascending → Ollama still wins numerically, but hex's health-check logic skips unhealthy endpoints before sorting, so OpenRouter is chosen.
4. Request goes to `https://openrouter.ai/api/v1/chat/completions`.
5. Response includes `data["usage"]["cost"]` (e.g. `0.00045` for a 1K token request).
6. Hex logs:
   ```
   tracing::info!(
       openrouter_cost_usd = "0.00045000",
       model = "anthropic/claude-3.5-sonnet",
       "OpenRouter actual cost"
   );
   ```
7. The `openrouter_cost_usd` string is returned in the inference result and persisted via the session port (ADR-042 message.token_usage).

**Observable artifact:**
- `grep "OpenRouter actual cost" hex-nexus.log` shows per-request cost
- Cost watchdog (future P3.1 task in wp-cost-and-token-efficiency) aggregates `openrouter_cost_usd` from session messages

**Implementation reference:**  
`hex-nexus/src/routes/chat.rs:1131-1142` (OpenRouter cost extraction)

---

### 3. `gemini_compat_endpoint_at_tier_2`

**Scenario:** Operator registers a Google Gemini endpoint via the OpenAI-compatible adapter (`generativelanguage.googleapis.com/v1beta/openai`).

**Preconditions:**
- `hex inference add openai-compat https://generativelanguage.googleapis.com/v1beta/openai gemini-2.0-flash-exp`
- Ollama (tier 0) registered
- OpenRouter (tier 3) registered

**Expected behavior:**
1. `priority_for_tools` assigns `provider_tier=2` (openai-compat).
2. Priority ranking (all healthy):
   - Ollama: `0`
   - Gemini (openai-compat): `20`
   - OpenRouter: `30`
3. If Ollama fails, hex tries Gemini next (before OpenRouter).
4. Gemini is **free for personal use** but rate-limited → sits between free local and paid remote.

**Observable artifact:**  
`model="gemini-2.0-flash-exp"` in logs when Ollama is down.

**Implementation reference:**  
`hex-nexus/src/routes/chat.rs:779-793` (openai-compat at tier 2)

---

## Tier Ranking Table

From `hex-nexus/src/routes/chat.rs:770-793`:

| Tier | Provider Family           | Priority Formula      | Cost Profile       |
|------|---------------------------|-----------------------|--------------------|
| 0    | ollama, vllm, llama-cpp   | `0 * 10 + health`     | Free (local)       |
| 1    | anthropic-direct          | (handled separately)  | Paid (Anthropic)   |
| 2    | openai-compat, openai     | `2 * 10 + health`     | Varies (often free)|
| 3    | openrouter                | `3 * 10 + health`     | Paid (usage-based) |
| 5    | unknown                   | `5 * 10 + health`     | Unknown            |

**Health modifier:**
- `healthy` → `+0`
- `unknown` → `+1`
- `unhealthy` → `+2`

**Sort order:** Ascending priority (lower = preferred).

---

## Cost Tracking Flow

```
┌─────────────────┐
│ User message    │
└────────┬────────┘
         │
         v
┌────────────────────────────────────┐
│ priority_for_tools sorts endpoints │
│ (tier * 10 + health)               │
└────────┬───────────────────────────┘
         │
         v
┌────────────────────────────────────┐
│ Pick lowest-priority healthy       │
│ endpoint (e.g. Ollama = 0)         │
└────────┬───────────────────────────┘
         │
         v
┌────────────────────────────────────┐
│ call_inference_endpoint_with_tools │
│ (tools-aware POST)                 │
└────────┬───────────────────────────┘
         │
         ├─ Ollama → no cost field
         ├─ OpenRouter → parse usage.cost
         └─ openai-compat → no cost field
         │
         v
┌────────────────────────────────────┐
│ Return (InferenceResult, tool_calls)│
│ with openrouter_cost_usd string    │
└────────┬───────────────────────────┘
         │
         v
┌────────────────────────────────────┐
│ Persist to session message via     │
│ session_port.message_append        │
│ (token_usage.openrouter_cost_usd)  │
└────────┬───────────────────────────┘
         │
         v
┌────────────────────────────────────┐
│ Cost watchdog rollup (P3.1) reads  │
│ session messages and sums cost     │
└────────────────────────────────────┘
```

---

## References

- **Source:** `hex-nexus/src/routes/chat.rs:779-793` (`priority_for_tools`)
- **Cost extraction:** `hex-nexus/src/routes/chat.rs:1131-1142`
- **ADR:** ADR-2026-05-12-0900-cost-and-token-efficiency.md
- **Workplan:** `docs/workplans/wp-cost-and-token-efficiency.json`
- **Commit:** 9fadbf42 (inference tier ranking)

---

## Success Criteria

1. **Zero-cost default:** When Ollama is registered and healthy, 100% of requests use tier-0 → no OpenRouter spend.
2. **Cost visibility:** Every OpenRouter request logs `openrouter_cost_usd` in structured logs.
3. **Tier insertion:** Adding a tier-2 provider (e.g. Gemini) automatically slots between local (0) and paid remote (3) without code changes.

**Verification command:**
```bash
# Should show tier-0 Ollama chosen when healthy
hex chat "what is 2+2" --debug | grep "selected_endpoint"

# Should show OpenRouter cost when Ollama is down
systemctl stop ollama
hex chat "explain ADR-035" --debug | grep "openrouter_cost_usd"
```

---

**Next:** Cost watchdog rollup (wp-cost-and-token-efficiency P3.1) will aggregate `openrouter_cost_usd` from session messages and surface monthly burn rate on the dashboard.
