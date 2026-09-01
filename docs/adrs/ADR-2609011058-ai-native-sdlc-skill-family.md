# ADR-2609011058: AI-native SDLC skill family

**Status:** Proposed
**Date:** 2026-09-01
**Drivers:** hex's embedded skills covered the build stage only. The stages around build — intent capture, requirements/design, self-verification, review policy, and the autonomous maintenance trigger — had no artifact and no skill, which is where the bottleneck moves once agents write most of the diff.
**Applies-To:** hex-cli/assets/skills/, .claude/skills/

## Context

The AI-native SDLC playbook (claude.com/blog/the-ai-native-sdlc-playbook,
2026-08-21) describes six stages, each ending by committing an artifact that
triggers the next: intent → spec → plan → verified diff → reviewed PR → incident
record → intent. hex already implements the middle of that chain in its own
vocabulary (behavioral specs, workplans, validation judge, boundary enforcement,
hooks) but exposes no skill for the ends of it, and nothing tells an agent which
stage a request belongs to or which artifact is missing.

Two properties of the playbook matter for hex specifically:

1. **A skill is an advisory control; a hook is the deterministic one.** Any policy
   that must always hold needs both. hex already has the hook layer (`hex hook`,
   `hex enforce`) but had no written mapping from policy → skill → hook.
2. **The configuration that steers the agent deserves the regression testing that
   code gets.** `CLAUDE.md`, skills, hooks and agent definitions change agent
   behavior and were previously untested.

## Decision

Ship seven skills in `hex-cli/assets/skills/`, embedded and extracted on init:

| Skill | Stage | Artifact |
|---|---|---|
| `hex-sdlc-loop` | index | routes a request to the stage whose artifact is missing |
| `hex-intent` | 1 Plan | `docs/intent/<date>-<slug>.md` |
| `hex-spec-design` | 2 Design | `docs/specs/<slug>.md` |
| `hex-plan-mode` | 3 Build | workplan JSON + the diff |
| `hex-feedback-loop` | 4 Test | verification command, tests, eval cases |
| `hex-review-gate` | 5 Deploy | `REVIEW.md`, approval-gate hooks, the PR |
| `hex-close-loop` | 6 Maintain | control bands → a new intent file |

hex-native mappings are stated inside each skill rather than duplicated: the
Stage 3 plan artifact **is** the workplan JSON, the Stage 4 independent oracle
**is** the behavioral spec plus `hex validate` / `hex verify`, and lessons that
outlive one repo go to `hex memory store lesson:<topic>` rather than a local file.

`hex-workplan`, `hex-feature-dev` and `hex-validate` gain a short section naming
their position in the chain, so the family is reachable from the skills already
in use.

## Consequences

- A request with no committed artifact behind it is now a routable condition, not
  a judgment call.
- Review gains a compliance pass with something to compare against: the diff
  versus the committed spec and plan.
- New convention: `docs/intent/` exists in projects that adopt Stage 1. Projects
  that do not adopt it are unaffected — every skill declares its prerequisites and
  two of them (`hex-feedback-loop`, `hex-intent`) have none.
- Skills are advisory. Nothing here blocks an action; the blocking layer stays
  with hooks and `hex enforce`, and `hex-review-gate` says so explicitly.
- Embedded asset count grows by seven directories; `hex ci` gate 5 (assets
  generic) covers them and passes.

## Implementation

- `hex-cli/assets/skills/{hex-sdlc-loop,hex-intent,hex-spec-design,hex-plan-mode,hex-feedback-loop,hex-review-gate,hex-close-loop}/SKILL.md` — new.
- `hex-cli/assets/skills/{hex-workplan,hex-feature-dev,hex-validate}/SKILL.md` — chain cross-links added.
- Mirrored into `.claude/skills/` so they are live in this repo before the next
  release build; a CLI rebuild re-embeds them for target projects.
- `docs/intent/2026-09-01-ai-native-sdlc-skills.md` — the intent this ADR answers,
  written in the Stage 1 format as the first use of the convention.

## References

- The AI-native SDLC playbook — https://claude.com/blog/the-ai-native-sdlc-playbook
- Skills — code.claude.com/docs/en/skills
- Hooks — code.claude.com/docs/en/hooks
- Settings reference — code.claude.com/docs/en/settings
