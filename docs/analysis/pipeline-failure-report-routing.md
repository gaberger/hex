# Pipeline Failure Report: Model Routing Investigation

**Date:** 2026-03-25
**Swarm:** investigate-dev-pipeline-failures (6a4f27aa)
**Task:** Model routing — wrong model used for code generation, RL fallback chain analysis

---

## 1. What Model Is Actually Used for Code Generation

### Expected (from YAML)

`hex-coder.yml` declares:
```yaml
model:
  tier: 2
  preferred: sonnet
  fallback: haiku
  upgrade_to: opus
```

This should resolve to `claude-sonnet-4-6` for normal iterations and `claude-opus-4-6` after 3 failed iterations.

### Actual (from `agent_def.rs`)

**Critical bug:** `ModelConfig::resolve_model_id` ignores the model name entirely:

```rust
pub fn resolve_model_id(_name: &str) -> &'static str {
    "openai/gpt-4o-mini"
}
```

The parameter is `_name` (discarded). Every YAML model name — `sonnet`, `haiku`, `opus` — resolves to `openai/gpt-4o-mini`. This means:

- `preferred_model_id()` → `"openai/gpt-4o-mini"` (not `claude-sonnet-4-6`)
- `fallback_model_id()` → `"openai/gpt-4o-mini"` (not `claude-haiku-4-5-20251001`)
- `upgrade_model_id()` → `Some("openai/gpt-4o-mini")` (not `claude-opus-4-6`)

The test in `agent_def.rs` at line 764 asserts:
```rust
assert_eq!(def.model.preferred_model_id(), "claude-sonnet-4-6");
```
This test **must be failing** (or was written before `resolve_model_id` was broken). The implementation and its own unit test are contradictory.

### What Actually Runs

For code generation, `supervisor.rs` calls `select_model_for_role("hex-coder", 0)` → `select_from_yaml()` → `preferred_model_id()` → `"openai/gpt-4o-mini"`.

**All code generation runs on `openai/gpt-4o-mini` (GPT-4o Mini), regardless of YAML configuration.**

---

## 2. Is RL Q-Learning Routing Wired Up or Bypassed?

### The RL Path

`code_phase.rs` calls `selector.select_model(TaskType::CodeGeneration, model_override, provider_pref)`. When no override and no provider preference, it calls `query_rl_engine()` via `POST /api/rl/action`.

However, `supervisor.rs` does NOT use `select_model()` (the async RL path) for the coder. Instead it uses `select_model_for_role()` → `select_from_yaml()` which is **synchronous and bypasses the RL engine entirely**.

The flow in `supervisor.rs` (lines 1442-1444):
```rust
let yaml_selected = self.select_model_for_role("hex-coder", 0);
let yaml_model_id = yaml_selected.model_id.clone();
let effective_model: Option<&str> = model_override.or(Some(&yaml_model_id));
```

This passes `yaml_model_id` as a hard `model_override` into `CodePhase::execute_step_for_phase()`. Inside `execute_step_for_phase`, when `model_override` is `Some`, the code takes the fast-path that **skips the RL query entirely**:

```rust
// In model_selection.rs select_model():
if let Some(model) = model_override {
    return Ok(SelectedModel { source: SelectionSource::UserOverride, ... });
}
```

**RL Q-learning is completely bypassed for code generation.** The YAML-derived model is always treated as a hard override.

### RL Outcome Reporting Gap

`report_outcome()` only posts to `/api/rl/reward` when `selected.state_key.is_some()` — which only happens for `SelectionSource::RlEngine` selections. Since all code generation selections have `source: YamlDefinition` or `UserOverride`, **no reward signals are ever sent for code generation**. The Q-table receives zero training data from the coder.

The reviewer agent (`reviewer.rs`) does call `report_outcome()` — but it uses `TaskType::Reasoning`, not `TaskType::CodeGeneration`. The code phase has no `report_outcome` call at all (confirmed: no matches in `code_phase.rs`).

---

## 3. Free-Tier Fallback Chain

The fallback chain is defined in `model_selection.rs`:

```rust
pub fn fallback_chain_for(task_type: TaskType) -> Vec<&'static str> {
    vec![
        default_model_for(task_type),        // "openai/gpt-4o-mini"
        "google/gemma-2-9b-it:free",         // Fallback 1
        "qwen/qwen-2.5-7b-instruct:free",    // Fallback 2
    ]
}
```

All defaults are `openai/gpt-4o-mini` regardless of task type. This chain is used by the TUI (`tui/mod.rs`) for 402 retry, but the free-tier models in the chain are:

- `openai/gpt-4o-mini` — adequate for simple tasks, limited code quality
- `google/gemma-2-9b-it:free` — 9B parameter open model, weak at complex code
- `qwen/qwen-2.5-7b-instruct:free` — 7B parameter, acceptable for boilerplate

**Assessment:** The free-tier chain does NOT produce adequate code for multi-file TypeScript/Rust projects with hexagonal architecture constraints. GPT-4o Mini struggles with complex type systems and port interfaces. The 7-9B open models are substantially worse. None of these models reliably generate code that passes `hex analyze` boundary checks.

Note: The workplan `feat-fix-rl-model-routing.json` step-2 references stale defaults (`google/gemini-2.0-flash-001`, `meta-llama/llama-3.3-70b-instruct:free`) that no longer match the current code — the defaults were changed to `openai/gpt-4o-mini` across all task types.

---

## 4. Root Cause Hypothesis

### Primary Root Cause: `resolve_model_id` is a stub

`agent_def.rs` line 166-168: `resolve_model_id` ignores its argument and always returns `openai/gpt-4o-mini`. This is almost certainly an incomplete implementation — the function signature has `_name` (the underscore prefix means "intentionally unused" in Rust). The body was never filled in, or was wiped and replaced with a placeholder.

**Effect:** Every agent in the pipeline runs on GPT-4o Mini regardless of what the YAML specifies. The hex-coder gets GPT-4o Mini instead of claude-sonnet-4-6. For complex hexagonal architecture code generation (port interfaces, dependency injection, TypeScript with NodeNext resolution), GPT-4o Mini reliably produces code that fails compile and lint gates, driving the feedback loop to max iterations without resolution.

### Secondary Root Cause: RL bypassed via YAML override path

Even if `resolve_model_id` were fixed to return `claude-sonnet-4-6`, the RL engine would still never run for code generation because YAML model selection always produces `source: YamlDefinition` which passes as a hard `model_override` into `select_model()`, bypassing the RL query. The Q-table can never learn which models work best for code generation because no reward signals are ever sent.

### Tertiary Root Cause: Free-tier chain inadequate for task complexity

Even if both above were fixed, the fallback chain (GPT-4o Mini → Gemma 9B → Qwen 7B) is insufficient for the complexity of code being generated (hexagonal adapters, TypeScript with strict types, Rust with lifetimes). If paid credits are exhausted and the system falls back to free models, it will always produce non-compiling code.

---

## 5. Summary Table

| Issue | Location | Impact |
|-------|----------|--------|
| `resolve_model_id` ignores all arguments, returns gpt-4o-mini | `agent_def.rs:166` | **Critical** — all agents use wrong model |
| YAML model passed as hard override, bypassing RL entirely | `supervisor.rs:1444` | High — RL never runs for code gen |
| No `report_outcome` call in code_phase | `code_phase.rs` | Medium — Q-table starved of training data |
| Free-tier fallback chain too weak for complex code gen | `model_selection.rs:104` | High — 402 fallback always fails |
| Unit test assertions in `agent_def.rs` contradict implementation | `agent_def.rs:764` | Medium — test should be catching the stub bug |

---

## 6. Fix Priority

1. **Fix `resolve_model_id`** — implement the name→ID mapping (sonnet→claude-sonnet-4-6, haiku→claude-haiku-4-5-20251001, opus→claude-opus-4-6). This is the single highest-leverage fix.
2. **Fix YAML override path** — pass YAML model as a soft preference (not hard override) so RL can still recommend, consistent with how reviewer.rs handles it (`execute_with_preference`).
3. **Add `report_outcome` to code_phase** — close the RL feedback loop for code generation.
4. **Upgrade free-tier chain** — replace 7-9B models with `deepseek/deepseek-r1:free` or `meta-llama/llama-3.3-70b-instruct:free` which are significantly stronger at code.
