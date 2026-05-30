# ADR-2605301224: Dynamic, loosely-coupled inference model selection

**Status:** Proposed
**Date:** 2026-05-30
**Applies-To:** inference routing, model selection, tier_models, sop_executor.rs, drafter.rs, org_responder.rs, inference_strategy_builder.rs, IInferenceRouterPort
**Superseded-By:** none
**Drivers:** Adding the Tenstorrent provider on 2026-05-30 required source edits in 5+ files and exposed that model/provider selection is hardcoded and tightly coupled at the consumption layer — new models cannot be added as configuration. New models are introduced continuously; the architecture must absorb them without code changes.
**Supersedes:** none (reframes ADR-2026-05-22-1710 — see Consequences)

## Context

hex is built on Ports & Adapters, and the inference seam already has the right bones:

- **Ports exist.** `IInferencePort` (`hex-core/src/ports/inference/`) and `IInferenceRouterPort` (`hex-nexus/src/ports/inference_router.rs`) define the hexagonal interface, with a `MockInferencePort` for tests.
- **The adapter registry is already dynamic.** Providers are registered at runtime into the SpacetimeDB `inference_provider` table via `hex config inference add` (`spacetime_inference.rs`). The nexus inference router (`routes/inference.rs::inference_complete`) selects a provider by matching the requested model against the live registry, resolves vault keys, and falls back across providers. Adding a provider is already a no-code operation.

**The coupling violation is at the consumption layer.** The orchestration code that actually drives agents bypasses the port:

- **28 hardcoded model-name string literals** in `hex-nexus/src/orchestration/*.rs` (e.g. `qwen2.5-coder:14b`, `nemotron-mini`, `anthropic/claude-sonnet-4.5`, `gpt-4o-mini`).
- **2 files post directly to provider URLs**, bypassing the port entirely: `sop_executor.rs` (`reason_via_openrouter` → `openrouter.ai`, `reason_via_anthropic` → `api.anthropic.com`, `reason_via_ollama_fallback` → `:11434`) and `inference_strategy_builder.rs`.
- Model defaults are scattered across source constants (`org_responder.rs` `REPLY_MODEL_*`, `drafter.rs` `DRAFTER_MODEL_*`, `tier_config.rs` defaults) and per-path env vars (`HEX_SOP_REASON_MODEL`, `HEX_SOP_REASON_OR_MODEL`, `HEX_SOP_OLLAMA_MODEL`), none of which consult the live registry.

Consequences observed this session:
- Adding Tenstorrent to the *tier* path worked via config, but the *persona/SOP* hot path never used it — it is wired to its own hardcoded providers.
- A proposed change to route personas at Tenstorrent nearly **silently reversed an accepted ADR** (2026-05-22-1710), because model policy is embedded in code rather than a governed, declarative surface.

Alternatives considered: (a) keep editing source per model — rejected, doesn't scale, causes drift; (b) one more env var per model — rejected, same coupling; (c) route all consumers through the existing port with declarative selection — chosen.

## Decision

1. **All inference consumers MUST call through the inference port.** No orchestration code may post directly to a provider URL or embed a provider-specific HTTP shape. `sop_executor`, `drafter`, `org_responder`, `auto_repair` dispatch, `workplan_executor`, and `inference_strategy_builder` route every reasoning/codegen/reply call through `IInferenceRouterPort` (the nexus router already does provider selection, tools, vault-key resolution, and fallback).

2. **Consumers request a ROLE or TIER, never a hardcoded model string.** The call site says "T2.5 codegen for role=hex-coder," not `"qwen2.5-coder:14b"`. A declarative resolver maps role/tier → model at runtime.

3. **Model selection is declarative config, resolved against the live registry.** The role/tier → model mapping lives in `.hex/project.json` (`inference.tier_models`) and the agent YAMLs — these are the *policy surface*. Adding a new model is: `hex config inference add …` (register the adapter, already dynamic) + optionally one mapping line. **Zero code changes.**

4. **Adapters are rewireable at runtime.** Provider registration and the role/tier → model mapping are hot-reloadable without a nexus restart (closes the restart-coupling gap surfaced this session).

5. **Hardcoded model literals and direct provider URLs are removed from `orchestration/`.** Any remaining default is a single declarative fallback resolved through the port, not an inline string.

## Consequences

- **New models are config, not code.** Tenstorrent today, the next model tomorrow — register + map, no rebuild.
- **Model policy becomes governable.** ADR-2026-05-22-1710 ("T2/T2.5 codegen → local Ollama for credit-independence") is **reframed, not superseded**: local-Ollama-for-hot-path becomes a *declarative policy value*, changeable (e.g., to Tenstorrent, with Ollama as the outage fallback) via config + a governing ADR — never by editing source. This dissolves the Tenstorrent-vs-2026-05-22-1710 tension into a policy decision.
- **Less drift.** Centralizing selection behind the port makes "which model does role X use?" answerable and ADR-checkable in one place (pairs with the proposed ADR-governance/decision-card retrieval so policy changes are surfaced at SOP GROUND).
- **Cost/availability trade-off is explicit.** Routing the hot path to cloud reintroduces an availability dependency; the declarative fallback (local Ollama) must remain wired for outages — enforced by the resolver, not by scattered fallback code.
- **Migration risk.** The SOP executor is core; rerouting its reasoning through the port must preserve the multi-round tool-call loop and the content-filter/credit fallbacks. Phased rollout behind a flag.

## Implementation

1. Inventory: the 28 model literals + the 2 direct-post files (this ADR's grounding).
2. Extend `IInferenceRouterPort` (or a thin `resolve(role, tier) -> model` resolver) backed by the live registry + declarative mapping.
3. Route `sop_executor::reason_with_tools` through the port (gated by a flag for safe rollout), preserving the tool-loop and fallback semantics; then `drafter`, `org_responder`, `inference_strategy_builder`.
4. Replace hardcoded constants/env defaults with resolver lookups; keep one declarative fallback.
5. Make the resolver + registry hot-reloadable (no restart).
6. Add the ADR-governance conflict gate so model-policy changes are checked against governing ADRs at `code_patch`/`adr_draft` time.

## References

- Reframes: ADR-2026-05-22-1710 (codegen-tier-local-ollama)
- Related: ADR-2026-04-12-0202 (tiered-inference-routing), ADR-2026-05-17-2030 (sop-pipeline-redesign)
- Companion (proposed): ADR-governance via decision-cards + hybrid retrieval injected at SOP GROUND
- Code: `hex-core/src/ports/inference/`, `hex-nexus/src/ports/inference_router.rs`, `routes/inference.rs::inference_complete`, `orchestration/{sop_executor,drafter,org_responder,inference_strategy_builder}.rs`
- Session commits enabling Tenstorrent as the first dynamic cloud provider: 333c32ca, eaae6268, ce629902, 9594a517, 5231ca03, d7b450b9, 95c11aa0
