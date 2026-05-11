# ADR-2026-05-08-2400 — Personas as Commitment-Creators (Not Artifact-Producers)

Status: **Accepted**
Date: 2026-05-08
Supersedes: nothing (refines ADR-2026-03-24-0130 Declarative Swarm Behavior, ADR-2026-05-08-2300 Digital Twin)
Related: paradigm-debate-judge-verdict (`docs/specs/paradigm-debate-judge-verdict.md`), ADR-2026-05-08-1126 (merge gate), ADR-2026-05-08-2300 (digital twin), feedback_no_persona_fabrication

## Context

Today's adversarial debate (red flat-factory vs blue hierarchical-org, judge
arbitrated) landed a HYBRID verdict: **the factory pipeline owns artifact
production and validation; the hierarchy is a thin chat-rendering surface
only**. The judge cited the load-bearing evidence: today's session proved
that drafter→twin→executor (ADR-2026-05-08-2300) is the only path that has
shipped a validated artifact, while exec personas LARP'd off-topic content
even with `repo_grounding.rs` and the anti-fabrication system prompt in
place.

What is broken about the current persona contract:

1. **Persona output is unstructured prose.** `org_responder.rs` accepts
   any reply (1-3 sentences, board-mode plan, etc.). The `commitment_parser`
   tries to extract structure post-hoc but most replies produce zero
   commitments because they're filler ("I'll communicate with the
   engineering lead").
2. **No atomic claim.** Five execs simultaneously write PLANs because the
   plan-exists detection happens at history-fetch time, before STDB lag has
   propagated peer responses. Same race regardless of how good the
   prompt is.
3. **Personas pretend to act.** Every "I'll draft", "I'll communicate",
   "I'll ensure" is a lie — personas have no tools to do those things.
   The drafter→twin→executor loop only fires when a Confirm: with a
   `verifiable_path` artifact is parsed; everything else burns inference
   tokens for theater.

## Decision

The persona layer is reduced to ONE function: **emit zero or one
`commitment_open` row per operator turn, OR stay silent.** Personas have
no other authority. They do not produce prose, plans, recommendations,
acknowledgments, or any free-form reply visible in chat outside the
structured Confirm format.

### Mechanical changes

1. **System prompt.** Persona system prompt becomes a strict template:

   ```
   You are <role>. The CEO sent the following message addressed to you
   or your role tier.

   You may emit EXACTLY ONE of the following two outputs and nothing else:

   (A) Confirm: I (<role>) will <one-line concrete action> by <deadline>
       — success: <repo path | dashboard hashroute | requires-operator-action>

   (B) Silent

   Constraints:
     - The action MUST be something the factory pipeline can execute on
       your behalf. Today the only sink is file_write under docs/, src/,
       tests/, examples/, scripts/, hex-nexus/assets/src/. If your
       proposed action requires a tool that doesn't exist, say
       `success: requires-operator-action — <one-line description>`.
     - Pick ONE concrete deliverable. Do not pad. Do not "I'll also...".
     - If you have nothing to add or someone better-fit should own this,
       reply Silent. Silence is respected and routed correctly.

   No prose outside (A) or (B). No preamble. No "as the CTO,". No
   acknowledgment of the CEO's message. No emoji. Output is parsed by
   regex.
   ```

2. **STDB atomic claim.** New `commitment_thread_claim` table:
   `(thread_id PK, claimed_by, claimed_at)`. The org_responder
   tries to insert a claim BEFORE inferencing; if the insert fails
   (peer already claimed), the persona stays silent and never burns
   inference. First-claim-wins, deterministic, no race.

3. **org_responder reply parser.** Replies that don't match
   `^(?:Confirm:.*|Silent)$` (modulo whitespace) are dropped silently
   — the persona is logged as "off-contract" but no DM is posted to
   the operator. This kills the "I'll facilitate coordination" filler at
   the source.

4. **Persona YAMLs.** The `phases:` blocks for cto/cpo/coo/ciso/
   chief-visionary/engineering-lead/product-lead/sre-lead/sre-engineer
   are replaced with `tools: [hex_commitment_create]` and a 3-line
   description. Phase definitions like "assess/plan/coordinate/review/
   report" are deleted — they encoded the wrong contract.

5. **Dashboard.** The `#/team` chat surface continues to render persona
   responses, but each persona reply is rendered as a styled card showing
   the structured Confirm row (action / deadline / artifact / status)
   pulled from the `commitment` table, not raw prose. Silent responses
   render as "<role> deferred to peers."

### What personas STILL provide

- Operator-facing addressing: `@cto fix the inference router` is still a
  valid, ergonomic delegation — it just routes to a single-Confirm
  inference call.
- Domain biasing: cto's system prompt is biased toward technical artifacts
  (rs, ts, ADRs); cpo toward product specs; ciso toward security ADRs.
  This is fine and load-bearing for `requires-operator-action` framing.
- A natural-language operator interface that maps to typed factory rows.

### What personas STOP providing

- Free-form opinions or analysis (use `@hex-coder` or run an explicit
  swarm for that).
- Multi-paragraph plans (the PLAN/Confirm/Amend/Silent protocol from
  earlier today is retired — too much room for divergence).
- Acknowledgments, status updates, "I'll get back to you" strings.
- Cross-talk between personas (no peer "@cpo I disagree" replies; if
  the operator wants debate, run an adversarial swarm explicitly).

## Consequences

Positive:
- Inference budget collapses 80%+ — most operator messages will produce
  one Confirm + four Silents instead of five PLANs.
- Off-topic theater becomes structurally impossible: replies that don't
  match the regex are dropped.
- The 5-PLANs race is closed by atomic claim, not by prompt engineering.
- Operator dashboard is denser: each board ask becomes one card with
  one named owner, not five cards of overlapping prose.

Negative:
- Loses the "feel" of an executive team chatting. The org becomes more
  obviously a typed coordination substrate.
- Domain experts (cto/ciso) can no longer push back on a CEO ask in
  conversation — they either Confirm or Silent. Pushback requires the
  operator to ask differently or invoke an adversarial swarm.
- Existing personas with detailed `phases:` lose work — those YAMLs were
  invested in by prior workplans. Mitigated by archiving the old YAMLs
  under `hex-cli/assets/agents/hex/hex/_archived-2026-05-08/` rather
  than deleting.

## Validation

- A board ask produces ≤ 1 prose-bearing reply per turn (the Confirm:);
  all other personas log Silent.
- A board ask with no clear domain owner produces all-Silent and emits
  an inbox notification "no persona claimed turn for thread <id>" —
  operator decides who to address explicitly.
- The persona-tooling-gap.md test re-runs end-to-end and produces
  exactly ONE proposed_action per board turn (not five duplicates).
- `commitment_parser` no longer needs the loose `Confirm:` heuristics —
  every reply is either a strict Confirm or Silent.

## Out of scope

- Cross-domain delegation flows ("CTO asks engineering-lead to deliver
  Y") — handled by the operator manually invoking the lead with the
  Confirm content from the exec, until a future ADR adds
  `commitment_delegate(parent_id, to_role)`.
- Replacement of leads (engineering-lead, product-lead, sre-lead) — same
  contract applies; YAMLs updated in the same workplan.
