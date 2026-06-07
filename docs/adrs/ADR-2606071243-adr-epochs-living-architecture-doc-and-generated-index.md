# ADR-2606071243: ADR epochs, a living ARCHITECTURE.md, and a generated index — keep the architecture story legible as the design pivots

**Status:** Proposed
**Date:** 2026-06-07
**Drivers:** The ADR corpus has grown to 245 records (~37k lines) spanning 2026-03-22 → 2026-07-10 across at least two hard philosophical pivots (foundation → org-sim → single-agent loop). Reading the ledger chronologically now yields a *wrong* mental model of the present system, because the current philosophy contradicts a large fraction of the log. Only ~13 ADRs carry a clean machine-readable `Accepted` status; 114 mention supersession in prose but only 5 carry a `Superseded-By:` backlink — so `hex adr doctor` and the MAPE-K GROUND phase cannot reliably distinguish live decisions from dead ones.
**Relates-To:** ADR-2606061359 (collapse org-sim to single-agent loop — the most recent pivot, and the first ad-hoc "capstone" supersession), ADR-041 (ADR consistency review), ADR-2026-04-27-0800 (`hex adr doctor` self-consistency checker), ADR-2605301228 (`Applies-To`/`governing` retrieval). Supersedes none — this is additive governance.

<!-- LIFECYCLE: Proposed → Accepted → Completed. Change Status only via adr_status_set / the adr-steward. -->

## Context

ADRs are a **decision ledger**: append-only, immutable, valuable precisely because they
record *why* a past decision was made in the context of its time — including the wrong
turns. The self-improvement loop, the GROUND phase, and persona/agent memory all depend
on being able to ask "why did we decide X" and get the honest historical answer.

But a ledger is not a map. As hex pivoted, the ledger accumulated three distinct,
mutually-contradictory design philosophies:

1. **Foundation** (≈2026-03-22 → 2026-04): hexagonal microkernel, SpacetimeDB state
   core, hex-nexus FS bridge, tiered inference, embedded-assets/generic rules.
2. **Org-sim** (≈2026-04 → 2026-06-06): a multi-agent *organization simulation* — C-suite
   personas, the `sched`/brain autonomous spawn daemon, the SOP state machine, HexFlo
   swarm coordination, declarative-swarm YAMLs, and a proposed MAPE-K self-improvement
   loop on top.
3. **Single-agent** (2026-06-06 → present): per ADR-2606061359, the org-sim was collapsed
   to a single gateway-mediated agent loop with code-graph context as the differentiator;
   org-sim telemetry and reducers were purged (commits 9e970c3f, f149369b).

Today there is **no artifact that describes the present system without archaeology**.
`CLAUDE.md` mixes "what hex IS" with hundreds of lines of operational rules, and still
encodes org-sim-era machinery (33 personas, SOP dispatch, the brain daemon) as live
guidance. A fresh LLM or contributor pointed at `docs/adrs/` builds a model that is
~60% obsolete.

**The trap to avoid.** The naive fix — "refine / replace / remove" old ADRs — destroys
the one thing ADRs are for. Editing an ADR body to keep it current is the equivalent of
rebasing published git history: it erases the *why* that the platform's own retrieval
depends on. The correct move is not to mutate the ledger but to **stop asking the ledger
to be the map**, and to keep the ledger *honest* via status transitions rather than edits.

**Forces:**
- Legibility for LLMs/contributors (the immediate pain) vs. fidelity of the historical record.
- Machine-readability (`hex adr doctor`, GROUND retrieval, dashboard) requires clean,
  structured status — which the corpus currently lacks.
- Any hand-maintained index over 245 files rots immediately; it must be generated.
- This is itself an architecture decision about how decisions are governed, so it must
  route through the ADR pipeline, not a one-off doc edit.

**Alternatives considered:**
- *Mass-delete/rewrite obsolete ADRs.* Rejected — loses the "why" the self-improvement
  loop feeds on, and breaks every inbound reference from code/specs/memory.
- *Do nothing; rely on dates.* Rejected — dates don't encode *philosophy*, and the
  corpus already proves chronological reading misleads.
- *One giant rewritten ADR-of-record.* Rejected — that is a living spec wearing an ADR
  costume; conflates the two artifacts again.

## Decision

Adopt a three-layer model that **separates the decision ledger from the current-state
map**, and mechanize the parts that rot.

### 1. ADR Epochs (a new optional frontmatter field)

Introduce `**Epoch:** <name>` as recognized ADR frontmatter. An *epoch* is a named era of
the system's design philosophy. The canonical epochs are defined here and owned by this ADR:

| Epoch | Span | Defining ADRs | One-line identity |
|-------|------|---------------|-------------------|
| `foundation` | 2026-03-22 → 2026-04 | ADR-001 (hex arch), ADR-025 (SpacetimeDB), ADR-039/043 (nexus), inference-tier ADRs | Hexagonal microkernel + STDB state core + FS-bridge daemon |
| `org-sim` | 2026-04 → 2026-06-06 | ADR-027 (HexFlo), ADR-2026-03-24-0130 (declarative swarm), ADR-2026-05-13-1849 (personas), ADR-2026-05-19-0721 (MAPE-K) | Multi-agent organization simulation: personas + SOP + autonomous spawn |
| `single-agent` | 2026-06-06 → present | **ADR-2606061359** (capstone), ADR-2026-05-08-2500 (typed tools), ADR-38fc9e3f-era graph-memory | One gateway-mediated agent loop; code-graph context as the differentiator |

- Epoch membership is assigned by date-of-decision **and** subject; cross-cutting concerns
  (e.g. inference tiers) belong to the epoch in which they were *introduced*.
- Exactly one epoch is `current` at any time. New ADRs default to the current epoch.
- When `hex adr` cannot determine an epoch, it reports `unassigned` (a doctor warning),
  never guesses.

### 2. Capstone ADR per retired epoch (formalize the existing pattern)

When an epoch ends, exactly one **capstone ADR** supersedes the set, enumerating the ADRs
it retires. ADR-2606061359 already did this ad-hoc for org-sim; this ADR makes it the
standing convention. A capstone:
- carries `**Supersedes:**` listing the retired epoch's now-obsolete decisions,
- is the single forward-pointer a reader follows from any stale ADR in that epoch,
- does **not** require editing the retired ADRs' bodies — only their `Status`/`Superseded-By`
  headers (see §3).

### 3. Status-header hygiene (make the ledger machine-honest)

The ledger stays append-only. The only permitted mutations to existing ADRs are to their
**lifecycle headers**, never their Context/Decision/Consequences bodies:
- Every ADR's `**Status:**` MUST be exactly one of the valid statuses (`Proposed`,
  `Accepted`, `Completed`, `Rejected`, `Abandoned`, `Superseded`, `Deprecated`).
- Every `Superseded`/`Deprecated` ADR MUST carry a `**Superseded-By:** ADR-XXXX` backlink.
- These transitions are applied only via `hex adr accept|complete|supersede` /
  `adr_status_set` / the adr-steward — never by free-hand body edits.
- Target: `hex adr doctor` passes clean over the whole corpus.

### 4. Generated index — `hex adr reindex`

Add a `hex adr reindex` subcommand that regenerates **`docs/adrs/INDEX.md`** (a *new*
generated file — `README.md` stays human prose and is never clobbered). The index is
grouped by epoch → status, shows each ADR's id, title, status, and `Superseded-By` link,
and is reproducible from the corpus alone. The dashboard and the GROUND phase consume the
structured form; humans read the rendered table. `hex adr list`/`doctor` become
epoch-aware (filter to `--epoch current` by default in summary views).

### 5. Living `ARCHITECTURE.md` (the map, distinct from the ledger)

Create and maintain a single current-state document, `ARCHITECTURE.md` at the repo root.
It describes the system **as it is today**, with zero archaeology, and links *down* to the
ADRs that justify each component. It is rewritten freely whenever the design shifts — it is
a living spec, not a ledger entry. It is the first artifact an LLM or contributor is pointed
at. `CLAUDE.md` retains operational rules; "what hex IS" moves to / is summarized in
`ARCHITECTURE.md`.

**Invariant (the mental model):** *ADRs are git history for decisions — you do not rebase
published history; you add commits that supersede it. The thing that always describes HEAD
is a separate living doc (`ARCHITECTURE.md`).*

## Consequences

**Positive:**
- An LLM/contributor can read `ARCHITECTURE.md` (current) and, if they need *why*, drop
  into the epoch-filtered ledger — never reconstructing the present from contradictory history.
- `hex adr doctor` gains a real, clean signal; the dashboard and GROUND retrieval can filter
  to live decisions.
- The index never rots (generated); the capstone convention makes the *next* pivot cheap.
- No historical "why" is lost — bodies are never edited, ADRs never deleted.

**Negative:**
- Up-front cost: assign epochs across 245 ADRs and repair ~100+ status headers.
- Two artifacts to keep in sync (ledger + `ARCHITECTURE.md`); drift is possible if
  `ARCHITECTURE.md` is neglected.
- `Epoch:` adds a field the parser and doctor must learn.

**Mitigations:**
- Epoch assignment is largely date-bucketed and can be scripted/assisted; the doctor flags
  `unassigned` so coverage is auditable.
- `ARCHITECTURE.md` drift is bounded by making "update ARCHITECTURE.md" part of any
  epoch-ending capstone ADR's definition of done.
- The `Epoch:` field is optional and backward-compatible; absence degrades to `unassigned`,
  not an error.

## Implementation

| Phase | Description | Status | Verification |
|-------|------------|--------|--------------|
| P1 | This ADR (epoch/index/living-doc governance) | Pending | code:docs/adrs/ADR-2606071243-adr-epochs-living-architecture-doc-and-generated-index.md |
| P2 | Author living `ARCHITECTURE.md` (current single-agent epoch), link down to ADRs | Pending | code:ARCHITECTURE.md |
| P3 | Assign `Epoch:` + repair status/`Superseded-By:` headers so `hex adr doctor` passes | Pending | test:hex adr doctor |
| P4 | Implement `hex adr reindex` → generated `docs/adrs/INDEX.md`; make list/doctor epoch-aware | Pending | code:hex-cli/src/commands/adr.rs, test:cargo build -p hex-cli --release |

## References

- ADR-2606061359 — the org-sim → single-agent pivot (first ad-hoc capstone supersession)
- ADR-2026-04-27-0800 — `hex adr doctor` self-consistency checker
- ADR-041 — ADR consistency review
- ADR-2605301228 — `Applies-To` / `governing` retrieval
- `docs/adrs/TEMPLATE.md`, `docs/adrs/README.md`
