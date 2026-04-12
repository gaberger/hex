# Research Synthesis: Making Local Models Effective for hex

**Date:** 2026-04-11
**Sources:** Three parallel research agents — prompt compression, context optimization, small model code gen quality.

## The Problem

hex dispatches 1500+ word one-shot prompts that work with frontier models (Claude Opus ~95% HumanEval) but fail with local models (qwen2.5-coder:32b at ~85% HumanEval, drops to ~20% on SWE-Bench repository-level tasks). The gap is not in single-function generation — it's in multi-file reasoning, instruction-following density, and architectural context.

## Key Benchmark Reality

| Model | HumanEval+ | SWE-Bench Lite | Notes |
|---|---|---|---|
| Claude Opus 4 | ~95% | ~55%+ | Reference ceiling |
| qwen2.5-coder:32b | ~85% | ~18-22% | Best local for pure codegen |
| devstral-small:24b | ~80% | ~24% (agentic) | Best local for agent loops |
| qwen3:8b | ~65% | <5% | Only viable for T1 |
| qwen3:4b | ~55% | <5% | T1 only, but 100 tok/s |

**The cliff**: single-function tasks are nearly solved at 32B. Multi-file/architectural tasks collapse to 20%. Scaffolding (agent loops, compile gates) can recover ~30-50% relative improvement.

## Tier Mapping for hex (Consensus)

| Tier | Task Type | Model | Strategy | Latency |
|---|---|---|---|---|
| **T1** | Typos, renames, comments, JSON edits | qwen3:4b | One-shot | <5s |
| **T2** | Single function/test, trait impl (1 file) | qwen2.5-coder:32b | Best-of-3 + compile gate | 30-90s |
| **T2.5** | Multi-function, simple mocks, 2-file edits | devstral-small:24b | Agent loop (read-plan-write-compile-fix, max 3 iter) | 2-5min |
| **T3** | Multi-file features, integration tests, architecture | Frontier (Claude) | Full hex pipeline | API-bound |

## Implementation Roadmap (Priority Order)

### Phase 1: Zero-Infrastructure Changes (week 1)

| # | Technique | Impact | Effort | How |
|---|---|---|---|---|
| 1 | **Task-tier model routing** | Saves 50% latency on T1, reserves 32B capacity for T2+ | Low | Extend `inference_router.rs` to read task tier from `CodeGenRequest`, map to model per tier table. hex already classifies T1/T2/T3 in hook.rs. |
| 2 | **KV-cache prefix sharing** | Saves 30-50% prompt compute per request | Low | Structure agent prompts as [shared_prefix + task_suffix]. Ollama caches prefix automatically via `--keep-alive`. All hex agent prompts share ~800 tokens of role/arch rules. |
| 3 | **Quantization-aware prompt restructuring** | Recovers 3-8% accuracy lost to Q4 | Low | Flatten nested instructions, front-load file contents, repeat key constraints at end, use explicit step numbering. Modify agent YAMLs only. |

### Phase 2: Scaffolding (weeks 2-3)

| # | Technique | Impact | Effort | How |
|---|---|---|---|---|
| 4 | **Compile-gate Best-of-N** | Closes ~50% of gap to frontier on T2 tasks (85% → ~95% HumanEval) | Medium | Generate N=3 completions, run `cargo check` on each, return first that compiles. Diminishing returns past N=5. |
| 5 | **Error-feedback retry loop** | +15-25% pass rate on T2 at 32B scale | Medium | On compile failure, feed error back with the code for up to 2 retries. Third retry almost never helps at this scale. |
| 6 | **GBNF grammar constraints** | +15-30% on structured output tasks, eliminates format hallucinations | Medium | Define output grammar via llama.cpp GBNF: { file_path, code_block, commit_msg }. Ollama supports grammar parameter. |

### Phase 3: Context Engineering (weeks 3-4)

| # | Technique | Impact | Effort | How |
|---|---|---|---|---|
| 7 | **RAG for file contents** | Reduces prompt 1500 → 600 tokens; 10-20% quality tradeoff on cross-file, competitive on single-file | Medium | Embed codebase with tree-sitter chunking (hex already has tree-sitter). Replace inlined file contents with top-3 retrieved chunks per task. |
| 8 | **LLMLingua-2 on prose only** | Additional 600 → 400 token compression | Medium | Compress architectural constraints and role descriptions. Preserve code, paths, and commit commands verbatim. Code tokens are load-bearing — never compress them. |
| 9 | **Multi-turn decomposition** | +10-15% accuracy on complex tasks | Medium | Break one-shot 1500-word prompts into: (a) read+summarize files, (b) plan changes, (c) write code, (d) self-check. Each step is within small-model capability. |

### Phase 4: High-Ceiling Investments (month 2+)

| # | Technique | Impact | Effort | How |
|---|---|---|---|---|
| 10 | **LoRA distillation from Opus traces** | Approach frontier quality on hex-specific tasks | High | Collect 200+ (prompt, code) pairs from Opus traces. QLoRA fine-tune qwen2.5-coder:32b. Encodes hex conventions without prompt instructions. |
| 11 | **Cascading with frontier verification** | 60-70% fewer frontier tokens | Medium | Local model generates; hex runs `cargo check` + `hex analyze`. Only escalate to frontier on failure. Already partially supported in agent YAML (`upgrade: { after_iterations: 3, to: opus }`). |
| 12 | **Speculative decoding** | 2-3x inference speedup with identical quality | Low | Use qwen3:4b as draft model for qwen2.5-coder:32b verification. Supported in llama.cpp. |

## What Won't Work

- **Gist tokens / ICAE / AutoCompressors**: Require model fine-tuning, incompatible with GGUF/Ollama stack.
- **Sliding window attention for hex prompts**: hex prompts have scattered cross-file references; sliding window loses them (~10% HumanEval drop).
- **Self-correction at 8B**: Models cannot parse compiler errors correctly. Fix loop becomes random walk. Only viable at 32B+.
- **Best-of-N at 8B**: Even N=20 cannot compensate for fundamental capability gaps. Use 8B for T1 only.

## Compound Effect Estimate

Applying Phases 1-3 together:

| Metric | Before | After (estimated) |
|---|---|---|
| T1 latency | Not routed (uses frontier) | <5s (qwen3:4b) |
| T2 pass rate (single function) | ~85% one-shot | ~93-95% (Best-of-3 + compile gate + retries) |
| T2 prompt size | ~1500 tokens | ~500 tokens (RAG + compression) |
| Frontier API calls | 100% of tasks | ~30% (T3 only + T2 escalations) |
| T2.5 pass rate (multi-function) | ~50% one-shot | ~70% (agent loop + devstral) |

## Recommended First ADR

**ADR: Tiered Inference Routing with Local Model Scaffolding**

Scope: Phases 1-2 (techniques 1-6). Deliverables:
- Extend `inference_router.rs` route_request to accept task tier
- Implement Best-of-N with compile gate in orchestration layer
- Restructure agent YAML prompts for quantization-aware prefix sharing
- Add GBNF grammar support to OllamaInferenceAdapter
- Wire escalation from local failure → frontier

This covers the highest-ROI techniques with the lowest infrastructure cost, all buildable on the standalone dispatch path (ADR-2604112000) just shipped.
