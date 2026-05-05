# Component: hex-agent

## One-Line Summary

Architecture-enforcement runtime — the agent runtime for AI dev agents (local or remote). Enforces hex rules via skills, hooks, ADRs, and workplans.

> **Not** the same as the hexagonal "adapter" concept. "hex-agent" is the deployment unit; "adapter" is an architectural layer.

## Key Facts

- Required on any host running hex dev agents.
- Rust binary in `hex-agent/`.
- Runs alongside hex-nexus (often on the same host) but as a separate process.
- Owns: skill dispatch, hook execution, ADR/workplan enforcement, dev-agent supervision.
- Talks to SpacetimeDB for state and to hex-nexus for FS / git ops the agent itself shouldn't perform directly.
- Heartbeats to the `agent-registry` WASM module on every `UserPromptSubmit` (via `hex hook route`).

## What "enforcement" means here

The hexagonal architecture rules in `CLAUDE.md` are not advisory — they are checked by tooling that hex-agent is responsible for invoking on each phase of a workplan:

1. `domain/` imports only `domain/`.
2. `ports/` imports `domain/` only.
3. `usecases/` imports `domain/` + `ports/` only.
4. `adapters/{primary,secondary}/` import `ports/` only.
5. Adapters never import other adapters.
6. `composition-root.ts` is the only file that imports from adapters.
7. All relative imports MUST use `.js` extensions (NodeNext).

`hex analyze .` (driven by hex-nexus tree-sitter) reports violations; `dead-code-analyzer` agent flags orphans + unused exports.

## API Surface

`hex-agent` is invoked as a CLI:

| Command | Purpose |
|---------|---------|
| `hex-agent workplan <wp.json>` | Execute a single workplan task by id |
| `hex-agent workplan <wp.json> --background` | Background variant — be careful: SafeFileWriter (commit `0346eff8`) blocks hex-infra writes; adapter/domain files in the *target* project remain in scope |
| `hex-agent status` | Local agent runtime liveness |
| `hex-agent skills list` | Enumerate available skills |
| `hex-agent hook <name> [args...]` | Run a hook by name |

Most users never invoke `hex-agent` directly — `hex` CLI commands and HexFlo task dispatch are the normal entry points.

## Hooks (selected)

Hooks live in `hex-cli/assets/hooks/hex/` and are triggered by the harness on lifecycle events. Notable ones:

| Hook | Trigger | Behavior |
|------|---------|----------|
| `route` | `UserPromptSubmit` | Classifies work tier (T1/T2/T3), emits heartbeat, optionally drafts a T3 workplan |
| `subagent-start` | Subagent spawn | Reads `HEXFLO_TASK:{id}` from prompt → PATCH `/api/hexflo/tasks/{id}` → `in_progress` (ADR-048) |
| `subagent-stop` | Subagent completion | PATCH same task → `completed` with result |
| `inbox-check` | `UserPromptSubmit` | Priority-2 inbox notifications block current work (ADR-060) |

## Skills

Skills are slash-command surfaces (e.g. `/hex-feature-dev`, `/hex-scaffold`) declared as YAML in `hex-cli/assets/skills/`. The supervisor loads them at startup. See the `skills/` directory for the canonical list.

## Configuration

| Var | Default | Purpose |
|-----|---------|---------|
| `HEX_AGENT_ID` | auto-generated (UUID v4) | Agent identity; persisted in `~/.hex/sessions/` |
| `CLAUDE_SESSION_ID` | unset | Claude harness session id (when running under Claude Code) |
| `HEX_AGENT_BACKGROUND` | unset | Marks the run as background (engages SafeFileWriter guards) |

Project-level: `.hex/project.json` is read for tier-routing overrides and per-project agent constraints.

## Background-agent guardrails

> **Memory note**: a known failure mode — `hex-agent workplan <wp> --background` could satisfy literal-grep evidence by replacing whole files with stubs. Commit `0346eff8` introduced `SafeFileWriter` which blocks hex-infra writes. Adapter/domain files in *target* projects remain in scope and are still rewritable.

If you are running a background agent against the hex-intf repo itself, expect SafeFileWriter to refuse writes to `hex-cli/`, `hex-nexus/`, `hex-core/`, `hex-parser/`, `hex-desktop/`, and `spacetime-modules/`.

## Depends On

- **SpacetimeDB** — heartbeat + task state.
- **hex-nexus** — filesystem / git operations + outbound HTTP.
- **hex-core** — shared port traits + domain types.

## Depended On By

- **HexFlo coordination** — when a swarm dispatches a task, an `hex-agent` process picks it up.
- **hex-cli** — `hex feature dev` and friends spawn `hex-agent` runs.

## See also

- `docs/adrs/ADR-001-hexagonal-architecture.md` — the rules being enforced.
- `docs/adrs/ADR-027-hexflo-swarm-coordination.md` — task dispatch.
- `docs/adrs/ADR-048-task-state-sync.md` — `subagent-start`/`stop` hook contract.
- `docs/adrs/ADR-2603240130-declarative-swarm-behavior.md` — agent + swarm YAMLs.
- `docs/reference/system-architecture.md` — where hex-agent sits.
- `docs/reference/components/hex-nexus.md` — the bridge hex-agent calls into.
