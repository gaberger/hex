# ADR-2026-05-22-1710 — codegen-tier-local-ollama

Status: **Accepted**
Date: 2026-05-22

## Context

For ~24 hours leading up to 2026-05-22, **every autonomous codegen task across 4 active swarms failed within 10 seconds** with one of:

- `HTTP 500: provider or API key not configured`
- `HTTP 402: insufficient credits, can only afford 396 tokens, requested 1024`

Tally:

| Swarm | Failed | Completed |
|---|---|---|
| `brain-lease` | 57 | 1 |
| `wp-sop-pipeline-redesign-phase-1` (across 4 dispatches before this session's fixes) | 8 | 0 |

Path B (Claude Code subprocess) is gated on Anthropic credentials and worked when invoked. Path C (headless inference) routed through `inference_gateway` → OpenRouter → `openai/gpt-4o-mini` (the hex-coder agent's `model.preferred`). The `model.fallback` was `claude-haiku-4-5` via OpenRouter — same provider, same credit pool, same failure.

The `inference-bridge` and `hexflo-coordination` WASM modules don't know whether OpenRouter has credit. They forward the request, receive the 500/402, and return failure to the executor. The executor's heartbeat poller then waits its full `timeout_secs` (240s on T2, 420s on T2.5) for a "task completed" signal that never comes — surfacing as a `timed out after Ns` error when the underlying failure was instant.

### Why a model swap is the right shape of fix

- **Topping up OpenRouter** restores the loop but leaves a credit-exhaustion failure mode latent. Next time credits run out, the loop dies again silently.
- **Switching to direct Anthropic API** preserves the credit dependency at a different vendor.
- **Routing codegen to local Ollama** eliminates the credit dependency entirely for the hot path. The hardware on this fleet runs an iGPU + 128 GB RAM Strix Halo; Ollama already serves `qwen2.5-coder:14b` at 4.4s for trivial prompts and ~60s for production-sized codegen tasks (measured 2026-05-22 against P1.1 of `wp-verify-loop-2026-05-22.json`).
- The May 2026-05-13 bench (memory `project_t2_5_bench_results`) found `qwen2.5-coder:14b` ties `qwen2.5-coder:32b` quality at 2× speed — already the bench-pinned T2/T2.5 model in `.hex/project.json`.

## Decision

For all **T2 / T2.5 codegen and persona-reply paths**, route `model.preferred` and `model.fallback` to local Ollama models. T3 frontier escalation (`model.upgrade_to`) still routes via Anthropic API for genuinely hard tasks.

### Specifically:

**Agent YAMLs** (`hex-cli/assets/agents/hex/hex/*.yml`):

| Agent | Before | After |
|---|---|---|
| `hex-coder` | preferred=gpt-4o-mini, fallback=haiku | preferred=qwen2.5-coder:14b, fallback=qwen2.5-coder:14b |
| `hex-documenter` | preferred=qwen-coder, fallback=gpt-4o-mini | preferred=qwen2.5-coder:14b, fallback=qwen2.5-coder:14b |
| `hex-ux` | preferred=qwen-coder, fallback=gpt-4o-mini | preferred=qwen2.5-coder:14b, fallback=qwen2.5-coder:14b |
| `cli-designer` | preferred=sonnet, fallback=gpt-4o-mini | preferred=sonnet (kept), fallback=qwen2.5-coder:14b |
| `adversarial-blue` | preferred=gpt-4o, fallback=gpt-4o-mini | preferred=devstral-small-2:24b, fallback=qwen2.5-coder:14b (`provider_lock: openai_or_local` preserved — still NOT Anthropic; that is red's stack) |

**Source-code defaults** (`hex-nexus/src/orchestration/org_responder.rs`):

| Constant | Before | After |
|---|---|---|
| `REPLY_MODEL_CHAT_DEFAULT` | `openai/gpt-4o-mini` | `qwen2.5-coder:14b` |
| `REPLY_MODEL_COMMIT_DEFAULT` | `openai/gpt-4o-mini` | `qwen2.5-coder:14b` |
| `THOUGHT_SUMMARIZER_MODEL_DEFAULT` | `openai/gpt-4o-mini` | `nemotron-mini` (per the 2026-05-13 bench commit `7671f7d3` that had regressed) |

**Intentionally not changed:**

- `hex-nexus/src/routes/inference.rs:430-438` — `openrouter/free` pseudo-model resolution. This path routes via OpenRouter on explicit user request and needs an OR-namespaced slug. Not a silent default.
- `hex-cli/src/commands/chat.rs:344` — model-name translation table (`"gpt-4o-mini" => "openai/gpt-4o-mini"`). Enables the alias when someone explicitly requests it.

### Original rationale for gpt-4o-mini, addressed

`org_responder.rs` comments cited Anthropic's safety filter as the reason for preferring gpt-4o-mini over haiku — Anthropic blocks "security audit"/"OWASP"/"exploit" terms common to CTO/CISO personas. **Local Ollama has no hosted-safety-filter**, so `qwen2.5-coder:14b` is strictly better than either OpenAI or Anthropic cloud for that use case while eliminating the credit dependency.

## Consequences

- **Autonomous loop is credit-independent.** Routine codegen and persona replies don't consume API budget. Verified end-to-end with `wp-verify-loop-2026-05-22.json` — 76s workplan completion via Ollama only.
- **T3 escalation still uses cloud.** Hard tasks (`model.upgrade_condition` met, OR explicit `tier: T3` workplan tasks) still route to Anthropic API. The Claude API key remains the only paid credential the loop strictly needs.
- **Provider divergence for adversarial reviewers preserved.** `adversarial-blue` (`provider_lock: openai_or_local`) now uses Ollama, which is non-Anthropic — the divergence-from-red contract holds without OpenRouter.
- **Cold-start latency.** First Ollama task per model takes 5–15s for model load; subsequent calls hit warm cache. The default `timeout_secs=600` (per ADR-2026-05-22-1720) absorbs cold loads.
- **Quality.** `qwen2.5-coder:14b` ties 32B-class output for codegen + small-doc tasks (memory `project_t2_5_bench_results`). Adversarial review uses `devstral-small-2:24b` which is heavier but Ollama-served.

## Verification

- `wp-verify-loop-2026-05-22.json` — single trivial task, completed 76s via Ollama, file landed, gate passed, evidence reconciled.
- `cargo test -p hex-cli --lib parse_hex_coder_yaml` — updated assertion confirms binary parses the new model defaults correctly.
- Source greps (`grep -rln 'preferred:.*gpt-4o-mini' hex-cli/assets/` and same for `fallback:`) return zero matches.
- Nexus binary `strings | grep -c qwen2.5-coder:14b` → 14 occurrences (was 0 before this session).

## References

- Commit `c2ab4a3a` (P1 — hex-coder.yml swap)
- Commit `17f1b154` (P1b — remaining 4 YAMLs + org_responder constants)
- Bench memory: `project_t2_5_bench_results` (2026-05-13 — qwen2.5-coder:14b ties 32B)
- Thought summarizer pin: commit `7671f7d3` (2026-05-13 — nemotron-mini)
- Companion ADR: ADR-2026-05-22-1720-glibc-arena-cap.md
- Companion ADR: ADR-2026-05-22-1700-workplan-executor-skip-completed.md
