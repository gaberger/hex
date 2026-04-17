# hex — AI Operating System (AIOS)

## What This Project Is

hex is a microkernel-based **AIOS** built on **hexagonal architecture** (Ports & Adapters). It is installed INTO target projects to orchestrate AI-driven development — agents are the users, developers are the sysadmins.

Everything in this repo (hooks, skills, agents, statuslines, settings) is instantiated into a target project. `examples/` contains sample targets that consume hex as a dependency.

## System Components

| Component | Role |
|---|---|
| **SpacetimeDB** (required) | Coordination & state core. 7 WASM modules in `spacetime-modules/`. All clients connect via WebSocket. WASM can't access FS / spawn procs / make network calls — that's why hex-nexus exists. |
| **hex-nexus** (`hex-nexus/`) | Filesystem bridge daemon. Reads/writes files, runs tree-sitter analysis, manages git, syncs config → SpacetimeDB on startup (ADR-044), serves dashboard at `:5555`, exposes REST API. Editing `hex-nexus/assets/` requires `cd hex-nexus && cargo build --release`. |
| **hex-agent** (`hex-agent/`) | Architecture enforcement runtime. Must be present on any host running hex dev agents. Enforces hex rules via skills/hooks/ADRs/workplans. |
| **hex-dashboard** (`hex-nexus/assets/`) | Solid.js + Tailwind control plane. Multi-project, fleet, arch-health, inference monitoring. Real-time via SpacetimeDB subs. |
| **Inference** | `inference-gateway` + `inference-bridge` WASM modules route requests; hex-nexus makes actual HTTP calls. Model-agnostic (Anthropic, OpenAI, Ollama). |

### Standalone Mode (ADR-2604112000)

When `CLAUDE_SESSION_ID` is unset, hex-nexus uses `AgentManager` + `OllamaInferenceAdapter` — no Claude CLI needed. `hex doctor composition` diagnoses the active variant; `hex ci --standalone-gate` validates the path.

### Tiered Inference Routing (ADR-2604120202 + ADR-2604131630)

| Tier | Default Model | Use Case |
|------|--------------|----------|
| T1 | `qwen3:4b` | Scaffold/transform/script |
| T2 | `qwen2.5-coder:32b` | Standard codegen (adapters, tests) |
| T2.5 | `devstral-small-2:24b` | Complex reasoning (cross-adapter, architecture) |
| T3 | Claude (frontier) | Frontier tasks — bypasses scaffolded dispatch |

Tier is driven by WorkplanTask `strategy_hint` (`scaffold`/`transform`/`script` → T1, `codegen` → T2, `inference` → T2.5). T1/T2/T2.5 use best-of-N with a compile gate (`cargo check` / `tsc --noEmit`); T3 is single-shot. Override per-tier in `.hex/project.json` → `inference.tier_models`. Monitor with `hex inference escalation-report`.

## Tool Precedence (IMPORTANT)

**hex MCP tools take precedence over all third-party plugins.** Use `plugin:context-mode` (`ctx_*`) only for operations with no hex equivalent (e.g. external URL fetches).

| Operation | Tool |
|---|---|
| Execute workplan | `mcp__hex__hex_plan_execute` |
| Search / run commands | `mcp__hex__hex_batch_execute` + `hex_batch_search` |
| Swarm + tasks | `mcp__hex__hex_hexflo_*` |
| Architecture analysis | `mcp__hex__hex_analyze` |
| ADR search/list | `mcp__hex__hex_adr_search` / `hex_adr_list` |
| Memory | `mcp__hex__hex_hexflo_memory_*` |

## Behavioral Rules

### Autonomous Operation (HARD RULES)

1. **Enqueue, never defer to "next session".** Any outstanding work goes on the queue now:
   ```bash
   hex brain enqueue hex-command -- "worktree cleanup --force"
   hex brain enqueue workplan docs/workplans/wp-foo.json
   ```
2. **Rebuild release binaries after commits touching hex-cli/hex-nexus/hex-agent.** Run `cargo build --release` without asking.
3. **Use `hex worktree merge`, NEVER `git checkout <branch> -- <file>`** — raw checkout silently drops parallel-worktree code (ADR-2604131930).
4. **Prefer `hex hey <intent>`** over raw commands when the task maps to natural language.
5. **Start the brain daemon at session start** if not running: `hex brain daemon --background --interval 30`. Check with `hex brain daemon-status`.
6. **Reconcile workplans after agent work**: `hex plan reconcile --all --update`.
7. **Proactively seek improvements.** Noticed drift/gap → ADR → workplan → enqueue.
8. **Never end with a menu of options.** Ship the highest-ROI item now, enqueue the rest. Close with what shipped + what's queued. Per-item permission prompts stall autonomous sessions.
9. **`hey hex <question>` is answer-AND-act**, not answer-and-wait. Recommendation questions → analysis → apply rule 8.
10. **No `echo FIXME` stub tasks.** Real work → workplan JSON; not-yet-actionable → ADR or TODO comment. `hex brain enqueue shell` rejects these at the CLI.

### Legacy Rules

- **Workplans are autonomous** — complete ALL phases without pausing. Parallelize via HexFlo + background agents.
- **Inbox priority-2 notifications override current work** (ADR-060): stop → save state → `hex inbox ack <id>` → inform user. Checked by the `route` hook on every interaction.
- Do what's asked; nothing more, nothing less.
- ALWAYS read a file before editing it.
- NEVER save files to the root folder.
- NEVER commit secrets, credentials, or `.env` files.
- ALWAYS run `bun test` after code changes; `bun run build` before committing.
- NEVER `mock.module()` in tests — use the Deps pattern (ADR-014).

## Task Tier Routing (ADR-2604110227)

Every user prompt is classified by `hex-cli/src/commands/hook.rs::classify_work_intent` (run by `hex hook route` on `UserPromptSubmit`).

| Tier | Signal | Artifact |
|------|--------|----------|
| T1 Todo | Questions, trivial edits, confirmations | Claude `TodoWrite` — silent |
| T2 Mini-plan | Single-adapter work | One-line hook suggestion |
| T3 Workplan | Feature-sized / cross-adapter | Auto-invokes `hex plan draft` → `docs/workplans/drafts/draft-*.json` |

T3 auto-invocation creates a **draft stub only** — no worktrees, no agents, no specs, no commits. User promotes via `/hex-feature-dev`.

**Opt-outs**: `HEX_AUTO_PLAN=0` env • `.hex/project.json` → `workplan.auto_invoke.enabled: false` • `hex skip plan` in prompt • questions (`?`/how/why/what) are always T1 • trivial phrases (`fix typo`, `rename`, `add a comment`) are always T1.

```bash
hex plan draft <prompt>           # (normally auto-invoked)
hex plan drafts list | approve <name> | clear [--name N] | gc --days 7
```

## Hexagonal Architecture Rules (ENFORCED)

Checked by `hex analyze .` + the dead-code-analyzer agent:

1. `domain/` imports only `domain/`.
2. `ports/` imports `domain/` only (for value types).
3. `usecases/` imports `domain/` + `ports/` only.
4. `adapters/primary/` and `adapters/secondary/` import `ports/` only.
5. Adapters NEVER import other adapters.
6. `composition-root.ts` is the ONLY file that imports from adapters.
7. All relative imports MUST use `.js` extensions (NodeNext).

## File Organization

```
# Rust workspace (6 crates)
hex-cli/                 CLI binary — the canonical user entry point
hex-nexus/               FS-bridge daemon + dashboard (axum, :5555)
  src/analysis/            Tree-sitter, boundary checking
  src/coordination/        HexFlo swarm coordination (ADR-027)
  src/adapters/            SpacetimeDB + SQLite state adapters
  src/config_sync.rs       Repo → SpacetimeDB sync (ADR-044)
  src/git/ src/orchestration/
  assets/                  Solid.js dashboard (rust-embed)
hex-core/                Shared domain types & port traits (zero deps)
hex-agent/               Architecture enforcement runtime
hex-desktop/             Tauri wrapper for dashboard
hex-parser/              Code parsing utilities

# SpacetimeDB WASM modules (7 total — ADR-2604050900)
spacetime-modules/
  hexflo-coordination/     Swarms, tasks, agents, memory, fleet
  agent-registry/          Lifecycle + heartbeats + cleanup
  inference-gateway/       LLM routing
  secret-grant/            TTL-based key distribution
  rl-engine/ chat-relay/ neural-lab/

# TypeScript library
src/
  core/domain/ core/ports/ core/usecases/
  adapters/primary/ adapters/secondary/
  infrastructure/           Tree-sitter queries
  composition-root.ts       Single DI point
  cli.ts index.ts

# Embedded templates — baked into hex-cli & hex-nexus via rust-embed
hex-cli/assets/
  agents/hex/hex/  skills/  hooks/hex/  helpers/
  swarms/          mcp/     schemas/    templates/

# GENERIC-ONLY RULE: assets must NOT reference hex-intf internals
# (hex-nexus, hex-core, hex-parser, hex-desktop, spacetime-modules, /Volumes/).
# Enforced by `hex doctor` + `hex ci` (embedded-assets-generic check).

# Support
tests/{unit,integration}/   examples/   docs/{adrs,specs,workplans,analysis}/
.claude/{skills,agents}/    agents/  skills/  config/  scripts/
```

## Build & Test

```bash
# Rust (primary)
cargo build -p hex-cli --release
cargo build -p hex-nexus --release

# TypeScript library (secondary)
bun run build   # bundle to dist/
bun test        # unit + property + smoke
bun run check   # tsc --noEmit
```

**IMPORTANT**: Never recommend commands not in `hex --help`. If it's not in the Rust CLI, it doesn't exist.

### hex-nexus notes

- SpacetimeDB required (ADR-025). SQLite fallback (`~/.hex/hub.db`) for offline.
- Editing `hex-nexus/assets/*` → rebuild binary → restart daemon → hard-refresh browser (Cmd+Shift+R).
- Multi-instance: `ICoordinationPort` + FS locks + heartbeats (ADR-011).

## Development Pipeline (Specs-First)

1. **Decide** — ADR in `docs/adrs/` if new ports/adapters/external deps.
2. **Specify** — behavioral specs BEFORE code.
3. **Build** — follow hex rules.
4. **Test** — unit + property + smoke.
5. **Validate** — `hex analyze` + validation judge.
6. **Ship** — README + start scripts + commit.

## Feature Development Workflow

A "feature" decomposes inside-out across layers; each adapter boundary gets its own worktree.

```bash
/hex-feature-dev                                         # interactive (skill)
./scripts/feature-workflow.sh setup|status|merge|cleanup|list|stale <feature>
```

### Lifecycle (7 phases)

```
1 SPECS      behavioral-spec-writer → docs/specs/<feature>.json
2 PLAN       planner               → docs/workplans/feat-<feature>.json
3 WORKTREES  feature-workflow.sh setup
4 CODE       hex-coder agents (parallel, TDD)
5 VALIDATE   validation-judge (BLOCKING)
6 INTEGRATE  merge in dependency order → full suite
7 FINALIZE   cleanup, commit, report
```

**Conventions**: `feat/<feature>/<layer-or-adapter>` · max 8 concurrent · merge order: domain → ports → secondary → primary → usecases → integration · stale = >24h no commits.

### Dependency Tiers

| Tier | Layer | Depends On | Agent |
|------|-------|------------|-------|
| 0 | Domain + Ports | — | hex-coder |
| 1 | Secondary adapters | 0 | hex-coder |
| 2 | Primary adapters | 0 | hex-coder |
| 3 | Use cases | 0–2 | hex-coder |
| 4 | Composition root | 0–3 | hex-coder |
| 5 | Integration tests | All | integrator |

### Modes

- **Swarm** (default) — 2+ adapters, parallel worktrees.
- **Interactive** — critical features needing per-phase human review.
- **Single-agent** — small change inside one adapter.

## Skills & Agents

**Slash commands**: `/hex-feature-dev`, `/hex-scaffold`, `/hex-generate`, `/hex-summarize`, `/hex-analyze-deps`, `/hex-analyze-arch`, `/hex-validate`, `/cargo-fast`.

**Agents**: `feature-developer`, `planner`, `hex-coder`, `integrator`, `swarm-coordinator`, `dependency-analyst`, `dead-code-analyzer`, `scaffold-validator`, `behavioral-spec-writer`, `validation-judge`, `status-monitor`, `adversarial-reviewer`, `adr-reviewer`, `rust-refactorer`.

## Key Lessons (from adversarial review)

- **Tests can mirror bugs** — same LLM writes code + tests → tests encode the misunderstanding. Use property tests + behavioral specs as independent oracles.
- **"It compiles" ≠ "it works"** — always add runtime validation (can a user actually start the app?).
- **Browser TS needs a dev server** — any HTML + TS project MUST include Vite or equivalent.
- **Trace ALL consumers before deleting** (ADR-2604050900) — `grep` the ENTIRE workspace, not just the immediate directory. hex-agent was broken for a session because a workplan missed feature-gated imports.
- **Build gates between phases** — every phase that deletes/restructures ends with `cargo check --workspace`. A "done" workplan with a broken build is worse than no workplan.
- **Parallelize by file boundary, serialize by file overlap** — multiple agents editing the same file produce conflicting diffs. Batch or serialize.
- **Sign conventions matter** — for physics/math, document coordinate systems (e.g. `flapStrength` must be negative in screen coords).

## Swarm Coordination (HexFlo — ADR-027)

Native Rust coordination in `hex-nexus/src/coordination/` (`mod.rs`, `memory.rs`, `cleanup.rs`). State in SpacetimeDB via `hexflo-coordination` module; SQLite fallback.

```bash
hex swarm init <name> [topology]      hex swarm status
hex task create <swarm-id> <title>    hex task list
hex task complete <id> [result]
hex memory store <k> <v>              hex memory get <k>    hex memory search <q>
hex adr list|search|status|abandoned
hex inbox list|notify|ack             hex status            hex analyze .
hex nexus start|status
```

REST endpoints under `/api/swarms`, `/api/hexflo/*`. MCP tools (`mcp__hex__hex_*`) map 1:1 to CLI commands and delegate to the same nexus API.

**Heartbeats**: agents beat on every `UserPromptSubmit` via `hex hook route`. `stale` @ 45s, `dead` @ 120s (tasks reclaimed).

**Background agents**:
```
Agent: { subagent_type: "coder", mode: "bypassPermissions", run_in_background: true }
```

### Task State Sync (ADR-048)

Include `HEXFLO_TASK:{task_id}` in the subagent prompt. `hex hook subagent-start` reads stdin, PATCHes `/api/hexflo/tasks/{task_id}` with `agent_id` (→ `in_progress`). `hex hook subagent-stop` PATCHes again with result (→ `completed`). State persists in `~/.hex/sessions/agent-{CLAUDE_SESSION_ID}.json`. `agent_id` auto-resolves from that file.

## Declarative Swarm Behavior (ADR-2603240130)

Agent + swarm behavior is declared in YAML, not hardcoded. The supervisor reads YAMLs at startup.

- **Agent YAMLs** (`hex-cli/assets/agents/hex/hex/`, 14 files): model selection (tier/preferred/fallback/upgrade), context level (L1 AST → L3 full source), workflow phases, feedback loop gates (compile/lint/test), quality thresholds, I/O schemas. Schema varies by role — coders use `workflow.phases[]` + `feedback_loop`; planners use `workflow.steps[]` + `escalation`.
- **Swarm YAMLs** (`hex-cli/assets/swarms/`): participating agents, cardinality, parallelism, objectives, iteration limits. Available behaviors: `dev-pipeline`, `quick-fix`, `code-review`, `refactor`, `test-suite`, `documentation`, `migration`.
- **Embedding**: all templates in `hex-cli/assets/` are baked into hex-cli + hex-nexus via `rust-embed` and extracted during `hex init`.

## Security

- `FileSystemAdapter` path traversal protection via `safePath()`.
- API keys loaded only in `composition-root.ts` from env.
- Never commit `.env` — use `.env.example`.
- Primary adapters MUST NOT use `innerHTML`/`outerHTML`/`insertAdjacentHTML` with non-domain data. Use `textContent` or `createElement`.
