# ADR-2606072243: Milestone — the single-agent loop works; open the `hybrid-inference` epoch

**Status:** Accepted
**Date:** 2026-06-07
**Epoch:** hybrid-inference
**Drivers:** A release milestone. This session took the single-agent loop from *"designed but
didn't actually run on local models"* to *"works, benchmark-validated, with a frontier fallback."*
That's a large enough operational shift to mark a new epoch and make a clean start — without
deleting any history (ADRs are an append-only ledger).
**Supersedes:**
**Superseded-By:**

## Context

The `single-agent` epoch (opened 2026-06-06, ADR-2606061359) established the *thesis*: one strong
agent loop fed by code-graph context + ranked lessons, not a simulated org. This session
established that the loop **actually works** — and uncovered that it never had, locally, because of
three stacked gateway bugs:

1. `--model` was ignored on the tools path → every run silently used the registered default.
2. The provider-walk fell through to a cloud model → "local" passes weren't local.
3. Ollama 400'd on the loop's multi-turn tool messages → no local model could complete a run.

With those fixed (`c73a04d8`, `b6964195`), the local agentic loop ran for the first time — and a
purpose-built benchmark (`hex bench agentic`) showed **no single local model dominates**, with
anti-correlated blind spots. That motivated evidence-gated **best-of-N across complementary models**
and a **`claude -p` frontier fallback**, both shipped and validated live. The loop is no longer a
thesis; it is a working, hybrid (local-first → frontier-recover), self-deploying system.

The `single-agent` *thesis is unchanged* — this epoch is its maturation, not a repudiation.

## Decision

Open the **`hybrid-inference`** epoch as of this milestone. What it is:

- **The loop works on local hardware**, evidence-gated, worktree-isolated (the three gateway bugs
  are fixed; ARCHITECTURE.md is current).
- **Inference is hybrid and evidence-selected**: `hex do` runs an ordered candidate list and commits
  the first to pass the gate — local models first (free/fast), `claude -p` frontier fallback when
  they fail. No classifier; the evidence gate is the arbiter. (ADR-2606072044.)
- **Model choice is benchmark-driven, not leaderboard-driven**: `hex bench agentic` + the
  `docs/benchmarks/` corpus measure per-model pass-rates through the *real* loop. (ADR-2606071734.)
- **The platform deploys itself**: `hex dev deploy` (ADR-2606071702).

**Ledger hygiene for the fresh start** (governance-safe — nothing deleted):
- This session's *implemented* ADRs are **Completed**: ADR-2606071702 (deploy), ADR-2606071734
  (benchmark), ADR-2606072044 (best-of-N).
- Carried forward as **Accepted, unbuilt** into this epoch's backlog: ADR-2606071651 (loop-guard
  edit-nudge — would convert `no_edit`/`max_steps` losses to passes), ADR-2606071713 (code-graph as
  a first-class harness tool).
- Prior-epoch ADRs remain in the ledger as history; `hex adr reindex` groups them by epoch. The
  in-nexus `adr-steward` advances Accepted→Completed only where implementation is *confirmed* — it
  advanced 0 of 110 this run, and we did **not** force-complete unverified decisions (that would be
  theater). Their `Accepted` status is honest: decided + in force, not yet confirmed complete.

## Consequences

**Positive**
- A clean line: new work is `Proposed/Accepted` under `hybrid-inference`; everything before is closed
  history grouped by its own epoch.
- The honest operating posture is now in the ledger: local AI alone is *not* sufficient on this
  hardware (see ADR-2606072044's measured grid and the 30 GB-RAM ceiling) — the frontier fallback is
  load-bearing, not a nicety.

**Negative / risks**
- 110 prior `Accepted` ADRs were not auto-completed; the ledger carries "Accepted-but-unconfirmed"
  history. That is accurate, not tidy — and far better than falsely stamping them Completed.

## Implementation

- ADR statuses set (above); `hex adr reindex` regenerates `INDEX.md` grouped by epoch.
- `ARCHITECTURE.md` epoch table updated to add `hybrid-inference` as current.
- Workplans: 48 draft stubs cleared, 124 prior workplans archived to `docs/workplans/archive/`.
- `README.md` rewritten to match (honest local-AI posture; graphify attestation).

## References

- ADR-2606061359 — opened `single-agent`; this matures it.
- ADR-2606072044 — best-of-N + frontier fallback (the epoch's inference model).
- ADR-2606071734 — agentic benchmark (the epoch's model-selection basis).
- ADR-2606071702 — `hex dev deploy`.
- ADR-2606071651 / ADR-2606071713 — carried-forward backlog.
