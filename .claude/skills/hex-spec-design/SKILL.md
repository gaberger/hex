---
name: hex-spec-design
description: Turn an accepted intent.md into one requirements-and-design spec in a single session, with organizational policy applied while the spec is written and areas of concern flagged. Use when the user says "write the spec", "turn this into requirements", "design this", "what should we build", or points at a committed intent file.
---

# Hex Spec Design — Stage 2, Design

**What changes:** requirements and design collapse into one session. Policy is
applied while the spec is written, not discovered in a review weeks later.

Traditional: analysts formalize the idea into requirements; designers parse those
back into a design. The separation exists for accountability, but it is slow and
lossy.

AI-native: one prompted session takes `intent.md` and produces a requirements and
design spec, constrained by the organization's skills, with areas of concern
flagged for the humans who own those policies.

## Prerequisites

A committed intent file (`hex-intent`). Brand, security, compliance and UX
policies written as skills — a spec is only as constrained as the skills loaded
in the session that wrote it.

## How to execute it

1. **Load the constraint skills and attach the intent.** Name the constraints
   explicitly in the prompt; do not assume a skill triggers on its own.
2. **Produce the spec.** The prompt that works:

   > Read the attached intent and produce a requirements and design spec for
   > integrating it into our existing codebase. Apply the skills available to you
   > so the plan conforms to our brand guidelines, security policies and UX
   > standards. Document the spec fully as `docs/specs/<slug>.md`, ready to hand
   > to the engineering team. Describe clearly any areas of concern, especially
   > where you cannot satisfy contradicting policies.

3. **Demand the flagged concerns.** A spec with no flagged concerns on a
   non-trivial change usually means the policies were not read. These flags are
   the points an analyst would have escalated.
4. **Review the spec against the idea.** Does it solve the stated problem? Are
   the open questions from the intent answered or explicitly carried forward?
   The product owner reviews the spec but does not write it.
5. **Resolve each flagged concern with its policy owner** before engineering sees
   the spec.
6. **Commit the spec alongside the intent.** The file pair records what was asked
   for and what was decided.

Run this by hand first. Once the shape is stable, codify it as a slash command,
then make the merge of an intent file the trigger for a non-interactive job that
runs the pass with the policy skills loaded and opens the spec as a PR.

## What the spec must contain

- The behavior a user can observe, in user-facing terms — no function names, no
  internal state. This is what makes it an independent oracle for the build.
- The boundaries the change crosses: which ports, which adapters, which external
  systems. In a hexagonal codebase, one adapter boundary per unit of work is what
  makes Stage 3 parallelizable.
- Non-functional constraints with numbers: rate limits, payload sizes, latency,
  data classification.
- Areas of concern, each named with the policy it touches and the person who owns
  that policy.
- Anything explicitly out of scope.

## In a hex project

- Prose spec → `docs/specs/<slug>.md`; check it in with `hex spec list` visible.
- Machine-checkable scenarios → behavioral specs (see the `hex-workplan` skill).
  The prose spec is what the product owner signs; the behavioral specs are what
  the validation judge runs. Write them from the same session, not from the code.
- If the change introduces a new port, a new adapter, an external dependency, a
  persistence choice or a trust boundary, it needs an ADR too — `hex adr schema`
  for the template, and the ADR is what carries the decision, not the spec.

## Governance

Live policy is read and applied while the spec is written. The spec, the prompt
that produced it, and the skill versions in force are all in version control. The
product owner signs the spec off and routes flagged concerns to named policy
owners. The decision to progress to build is a human's, taken with a technical
lead for anything the organization classes as higher risk.

## Measure it

- **Leading:** elapsed time between the intent commit and the spec commit for the
  same change — two git timestamps, against the old requirements-plus-design
  cycle.
- **Lagging:** requirements rework after build starts — spec commits dated after
  the first plan commit for the same change. `git log` gives this directly.

## Next

An accepted spec triggers `hex-plan-mode`.
