# Feature Development Workflow

Skills: `/hex-feature-dev`, `/hex-worktree`.

A "feature" in hex architecture is NOT a vertical slice. It decomposes inside-out across layers, with each adapter boundary getting its own git worktree for isolation.

## Specs-first pipeline

1. **Decide** — if new ports / adapters / external deps, write an ADR in `docs/adrs/`
2. **Specify** — behavioral specs BEFORE code (what "correct" looks like)
3. **Build** — codegen following hex architecture rules
4. **Test** — unit + property + smoke (3 levels)
5. **Validate** — `hex analyze` + validation judge
6. **Ship** — README + start scripts + commit

## How to start

```bash
/hex-feature-dev                                      # interactive skill
./scripts/feature-workflow.sh setup <feature>         # worktrees from workplan
./scripts/feature-workflow.sh status <feature>        # progress
./scripts/feature-workflow.sh merge <feature>         # merge in dep order
./scripts/feature-workflow.sh cleanup <feature>       # remove worktrees + branches
./scripts/feature-workflow.sh list                    # all feature worktrees
./scripts/feature-workflow.sh stale                   # abandoned worktrees
```

## 7-phase lifecycle

```
Phase 1: SPECS       behavioral-spec-writer → docs/specs/<feature>.json
Phase 2: PLAN        planner → docs/workplans/feat-<feature>.json
Phase 3: WORKTREES   feature-workflow.sh setup → one worktree per adapter
Phase 4: CODE        hex-coder agents (parallel, TDD) in isolated worktrees
Phase 5: VALIDATE    validation-judge → PASS/FAIL verdict (BLOCKING)
Phase 6: INTEGRATE   merge worktrees in dep order → run full suite
Phase 7: FINALIZE    cleanup worktrees, commit, report
```

## Worktree conventions

- Naming: `feat/<feature-name>/<layer-or-adapter>`
- Max concurrent: 8
- Merge order: domain → ports → secondary adapters → primary adapters → usecases → integration
- Always cleanup after successful merge
- Stale detection: >24h with no commits = flagged

## Dependency tiers

| Tier | Layer | Depends on | Agent |
|------|-------|------------|-------|
| 0 | Domain + Ports | Nothing | hex-coder |
| 1 | Secondary adapters | T0 | hex-coder |
| 2 | Primary adapters | T0 | hex-coder |
| 3 | Use cases | T0–2 | hex-coder |
| 4 | Composition root | T0–3 | hex-coder |
| 5 | Integration tests | Everything | integrator |

## Modes

| Mode | When |
|------|------|
| Swarm (default) | 2+ adapters — parallel worktrees |
| Interactive | Critical features needing human review each phase |
| Single-agent | Small changes within one adapter |
