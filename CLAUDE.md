# hex — AI Operating System (AIOS)

hex is a microkernel runtime built on **hexagonal architecture** (Ports & Adapters) that orchestrates AI-driven development. It is installed **into** target projects — agents, hooks, skills, and templates exist to be instantiated elsewhere. Examples in `examples/` are consumer test beds, not hex itself.

This file is the kernel: hard rules and a skill-map. Everything else lives behind a slash command or `docs/reference/`. **Use skills first** — they load focused context without inflating every session.

## Skill-first discovery (use these before reading docs)

Common intents → skill triggers:

| Intent | Skill |
|---|---|
| Start a feature / decompose a workplan | `/hex-feature-dev` |
| Scaffold a new hex project | `/hex-scaffold` |
| Generate code inside one adapter | `/hex-generate` |
| Run architecture health check | `/hex-analyze-arch` |
| Pick a tech stack / runtime | `/hex-analyze-deps` |
| Validate a build (specs + property tests) | `/hex-validate` |
| Create / edit / approve a workplan | `/hex-workplan` |
| Coordinate a multi-agent swarm | `/hex-swarm` |
| Manage worktrees (setup / merge / cleanup) | `/hex-worktree` |
| Token-efficient code summary | `/hex-summarize` |
| Open / inspect the dashboard | `/hex-dashboard` |
| Configure inference providers + tier models | `/hex-inference` |
| SpacetimeDB module dev | `/hex-spacetime` |
| Publish a SpacetimeDB module | `/hex-publish-module` |
| Rebuild hex-nexus after asset/Rust changes | `/hex-dev-rebuild` |
| Speed up Rust builds (lld, sccache, nextest) | `/cargo-fast` |
| ADRs: create / search / review / status | `/hex-adr-create`, `/hex-adr-search`, `/hex-adr-review`, `/hex-adr-status` |

Full catalog: `docs/reference/skills-and-agents.md`. If no skill fits, check `docs/reference/README.md` for the long-form index. If neither exists for the need, **write a new skill** rather than appending to this file.

## Hard Rules (non-negotiable)

1. **Enqueue, never defer.** Outstanding work → `hex brain enqueue workplan|hex-command|shell`. "Next session" is a symptom of not using hex.
2. **No stub tasks.** Never `echo FIXME` / `echo TODO` enqueues — the CLI rejects them. Real work → workplan JSON; ideas → ADR or code comment.
3. **Rebuild release binaries after touching hex-cli / hex-nexus / hex-agent.** Run `cargo build --release` unprompted. Or invoke `/hex-dev-rebuild`.
4. **Use `hex worktree merge`, never `git checkout <branch> -- <file>`** (ADR-2604131930 — raw checkout silently drops parallel work).
5. **Prefer `hex hey <intent>`** for natural-language tasks — it routes + confirms destructive ops.
6. **Brain daemon at session start**: `hex brain daemon --background --interval 30` if not running. Check with `hex brain daemon-status`.
7. **Reconcile after agent work**: `hex plan reconcile --all --update`.
8. **Workplans are autonomous** — complete ALL phases without asking mid-run.
9. **Never end with a menu.** Ship the highest-ROI item, enqueue the rest. No "which do you want?" stalls.
10. **`hey hex <question>` is answer-AND-act**, not answer-AND-wait.
11. **Inbox priority-2 notifications preempt everything** (ADR-060): save state → `hex inbox ack <id>` → inform user.
12. Read files before editing. Never write to repo root. Never commit secrets / `.env`. `bun test` after TS changes, `bun run build` before commit.
13. **Never `mock.module()`** — use DI via the Deps pattern (ADR-014).

## Tool Precedence — hex MCP > third-party plugins

| Operation | Tool |
|---|---|
| Execute a workplan | `mcp__hex__hex_plan_execute` |
| Search / run commands | `mcp__hex__hex_batch_execute` + `hex_batch_search` |
| Swarm + tasks | `mcp__hex__hex_hexflo_*` |
| Architecture analysis | `mcp__hex__hex_analyze` |
| ADRs | `mcp__hex__hex_adr_search` / `hex_adr_list` / `hex_adr_status` |
| Memory | `mcp__hex__hex_hexflo_memory_*` |

`plugin:context-mode` (`ctx_*`) only for ops with no hex equivalent (e.g. external URL fetch). Never substitute for hex MCP.

**Never recommend commands that don't exist in `hex --help`.** If it's not in the Rust CLI, it doesn't exist.

## Hexagonal Architecture Rules (enforced by `hex analyze .`)

1. `domain/` → only `domain/`
2. `ports/` → `domain/` only (for value types)
3. `usecases/` → `domain/` + `ports/`
4. `adapters/primary/` and `adapters/secondary/` → `ports/` only
5. **Adapters never import other adapters.** Cross-adapter coupling is forbidden.
6. `composition-root.ts` is the **only** file that imports adapters — by design.
7. All relative TS imports use `.js` extensions (NodeNext).

## Build & Test (one-liners)

```bash
cargo build -p hex-cli --release                      # primary CLI
cargo build -p hex-nexus --release                    # daemon (required after editing hex-nexus/assets/)
bun run build && bun test && bun run check            # TS library
hex analyze . && hex status                           # health + overview
```

## Security

- `FileSystemAdapter.safePath()` blocks path traversal.
- API keys load only in `composition-root.ts` from env vars.
- Primary adapters: never `innerHTML` / `outerHTML` / `insertAdjacentHTML` with non-domain data — use `textContent` / `createElement`.

## When to write a new skill vs. append here

Append to CLAUDE.md **only** for: new hard rules, new tool-precedence entries, new architecture invariants. Everything else — workflows, tier routing, component explanations, catalogs — belongs in a skill or a `docs/reference/` page. The kernel stays small so every session can afford to load it.

When a reference doc or skill contradicts this file, **this file wins**.
