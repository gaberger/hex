# ADR-2604170905: `/hex-orient` Skill for Agent Onboarding

**Status:** Proposed
**Date:** 2026-04-17
**Drivers:** ADR-2604170900 condensed CLAUDE.md into a terse kernel. Fresh sessions or new contributors who need broad grounding ("what is hex? what are the rules? where does what live?") no longer get it from CLAUDE.md alone — the pointers are correct but not self-contained. A dedicated orientation skill fills that gap without reinflating CLAUDE.md.

## Context

With CLAUDE.md reduced to hard rules + a skill-map, sessions that benefit from passive context now have two paths:

1. Invoke a specific skill for the task at hand (the common case).
2. Read `docs/reference/README.md` + selected reference docs manually (the onboarding case).

Path 2 is friction-ful. A human would type `/hex-orient` and expect a loaded agent that understands the ethos, knows which hard rules matter, and has seen the system-components overview — without having to pick reference docs one by one.

This is exactly the skill system's job: a single trigger that loads a focused context bundle.

Alternatives considered:

1. **Inline the orientation material back into CLAUDE.md.** Rejected — defeats ADR-2604170900.
2. **Rely on `docs/reference/README.md` as a manual bootstrap.** Rejected — requires the agent to know to read it and to know what to read next. Friction is the problem.
3. **Auto-load `docs/reference/README.md` via a hook at session start.** Rejected — same context tax as the old CLAUDE.md; orientation isn't needed every session.

## Decision

Create `/hex-orient` as a skill under `hex-cli/assets/skills/hex-orient.md`. It is a **pull**, not a push: invoked when the agent (or user) explicitly asks for grounding.

The skill loads:

1. CLAUDE.md hard rules (re-stated, not just linked — so the skill is self-contained)
2. `docs/reference/README.md` (the index of all reference docs)
3. `docs/reference/components.md` (system components overview)
4. `docs/reference/lessons.md` (hard-won lessons)

It provides a short guided tour: "here is what hex is, here are the rules you must follow, here is where to look when you need more." It does **not** load every reference doc — the agent pulls those via other skills as needed.

Deployment: the skill file lives in `hex-cli/assets/skills/` and is baked into the hex-cli / hex-nexus binaries via `rust-embed`. It deploys to target projects via `hex init`.

## Consequences

**Positive:**
- Fresh sessions have a single trigger for "bring me up to speed."
- Reference material stays out of the always-on kernel but remains one hop away.
- Future expansion: `/hex-orient --deep` could chain into architecture + workflow docs; `/hex-orient --rules-only` for quick rule refresh.

**Negative:**
- One more skill to maintain. The skill-map in CLAUDE.md must list it.
- If the rules in CLAUDE.md change and the skill's inlined copy is not updated, they drift.

**Mitigations:**
- Skill body does not duplicate the rules verbatim — it points back to `/CLAUDE.md` as the authoritative source, then summarises. Drift surfaces as a summary mismatch, caught by `/hex-adr-review` or manual read.
- Add to `hex doctor` a check that `/hex-orient` exists and references the current CLAUDE.md.

## Implementation

| Phase | Description | Status |
|-------|------------|--------|
| P1 | Write `hex-cli/assets/skills/hex-orient.md` with YAML frontmatter + body | Pending |
| P2 | Add entry to CLAUDE.md skill-map table | Pending |
| P3 | Add `/hex-orient` to `docs/reference/skills-and-agents.md` catalog | Pending |
| P4 | Rebuild hex-cli + hex-nexus (`cargo build --release`) | Pending |
| P5 | Verify deployment via `hex init` into an `examples/` project | Pending |

Enqueue after acceptance: `hex brain enqueue workplan docs/workplans/wp-hex-orient-skill.json` (workplan to be drafted).

## References

- ADR-2604170900: CLAUDE.md kernel pattern (parent decision)
- `hex-cli/assets/skills/` — skill-builder skill documents the frontmatter schema
- `/hex-orient` will follow the same progressive-disclosure pattern as `/hex-feature-dev`
