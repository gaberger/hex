# ADR-2604061100: hex CI Enforcement — Done-Conditions, `hex ci`, and Deployed Workflows

**Status:** Accepted  
**Date:** 2026-04-06  
**Drivers:** During ADR-2604021215 implementation, `hex stdb query` was specified in the workplan with a `done_condition` but was never implemented. The condition was never verified. The binary shipped without the feature. hex's specs-first pipeline only prevents drift if done-conditions are machine-enforced, not human-reviewed.

## Context

hex workplans define `done_condition` per step — a human-readable description of what "done" means:

```json
{
  "id": "step-2",
  "done_condition": "hex stdb query 'SELECT * FROM project' prints formatted table"
}
```

These conditions are currently **documentation only**. The workplan executor marks steps complete based on agent exit status, not on whether the condition holds. This creates a class of silent drift: workplan says done, binary disagrees.

The same gap exists at the CI level. hex deploys hooks, agents, and skills to target projects but provides no GitHub Actions workflow. Projects have no automated gate that runs after every push.

There are three distinct enforcement gaps:

1. **Step-level**: `done_condition` not verified after agent completes
2. **Project-level**: No `hex ci` command for CI systems to call
3. **Deployment-level**: No CI workflow template shipped with `hex init`

## Decision

### Gate 1: Done-Condition Verification in Workplan Executor

`done_condition` strings that match a runnable pattern are executed as shell commands after each step completes. A step is only marked `completed` if its done-condition exits zero.

**Runnable pattern**: the condition string starts with a known CLI prefix (`hex `, `cargo `, `bun `, `npm `) or the workplan step provides an explicit `done_command` field alongside `done_condition`.

```json
{
  "id": "step-2",
  "done_condition": "hex stdb --help shows query subcommand",
  "done_command": "hex stdb --help | grep -q query"
}
```

When `done_command` is present, the executor runs it after the agent completes. Non-zero exit sets step status to `failed` with the message `done_condition not met: <condition text>`.

When `done_command` is absent, the condition is treated as documentation (current behavior preserved for backward compatibility).

**Implementation**: `workplan_executor.rs` — add `verify_done_condition()` called from `run_step()` after agent returns success.

### Gate 2: `hex ci` Command

A new top-level command that runs all enforcement gates in sequence:

```
hex ci [--workplan <path>] [--fix]
```

Gate sequence (all must pass):

| Gate | Command | Blocking |
|------|---------|---------|
| Architecture boundaries | `hex analyze .` | yes |
| ADR rule compliance | `hex enforce list` | yes |
| Workplan done-conditions | sweep `docs/workplans/*.json` for `done_command` fields, run each | yes |
| Spec coverage | verify every workplan step references at least one spec ID | yes |

Exit code: 0 if all pass, 1 if any fail. Outputs a structured summary:

```
⬡  hex ci
  ✓ Architecture: 0 violations
  ✓ ADR rules: 12 rules, 0 violations  
  ✗ Workplan gates: 1 failed
      feat-spacetimedb-direct-query-cli / step-2: done_condition not met
        command: hex stdb --help | grep -q query
        exit: 1
  ✓ Spec coverage: 34 specs, all referenced
  
Exit 1
```

**Implementation**: new `hex-cli/src/commands/ci.rs`, wired into `main.rs`.

### Gate 3: CI Workflow Template

A GitHub Actions workflow template is added to `hex-cli/assets/ci/hex-ci.yml` and embedded via `rust-embed`. `hex init` extracts it to `.github/workflows/hex-ci.yml` in target projects.

```yaml
name: hex CI
on: [push, pull_request]
jobs:
  hex-ci:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Install hex
        run: curl -fsSL https://hex.sh/install | bash
      - name: Run hex CI gates
        run: hex ci
```

Projects can extend the template but `hex ci` remains the canonical gate.

## Consequences

**Positive:**
- Workplan drift caught at step completion, not at user review
- CI has a single entry point (`hex ci`) instead of ad-hoc script chains
- Every project initialized with hex gets enforcement gates on every push
- `done_command` is additive — existing workplans without it continue to work

**Negative:**
- `done_command` must be maintained alongside `done_condition` — two fields per step
- CI gate adds ~5-30s per push depending on workspace size
- Workplan executor changes require careful testing — false positives mark valid steps failed

## Implementation

| Change | File | Description |
|--------|------|-------------|
| Done-condition runner | `hex-nexus/src/orchestration/workplan_executor.rs` | Add `verify_done_condition()` called after agent completes a step |
| `done_command` field | `hex-core/src/domain/workplan.rs` | Add `done_command: Option<String>` to `WorkplanTask` |
| `hex ci` command | `hex-cli/src/commands/ci.rs` | New command — runs all 4 enforcement gates |
| Wire `hex ci` | `hex-cli/src/main.rs` | Add `Ci` variant to `CliCommand` enum |
| CI workflow template | `hex-cli/assets/ci/hex-ci.yml` | GitHub Actions template, deployed by `hex init` |
| Embed CI template | `hex-cli/src/assets.rs` | Add CI template to rust-embed Assets struct |

## Workplan

`docs/workplans/feat-hex-ci-enforcement.json`

## References

- ADR-2604051700: Enforce Workplan Gates (introduced gate infrastructure in executor)
- ADR-2604021215: SpacetimeDB direct query CLI (the feature whose drift triggered this ADR)
- ADR-046: Workplan Execution Engine
