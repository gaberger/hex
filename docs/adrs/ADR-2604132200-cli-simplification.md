# ADR-2604132200: CLI Simplification — Progressive Disclosure for an AIOS

**Status:** Proposed
**Date:** 2026-04-13
**Drivers:** hex has 40+ top-level CLI commands. Users can't discover or remember them. An AIOS should be ambient — the default experience should require zero knowledge of the command surface.

## Context

hex's CLI grew organically as features were added. Today `hex --help` shows 40+ commands. This is a power-user interface disguised as a user interface. The target user of hex is a developer who installs it and expects it to work — not someone who memorizes 40 subcommands.

ADR-2604131500 defined 4 UX layers (Pulse/Brief/Console/Override) but the CLI doesn't enforce them — every command is top-level, equally visible.

### Current state (40+ commands)

```
nexus, agent, secrets, brief, brain, stdb, swarm, task, inbox, memory,
neural-lab, adr, spec, project, analyze, plan, inference, chat, init,
hook, mcp, test, skill, enforce, assets, git, worktree, status, opencode,
dev, report, sandbox, fingerprint, doctor, context, validate, ci,
self-update, decide, new, override, pause, taste, trust, steer, brief
```

## Decision

### 1. `hex` with no args = `hex pulse`

Running `hex` alone shows the one-glance system pulse — not `--help`. Help is available via `hex --help` or `hex help`.

### 2. `hex go` = autonomous next action

New command that examines project state and takes the best next action:
- Stale binary? Rebuild.
- Pending workplan? Execute next phase.
- Failing tests? Show failures.
- Stale worktrees? Offer cleanup.
- Nothing to do? Say so.

This is the "I don't know what command to run" entry point.

### 3. Consolidate 40 commands into 8 top-level groups

| Command | Subcommands | Layer |
|---------|-------------|-------|
| `hex` (no args) | pulse | 1 |
| `hex go` | autonomous next action | 1 |
| `hex brief` | `--full`, `--since`, `--decisions-only` | 2 |
| `hex plan` | `create`, `list`, `status`, `execute`, `reconcile`, `resume`, `pause`, `draft` | 3 |
| `hex agent` | `list`, `connect`, `spawn`, `audit`, `swarm`, `task`, `memory`, `inbox` | 3 |
| `hex config` | `trust`, `taste`, `inference`, `enforce`, `settings`, `secrets` | 3 |
| `hex dev` | `analyze`, `validate`, `test`, `ci`, `init`, `new`, `report`, `worktree` | 3 |
| `hex doctor` | `validate` (brain), `composition`, `embedded-assets` | 3 |
| `hex override` | `steer`, `pause`, `resume`, `decide` | 4 |

### 4. Migration strategy — aliases, not removal

Old commands become hidden aliases that print a deprecation notice:
```
$ hex trust show
⚠ hex trust is now hex config trust — redirecting...
```

This prevents breakage while training muscle memory.

### 5. Help hierarchy

```
$ hex
⬡ hex pulse — all idle, A+ architecture, 271 tests passing

$ hex --help
hex — AI Operating System

  hex              System pulse (one-glance status)
  hex go           Do the next right thing
  hex brief        Structured summary of recent activity
  hex plan         Workplan lifecycle (create, execute, status)
  hex agent        Agent fleet management
  hex config       Trust, taste, inference, settings
  hex dev          Development tools (analyze, test, validate)
  hex doctor       System health and self-consistency
  hex override     Emergency controls (steer, pause, decide)

Run `hex <command> --help` for subcommand details.
```

## Consequences

**Positive:**
- New users see 9 commands, not 40
- `hex` alone does something useful (pulse)
- `hex go` eliminates "what do I run next?" friction
- Progressive disclosure: Layer 1-2 require zero CLI knowledge
- No breaking changes — old commands become aliases

**Negative:**
- Existing scripts using `hex trust show` need updating (eventually)
- Two-level subcommands (`hex config trust show`) are slightly more typing
- `hex go` needs good heuristics to be useful

**Mitigations:**
- Aliases persist for 6 months with deprecation notice
- Tab completion for subcommands
- `hex go` starts conservative (suggest, don't act) until trust is elevated

## Implementation

| Phase | Description | Status |
|-------|------------|--------|
| P1 | `hex` with no args → `hex pulse` | Pending |
| P2 | `hex go` command with state-based next-action | Pending |
| P3 | Group config commands (trust, taste, inference, enforce, secrets) | Pending |
| P4 | Group dev commands (analyze, validate, test, ci, worktree) | Pending |
| P5 | Group override commands (steer, pause, decide) | Pending |
| P6 | Deprecation aliases for old top-level commands | Pending |
| P7 | Updated help hierarchy + tab completion | Pending |

## References

- ADR-2604131500: AIOS Developer Experience (4-layer UX)
- ADR-2604131945: Brain Self-Consistency Daemon (`hex doctor validate`)
- Session 2026-04-13: 40+ commands observed as usability barrier
