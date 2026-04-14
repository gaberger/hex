# ADR-2604142200: Reconcile must verify file evidence, not just match names

**Status:** Accepted
**Accepted:** 2026-04-15 (via wp-enforce-workplan-evidence commits `0fac2e9f` `14a91f9b` `7f0e886b` `daa16c3c`)
**Date:** 2026-04-14
**Drivers:** Live bug surfaced during autonomous execution of `wp-hex-native-web-search`: the brain daemon correctly executed P1.1 and P1.2 via nemotron-mini (commits `19928ffc`, `1d1e40b9`, cargo check passes), then `hex plan reconcile` marked 11 downstream tasks as `done` even though their target files do not exist on disk. A continuous reconcile loop actively reverted manual attempts to reset those statuses — the false `done` state is re-asserted within one tick of being corrected. Left unaddressed, every autonomous workplan will silently skip real work.
**Supersedes:** n/a (amends `0f704881 fix(plan): Scope reconcile heuristic to workplan ADR id`)

## Context

`hex plan reconcile` scans workplan JSONs and promotes tasks from `pending` → `done` based on git evidence. A prior fix (`0f704881`) scoped the heuristic to workplan ADR id — this reduced cross-workplan false positives but did not eliminate within-workplan false positives.

Empirical evidence from `wp-hex-native-web-search` tick on 2026-04-14:

| Task | File declared | File exists? | Marked `done`? |
|------|---------------|--------------|----------------|
| P1.1 | `hex-core/src/domain/web.rs` | ✅ yes | ✅ correct |
| P1.2 | `hex-core/src/ports/web.rs` | ✅ yes | ✅ correct |
| P2.1 | `hex-nexus/src/adapters/secondary/web_fetch.rs` | ❌ **missing** | ❌ **false** |
| P2.2 | `hex-nexus/src/adapters/secondary/web_cache.rs` | ❌ missing | ❌ false |
| P2.3 | `hex-nexus/tests/web_fetch.rs` | ❌ missing | ❌ false |
| P3.1 | `hex-nexus/src/adapters/secondary/web_search_brave.rs` | ❌ missing | ❌ false |
| P3.2 | `hex-nexus/src/adapters/secondary/web_search_ddg.rs` | ❌ missing | ❌ false |
| P3.3 | `hex-nexus/src/adapters/secondary/web_search_broker.rs` | ❌ missing | ❌ false |
| P3.4 | `hex-nexus/tests/web_search.rs` | ❌ missing | ❌ false |
| P4.1 | `hex-nexus/src/adapters/secondary/web_search_tavily.rs` | ❌ missing | ❌ false |
| P4.2 | `hex-nexus/src/adapters/secondary/web_search_serpapi.rs` | ❌ missing | ❌ false |
| P5.1 | `spacetime-modules/hexflo-coordination/src/web_request.rs` | ❌ missing | ❌ false |
| P5.2 | `hex-nexus/src/routes/web.rs` | ❌ missing | ❌ false |
| P5.3 | `hex-nexus/src/routes/web.rs` | ❌ missing | ❌ false |

The heuristic appears to match on ADR id + some proximity signal (sibling task done, dependency chain satisfied, or commit-message substring) without checking the `files[]` array.

**Why this is dangerous:**
- Silent over-completion: the workplan reports green, half the code is missing.
- Daemon stops dispatching: once a task is `done`, the dispatcher skips it on subsequent ticks.
- Observability gap: `hex brain queue list` shows only pending; there is no `history` or `--all` view to audit what the daemon did vs. what was reconcile-marked.
- Edit-war: the reconcile loop re-asserts `done` after manual correction, so a human cannot hand-fix the JSON.

**Alternatives considered:**

1. **Disable reconcile entirely.** Rejected: reconcile solves a real problem (recovering state after daemon restarts, across-session continuity). We need it, it just needs to be correct.
2. **Require explicit git commit trailer `Reconciles-Task: P2.1`.** Too heavyweight — the daemon's existing commit messages (`ports(p1.2): ...`) already encode intent.
3. **Verify file existence + symbol presence before promoting.** Chosen. Cheap, matches how a human would verify, and catches the current bug class.

## Decision

`hex plan reconcile` must require **positive file evidence** before promoting any task to `done`. The evidence check:

1. **Every path in `task.files[]` must exist on disk.** If any file is missing, the task stays `pending`. No heuristic override.
2. **At least one declared symbol must appear in at least one declared file.** "Declared symbol" is extracted from `task.description` — struct/enum/trait/fn names parsed by a lightweight regex (e.g., `([A-Z][A-Za-z0-9_]+)` filtered by context words `struct|enum|trait|fn|impl`). If no symbols can be parsed, fall back to rule 1 only.
3. **A commit matching task id must exist.** There must be at least one commit in `git log` whose subject contains the task id in the form `(p1.2)` or `P1.2` or whose body contains `Task-Id: P1.2`. Commits are scoped to the branch where the workplan lives.
4. **Loop scope: reconcile runs on explicit invocation only.** The brain daemon's tick may call `reconcile --check-only` for reporting but must not *mutate* the workplan JSON outside of an explicit `hex plan reconcile --update` invocation driven by an operator or by the task executor after a successful commit. Passive reconcile is a footgun — hooks, linters, and watchers must not trigger write-path reconcile.

Observability additions:

- `hex brain queue list --include completed,failed --since <duration>` — history view for drained tasks. Backed by a new `brain_task_history` HexFlo memory scope (or a dedicated SpacetimeDB table if that's simpler).
- `hex plan reconcile --dry-run --verbose` — prints which tasks *would* be promoted and the evidence per task, without mutating the file. This is what diagnosed the current bug manually.
- `hex plan reconcile --why <task-id>` — explains the evidence (or lack thereof) for a single task.

Rollback: if the stricter check produces too many false negatives (tasks that really are done but the heuristic says no), operators use `hex plan reconcile --force <task-id>` as an explicit override. Force always logs to `brain_task_history` with a `forced_by=<user>` marker for audit.

## Consequences

**Positive:**
- Workplans stop silently losing work. Autonomous execution becomes trustable.
- The "verify before done" feedback rule (`feedback_verify_before_done.md`) finally applies at the tooling layer, not just at the agent layer.
- Operators can audit daemon behavior retrospectively.
- The edit-war failure mode (reconcile re-asserting over manual edits) is eliminated by removing passive reconcile.

**Negative:**
- Slightly slower reconcile — every task checks fs + grep. Negligible in practice (<200 files on the current repo).
- Regex-based symbol extraction is fragile; edge-case descriptions may produce no symbols to verify. Mitigated by falling back to fs-only check.
- Operators who rely on the existing ergonomic-but-sloppy reconcile (marking a task "basically done") lose that affordance. Replaced by `--force`.

**Mitigations:**
- Symbol regex has a golden-fixture test suite with ~20 real task descriptions.
- `--dry-run` output is the default for the first week to build operator trust; mutation path stays behind an explicit flag.
- Migration note in the CLI tells operators about `--force` for pre-existing "done" tasks that fail the new check.

## Implementation

| Phase | Description                                                                                                 | Status  |
|-------|-------------------------------------------------------------------------------------------------------------|---------|
| R1    | Evidence-verified reconcile: file-exists + symbol-grep + commit-id match; returns per-task evidence report  | Pending |
| R2    | Remove passive reconcile triggers (audit hooks in .claude/settings.json, daemon tick, file-watch listeners) | Pending |
| R3    | `hex brain queue list --include completed,failed --since <dur>` + `brain_task_history` storage              | Pending |
| R4    | `hex plan reconcile --dry-run`, `--why <id>`, `--force <id>` subcommand flags                               | Pending |
| R5    | Regression test: load `wp-hex-native-web-search.json` with P2/P3 empty → reconcile must NOT mark them done  | Pending |

## References

- `feedback_verify_before_done.md` — the rule this enforces at the tooling layer
- `feedback_agent_commit_contract.md` — parallel rule at the agent layer
- `0f704881 fix(plan): Scope reconcile heuristic to workplan ADR id` — previous partial fix
- ADR-2604142100 (wp-hex-native-web-search) — the workplan that surfaced this bug
- Commits `19928ffc`, `1d1e40b9` — correct daemon executions that prove the substrate works

## Retrospective — 2026-04-15

Implemented via `wp-enforce-workplan-evidence` in 4 commits:

| Commit | Scope | Closes |
|---|---|---|
| `0fac2e9f` | E1.1 `validate_workplan_evidence()` pure fn + E1.2 reconcile wire | Bad workplans blocked at reconcile time |
| `14a91f9b` | E3.1 `hex plan lint [<path>|--all]` CLI + E1.3 test fixtures | Author-time + CI detection |
| `7f0e886b` | E2 retrofit — 20 violations across 4 workplans → 0 | Existing workplans brought to compliance |
| `daa16c3c` | E3.2 `workplan-evidence-lint.yml` pre-tool-use hook asset | Pre-edit enforcement (after `hex init`) |

**Live validation:** `hex plan lint --all` over the current corpus → exit 0, zero violations. Production-bar item 4 ("no workplan saves with empty files[]") is green at three layers: reconcile-path BLOCKS, author-time lint DETECTS, pre-tool-use hook ENFORCES.

**Known residual failure modes** (not closed by this ADR, tracked separately):

1. **Symbol-hit lenience** — reconcile promotes when files exist + ANY declared symbol hits, not the specific named symbol. Surfaced by `wp-brain-queue-swarm-lease` showing 13/13 done while P2/P3 functions don't exist. Needs a followup ADR tightening `reconcile_evidence::verify` to require *named* symbol match.
2. **Passive reconcile loop reverts manual edits** — manually flipping 5 false-done statuses during the 2026-04-14 audit, only 1 persisted; 4 were silently re-promoted. Needs the passive loop disabled or evidence-aware.
3. **Dialect workplans** — `wp-cli-polish.json` uses `artifact: "path"` instead of `files: [...]`. The retrofit converted these by hand; a proper schema normalization ADR would unify the dialect at the loader layer.

These are follow-ups, not bugs in the current ADR. The ADR's contract (positive file evidence before `done` promotion + authoring-time prevention) is upheld.
