# LoRA Knowledge Injection — Findings

**Date:** 2026-06-17
**Evaluates:** ADR-2606161300 (Encoding hex into a model via LoRA — idiom injection vs. knowledge injection)
**Branch:** `feat/hex-lora-idiom-phase01`
**Source paper:** Yue et al., *Decoupled Mixture-of-Experts for Parametric Knowledge Injection* (DMoE), arXiv:2606.14243 (verified real)
**Hardware:** RTX 5070 Ti (16 GB), local Ollama fleet; bases trained in-process with PEFT/transformers.

---

## Executive summary

We set out to bake hex's own conventions into local models via LoRA "cartridges," then followed the evidence wherever it went across nine experiments. The single sentence that captures all of it:

> **A LoRA cartridge is a learned point-lookup table: cue in → specific memorized answer out.**

It is a **knockout** when the task *is* a point lookup (recalling a private API, tool names, named facts), and it **degrades** the moment the task needs **composition** (reasoning) or **bulk access** (enumerate / scan / retrieve-by-relevance). Concretely:

- **Rules / idioms → no.** Injecting hex's coding rules was neutral-to-negative. A rule is a discipline the model already knows but doesn't always follow, not a fact it lacks. Enforcement belongs in the external analyzer, not the weights.
- **Facts → yes, scaled by how *private* the fact is.** Generic facts (HotpotQA) lifted modestly (+53% rel F1). Hex's own API — a *total* knowledge gap — was a knockout: command hallucination 83% → 0%, recall 0.04 → 0.98.
- **Reasoning → mostly no.** Knowledge in weights recalls ≈ as well as knowledge in context, but reasons substantially worse, and the gap *widens* on more capable models.
- **As a knowledge database → only as a key-value point lookup.** Bulk extraction (dump-all, retrieve-relevant) comes back lossy and confabulated. It supports lookup-by-key, not joins, scans, or relevance queries.

**Practical upshot for hex:** use cartridges for fast, private, point-lookup recall (hex API, MCP tool catalogs, dependency/framework APIs). Keep rules in the analyzer and reasoning-grade knowledge in a real store fed into context.

---

## How everything was measured

Every result below uses an **objective gate** and an **explicit control**, because early runs proved that without them you measure your own wishful thinking (a reasoning-model's `<think>` truncation once produced a fake +0.33 "win"). The recurring methodology:

- **Closed-book evaluation.** The knowledge under test is *not* in the prompt; the model must answer from weights. This is the only honest way to test "knowledge in weights."
- **A control condition** — usually open-book (the same facts placed *in context*) — so a number means something relative to a ceiling.
- **A toggle, not two models.** Base vs. base+cartridge is the *same* model with the LoRA enabled/disabled (`disable_adapter()`), so the comparison is clean.
- **An external oracle for grading** — `cargo check`/`hex analyze` for code, JSON-Schema validation for tool calls, exact-match/F1 for QA — never the training loss.

---

## The experiments

### 1. Idiom injection: baking hex's *rules* into a model (the original ADR hypothesis)

**Goal:** make a local model emit hex-idiomatic, boundary-respecting code by default, raising first-draft acceptance through `hex analyze` + the compile gate.

**Base:** qwen3:4b. **Eval:** 8 tasks mixing generic codegen with hex-boundary idioms (no cross-adapter imports, `.js` extensions, ports-only deps, composition-root wiring), graded by a boundary checker.

| Run | Corpus | base → adapter (acceptance) | Note |
|---|---|---|---|
| 1 | doc-chunks, thinking on | 0.00 → 0.33 | **false lift** — reasoning-model `<think>` truncation artifact |
| 2 | doc-chunks, harness fixed | 0.75 → 0.50 | regression |
| 3 | model-augmented, 19 rec | 0.62 → 0.62 | neutral |
| 4 | 32B-distilled prose, 131 rec | 0.88 → 0.62 | regression |
| 5 | analyzer-verified code, 24 rec | 0.75 → 0.75 | neutral |

**Verdict: neutral-to-negative, root-caused.** The corpus built from hex's prose docs taught the model to *talk about hex workflow* ("route through hex-agent") instead of writing code, so harder training made codegen *worse* (run 4). Even with a corpus of analyzer-**verified** boundary-correct code (run 5), the result was neutral — because the base already knows the rules; there's no knowledge gap to fill. Every variant carried an inherent **~20% throughput cost** from runtime LoRA application.

**Lesson:** *rules are a verification problem, not a knowledge problem.* The independent external gate (which hex already has) is the right tool; the cartridge adds nothing.

### 2. DMoE reproduction: does the method work in its *native* domain (facts)?

We verified arXiv:2606.14243 is a real paper, then reproduced its core claim on the paper's own model family and dataset.

**Base:** Qwen2.5-1.5B-Instruct. **Data:** HotpotQA. **Method:** PRAG-augment gold supporting passages → train a knowledge cartridge → measure closed-book EM/F1, with an open-book RAG upper bound.

| Condition | EM | F1 |
|---|---|---|
| Dense (base, closed-book) | 0.100 | 0.116 |
| **DMoE-lite (base+cartridge, closed-book)** | **0.133** | **0.176** |
| RAG upper-bound (base, open-book) | 0.117 | 0.157 |

**Verdict: real but modest (+53% relative F1).** Injected knowledge lifted closed-book QA and even edged out open-book RAG. The lift is modest because HotpotQA facts are *partly* in the base's pretraining (it's public Wikipedia) — the knowledge gap is partial.

### 3. The hex-API knowledge expert: the right hex application (a *total* knowledge gap)

hex's own ~50 CLI verbs / MCP tools are private — never in any model's training data — and local models demonstrably hallucinate them (hence CLAUDE.md's "never recommend commands not in `hex --help`"). We injected the API from `mcp-tools.json`.

**Base:** Qwen2.5-1.5B-Instruct. **Eval:** held-out intent phrasings → command (no leakage), 50 commands.

| | recall | exact | hallucination |
|---|---|---|---|
| Bare base | 0.04 | 0.02 | **0.83** |
| **base + hex-api expert** | **0.98** | **0.96** | **0.00** |

**Verdict: knockout.** The bare model invents fakes (`hex inspect` for `hex analyze`, `hex search` for `hex adr search`) 83% of the time; the cartridge recalls the real command 98% with zero hallucination. The bigger the *private* knowledge gap, the bigger the win.

### 4. MCP tool calls (schema-validated): can this build "better MCP servers"?

`mcp-tools.json` *is* an MCP server registry, so the question generalizes. We extended the test from tool **names** to full tool **calls** (name + JSON args), validated against each tool's JSON Schema as the oracle.

**Base:** Qwen2.5-1.5B-Instruct, 50 tools.

| | valid call (right tool + valid args) | right tool | schema-valid args | hallucination |
|---|---|---|---|---|
| Bare base | 0.00 | 0.00 | 0.00 | **1.00** |
| **base + MCP expert** | **0.60** | **0.94** | 0.60 | **0.02** |

**Verdict: tool *selection* is solved (0→0.94, hallucination 1.00→0.02); argument *generation* is strongly helped but partial (0→0.60).** The production recipe is the same inject-prior/gate-externally pattern: **cartridge for selection + a JSON-Schema validator as a runtime retry gate for arguments.** Constraint: this only applies to *open* models you can attach a LoRA to (local agents), not frontier models.

### 5. Recall vs. reasoning: does injection *reason*, or only *recall*?

The key question behind "teach a model my business." We built a synthetic company of atomic policy facts (non-calendar fiscal year, EOY freeze windows, approval thresholds, hierarchy, named CFO), trained a cartridge on the **facts only** (never the reasoning answers), and tested **recall** (held-out fact phrasings) vs. **reasoning** (scenarios composing 2–3 facts; answers never stated), across Dense / Cartridge / Open-book.

| Qwen2.5-1.5B | base | cartridge (weights) | open-book (context) |
|---|---|---|---|
| Recall | 0.00 | 0.88 | 0.88 |
| Reasoning | 0.20 | 0.50 | 0.60 |

| Qwen2.5-3B | base | cartridge (weights) | open-book (context) |
|---|---|---|---|
| Recall | 0.25 | 0.88 | 1.00 |
| Reasoning | 0.10 | **0.30** | **0.70** |

**Verdict: recall transfers to weights ≈ fully; reasoning transfers *partially*, and the gap *widens* as the base gets more capable.** The weak 1.5B masked this (its reasoning ceiling was low either way); the stronger 3B exposed it (it reasons at 0.70 *with facts in context* but only 0.30 *from weights*). **Mechanism:** in-context facts are working memory the model composes over; in-weight facts are long-term memory good for recall but poor for multi-hop composition.

### 6. The access model: is a cartridge a "knowledge database"?

If the cartridge can't reason, maybe it's a *reference store* you query and reason over externally. We tested the two database-like access patterns.

- **Retrieve-by-relevance** (ask the cartridge to list facts relevant to a scenario): it **confabulates** — invents an "$10k–$75k" threshold, mashes "EOY freeze" and "quarter-close" into a false "every quarter ends in a freeze." Judging relevance is itself reasoning, which it can't do.
- **Dump-all-then-reason** (cartridge regenerates the whole corpus → bare base reasons over the dump): the dump is **lossy and partly fabricated** ("the Controller is Sam Okoro; the Treasurer is Sam Okoro" — both invented). Reasoning over the dump scores **0.20** vs. **0.70** over the true facts.

**Verdict:** only the **direct point query** works (recall 0.62–0.88 ≈ context). In database terms a cartridge supports **lookup by exact key**, but not **joins** (reasoning), **range scans / `SELECT *`** (the dump is lossy), or **query-by-relevance** (faked). It is a *content-addressable associative memory*, closer to human cued recall than to a queryable database.

---

## The unifying law

> **A cartridge is a learned point-lookup table: cue in → specific memorized answer out.**

It explains every result:

- **Knockout** where the task *is* a point lookup — hex API (0.98), tool names (0.94), direct org facts (0.88).
- **Degrades** the moment the task needs **composition** (reasoning/joins: 0.30–0.50, never reaching the in-context ceiling) or **bulk access** (enumerate/scan/retrieve-relevant: confabulates).

And it cleanly separates the two things the ADR was right to distinguish: **knowledge injection** (filling a factual gap — works) vs. **constraint enforcement** (obeying a known rule — doesn't, that's the analyzer's job).

---

## Practical implications for hex

1. **Ship a `hex-api` cartridge for the local tier models.** It ~eliminates command hallucination (the exact thing CLAUDE.md has a hard rule about) and lets you stop injecting the tool list into every prompt. The API is stable, so retrain only on release; the corpus staleness hash already detects when.
2. **Stop pursuing idiom/rule cartridges.** Boundary rules are owned by `hex analyze`; the cartridge adds nothing and costs throughput. (ADR-2606161300's §1 invariant held: enforcement stayed external the whole time.)
3. **Where a cartridge fits (the stability rule):** STABLE private facts (hex API, dependency/framework APIs, slow-moving domain facts) → cartridge. FAST-changing state (project source symbols, git diff, live violations) → keep RAG / context injection.
4. **For MCP tool-calling agents:** cartridge for tool selection + JSON-Schema validator as a runtime retry gate for arguments. Open/local models only.
5. **For "business knowledge" that needs reasoning or completeness:** keep the authoritative facts in a real store and put the relevant ones in context. A cartridge is the fast point-lookup recall layer *in front of* a knowledge base, not a replacement for one.

### Architectural sketch (forward-looking, partially tested)

A two-tier fleet: **frontier models reason** (they hold the world's knowledge and the best reasoning, but their weights aren't yours to write); **local cartridge-models hold your stable private knowledge in weights** and answer point lookups against your systems, locally and fast. The cartridge is the deeply-integrated *recall/translation* layer; the frontier model is the *reasoner*. This separates the **stable** (→ weights) from the **transient** (→ context), instead of cramming both into the context window. hex's existing tiered inference (local T1/T2/T2.5 + frontier T3) is already this topology. **Untested:** the end-to-end frontier↔local loop, and whether a cartridge-as-retriever feeding a *strong* reasoner recovers the in-context ceiling (our self-reasoning hybrid did not — but a stronger downstream reasoner might).

---

## Honest limitations

- **Small eval sets.** Reasoning/recall probes were 8–10 items; magnitudes are noisy. The *directions* (and the gaps vs. the controls) are robust; the exact decimals are not.
- **Small bases.** Tested on 1.5B–4B local models. Effects likely shift on larger bases — notably, the reasoning gap *grew* from 1.5B to 3B, so don't assume it shrinks with scale.
- **Scoped DMoE.** We reproduced the *core claim*, not the full architecture — single merged expert, no BM25 router, no token-uncertainty gate, no per-document experts (those need a custom decode loop Ollama can't provide), and an all-layer FFN rather than strict final-layer attachment.
- **Teacher-generated evals** (PRAG intents/Q&A) may be easier than real user phrasing, though the base failing the same probes makes the contrast valid.
- **~20% throughput cost** for runtime LoRA application is real and did not go away in any run.
- **Did not test the frontier↔local loop** end to end — the architectural vision is reasoned from component results, not demonstrated as a whole.

---

## Artifacts (all on `feat/hex-lora-idiom-phase01`)

| File | Purpose |
|---|---|
| `hex-core/src/corpus.rs`, `hex-nexus/src/corpus_build.rs` | corpus extraction (globs + PRAG augmentation via Ollama) |
| `hex-nexus/src/lora_registry.rs`, `lora_attach.rs`, `lora_eval.rs` | adapter registry, serving-path attachment, bench-gate |
| `scripts/lora/` (`setup.sh`, `train_lora.py`, `run.sh`) | offline QLoRA training toolchain (GGUF export) |
| `scripts/lora/gen_verified_corpus.py` | generate→`hex analyze`→keep-clean verified-synthetic corpus |
| `scripts/lora/dmoe_repro.py` | DMoE reproduction (HotpotQA EM/F1) |
| `scripts/lora/hex_api_expert.py` | hex-API recall + hallucination experiment |
| `scripts/lora/mcp_toolcall_expert.py` | schema-validated MCP tool-call experiment |
| `scripts/lora/org_reasoning_expert.py` | recall-vs-reasoning + access-model (dump/retrieve) experiment |

The reusable keepers beyond this study: the **verified-synthetic corpus generator** (generate-and-certify against an objective gate) and the **two tool-knowledge harnesses** (point-lookup recall + schema-validated calls).
