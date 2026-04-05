---
name: hex-worktree
description: Manage git worktree lifecycle for hex feature development. Use when the user asks to "create worktrees", "setup worktrees", "merge worktrees", "cleanup worktrees", "list worktrees", "worktree status", or "feature branches".
---

# Hex Worktree — Git Worktree Lifecycle for Hex Features

In hex architecture, each adapter boundary gets its own git worktree during feature development. This skill manages the full worktree lifecycle: creation, status tracking, merge ordering, and cleanup.

## Why Worktrees?

Hexagonal architecture enforces strict import boundaries between layers. By giving each adapter its own worktree, hex ensures:
- Adapters cannot accidentally import each other (they're in separate directories)
- Parallel development — multiple agents can code different adapters simultaneously
- Clean merge ordering — domain/ports merge first, then adapters, then integration

## Parameters

Ask the user for:
- **action** (required): One of: setup, status, merge, cleanup, list, stale
- **feature_name** (required for setup/status/merge/cleanup): The feature identifier
- **skip_specs** (optional, default: false): Bypass specs requirement (emergency hotfixes only)

## Action: setup

Create worktrees from an existing workplan.

### Prerequisites
1. Behavioral specs must exist at `docs/specs/<feature-name>.json` (enforced by hex-specs-required hook)
2. Workplan must exist at `docs/workplans/feat-<feature-name>.json`

### Steps

1. Read the workplan:
```bash
cat docs/workplans/feat-<feature-name>.json
```

2. For each step in the workplan, create a worktree:
```bash
# Domain/ports changes (Tier 0)
git worktree add ../hex-feat-<feature>-domain feat/<feature>/domain
git worktree add ../hex-feat-<feature>-ports feat/<feature>/ports

# Secondary adapters (Tier 1)
git worktree add ../hex-feat-<feature>-<adapter> feat/<feature>/<adapter>

# Primary adapters (Tier 2)
git worktree add ../hex-feat-<feature>-<adapter> feat/<feature>/<adapter>

# Integration (Tier 5)
git worktree add ../hex-feat-<feature>-integration feat/<feature>/integration
```

3. Register worktrees in HexFlo memory:
```tool
mcp__hex__hex_hexflo_memory_store({
  key: "feature/<feature>/worktrees",
  value: {
    feature: "<feature>",
    worktrees: [
      { branch: "feat/<feature>/domain", path: "../hex-feat-<feature>-domain", tier: 0, status: "active" },
      ...
    ],
    created_at: "ISO timestamp"
  }
})
```

4. If `scripts/feature-workflow.sh` exists, prefer using it:
```bash
./scripts/feature-workflow.sh setup <feature-name>
```

## Action: status

Show the current state of all worktrees for a feature.

### Steps

1. List all worktrees:
```bash
git worktree list
```

2. Filter to the feature's worktrees (branches matching `feat/<feature>/`)

3. For each worktree, report:
   - Branch name and path
   - Tier (from workplan)
   - Commits ahead of main: `git log main..feat/<feature>/<adapter> --oneline`
   - Test status: run `bun test` in the worktree (if requested)
   - Merge readiness: all lower-tier worktrees merged?

4. Cross-reference with HexFlo task status:
```tool
mcp__hex__hex_hexflo_task_list()
```

## Action: merge

Merge worktrees in dependency order.

### Merge Order (ENFORCED)

```
Tier 0: domain → ports
Tier 1: secondary adapters (parallel within tier)
Tier 2: primary adapters (parallel within tier)
Tier 3: usecases
Tier 4: composition root
Tier 5: integration tests
```

### Steps

1. Verify merge preconditions for each worktree (hex-merge-validation hook):
   - Tests pass in the worktree
   - All lower-tier worktrees already merged
   - No cross-adapter imports
   - Commit conventions followed

2. Merge in order:
```bash
git checkout main
git merge feat/<feature>/domain --no-ff -m "feat(domain): <feature> domain types"
git merge feat/<feature>/ports --no-ff -m "feat(ports): <feature> port contracts"
# ... secondary adapters ...
# ... primary adapters ...
git merge feat/<feature>/integration --no-ff -m "test(<feature>): integration tests"
```

3. After each merge, run the test suite:
```bash
bun run check && bun test
```

4. If conflicts arise:
   - Resolve manually (adapters should not conflict with each other)
   - Port/domain conflicts indicate a planning error — escalate to planner agent

5. If `scripts/feature-workflow.sh` exists:
```bash
./scripts/feature-workflow.sh merge <feature-name>
```

## Action: cleanup

Remove worktrees and branches after successful merge.

### Steps

1. Verify all worktrees are merged:
```bash
git branch --merged main | grep "feat/<feature>/"
```

2. Remove worktrees:
```bash
git worktree remove ../hex-feat-<feature>-domain
git worktree remove ../hex-feat-<feature>-ports
# ... all worktrees ...
```

3. Delete branches:
```bash
git branch -d feat/<feature>/domain
git branch -d feat/<feature>/ports
# ... all branches ...
```

4. Clean up HexFlo memory:
```tool
mcp__hex__hex_hexflo_memory_store({
  key: "feature/<feature>/worktrees",
  value: { status: "cleaned", cleaned_at: "ISO timestamp" }
})
```

5. If `scripts/feature-workflow.sh` exists:
```bash
./scripts/feature-workflow.sh cleanup <feature-name>
```

## Action: list

List all active feature worktrees across all features.

```bash
git worktree list
```

Group by feature name and show tier/status for each.

## Action: stale

Find abandoned worktrees (older than 24h with no new commits).

```bash
# List worktrees and check last commit date
for wt in $(git worktree list --porcelain | grep "^worktree" | cut -d' ' -f2); do
  last_commit=$(git -C "$wt" log -1 --format="%ci" 2>/dev/null)
  echo "$wt: $last_commit"
done
```

Flag worktrees with no commits in 24+ hours. Recommend cleanup or reassignment.

## Worktree Limits

- **Maximum 8 concurrent worktrees** per feature (matches max parallel agents)
- Worktrees are created in the parent directory of the project root (../hex-feat-*)
- Each worktree gets a full copy of the repo — disk space scales linearly

## Agent Worktree Anti-Patterns (ADR-2604050900 Learnings)

### Anti-Pattern: Multiple Agents, Same Target File

When multiple agents in separate worktrees all edit the same file (e.g., appending sections
to `lib.rs`), each produces a valid diff against the *base* version — but the diffs conflict
when merged because they all modify the same region (end of file).

**Symptom**: Cherry-picks fail; manual rework required; agent time wasted.

**Fix**: Batch all edits to a shared file into a single agent. Only use worktree isolation
when agents modify *disjoint file sets*.

### Anti-Pattern: Worktree Branches Diverge from Feature Branch

Worktree agents fork from whatever commit exists when the worktree is created. If you've
made commits on the feature branch since then, the worktree agent's branch diverges.

**Symptom**: `git cherry-pick` shows unexpected conflicts on unrelated code.

**Fix**: Include explicit `git fetch origin && git checkout <feature-branch>` as the
first instruction in every worktree agent prompt.

### Pattern: Good Worktree Parallelism

Tasks that touch **separate directories or separate files** parallelize perfectly:

```
# GOOD: each agent touches different dirs
Agent A: spacetime-modules/ (delete dirs, update Cargo.toml)
Agent B: hex-nexus/src/adapters/ (modify adapter files)
Agent C: hex-cli/src/commands/ (add CLI subcommand)
```

### Pattern: Sequential for Shared Files

When tasks must touch the same file, run them sequentially in the main context.
The coordination overhead of worktrees exceeds the time saved by parallelism
for <3 small tasks on a shared file.

## Quick Reference

| Command | What it does |
|---------|-------------|
| `/hex-worktree setup <feature>` | Create worktrees from workplan |
| `/hex-worktree status <feature>` | Show worktree status and merge readiness |
| `/hex-worktree merge <feature>` | Merge worktrees in dependency order |
| `/hex-worktree cleanup <feature>` | Remove worktrees and branches |
| `/hex-worktree list` | List all active worktrees |
| `/hex-worktree stale` | Find abandoned worktrees |
