# ADR-2605301228: ADR governance via decision-cards + hybrid retrieval injected at SOP GROUND

**Status:** Accepted
**Date:** 2026-05-30
**Applies-To:** ADR governance, docs/adrs, hex adr, decision retrieval, sop_executor.rs GROUND, conflict gate
**Superseded-By:** none
**Drivers:** On 2026-05-30 a change nearly reversed an accepted ADR (2026-05-22-1710) silently, because the relevant decision was never surfaced. `hex adr search` is keyword-only and returned nothing for obvious terms ("inference routing tier model", "SOP reason openrouter"). `hex adr doctor` shows live decay (duplicate `ADR-2026` IDs, an unparseable status, a dangling dependency). As the ADR corpus grows, decisions must be retrievable and *injected* into agent context before action — not just authored and forgotten.
**Supersedes:** none

## Context

ADRs are authored and stored, but nothing makes them *binding at decision time*:

- **Retrieval is weak.** `hex adr search` is substring/keyword only (`tui/skills.rs::search_adrs`); it missed obviously-relevant ADRs this session. There is no semantic/embedding layer for ADRs in `hex-nexus`/`hex-core`.
- **No injection.** The SOP GROUND phase (`sop_executor.rs`) lets personas *emit* `adr_draft`, but never *retrieves governing ADRs* into the ground pack. Agents (and Claude) can `code_patch` an area an Accepted ADR governs without that ADR entering context.
- **No conflict gate.** Nothing checks a proposed change against decisions that constrain it.
- **Hygiene rot is unmonitored in-loop.** `hex adr doctor` can detect duplicate IDs / bad statuses / dangling deps, but it isn't part of the authoring loop.

Available substrate: AgentDB (HNSW vector search — see the agentdb skills), SpacetimeDB for state, and the inference port (which can route an embeddings model, e.g. `nomic-embed-text` on Ollama or via inference-gateway).

Key constraint: **for governance, pure semantic RAG is insufficient.** Embeddings give recall but can silently miss the one binding rule; "don't violate this decision" cannot rely on similarity alone. A hybrid (structured + semantic + supersession-aware) is required.

## Decision

1. **Every ADR carries a structured Decision Card** in frontmatter — the unit that is embedded *and* injected:
   ```yaml
   id: ADR-YYMMDDHHMM
   status: Accepted
   decision: "<1-3 sentences>"
   constraints: ["<binding rule>", ...]
   applies_to: ["<code area / tag / glob>", ...]
   supersedes: []
   superseded_by: null
   ```
   Cards are small and high-signal so injecting 3-5 keeps context tiny.

2. **Hybrid retrieval, not pure RAG:**
   - **Deterministic backbone:** map touched files / intent → `applies_to` → pull governing ADRs exactly.
   - **Semantic booster:** embed the task/diff → top-K cards by cosine (AgentDB/STDB).
   - **Supersession filter:** drop any card with `superseded_by != null` — non-negotiable, prevents surfacing stale decisions.

3. **Inject governing ADRs at SOP GROUND.** Before any `code_patch` / `adr_draft` / `workplan_emit`, the ground pack includes "Governing ADRs (binding): [cards]". Same surface for `auto_repair` dispatch and for Claude.

4. **Conflict gate.** When a change touches an Accepted ADR's `applies_to`, require either compliance or an explicit `supersedes` link — automating the manual catch from 2026-05-30.

5. **Hygiene in-loop.** `hex adr doctor` runs in the authoring/CI loop; duplicate IDs, unparseable statuses, and dangling dependencies block until resolved.

## Consequences

- **Drift is structurally prevented**, not relied on memory. The 2026-05-30 near-miss becomes an automatic gate.
- **Retrieval is trustworthy for governance** because the deterministic `applies_to` index is the backbone; embeddings only add recall.
- **Stale decisions don't resurface** (supersession graph + status filter).
- **Cost is low.** Re-embed only on ADR create/status-change (hundreds of ADRs, ms each); cards are tiny.
- **Authoring cost.** Each ADR now needs a Decision Card; backfill is incremental (start with Accepted ADRs touching hot areas).
- **Pairs with ADR-2605301224:** model-selection policy changes (e.g. local-Ollama vs Tenstorrent hot path) are exactly the kind of decision this surfaces at GROUND, so the loosely-coupled inference resolver is governed in-loop.

## Implementation

1. Define the Decision Card frontmatter schema; extend `hex adr schema`/`doctor` to validate it; backfill the ~dozen hot-area Accepted ADRs first (incl. 2026-05-22-1710, 2605301224).
2. **Ship the deterministic slice first** (cheapest, would have prevented today's drift): `applies_to` index + conflict gate at `code_patch`/`adr_draft`.
3. Add the embedding layer: embed cards via the inference port → AgentDB/STDB; replace `adr search` keyword match with hybrid retrieval.
4. Inject "Governing ADRs (binding)" into the SOP GROUND pack + auto_repair dispatch.
5. **Turn the detector into a gate (prevents the 2026-05-30 decay class).** The decay found this session (3 date-only IDs collapsing to `ADR-2026`, prose in a Status field, a `Depends-on` to a never-written ADR) all crept in because `hex adr doctor` runs *out of loop* and the ID parser *enumerated formats with a swallow-everything fallback* instead of enforcing one. Fixes:
   - Run `hex adr doctor --strict` in `hex ci`, pre-commit, and the `adr_draft` SOP validation phase — block on bad ID format / non-enum status / missing `Depends-on` target. (Machinery already exists: `--strict`, tier-aware `--fix`, sched-daemon sharing.)
   - Enforce ONE canonical ID and **generate** the filename on creation (no hand-named files); reject non-canonical rather than coerce.
   - No silent ID fallback — unknown format is flagged malformed, never coerced to `ADR-2026` (parser hardened 2026-05-30, commit pending).
   - Validate at write time: Status ∈ vocabulary (prose → `**Note:**`), `Depends-on` targets exist or are marked `(planned)`.
   - Wire doctor `--fix` (Tier-A) into the sched daemon for auto-heal.

## References

- Companion: ADR-2605301224 (dynamic loosely-coupled inference selection)
- Findings: `hex adr search` keyword-only (`tui/skills.rs`), no embedding layer in nexus/core, SOP GROUND lacks ADR retrieval, `hex adr doctor` decay output (duplicate IDs / unparseable status / dangling dep)
- Substrate: AgentDB (HNSW vector search), SpacetimeDB, inference port (embeddings model)
