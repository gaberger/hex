---
name: hex-plan-mode
description: Nothing is implemented without an accepted plan. Produce a written implementation plan from the spec — files that change, order of work, risks, proof — get it accepted, commit it, then build against it. Use when the user says "implement this", "start building", "plan mode", "how would you do this", or hands over an accepted spec.
---

# Hex Plan Mode — Stage 3, Build

**What changes:** work starts with a written plan produced while the agent can
read the codebase but not change it. The engineer corrects the plan before any
code exists, and the approved version is committed for later stages to check
against.

Traditional: an engineer reads the design and starts writing code. How the change
will be made — which files, which tests — stays in their head or a ticket
comment. The first thing a reviewer sees is the finished diff, and by then rework
is slow.

## Prerequisites

The intent and spec if they exist, and a `CLAUDE.md` the session reads at start.

## How to execute it

1. **Start in plan mode.** Read-only exploration; no edits until the plan is
   accepted. The mode is the enforcement — it is not a matter of discipline.
2. **Ask for a plan that names the files that change, the order of the work, and
   the tests that prove it.** A plan without named files and named tests is a
   summary, not a plan.
3. **Interrogate it.** What could this break? Which step is riskiest? What
   options were considered and rejected, and why? What does it assume about the
   codebase that has not been checked?
4. **Iterate until an engineer who has never seen the conversation could
   implement the change from the plan alone.** That is the acceptance bar.
5. **Commit the approved plan.** It joins the audit trail; Stage 5 checks the
   eventual diff against it.
6. **Accept and implement.** With a solid plan, implementation is often a single
   pass.
7. **When implementation departs from the plan, update the plan in the same
   commit.** A hook can enforce that synchronization.

## The plan artifact

```markdown
# Plan: claims status self-service (from intent 2026-06-02)

## Files that change
portal/src/claims/StatusPanel.tsx (new), claims-api/routes/status.py,
claims-api/tests/test_status.py

## Order of work
1. Add the status endpoint behind existing auth.
2. Panel against the endpoint.
3. Wire into the portal nav.

## Risks
The claims-core API rate-limits at 50 rps; the panel must cache.

## Proof
test_status.py covers the four claim states; screenshot matches the
approved mock.
```

In a hex project the machine-actionable form of this artifact is the workplan:

```bash
hex plan draft "<the change, in one sentence>"   # writes a draft
hex plan drafts list | hex plan drafts approve <name>
hex plan lint <workplan>                          # evidence must be real
hex plan reconcile --all --update                 # status vs actual code
```

Every step in the workplan carries its own done-condition, so "the plan" and "the
definition of done" are the same file. Decompose by boundary: one task = one
adapter boundary = one worktree, ordered domain → ports → secondary → primary →
use cases → integration. Tasks that share a file are serialized in one session;
agents editing the same file produce conflicting diffs.

## Auto-accept, and when it is earned

Once the guardrails mature — a tuned `CLAUDE.md`, skills that encode policy,
hooks that block unsafe actions, and a test suite the agent can run — auto-accept
becomes the default for routine work: a tight spec, a small blast radius, and
code the tests already cover. The shift is away from watching each edit and
towards reviewing artifacts after longer autonomous sessions.

Two or three parallel sessions, each in its own worktree, is a sensible starting
point. The ceiling is how many streams one person can review properly; add a
session only while review is keeping up. Recurring jobs — a verifier that runs
the app, a simplifier, a researcher that explores without flooding the main
context — become subagents checked into `.claude/agents/` so the whole team
shares them.

## Institutional knowledge

`CLAUDE.md` gives the agent what a new joiner would need: build/test/lint
commands, the conventions that matter, the architecture in a paragraph, and the
things the agent keeps getting wrong. Keep it under a page — it is read in full
at the start of every session and anything stale is spending context for nothing.

**The working rule: when the agent makes the same mistake twice, the correction
goes into `CLAUDE.md`** (or, for a lesson the whole fleet should carry rather
than one repo, `hex memory store lesson:<topic> "<text>"`).

Write a skill instead of a `CLAUDE.md` line when the knowledge is institutional,
must be applied consistently, and is owned by someone outside the repo — a
security standard, an API convention, a brand rule. Skills live in
`.claude/skills/<name>/SKILL.md`, ship with the code, and update centrally when
policy changes.

## Governance

Design review happens before any code is generated, when changing course is still
a matter of editing a document. The plan and its revisions are logged along with
who accepted it. Routine changes are accepted by the engineer; higher-risk ones
go to a tech lead or architect.

## Measure it

- **Leading:** share of changes that merge from the first implementation pass;
  time from plan approval to merged PR.
- **Lagging:** rework cycles per change, and how often the merged diff still
  matches the committed plan.

## Next

The diff and its tests go to `hex-feedback-loop` before a human sees them.
