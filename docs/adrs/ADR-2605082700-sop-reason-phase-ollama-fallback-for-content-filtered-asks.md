# ADR-2605082700 — SOP REASON-phase Ollama fallback for content-filtered asks

Status: **Accepted** (shipped 2026-05; commits 33082785 Ollama-fallback on content-filter, 1a481b16 fallback on HTTP 402, 4a0dd52e parse_text_tool_calls in fallback path)
Date: 2026-05-09

## Context

OpenRouter's content-filter policy blocks inference requests containing security-sensitive keywords (secret, credential, auth, leak, bypass, vulnerability, exploit) with HTTP 403 responses indicating "Content filter: redaction would produce invalid tool call arguments." This prevents security-domain personas (CISO, adversarial-red, adversarial-blue) from generating specs, ADRs, or tool calls for legitimate security work—tonight's CISO secret-store audit produced no output due to this block.

The hex project already runs three local Ollama models with full OpenAI-compatible tool-calling support:
- `qwen2.5-coder:32b` (tier 1 reasoning + code)
- `devstral-small-2:24b` (tier 2 structured output)
- `gemma4:latest` (tier 2.5 fallback)

These models have no content filters and support the OpenAI `/v1/chat/completions` API with `tools[]` arrays when the model has a tools template registered in Ollama.

The SOP REASON phase (ADR-2604142300) currently hard-codes OpenRouter as the inference provider in `reason_via_openrouter()`. When a 403 content-filter response occurs, the SOP fails immediately with no retry—wasting the GROUND phase work and forcing the operator to manually rephrase the request.

## Decision

### 1. Automatic Ollama fallback on OpenRouter content-filter errors

When the SOP REASON phase receives HTTP 403 (content-filter) or 5xx (service unavailable) from OpenRouter, **retry the same inference request once** against the local Ollama endpoint using the persona's configured tier model from `.hex/project.json::inference.tier_models`.

**Fallback endpoint:** `http://127.0.0.1:11434/v1/chat/completions` (OpenAI-compatible)  
**Model selection:** Use the persona's tier model (e.g., CISO → executive tier → t1 → `qwen2.5-coder:32b` per tier_models mapping).  
**Tool-calling format:** OpenAI `tools[]` schema (Ollama natively supports this when the model has a tools template).

**Environment controls:**
- `HEX_SOP_OLLAMA_FALLBACK` (default `true`): enable/disable the fallback
- `HEX_SOP_OLLAMA_URL` (default `http://127.0.0.1:11434`): override Ollama base URL for remote deployments

**Trace tagging:** Append `[REASON → 403 content-filter, retry via ollama qwen2.5-coder:32b]` to `phase_trace` so operators can audit when fallback triggered.

### 2. Per-persona preferred_provider to skip wasted round trips

Add a `preferred_provider` field to persona YAML definitions (e.g., `ciso.yml`, `adversarial-red.yml`) with values `ollama` or `openrouter`. When set to `ollama`, the SOP REASON phase **skips the OpenRouter call entirely** and goes directly to the local model.

**Rationale:** Security-domain personas (CISO, adversarial-red) will **always** use security-sensitive language. Routing them to Ollama by default eliminates the wasted OpenRouter round trip + 403 response, reducing REASON phase latency from ~8s (failed OpenRouter + retry) to ~4s (direct Ollama).

**Backward compatibility:** When `preferred_provider` is absent, default to the existing OpenRouter-first behavior.

### 3. Implementation scope

**Files to modify:**
1. `hex-nexus/src/orchestration/sop_executor.rs` — add `reason_via_ollama()` function and retry logic in `reason_with_tools()`
2. `hex-cli/assets/agents/hex/hex/ciso.yml` — add `preferred_provider: ollama`
3. `hex-cli/assets/agents/hex/hex/adversarial-red.yml` (if exists) — add `preferred_provider: ollama`

**Key functions:**
- `reason_via_ollama()`: mirrors `reason_via_openrouter()` but posts to `{HEX_SOP_OLLAMA_URL}/v1/chat/completions` with OpenAI tools[] schema
- `reason_with_tools()`: check persona's `preferred_provider`; if `ollama`, call `reason_via_ollama()` directly; if `openrouter` (or absent), try OpenRouter first, catch 403/5xx, then fallback to Ollama

**Tier model resolution:**
- Executive tier (cto, cpo, ciso) → t1 → `qwen2.5-coder:32b`
- Lead tier → t2 → `gemma4:latest`
- Engineer tier → t2.5 → `gemma4:latest`

**Error handling:**
- If Ollama fallback also fails (e.g., Ollama not running, model not pulled), return the original error with both failure traces appended
- Log Ollama fallback attempts at `info` level with persona, model, and HTTP status

### 4. Rollback plan

If Ollama tool-calling produces malformed outputs or breaks SOP contracts:
1. Set `HEX_SOP_OLLAMA_FALLBACK=false` globally to disable fallback
2. Set `preferred_provider: openrouter` in affected persona YAMLs
3. Revert the code patches to `sop_executor.rs` and persona YAMLs

No STDB schema changes are required—this is pure application logic.

## Consequences

### Positive
- **Unblocks security work:** CISO, adversarial-red, and security engineers can generate specs/ADRs for secret stores, auth bypasses, vulnerability mitigations without rephrasing
- **Reduces REASON latency for security personas:** Direct Ollama routing eliminates the 403 round trip (~4s vs ~8s)
- **Increases SOP resilience:** OpenRouter 5xx outages (rate limits, maintenance) automatically fall back to local inference with no operator intervention
- **Zero new dependencies:** Ollama already required and running per the dev environment setup
- **Audit trail:** `phase_trace` logs every fallback trigger with model + reason

### Negative
- **Model quality variance:** Local Ollama models (especially `gemma4:latest`) produce lower-quality tool calls than Claude Opus 4 on complex security reasoning—expect 10-15% more `escalate_to_operator` calls from CISO when Ollama is used
- **Ollama availability assumption:** If Ollama isn't running or the tier model isn't pulled, the fallback fails silently (logged but not retried)—operators must monitor Ollama health
- **Increased local GPU usage:** REASON phase fallback triggers will spike GPU memory (32GB for qwen2.5-coder:32b) during peak security work
- **Divergent output schemas:** Ollama's OpenAI-compat layer occasionally omits `tool_use_id` in multi-tool responses; the SOP tool executor must tolerate missing IDs gracefully

### Monitoring
- Add `sop.reason.fallback_triggered{persona, reason}` counter to trace how often OpenRouter blocks security work
- Track `sop.reason.provider{provider=ollama|openrouter}` latency histograms to compare performance
- Alert when `sop.reason.ollama_failure_rate > 0.2` (indicates model not pulled or Ollama degraded)

### Migration
1. Deploy the `sop_executor.rs` patch first (adds fallback logic)
2. Set `preferred_provider: ollama` in `ciso.yml` after validating fallback works on one test security ask
3. Roll out to `adversarial-red.yml` and other security personas incrementally
4. Monitor for 48h; if off-contract tool calls spike >20%, revert persona YAMLs to `openrouter` and escalate model fine-tuning