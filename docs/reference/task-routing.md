# Task Tier Routing

Source ADR: 2604110227. Skills: `/hex-workplan`, `/hex-feature-dev`.

Every user prompt classifies into one of three tiers, routed to the right artifact.

| Tier | Intent | Artifact | What happens |
|------|--------|----------|--------------|
| **T1 Todo** | Questions, trivial edits (typos, renames, comments), confirmations, reformats | Claude `TodoWrite` | Silent — host agent handles |
| **T2 Mini-plan** | Work-sized within a single adapter | In-session note | One-line suggestion in hook output |
| **T3 Workplan** | Feature-sized / cross-adapter (`implement X`, subsystem nouns) | `docs/workplans/drafts/draft-*.json` | **Auto-invokes `hex plan draft`** — draft stub, picked up via `/hex-feature-dev` |

Classifier: `hex-cli/src/commands/hook.rs::classify_work_intent`, runs inside `hex hook route` on every `UserPromptSubmit`. Scoring is conservative — false negatives (T3 → T2) are cheap, false positives (T1 → T3) would spawn unwanted drafts, so the threshold errs high.

## Auto-invocation does NOT

- Create worktrees (still gated on `hex plan drafts approve` + `/hex-feature-dev`)
- Dispatch coder agents (still gated on `hex plan execute`)
- Write specs or steps (draft contains only the original prompt)
- Commit anything

The `hex-specs-required` hook and phase gates remain in place. The draft only removes the "which slash command?" friction.

## Controls & opt-outs

- `HEX_AUTO_PLAN=0` — env var (highest precedence)
- `.hex/project.json` → `workplan.auto_invoke.enabled: false` — per-project
- `hex skip plan` in the prompt — per-prompt escape
- Questions (`?`, `how`, `why`, `what`) — always T1
- Trivial phrases (`fix typo`, `rename`, `add a comment`, `run rustfmt`) — always T1

## Draft management

```bash
hex plan draft <prompt>           # create a draft (normally auto-invoked)
hex plan drafts list              # list pending
hex plan drafts approve <name>    # promote → docs/workplans/approved-*
hex plan drafts clear [--name N]  # delete all (or one)
hex plan drafts gc --days 7       # GC drafts older than N days
```
