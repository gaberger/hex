---
name: hex-intent
description: Capture an idea, ticket, or incident as a committed intent.md proto-spec in the originator's own words, before any spec or code exists. Use when the user says "I have an idea", "we should build", "capture this", "write this up", "raise a ticket for", "file this", or when a request arrives with no written artifact behind it.
---

# Hex Intent — Stage 1, Plan

**What changes:** ideas stop waiting for someone to write them up. Intent is
captured once, in the originator's own words, as a version-controlled artifact
the next stage can act on.

Traditional: an idea passes through backlog entries, user stories, story points
and refinement meetings before anyone can act on it. Ownership transfers at each
handoff, so what reaches engineering is several steps removed from what the
originator meant.

AI-native: the originator brainstorms with Claude and the result is written down
as `intent.md` — what is wanted, why, and under which constraints.

## Prerequisites

None. This is a clay play; nothing points into it.

## Where intent lives

`docs/intent/<YYYY-MM-DD>-<slug>.md` in the repo the change lands in. Keeping
intent next to the code derived from it keeps the artifact chain in one place
with one timestamp authority. A dedicated intent repo is only worth the overhead
when intent spans many repositories.

## How to execute it

1. **Let the originator describe the problem in their own words.** What they
   cannot do today, who is affected, what better looks like, what is out of
   scope. No formal language required.
2. **Brainstorm until the idea is concrete.** Ask what an analyst would ask:
   scope, users, constraints, what success looks like. Ask one question at a
   time; stop when the answers stop changing the shape of the idea.
3. **Write it as `intent.md`** using the template below. Write it in the
   originator's terms — do not translate their words into engineering language,
   because the point of the artifact is that it records what they meant.
4. **Read it back and correct anything misunderstood.** The originator owns the
   corrections, not the agent.
5. **Commit it.** Author and timestamp join the record. `git log` on
   `docs/intent/` is now the elicitation history.

Do **not** design the solution here. Constraints belong in the file; an
architecture does not. That is Stage 2 (`hex-spec-design`).

## Template

```markdown
# Intent: claims status self-service
Author: J. Ortiz (claims operations). Status: draft.

## Problem
Customers phone the contact center to ask where their claim is.
Handlers spend roughly a third of call time on status-only queries.

## Proposed outcome
Customers see claim status, next step and expected date in the portal.

## Affected users and systems
Claims handlers, portal team, claims-core API.

## Constraints
No new PII in the portal session. Existing authentication only.

## Open questions
Do third-party loss adjusters need access too?
```

## Intent from a non-human trigger

Intent also arrives from an alert, a scheduled scan, or a ticket. The steps are
identical and the file format is identical — an agent writes the draft, and the
product owner still reviews and corrects it before it is committed. See
`hex-close-loop` for the trigger side.

## Governance

The committed file is the evidence: author, timestamp, full revision history in
git. The product owner approves; the accept-or-reject decision that sends the
intent into Stage 2 is recorded as the merge or the closing review.

If the organization's record of work lives in a ticket tracker, note the record
ID in the intent file and the commit SHA on the ticket. Two systems are fine as
long as one is named the source of truth per artifact.

## Measure it

- **Leading:** time from first conversation to a committed intent file, read from
  `git log docs/intent/`. Expect hours, not a multi-week elicitation cycle.
- **Lagging:** survival rate — the share of intent files accepted into Stage 2
  rather than closed; and the number of edits to an intent file dated *after* the
  first spec commit for the same change (late edits mean the intent was thin).

## Next

An accepted intent triggers `hex-spec-design`.
