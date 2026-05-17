# ADR-2603250900: Reviewer RL Integration and Structured-Output Reliability

**Status:** Accepted
**Date:** 2026-03-25
**Deciders**: Gary

---

## Context

The `hex dev` pipeline gates tier progression on `ReviewPasses` — the reviewer agent must return a valid JSON verdict (`{"verdict":"PASS"|"NEEDS_FIXES","issues":[...]}`) before the next tier can start. In practice, tiers 0/2/3 consistently hit `max_iterations` without ever clearing `ReviewPasses` because:

1. `select_model_for_role("hex-reviewer")` always calls `select_from_yaml` — the RL Q-table is never consulted for the reviewer role.
2. Free-tier models (e.g. `meta-llama/llama-3.3-70b-instruct:free`) frequently return prose or markdown instead of JSON, producing invalid `.hex-review/*.json` output.
3. Reviewer outcomes are never fed back to the RL engine as reward signals — the system cannot learn that these models are unreliable for structured-output tasks.
4. There is no retry or fallback path when the reviewer produces invalid JSON.

The `extract_model_from_action` bug (tracked in feat-fix-rl-model-routing) compounds this: even if RL were queried, its model recommendations are silently discarded because the provider/model string is mangled from `provider/model` to `openrouter-provider-model`.

---

## Decision

Extend the neural intelligence loop to cover the reviewer role:

### 1. RL model selection for reviewer

The supervisor now passes the YAML-selected model as a **soft preference** (`model_preference`) to `ReviewerAgent::execute_with_preference()`, not as a hard override. Inside the agent, `select_model(TaskType::Reasoning, model_preference, ...)` is called — this allows the Q-table to override the preference when it has learned a better model. Previously, the YAML model was always passed as `model_override`, making `selected.source = UserOverride` and preventing RL from ever learning.

### 2. Reviewer outcome as RL reward

After each reviewer invocation, `report_outcome` is called:
- **Positive reward** — reviewer returned valid JSON with `verdict: "PASS"` or `"NEEDS_FIXES"` (structured output succeeded)
- **Negative reward** — reviewer returned non-JSON or unparseable response (fires per failed attempt and on complete exhaustion)

The reward fires on every attempt, not just on `source=RlEngine`, so the Q-table accumulates signal even before it has enough history to override the YAML preference.

### 3. JSON retry with model upgrade

When the reviewer returns non-JSON (detected by the `is_parse_failure` fingerprint on the `parse_review` fallback), retry up to 2 times:
- Retry 1: same model, append explicit JSON enforcement suffix to prompt
- Retry 2: upgrade to `HEX_REVIEWER_UPGRADE_MODEL` (env var, defaults to `anthropic/claude-haiku-4-5-20251001`) + JSON suffix

### 4. Structured-output fallback verdict

If all 3 attempts fail, emit a synthetic `"PASS"` verdict with `reviewer_skipped: true` in the `.hex-review/review-latest.json` file. The supervisor logs a `warn` and includes the flag in the written JSON so it is visible in pipeline reports.

This avoids the failure mode where a flaky free model permanently blocks tier progression.

---

## Consequences

**Positive**:
- RL engine will learn over time which models reliably produce structured JSON for the reviewer role
- Tier progression is no longer permanently blocked by flaky free-model JSON output
- Pattern store accumulates `reviewer:structured_output_success` patterns, informing future model routing decisions

**Negative**:
- Fallback auto-PASSES verdict means some code may advance without a real review; mitigated by the warning annotation and pattern learning
- Adds reviewer latency on retry path (2 extra calls in worst case)

**Neutral**:
- `extract_model_from_action` bug (mangling `provider/model` to `openrouter-provider-model`) was fixed in the same session as this ADR; both fixes shipped together

---

## Alternatives Considered

- **Always use Claude Sonnet for reviewer**: Solves JSON reliability but abandons the RL learning loop; costs more and doesn't improve the system's self-optimizing behaviour.
- **Structured output API mode (`response_format: json_object`)**: Only available on some providers; too fragile for the multi-provider routing we support.
- **Remove ReviewPasses gate**: Eliminates the blocker but removes architecture quality enforcement — not acceptable.
