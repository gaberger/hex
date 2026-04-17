# ADR-2604170910: Task-Tier-Routing Absorbed into `/hex-workplan` Skill

**Status:** Proposed
**Date:** 2026-04-17
**Drivers:** ADR-2604170900 moved the task-tier-routing explanation (T1 Todo / T2 mini-plan / T3 workplan, per ADR-2604110227) out of CLAUDE.md into `docs/reference/task-routing.md`. That's fine as backing material, but there's no skill trigger that owns it. Adding a `/hex-classify` skill would fragment the workplan domain; absorbing the content into `/hex-workplan` keeps it where users already look.

## Context

Task tier routing (ADR-2604110227) classifies every user prompt into T1/T2/T3 and routes to the appropriate artifact (TodoWrite / in-session note / `hex plan draft`). The classifier runs inside `hex hook route`. Users rarely need to think about classification directly — but they do need to understand it when:

- A prompt gets mis-classified (T3 → T2, or trivially T1 → T3).
- They want to override via `HEX_AUTO_PLAN=0`, `.hex/project.json`, or `hex skip plan`.
- They're debugging why a draft was or wasn't auto-created.

This is workplan-adjacent: classification is the precursor step to workplan creation. A user who types `/hex-workplan` is already in the workplan mental model — showing them classification rules there is contextually appropriate.

Alternatives considered:

1. **Create `/hex-classify` as a dedicated skill.** Rejected — classification is a precursor action, not a standalone domain. A dedicated skill fragments the workflow.
2. **Leave task-routing.md as reference-only, no skill owns it.** Rejected — breaks the skill-first discovery principle from ADR-2604170900. Every reference doc should map to at least one skill.
3. **Create a general `/hex-hooks` skill covering classifier + all hooks.** Rejected — too broad; hooks is a large topic (route, session-start, subagent-start/stop, pretool, posttool).

## Decision

Extend `/hex-workplan` to cover task-tier classification as an input-side concern:

1. Add a `## Task Tier Classification` section to `hex-cli/assets/skills/hex-workplan.md` explaining T1/T2/T3.
2. Document the opt-outs (`HEX_AUTO_PLAN=0`, config toggle, `hex skip plan`).
3. Cross-link to `docs/reference/task-routing.md` for the full classifier rule table.
4. Cross-link to ADR-2604110227 (original classifier decision) and ADR-2604142243 (classifier rule tables).

No new skill is created. The workplan skill grows by one section (~30 lines).

## Consequences

**Positive:**
- Preserves the "one intent → one skill" mental model without inventing `/hex-classify`.
- Users asking `/hex-workplan` get the full story: how prompts become drafts, how drafts become workplans, how workplans become execution.
- `docs/reference/task-routing.md` gains a primary skill owner, maintaining the skill-first discovery property.

**Negative:**
- `/hex-workplan` grows slightly. If it grows much more, the skill itself becomes a mini-manual.

**Mitigations:**
- Cap the classifier section at ~30 lines — details stay in `docs/reference/task-routing.md`.
- If `/hex-workplan` exceeds ~200 lines, split then.

## Implementation

| Phase | Description | Status |
|-------|------------|--------|
| P1 | Add `## Task Tier Classification` section to `hex-cli/assets/skills/hex-workplan.md` | Pending |
| P2 | Cross-link from section → `docs/reference/task-routing.md`, ADR-2604110227, ADR-2604142243 | Pending |
| P3 | Update `docs/reference/skills-and-agents.md` description for `/hex-workplan` to mention classification | Pending |
| P4 | Rebuild hex-cli + hex-nexus (`cargo build --release`) | Pending |
| P5 | Verify deployment via `hex init` into an `examples/` project | Pending |

Enqueue after acceptance: `hex brain enqueue workplan docs/workplans/wp-workplan-skill-classify.json` (workplan to be drafted).

## References

- ADR-2604170900: CLAUDE.md kernel pattern (parent decision)
- ADR-2604170905: `/hex-orient` skill (sibling follow-up)
- ADR-2604110227: Task tier routing (original classifier design)
- ADR-2604142243: Classifier rule tables
- `docs/reference/task-routing.md` (backing material)
