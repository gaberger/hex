# ADR-2604110227: Auto-Invoke Planner on Work-Intent Prompts

**Status:** Accepted
**Date:** 2026-04-11
**Drivers:** User-reported friction — "Why do we have to ask for a workplan to be created when Claude and others create todos automatically?" The intent classifier already exists in `hex hook route` but only emits passive warnings; it never actually invokes the planner.
**Supersedes:** None (extends ADR-050)

<!-- ID format: YYMMDDHHMM — 2604110227 = 2026-04-11 02:27 UTC -->

## Context

### The friction

Claude Code and similar agent harnesses auto-create per-agent todo lists (`TodoWrite`) whenever a prompt implies multi-step work. The user doesn't have to ask for it — the agent picks it up from context and a todo list materializes.

hex has a structurally richer task artifact — the **workplan** — but workplan creation is a **manual, user-initiated action**. Users have to either:

1. Know to invoke `/hex-feature-dev` or `/hex-workplan` by name, or
2. Type `hex plan create <name>` at a shell, or
3. Respond to a passive `WARNING: No active workplan…` that `hex hook route` prints (`hex-cli/src/commands/hook.rs:1685-1688`) and then manually take action.

The result: users skip the workflow for anything short of a weekend-long feature, and hex's parallelism/spec-traceability machinery goes unused on the tasks it was designed for.

### Why workplans are NOT todos

A workplan is **not** a personal scratch pad. It is a committed coordination contract (see `hex-cli/assets/skills/hex-workplan/SKILL.md`):

| Dimension | Claude TodoWrite | hex Workplan |
|---|---|---|
| Lifespan | Ephemeral, session-scoped | Durable, committed to `docs/workplans/feat-*.json` |
| Audience | One agent | planner → coder → reviewer → integrator, possibly across hosts |
| Side effects | None | `feature-workflow.sh setup` creates up to 8 git worktrees + dispatches HexFlo agents |
| Preconditions | None | Behavioral specs must exist first (`hex-specs-required` hook) |
| Validation | None | DAG check, tier consistency, spec coverage, build gates |
| Cost of being wrong | Retype a line | Wrong tier graph → parallel agent work wasted |

This is why workplans cannot be "silently auto-generated" the way todo lists are. But the **trigger** for starting the workplan-creation *process* is no different from the trigger for creating a todo — both are intent classification over the user's prompt.

### What already exists

`hex-cli/src/commands/hook.rs:1631-1695` (`async fn route`) runs on every `UserPromptSubmit` hook event and already does the following:

1. Calls `classify_prompt(&lower)` to detect hex-relevant intents and print hints
2. Loads `SessionState` and checks `state.workplan_id` for an active workplan
3. Computes `is_work` by scanning the prompt for work-intent verbs: `implement`, `create`, `add`, `fix`, `refactor`, `build`, `update`, `change`, `modify`, `write`, `generate`, `scaffold`, `wire`, `connect`, `remove`, `delete`, `migrate`, `upgrade`, plus confirmatory responses (lines 1658-1677)
4. If `is_work && mode == "mandatory"` → exits with code 2 and prints `BLOCKED: Cannot proceed without an active workplan. Run: hex plan create <name>` (line 1680)
5. Else if `is_work` → prints `WARNING: No active workplan for this work. Consider: hex plan create <name>` (line 1686)

The classifier is already there. The enforcement mode (`mandatory` vs advisory) is already there. The session state tracking is already there (ADR-050). What's missing is the last mile: **acting on the classification** instead of passing the burden back to the user.

### Three-tier reality

CLAUDE.md itself notes that single-agent mode exists "for small changes within one adapter boundary." So not every work-intent prompt should produce a workplan — many should produce a Claude TodoWrite, some should produce a lightweight in-session note, and only cross-adapter features should produce a full workplan with worktrees.

The intent classifier currently only has one dimension ("is this work?"). It needs a second dimension: **how big is this work?**

## Decision

We will close the last-mile gap by having `hex hook route` **actively invoke the planner** on feature-sized work-intent prompts, instead of emitting a passive warning that pushes the burden back to the user.

### 1. Three-tier task sizing

Extend the work-intent classifier in `hex-cli/src/commands/hook.rs` to emit a **tier** rather than a boolean:

| Tier | Signals | Artifact | Action |
|---|---|---|---|
| **T1: Todo** | "fix typo", "rename X", "update comment", single-file edits, questions | Claude TodoWrite | Do nothing — let the host agent handle it |
| **T2: Mini-plan** | Work-intent verbs + scope within one adapter, single-file focus | In-session markdown note + HexFlo memory entry | Print a suggestion; optionally auto-create a memory scratchpad |
| **T3: Workplan** | Feature-sized intent: "implement feature X", "add support for Y", cross-adapter verbs ("wire", "connect"), multi-file intent, proper nouns of subsystems | `docs/specs/<feature>.json` + `docs/workplans/feat-<feature>.json` | **Auto-invoke the planner** |

Classification is a heuristic scoring function (regex + keyword weights). When the score is below the T3 threshold OR `HEX_AUTO_PLAN=0` OR no TTY is attached, fall through to existing T1/T2 behavior (no new side effects).

### 2. Active planner invocation (not passive warning)

When the route hook detects a T3-sized intent AND no active workplan:

1. Print a **single-line banner** to stdout (visible in the Claude Code context):
   ```
   [HEX] Detected feature-sized task — starting planner in background. Reply "hex skip plan" to cancel.
   ```
2. Spawn the behavioral-spec-writer agent in the **background** via `Agent tool` (SDK) or `hex plan draft --background <prompt>`, passing the user's original prompt as the seed
3. Record `SessionState.pending_workplan_draft = true` so follow-up messages know a draft is in-flight
4. Continue processing the user's current prompt **without blocking** — the planner runs in parallel
5. When the planner produces a draft workplan, a `SubagentStop` hook surfaces it for user review via the existing `/hex-feature-dev` phase gates

**Critical safety boundary:** auto-invocation only starts the *planner* agent. It does NOT:
- Auto-create git worktrees (still gated on user approval of the workplan)
- Auto-dispatch coder agents (still gated on `hex plan execute`)
- Auto-commit anything
- Run in `mandatory` enforcement mode without user confirmation of the draft

This keeps all architectural guarantees (specs-first, DAG validation, tier ordering) while matching the "it just happened" ergonomics of TodoWrite.

### 3. Enforcement-mode interaction

Update the existing `mandatory` mode branch at `hex-cli/src/commands/hook.rs:1678-1682`:

| Mode | T1 | T2 | T3 |
|---|---|---|---|
| `advisory` (default) | silent | suggestion line | **auto-invoke planner** + banner |
| `mandatory` | silent | block with hint | **auto-invoke planner** + banner; still block coding until workplan is approved |
| `off` | silent | silent | silent (fall back to old warning) |

In `mandatory` mode, the auto-invocation **replaces** the `BLOCKED: Cannot proceed…` message — the user gets a draft handed to them instead of a scolding.

### 4. Opt-out and controls

Four ways to disable or tune:

- `HEX_AUTO_PLAN=0` env var — disables entirely, preserves old warning behavior
- `hex config set workplan.auto_invoke false` — persists to SpacetimeDB `hex_config`
- Per-prompt suppression: the phrase `hex skip plan` anywhere in the prompt bypasses auto-invocation
- Enforcement mode `off` — no work-intent processing at all

### 5. Configuration knobs

Add to `~/.hex/config.toml` (and SpacetimeDB `hex_config` table, synced per ADR-044):

```toml
[workplan.auto_invoke]
enabled = true              # master switch
tier_threshold = "T3"       # T2 or T3
banner = true               # print the one-line banner
background = true           # run planner in background (false = blocking)
opt_out_phrase = "hex skip plan"
```

## Consequences

**Positive:**
- **Ergonomic parity with TodoWrite**: feature-sized work no longer requires the user to remember the right slash command or CLI invocation
- **More workplans get created** for the right reason — because they were the right artifact, not because someone remembered to type the command
- **Spec-traceability becomes the default**: auto-invocation always routes through behavioral-spec-writer first, so specs get written even for features the user forgot to spec
- **HexFlo parallelism gets used**: today, users skip workplans and serialize everything into one agent; auto-invocation makes multi-worktree execution the path of least resistance
- **Zero new concepts**: all building blocks already exist — classifier, enforcement mode, planner agent, session state, hook router. This ADR just wires them together

**Negative:**
- **False positives**: a conversational "can you add a comment explaining X?" could trip the T3 classifier. Mitigated by weighted scoring and `hex skip plan` escape hatch.
- **Latency**: background planner spawn adds ~100ms to hook route execution. Mitigated by running truly in background and not blocking the prompt.
- **Draft workplan clutter**: auto-generated drafts could accumulate in `docs/workplans/` if users ignore them. Mitigated by writing drafts to `docs/workplans/drafts/` until the user approves them, with a 7-day auto-cleanup (reuses the `hex adr abandoned` pattern).
- **Silent state changes**: users might not notice a background planner was started. Mitigated by the one-line banner and a `hex plan drafts` command that lists in-flight drafts.

**Mitigations:**
- Classification is conservative by default — T3 threshold is tuned to err toward T1/T2
- All behavior is opt-out via env var, config, and per-prompt escape hatch
- Draft workplans are quarantined to `docs/workplans/drafts/` with automatic expiry
- The banner is deliberately one line so it doesn't dominate the context window
- `hex plan drafts list` and `hex plan drafts clear` commands make the pending state observable and reversible

## Implementation

| Phase | Description | Validation Gate | Status |
|-------|------------|-----------------|--------|
| P1 | Extract `classify_work_intent()` into a scoring function returning `Tier { T1, T2, T3 }`; unit tests for 30+ prompt samples across all three tiers | `cargo test -p hex-cli classify_work_intent` passes | Pending |
| P2 | Add `config.toml` + `hex_config` schema for `workplan.auto_invoke.*` keys; wire `hex config get/set` | `cargo check --workspace`; `hex config get workplan.auto_invoke.enabled` returns a value | Pending |
| P3 | Implement `hex plan draft --background <prompt>` subcommand that spawns behavioral-spec-writer + planner agents to `docs/workplans/drafts/` | `hex plan draft --background "implement oauth"` produces a draft file | Pending |
| P4 | Replace the `WARNING: No active workplan…` and `BLOCKED:` branches in `route()` at `hex-cli/src/commands/hook.rs:1678-1688` with tier-based dispatch; auto-invoke `hex plan draft --background` on T3 | Manual test: prompt "implement oauth login" triggers draft | Pending |
| P5 | Add `hex plan drafts list/clear/approve` subcommands | `hex plan drafts list` shows in-flight drafts | Pending |
| P6 | Wire `SessionState.pending_workplan_draft` field (ADR-050 extension); emit `SubagentStop` event when planner finishes | Event visible in session log | Pending |
| P7 | Add `hex skip plan` prompt-level opt-out parsing to `route()` | Prompt containing "hex skip plan" bypasses auto-invocation | Pending |
| P8 | Document in CLAUDE.md "Task Tier Routing" subsection + update `/hex-feature-dev` SKILL.md | `hex analyze .` passes; docs render correctly | Pending |
| P9 | 7-day cleanup job for `docs/workplans/drafts/*.json` older than threshold; reuses `hex adr abandoned` pattern | `hex plan drafts gc` purges stale drafts | Pending |
| P10 | Adversarial review gate: does auto-invocation ever fire on a read-only question? Regression suite of 20 false-positive prompts | All 20 return tier T1 or T2 | Pending |

## References

- **Existing code:**
  - `hex-cli/src/commands/hook.rs:1631-1695` — `route()` function with current classifier, enforcement mode, warning
  - `hex-cli/assets/skills/hex-workplan/SKILL.md` — workplan JSON format and validation rules
  - `hex-cli/assets/skills/hex-feature-dev/SKILL.md` — manual invocation of the lifecycle this ADR automates
  - `scripts/feature-workflow.sh` — worktree setup driven by workplan JSON
- **Related ADRs:**
  - ADR-050: Heartbeat + workplan enforcement on UserPromptSubmit (extended by this ADR)
  - ADR-060: Inbox notifications on UserPromptSubmit (same hook, same router)
  - ADR-044: Repo config sync to SpacetimeDB (used for `workplan.auto_invoke.*` keys)
  - ADR-2604101200: Workplan file scaffolding (complementary — this ADR creates the workplan, that ADR scaffolds files the workplan references)
  - ADR-2604051700: Enforce workplan gates (defines the `mandatory` mode this ADR re-uses)
- **Origin:** User question on branch `claude/review-codebase-7qKsD`, 2026-04-11. Transcript preserved in `/root/.claude/plans/wise-drifting-orbit.md`.
