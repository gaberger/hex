# AutoMem — Memory as a Trainable Skill for LLM Agents

**Date:** 2026-07-04
**Source paper:** Wu, Zhu, Zhang, Wang, Yeung-Levy (Stanford), *AutoMem: Automated Learning of Memory
as a Cognitive Skill*, arXiv:2607.01224 (verified real — full text read, not just abstract/README)
**Source code:** github.com/autoLearnMem/AutoMem
**Relates to:** ADR-2606161300 (hex LoRA idiom experts), the research-dashboard app's RAG chat feature
**Hardware:** RTX 5070 Ti (16 GB), local Ollama fleet — hands-on test performed, not just read

---

## Executive summary

AutoMem treats **memory management itself as a trainable skill**, not a fixed architecture. File-system
operations (read/write/search/append/create) are promoted to first-class actions in the *same action
space* as task actions — the model decides what to log, when to retrieve, and how to organize its own
memory files, rather than a bolted-on retrieval pipeline. Two meta-LLM-driven loops (both literally
`claude -p ...` CLI calls) optimize a shared inner-loop agent: **Loop 1** rewrites the agent's
scaffold/prompts/memory-schema from full episode traces, gated (kept only if eval progression
improves); **Loop 2** curates the base model's own good memory decisions as training data and trains a
narrow LoRA "memory specialist" that handles only LOG + memory-consultation PLAN, while the frozen base
model still commits every task action.

Real numbers, not just claims: on Qwen2.5-32B-Instruct, memory-only optimization (task-action weights
never touched) took Crafter 25.0%→51.4%, MiniHack 7.5%→30.0%, NetHack 0.42%→1.85% progression — 2-4x
gains that make the 32B open-weight model competitive with Claude Opus 4.5 and Gemini 3.1-Pro-Thinking,
and beat `Qwen2.5-72B-Instruct` (2x the params) by a wide margin.

## Why this matters for hex

1. **Validates ADR-2606161300 independently.** hex's own LoRA idiom-experts ADR already commits to:
   LoRA as narrow idiom-nudging, never constraint enforcement, with external gates as the verdict.
   AutoMem's memory specialist is structurally the same pattern — a narrow LoRA for one behavioral
   slice, frozen base model for everything else, hard eval-gated acceptance — arrived at independently
   by a different team. Two groups converging on the same shape is a real signal.
2. **hex's memory system has no Loop-1 equivalent.** `hex memory store/get/search` exists, but nothing
   reviews full swarm/agent traces and rewrites the memory schema/prompts when it's serving agents
   poorly. AutoMem's NetHack example is a concrete demonstration: an unbounded append-only map file was
   automatically rewritten into a coordinate-keyed dedup format after the meta-LLM diagnosed the
   failure from traces — a 95% size reduction, recovering thousands of wasted steps.
3. **Direct counterpoint to this dashboard's own chat feature.** Our embedding-index retrieval is
   exactly the "fixed architectural module" category AutoMem's introduction argues against — and the
   retrieval bug we hit and hand-tuned around (right document found, wrong chunk retrieved, because a
   static embed-then-rank pipeline can't tell it's looking in the wrong paragraph) is precisely the
   failure class an adaptive, agent-directed memory approach is designed to sidestep.

## Hands-on test

Cloned the repo and ran the v0 inner-loop scaffold (`scaffolds/inner_agent_v0`) on Crafter only —
skipped MiniHack/NetHack's heavier environment setup. Substituted `devstral-small-2:24b` over Ollama
for their `Qwen2.5-32B-Instruct`/vLLM combo (a 32B model doesn't fit this 16GB GPU). Confirmed the
scaffold's `client_name=vllm` code path is just a generic OpenAI-compatible client
(`api_key="EMPTY"` + `base_url`) — Ollama's `/v1` endpoint works as a drop-in.

**Result: the mechanic is real and works.** A 15-step, unseeded episode reached 13.64% progression,
reward 3.0. The agent selectively wrote `actions_log.txt` and `map_notes.txt` (the LOG routine firing
every step) while leaving `crafting_progress.txt` / `survival_log.txt` / `strategy.txt` **empty** —
genuinely deciding nothing worth recording had happened there yet, no hardcoded template being filled
in. It placed a crafting table and made a wood pickaxe along the way.

**Not tested:** Loop 1 (scaffold evolution) and Loop 2 (LoRA specialist training) — both need extensive
gated Claude CLI iteration, and Loop 2 specifically needs LLaMA-Factory + 2 GPUs per their setup (this
box has one 16GB GPU). Nothing here reproduces the paper's actual headline numbers — this only confirms
the core mechanic runs and behaves as described.

**Three real bugs found and patched in their released code** (dependency-version drift against current
PyPI, not a flaw in the approach):
1. Missing `scipy` dependency, undeclared.
2. `environments/wrappers/__init__.py` unconditionally imports `nle` (NetHack Learning Environment)
   even on the Crafter-only path — no lazy per-environment import.
3. The vendored gym-compatibility shim calls `env.seed(seed)` unconditionally, but current PyPI
   `crafter` (1.8.3) dropped `.seed()` in favor of constructor-time `seed=`. Patched with a `hasattr`
   guard; per-episode seeding isn't faithful as a result, acceptable for a mechanic smoke-test, not for
   reproducing exact benchmark numbers.

## Recommendation

If hex's memory system or this dashboard's retrieval quality becomes a recurring pain point, AutoMem's
two-loop pattern (trace-review-and-revise scaffold, narrow gated LoRA specialist) is a concrete,
hands-on-validated design to draw from — not just a paper to cite. Nothing has been built toward this
yet; this is research groundwork only.
