# Plan — hex → solo: strip to a software-engineering agent

**Date:** 2026-08-24 · **Author:** survey + plan, session `569f19b1`
**Question asked:** refactor (possibly fork) hex into a purely software-engineering agent —
no clustering, no coordination, just an inference server writing quality hex-architected code.
What can be stripped, starting with SpacetimeDB?

---

## 0. Verdict first

**Don't fork to get a clean build — the build is already clean.** Measured today:

| Crate | `cargo check` |
|---|---|
| hex-core, hex-parser, hex-git, hex-state, hex-exec, hex-graph, hex-analysis, hex-analyzer | 0 errors, 0 warnings |
| hex-nexus | 0 errors |
| hex-nexus `--no-default-features` (STDB off) | **0 errors** |
| hex-cli | 0 errors, 32 warnings |
| hex-agent | 0 errors |
| hex-desktop | fails — missing **system** libs (`gdk-3.0`, `dbus-1`), not code |

The only broken thing in the workspace is Tauri's system dependencies. `cargo check --workspace`
dies there and takes the whole workspace down with it, which is probably what "we can't get a
clean build" feels like from the outside. Dropping `hex-desktop` from `[workspace] members`
fixes that in one line.

So the problem is **not breakage. It's mass and daemon-mediation.**

**Recommendation: a long-lived branch (`epoch/solo`) off `main`, not a fork.** Reasons:
- `.git` is 93 MB across 2,978 commits — history is not the weight.
- The 264 ADRs are the project's ledger and the bench corpus in `docs/benchmarks/` is the only
  empirical basis for model selection. A clean fork throws away provenance you'd immediately
  want back.
- The work is ~75% deletion. Git is good at deletion. A fork buys nothing a `git rm` doesn't.
- Keeping one repo means `main` stays runnable while `epoch/solo` proves itself.

---

## 1. What the survey found

### 1.1 The mass

~261k LOC of Rust + ~35k LOC of dashboard TypeScript + 264 ADRs + 584 transitive crates + 23 GB `target/`.

| Crate | LOC | Files |
|---|---:|---:|
| hex-nexus | 96,843 | 317 |
| hex-cli | 92,582 | 151 |
| hex-agent | 23,383 | 114 |
| spacetime-modules (11 WASM) | 13,165 | 11 |
| hex-exec | 9,048 | 32 |
| hex-core | 8,520 | 67 |
| hex-analysis | 4,683 | 13 |
| hex-analyzer | 4,500 | 15 |
| hex-state | 3,252 | 4 |
| hex-graph | 2,397 | 12 |
| hex-git | 1,202 | 7 |
| hex-parser | 1,115 | 8 |
| hex-desktop | 782 | 7 |
| **dashboard TS** (`hex-nexus/assets`) | **35,096** | 263 |

Transitive crate counts: `hex-core` 34 · `hex-exec` 139 · `hex-cli` 244 · `hex-nexus` 363 · workspace 584.

### 1.2 Three findings that make this cheap

**(a) The canonical loop needs zero daemon state.**
`hex-nexus/src/routes/mod.rs:75`:

```rust
async fn direct_execute(
    Json(task): Json<crate::direct_exec::DirectTask>,
) -> Json<crate::direct_exec::DirectResult> {
    Json(crate::direct_exec::execute_direct(task).await)
}
```

No `State(...)` extractor. The entire `hex do` path — the thing ARCHITECTURE.md calls the
canonical execution model — is a pure pass-through. The daemon adds a localhost HTTP hop and
nothing else.

**(b) The dependency graph already points the right way.**

```
hex-core   (leaf, 0 hex deps)
hex-graph, hex-git, hex-analysis, hex-analyzer, hex-parser  (leaves)
hex-state  → hex-core
hex-exec   → hex-core, hex-graph, hex-git
hex-cli    → hex-core, hex-exec, hex-graph          ← does NOT depend on hex-nexus
hex-nexus  → hex-core, hex-graph, hex-analysis, hex-git, hex-exec, hex-state
hex-agent  → hex-core, hex-nexus                    ← nothing depends on hex-agent
hex-desktop→ hex-nexus
```

`hex-cli` already links `hex-exec` directly. `hex do` can become an in-process call today.

**(c) The SpacetimeDB seam already exists and already compiles.**
`spacetimedb` is an optional cargo feature on `hex-nexus`, `hex-agent`, and `hex-state`
(default-on). `cargo check -p hex-nexus --no-default-features` returns **0 errors**. And
`hex-exec` has **no** `spacetimedb-sdk` dependency at all — its 8 STDB touchpoints are plain
HTTP SQL calls behind `HEX_SPACETIMEDB_HOST`, swappable for a local store.

### 1.3 The one real blocker

**`hex-exec` calls back into the daemon for inference.**

```
hex-exec/src/direct_exec.rs:771   http://127.0.0.1:{port}/api/inference/complete
hex-exec/src/direct_react.rs:131  http://127.0.0.1:{port}/api/inference/complete
```

The provider adapters, tier routing, best-of-N, and the compile gate all live inside nexus:

| Location | LOC | What |
|---|---:|---|
| `hex-nexus/src/routes/inference.rs::inference_complete` | 1,033 | provider select, tiering, best-of-N, LoRA augment, tool fast-path |
| `hex-nexus/src/adapters/inference/ollama.rs` | 564 | Ollama |
| `hex-nexus/src/adapters/inference/claude_code.rs` | 500 | `claude -p` frontier |
| `hex-nexus/src/adapters/inference_router/` | 357 | tier config + routing |
| `hex-agent/src/adapters/secondary/openai_compat.rs` | 582 | OpenAI-compatible (salvage) |
| `hex-agent/src/adapters/secondary/anthropic.rs` | 433 | Anthropic direct (salvage) |

**This extraction is the whole critical path.** Everything else in this plan is `git rm`.

### 1.4 There are two agent implementations

- `hex-exec/` (9,048 LOC) — `direct_exec` / `direct_react` / `simple_agent` + ~18 guarded tools
  (`code_patch`, `typescript_check`, `workspace_boundary_check`, `adr_draft`, `dep_audit`,
  `secret_scan`, `delegate`, `cost_meter`, `web_search`, …). **This is the keeper** — it's what
  ARCHITECTURE.md declares canonical and where every recent commit lands.
- `hex-nexus/src/orchestration/agent_loop/` (2,506 LOC) — the older SOP loop, 4 tools
  (`repo_read`, `repo_grep`, `cargo_check`, `code_patch_propose`). Superseded.
- `hex-agent/` (23,383 LOC) — a *third* loop with its own conversation usecase and its own
  provider adapters. Nothing depends on it. Salvage the two adapters, delete the rest.

### 1.5 You already decided this

ARCHITECTURE.md, current epoch `hybrid-inference`:

> The current design is **one strong agent loop fed by tools, code-graph context, and memory** —
> *not* a simulated organization of many agents.

ADR-2606061359 retired the org-sim. ADR-2606071500 declared the ReAct loop. **The decision is
made; the code was never removed.** This plan is not a new direction — it is executing a
transition declared 2.5 months ago. That is why so much of it is deletion rather than design.

---

## 2. Target state

**One binary. No daemon. No database. No dashboard. No network peers.**

```
hex do "<task>" --file <f> --evidence "<cmd>"
   ├─ hex-graph      → code-graph context + ranked lessons (local graph-out/graph.json)
   ├─ hex-exec       → ReAct loop over guarded tools, in-process
   ├─ hex-infer      → provider adapters (Ollama / OpenAI-compat / Anthropic / claude -p)
   │                    + tier routing + best-of-N + compile gate      ← NEW CRATE
   ├─ hex-analysis   → hexagonal boundary enforcement (the quality bar)
   └─ hex-git        → apply → evidence gate → commit iff exit 0
```

Local state = files on disk (`.hex/`, `graph-out/`, `docs/`). Nothing else.

### Projected size

| | Today | Target |
|---|---:|---:|
| Rust LOC | ~261,000 | **~60,000** |
| Dashboard TS | 35,096 | 0 |
| Workspace crates | 13 | 8 |
| Transitive deps | 584 | ~180 |
| Processes to run | 3 (nexus + STDB + CLI) | **1** |
| Binaries | 4 | 1 |

---

## 3. Keep / strip

### KEEP (~60k LOC)

| Crate | LOC | Why |
|---|---:|---|
| `hex-core` | 8,520 → ~5,000 | Domain types + port traits. Trim: `ports/coordination.rs`, `ports/state.rs` (1,088), `domain/brain.rs`, `ports/experiment.rs`. |
| `hex-exec` | 9,048 | The agent loop + tool library. Untouched except the inference call site. |
| `hex-infer` | ~2,500 new | **Extracted** from nexus routes/adapters + hex-agent adapters. |
| `hex-graph` | 2,397 | Code-knowledge graph — the context quality differentiator. |
| `hex-analysis` | 4,683 | Tree-sitter hexagonal enforcement. This *is* the "quality hex-based code" claim. |
| `hex-analyzer` | 4,500 | god-types / cohesion / duplication / dead-layer. Fold into `hex-analysis`. |
| `hex-git` | 1,202 | libgit2 plumbing. |
| `hex-parser` | 1,115 | Parsing. |
| `hex-cli` | 92,582 → ~30,700 | See 3.1. |

### STRIP (~200k LOC)

| Target | LOC | Note |
|---|---:|---|
| `hex-nexus` | 96,843 | Salvage `adapters/inference/*` (1,079) and `routes/inference.rs::inference_complete` into `hex-infer`, then delete the crate. |
| `hex-nexus/assets` dashboard | 35,096 TS | Solid.js control plane. Whole point is watching a fleet. |
| `hex-agent` | 23,383 | Third agent loop. Salvage `openai_compat.rs` + `anthropic.rs` (1,015) into `hex-infer`. Nothing depends on it. |
| `spacetime-modules/` | 13,165 | All 11 WASM modules: hexflo-coordination (7,819), inference-gateway, agent-registry, secret-grant, rl-engine, chat-relay, neural-lab, knowledge-graph, agent-comms. |
| `hex-state` | 3,252 | The STDB state adapter. Its reason to exist goes with STDB. |
| `hex-desktop` | 782 | Tauri wrapper for the dashboard. **Also the only thing failing the workspace build.** |
| `hex-cli/src/pipeline/` | 16,850 | The retired SOP/persona pipeline (supervisor 3,646 · code_phase 2,701 · validate_phase 1,733 · swarm_phase · objectives · model_selection). Only 3 shallow consumers. |
| `hex-cli/src/tui/` | 6,589 | Ratatui chat UI. Only `dev.rs` and `chat.rs` reference it. |
| `hex-nexus/src/orchestration/agent_loop/` | 2,506 | Superseded by hex-exec. |
| CLI commands (3.1) | ~34,500 | |

### 3.1 CLI verb triage

66 top-level verbs today; 52 of 91 command files call the nexus daemon.

**Keep (~28.6k LOC of `commands/`):**
`do` · `analyze` · `graph` · `bench` · `inference` · `verify` · `dev` · `ci` · `test` ·
`init` · `new` · `worktree` · `git` · `fs` · `docs` · `spec` · `assets` · `skill` ·
`self-update` · `status` · `go` · `hey` · `readme` · `plan/` · `adr/` · `doctor/` · `bootstrap/` · `swarm/`

**Strip (~34.5k LOC):**

| Verb | LOC | Why |
|---|---:|---|
| `sched` + `sched/` | 10,122 | Background scheduler daemon. No daemon → no scheduler. |
| `mcp` | 1,794 | MCP server. Re-add later as a thin shim over the in-process loop if wanted. |
| `persona_prompt` | 1,276 | Org-sim residue (retired by ADR-2606061359). |
| `report` | 1,237 | Fleet/swarm reporting. |
| `nexus` | 1,216 | Daemon lifecycle. |
| `stdb` | 1,090 | SpacetimeDB console. |
| `adr_review` | 992 | Merge into `adr/`. |
| `project` | 705 | Multi-project registry. |
| `substrate` | 664 | Composition-swap governance (Layer 6). |
| `enforce` | 584 | STDB-synced enforcement modes → make file-local. |
| `opencode` | 549 | External tool integration. |
| `chat` | 546 | Chat relay. |
| `neural_lab` | 518 | Experiment orchestration. |
| `ops`, `secrets`, `monitor`, `trust`, `task`, `taste`, `inbox`, `pool`, `insight`, `memory`, `sandbox`, `pulse`, `fingerprint`, `context`, `steer`, `pause`, `decide`, `override`, `deploy`, `interview`, `agent_audit`, `refresh`, `plan_health` | ~7,700 | All daemon/fleet/coordination-mediated. |

**Two judgement calls flagged:**

1. **`hex swarm build` / `hex swarm review` — KEEP, rename.** You said "no clustering, no
   coordination." This isn't either. `hex-exec/src/adversarial.rs` (475 LOC) fans out
   *inference calls* in one process — N divergent designs → red-team → synthesize → build to a
   test gate; and parallel reviewers → skeptical default-refute verification → gated fixes. Per
   ARCHITECTURE.md it built a ~2,900-LOC durable job queue from a one-line spec and its
   adversarial pass found 6 real bugs the build's own passing tests missed. That is the
   quality-production mechanism, not the distribution mechanism. Suggest renaming to
   `hex build` / `hex harden` so "swarm" stops implying a cluster.
2. **`hex memory` — keep the idea, drop the backend.** Graph-ranked lesson retrieval is a
   context-quality feature worth keeping. Re-point it from STDB at a local JSONL/SQLite store.

---

## 4. Phases

Each phase ends green on `cargo check --workspace` — never leave the tree broken between phases
(the repo's own hard-won lesson: *"a 'done' workplan with a broken build is worse than no
workplan"*).

### Phase 0 — Unblock the workspace build (1 line, do it on `main` today)
Remove `hex-desktop` from `[workspace] members` in `Cargo.toml`.
**Gate:** `cargo check --workspace` exits 0 without gtk3/dbus system packages.

### Phase 1 — Extract `hex-infer` ⚠️ **the only real engineering**
New crate `hex-infer`, deps: `hex-core` + reqwest + tokio only.
1. Move `hex-nexus/src/adapters/inference/{ollama,claude_code}.rs` (1,079 LOC).
2. Move `hex-agent/src/adapters/secondary/{openai_compat,anthropic}.rs` (1,015 LOC).
3. Lift `routes/inference.rs::inference_complete` (lines 66–1099) into a
   `complete(req) -> Result<Completion>` library function. Drop the 18 `State(...)` extractions —
   config comes from `.hex/project.json` (`inference.tier_models`, `inference.react_models`), which
   is where it's already declared.
4. Keep: tier routing, best-of-N + compile gate, `claude -p` frontier fallback.
   Drop: LoRA augment (ADR-2606161300 is Proposed, not built), STDB inference logging, rate
   limiter, `/v1` OpenAI + Anthropic proxy shims, calibration endpoints.
5. Implement `hex_core::ports::inference::IInferencePort`. **G1 requires no consumer names a
   provider** — this is the enforcement point.
6. Repoint `hex-exec/src/direct_exec.rs:771` and `direct_react.rs:131` from HTTP loopback to the
   port.

**Gate:** `hex do` completes an evidence-gated task with nexus **stopped**. This is the proof the
whole plan rests on — do not proceed to Phase 2 until it passes.

### Phase 2 — Sever hex-exec's STDB touchpoints
8 call sites (`code_patch.rs:235`, `adr_draft.rs:114`, `spec_draft.rs:103`, `workplan_emit.rs:200`,
`adr_status_set.rs:147`, `escalate_to_operator.rs:67`, `cost_meter.rs:72`, `direct_exec.rs:226`).
All plain HTTP SQL. Replace with a `LocalStore` (SQLite at `~/.hex/hex.db` or JSONL) behind a
narrow port. The agent-run feed becomes a local file.
**Gate:** `grep -ri spacetime hex-exec/src` returns only comments.

### Phase 3 — Delete the daemon tier
`git rm -r hex-nexus hex-agent hex-desktop hex-state spacetime-modules`, and
`hex-cli/src/{pipeline,tui}`. Drop from `[workspace] members`.
**Trace every consumer before each delete** — ADR-2026-04-05-0900's lesson was that a missed
feature-gated import broke hex-agent for a session. Use `hex graph consumers <path>` per
ADR-2606071713; it exists for exactly this.
**Gate:** `cargo check --workspace` 0 errors. `cargo build --release` produces one binary.

### Phase 4 — CLI triage
Delete the strip-list verbs from `main.rs` + `commands/`. Fold `hex-analyzer` into `hex-analysis`.
Repoint the ~52 nexus-calling command files that survive (`analyze`, `graph`, `adr`, `plan`,
`doctor`, `bench`, `inference`) from `NexusClient` to in-process library calls.
**Gate:** `hex --help` fits on one screen. Every listed verb runs with no daemon.

### Phase 5 — Prove it
1. `hex bench agentic` — re-run the corpus in `docs/benchmarks/`; pass-rate must not regress.
   Model selection was chosen empirically and external leaderboards demonstrably don't predict
   agentic-loop performance here.
2. `hex analyze .` — the fork must pass its own analyzer. nexus scored **F (30/100)** before its
   split; the solo binary has no excuse.
3. Build one example end-to-end (`examples/lru-clean` or `ratelimiter-clean`) from a one-line spec.
4. Rewrite `ARCHITECTURE.md`; open the `solo` epoch ADR.

---

## 5. Resolve before Phase 3 — governance

### 5.1 Founding goal G2 blocks this

`founding-goals.md` — the one file agents may not amend, requiring a human commit + CODEOWNERS
approval:

> **G2 — Multi-host scaleout.** "A self-modifying substrate that runs only on the developer's
> laptop is a toy. The substrate must be able to run as a fleet — agents on one host, inference
> on another, coordination state shared… Without this, hex remains a single-tenant build helper
> instead of an operating system for AI work."

Stripping coordination **is** the direct negation of G2. The file's own rule: *"A goal is retired
only by a human commit that removes it from this file and lands a Retirement-ADR explaining why
the goal no longer serves the project."*

You're the human, so this is a ten-minute action, not a blocker — but it has to happen **before**
the deletion, or the repo's own governance layer contradicts its head commit. **Land the
Retirement-ADR for G2 first.** The honest argument writes itself: the fleet was built, measured,
and found to be the wrong axis — quality came from context and gates, not head-count, and
ADR-2606061359 already conceded that when it retired the org-sim.

G1 (model independence) and G3 (hexagonal rigor) survive intact and get *stronger* — G1 becomes
`hex-infer`'s single port; G3 becomes enforceable on 8 crates instead of 13.

### 5.2 The other two

- **CLAUDE.md rule 0** ("EVERYTHING ROUTES THROUGH HEX") maps most bypasses to nexus-backed verbs
  that are about to stop existing. It needs a rewrite in the same commit as Phase 4.
- **264 ADRs** — keep all of them (append-only ledger; deleting history is the mistake ADR epochs
  exist to prevent). Open the `solo` epoch and let `hex adr reindex` group the daemon-era ones
  under `hybrid-inference`.

---

## 6. Risks

| Risk | Severity | Mitigation |
|---|---|---|
| `inference_complete` is 1,033 lines with 18 `State(...)` reads — hidden coupling to daemon state | **High** — the only genuine unknown | Phase 1 gate is behavioral: `hex do` works with nexus stopped. Extract, don't rewrite. |
| Deleting nexus breaks a feature-gated import nobody traced | High | `hex graph consumers` per file before each `git rm` (ADR-2606071713). Exactly the ADR-2026-04-05-0900 failure mode. |
| Losing 1,000 nexus tests + 683 CLI tests deletes real coverage | Medium | Before Phase 3, port any test that exercises **hex-exec or hex-analysis** behavior into those crates. Daemon-routing tests go with the daemon. |
| Best-of-N + compile gate quietly degrades after extraction | Medium | `hex bench agentic` is the oracle. Baseline it in Phase 0, compare in Phase 5. |
| Sunk-cost pull to keep the dashboard | Medium | It has no job in a one-process tool. `hex do` printing to stdout is the monitor. |
| Repo idle since 2026-07-21; uncommitted `examples/hex-3d-nav-test/` + 2 workplan drafts | Low | Commit or discard before branching. 2 worktrees are live (`hex-ebay-mvp-scaffold`) — reconcile first. |

---

## 7. Sequencing summary

| Phase | Work | Gate |
|---|---|---|
| **0** | Drop `hex-desktop` from members; baseline `hex bench agentic` | `cargo check --workspace` = 0 |
| **G2** | Retirement-ADR for founding goal G2 | Human commit, CODEOWNERS |
| **1** | Extract `hex-infer`; repoint hex-exec ⚠️ | `hex do` works with nexus **stopped** |
| **2** | LocalStore replaces STDB in hex-exec | no `spacetime` in `hex-exec/src` |
| **3** | Delete nexus/agent/desktop/state/spacetime-modules/pipeline/tui | one binary, `cargo check` = 0 |
| **4** | CLI verb triage; fold analyzer into analysis | `hex --help` one screen, no daemon |
| **5** | bench + self-analyze + build an example + rewrite ARCHITECTURE.md | no bench regression; `hex analyze .` ≥ A |

**~261k → ~60k LOC. 3 processes → 1. 584 deps → ~180.**

The single highest-value action is Phase 1, because it is the only part that is engineering rather
than deletion — and its gate (`hex do` working with the daemon stopped) is the proof that the
other 200k lines are removable.

---
---

# Round 2 — what else goes

Round 1 covered crates. This covers everything else: the working tree, the embedded assets, the
harness config, and the dead weight *inside* the keep set. Several of these cost zero engineering
and some are worth more than the LOC suggests.

## 8. The repo is 20 MB. The working tree is 31 GB.

`git ls-files` → **2,159 files, 20 MB tracked**. `du -sh .` → **31 GB**.

| Untracked | Size | What |
|---|---:|---|
| `target/` | 23 GB | gitignored build output |
| `scripts/lora/llama.cpp/` | ~5.4 GB | a **vendored llama.cpp checkout** (only 11 files under `scripts/lora` are tracked) |
| `examples/*` build output | ~1.2 GB | `ebay-clone` alone is 724 MB on disk / 92 files tracked |
| `spacetime-modules/target/` | 453 MB | WASM build output |
| `hex-nexus/assets/node_modules/` | — | dashboard deps |

**~31 GB of the working tree is not in the repository.** A fresh clone is 20 MB. This is very
likely most of what "we can't get a clean build" actually feels like day to day — cargo walking a
23 GB target dir, `bun test` globbing into it (a known wedge: it silently blocked two workplans for
22+ hours), ripgrep and the analyzer paying for 5.4 GB of vendored C++.

**Action, costs nothing, do it before anything else:** `git clean -ndx` to review, then clean;
move `scripts/lora/llama.cpp` out of the tree entirely; add `examples/*/target`, `examples/*/node_modules`
to `.hexignore`. **This is not part of the refactor — it's free today, on `main`.**

## 9. Dead TypeScript tooling at the root

`src/` and `tests/` **do not exist** — the TS library was excised (`wp-excise-ts-phantom`), as
CLAUDE.md itself states. Still tracked and still wired into the build:

| File | Size | Points at |
|---|---:|---|
| `package.json` | 3.1 KB | `main: dist/index.js`, `bin: dist/cli.js`, `bun build src/cli.ts`, `bun test tests/unit tests/property tests/smoke` — **every path is deleted** |
| `bun.lock` | 235 KB | deps for that package |
| `tsconfig.json`, `tsconfig.test.json`, `eslint.config.js` | 2.9 KB | `src/`, `tests/` |

This is not cosmetic. **`hex dev validate` chains `bun test`**, and its test script globs
directories that no longer exist. The validation gate the whole workflow leans on is running a
command that cannot pass. Delete all five files; drop `bun test` from `hex dev validate`.

## 10. 1.5 MB of SpacetimeDB WASM is compiled into the `hex` binary

`hex-cli/assets/wasm/` — 7 committed `.wasm` blobs (`hexflo_coordination` 1.48 MB,
`inference_gateway` 488 KB, `neural_lab` 399 KB, `chat_relay` 249 KB, `rl_engine` 240 KB,
`agent_registry` 182 KB, `agent_comms` 181 KB), rust-embedded into a 14 MB binary. All die with
STDB. So do `scripts/build-wasm.sh`, `check-wasm-fresh.sh`, `check-no-sqlite.sh`,
`generate-ts-bindings.sh`, `stdb-watchdog.sh`, and `docker-compose.yml`.

## 11. The org-sim roster is still shipping — and the tree says so

`hex-cli/assets/agents/hex/hex/DEPRECATED.md` exists, names the roster, and says:

> Phase 4 of the workplan removes them once the SOP code consumers are migrated off.

**Phase 4 never ran.** 33 agent YAMLs ship today; 16 are the C-suite:
`ceo` `coo` `cto` `cpo` `ciso` `chief-architect` `chief-visionary` `product-lead`
`engineering-lead` `pm-agent` `sre-lead` `sre-engineer` `platform-engineer` `ux-designer`
`dashboard-ux-architect` `cli-designer`.

The same note names the keepers: `hex-coder` `hex-tester` `hex-reviewer` `hex-documenter`
`hex-ux` `hex-fixer` + the two stewards. It also marks `org_responder` / `sop_executor` deprecated.

Goes with them:
- `hex-cli/assets/prompts/agent-{coder,documenter,fixer,reviewer,tester,ux}.md` — per-persona prompts
- `hex-cli/assets/swarms/*.yml` — 8 swarm behaviors (`dev-pipeline`, `quick-fix`, `code-review`,
  `refactor`, `test-suite`, `documentation`, `migration`, `brain-self-improvement`)
- `hex-cli/assets/context-templates/services/hexflo-{agent,global,swarm}.md`
- `hex-cli/assets/helpers/{agent-register,hub-push,hook-handler,hex-statusline}.cjs` — daemon helpers
- 4 of 8 `assets/hooks/` YAMLs (`hex-merge-validation`, `hex-no-rest-state-mutation`,
  `hex-lifecycle-enforcement`); keep `hex-architecture-gate`, `hex-boundary-check`,
  `hex-specs-required`, `hex-adr-lifecycle`

## 12. ⭐ `.claude/skills/` — 31 foreign skills, tracked, loaded every session

This is the highest-leverage strip in the whole plan, and it isn't measured in LOC.

65 files tracked under `.claude/`. **~31 of the 64 skills have nothing to do with hex:**

| Group | Count | What it is |
|---|---:|---|
| `v3-*` | 9 | **claude-flow v3** — DDD architecture, MCP optimization, memory unification, security overhaul, 15-agent swarm coordination. Another product's ADRs. |
| `agentdb-*` | 5 | AgentDB vector search / RL / quantization |
| `github-*` | 5 | GitHub PM, release, multi-repo, workflow automation |
| `swarm-advanced`, `swarm-orchestration`, `sparc-methodology`, `stream-chain`, `reasoningbank-*` (2), `pair-programming`, `hooks-automation`, `verification-quality`, `skill-builder`, `browser`, `project-output` | 12 | agentic-flow / claude-flow tooling |

354 KB, and every one of them is advertised to the model on every single invocation. Skim the
skill roster any hex session sees and it is dominated by *swarm orchestration and distributed
memory for a different framework* — competing for attention with hex's own 17 skills, while you
are trying to build a tool whose entire thesis is that head-count is the wrong axis.

Also in there:
- **4 duplicate pairs** — `hex-adr-create.md` *and* `hex-adr-create/SKILL.md` (×4 for create/review/search/status)
- Dead-on-arrival after the strip: `hex-dashboard`, `hex-spacetime`, `hex-publish-module`,
  `hex-swarm`, `hex-dev-rebuild`
- `.claude/agents/neural-lab-researcher.yml`

**Target: 64 skills → ~12.** Costs one `git rm -r`. Do it early — it improves every agent
session between now and the end of the refactor.

## 13. `opencode.json` — 118 KB of config for a different agent runtime

Root-level: 18 agent definitions, 20 commands, MCP config, plugin, provider. Plus
`.opencode/plugins/hex-sidebar.tsx`, `hex-cli/assets/plugins/hex-sidebar.tsx`, and
`hex-cli/src/commands/opencode.rs` (549 LOC). A second harness integration to maintain for a tool
that isn't hex. Drop unless you actively drive hex from OpenCode.

## 14. hex-core: ~5,400 of 8,520 LOC dies with the daemon

| Group | Dead LOC | Files |
|---|---:|---|
| **Ports** | ~2,018 | `state.rs` **1,088** · `coordination.rs` 177 · `experiment.rs` 171 · `consolidation_memory.rs` 164 · `agent_comm.rs` 128 · `heartbeat.rs` 91 · `dead_letter.rs` 61 · `worker_pool.rs` 47 · `secret.rs` 35 · `sandbox.rs` 23 · `brain.rs` 17 · `agent_runtime.rs` 16 |
| **Domain** | ~2,138 | `brain.rs` 331 + `brain_tests.rs` 137 · `capability.rs` 367 · `api_optimization.rs` 305 · `experiment/` 253 · `messages.rs` 157 · `consolidation.rs` 138 · `heartbeat.rs` 114 · `sandbox.rs` 90 · `swarm_task.rs` 86 · `secret_grant.rs` 86 · `agents.rs` 74 |
| **Loose** | ~1,282 | `composition.rs` **633** (Layer-6 swap governance) · `corpus.rs` 267 · `quantization.rs` 204 · `research_finding.rs` 178 |

**Keep:** `ports/inference.rs` + `mock.rs` (492 — the G1 contract), `file_system`, `file_writer`,
`build`, `enforcement`, `validator`, `adapter_generator`, `context_compressor`, `web`;
`domain/{workplan,tokens,validation,enforcement}`; `telemetry`, `resource_governor`,
`inference_task`, `inference_q`, `config`.

**hex-core: 8,520 → ~3,100.** It becomes what it claims to be — a small contract surface.

## 15. Inside the kept CLI verbs

| Verb | Now | After | Cut |
|---|---:|---:|---|
| `inference.rs` | 3,348 | ~1,500 | drop `Queue` `Stats` `EscalationReport` `QReport` `Usage` `Discovered` `Corpus` `Adapter{register,list,remove,disable,enable,evaluate}` — all STDB telemetry or LoRA. Keep `Add` `List` `Test` `Discover` `Remove` `Setup` `Bench` `GpuCheck`. |
| `plan/` | 5,038 | ~3,900 | `reconcile.rs` (644) + `reconcile_evidence.rs` (479) exist to resync workplan state with daemon-run agents. In-process, the executor knows its own result. |
| `adr/doctor.rs` | 2,398 | ~1,400 | keep structural linting; drop STDB-synced status machinery |
| — | | | 27 `#[allow(dead_code)]` sites in hex-cli to resolve |

## 16. Docs and scripts

**Docs — keep all 264 ADRs** (append-only ledger; deleting it is the mistake epochs exist to
prevent). Everything else is auditable:
- `docs/workplans/` — 204 tracked, **192 of them in `archive/`**. That's git's job. Drop the archive.
- `docs/specs/` — 110, many for features about to be deleted (`dashboard-session-cleanup.json`,
  `cli-chat-tui.json`, `chat-tui-parity.json`, `coo-observability-baseline.md`,
  `agentic-brain-core.json`, `agent-notification-inbox.json`, `cost-reporting.json`, …). Expect ~60 to go.
- Stale status files: `IMPLEMENTATION-STATUS.md`, `LANGUAGE-INJECTION-STATUS.md`,
  `docs/STATUS-2026-05-23.md`.

**Scripts — ~24 of 63 tracked files die with the daemon:** `agents-start-all.sh`
`agents-status.sh` `agents-stop-all.sh` `stdb-watchdog.sh` `hud.sh` `hud-tui.sh` `hex-up.sh`
`hex-startup.sh` `overnight-cycler.sh` `push-dashboard-data.cjs` `push-graph.cjs`
`hex-statusline.cjs` `build-wasm.sh` `check-wasm-fresh.sh` `check-no-sqlite.sh`
`generate-ts-bindings.sh` `bench-persona-cron.sh` `bench-persona-prompts.py`
`test-coordination.ts` `test-conflict-prevention.ts` `feature-workflow.sh` `demo-build.sh`
`claude-on-hex.sh` `agent-smoke-test.txt`.

`scripts/lora/` (11 tracked files + the 5.4 GB vendored llama.cpp): the DSpark line is closed and
ADR-2606161300 is Proposed-not-built. Move to a sibling repo; don't carry it in the agent.

Also: `docker-compose.yml` (STDB), `bin/hex-wrapper.sh`, `.mcp.json`, `config/grammars/.gitkeep`.

## 17. Revised totals

| | Today | Round 1 | Round 2 |
|---|---:|---:|---:|
| Rust LOC | ~261,000 | ~60,000 | **~49,000** |
| Dashboard TS | 35,096 | 0 | 0 |
| Embedded WASM | 1.5 MB | 1.5 MB | **0** |
| `.claude` skills | 64 | 64 | **~12** |
| Agent YAMLs | 33 | 33 | **~13** |
| Tracked files | 2,159 | ~1,200 | **~700** |
| Working tree | 31 GB | — | **~200 MB** |
| Processes | 3 | 1 | 1 |

## 18. Do these three now — they're free and independent of the refactor

They need no ADR, no branch, and no decision about the daemon:

1. **`git clean` + evict `scripts/lora/llama.cpp`** → 31 GB working tree becomes ~200 MB.
   Immediately speeds up cargo, ripgrep, the analyzer, and unwedges `bun test`.
2. **`git rm -r` the 31 foreign `.claude/skills/`** (v3-*, agentdb-*, github-*, swarm-*, sparc,
   stream-chain, reasoningbank-*, pair-programming, hooks-automation, verification-quality,
   skill-builder, browser, project-output) **+ the 4 duplicate `.md` pairs** → every agent session
   from here on gets a clean, hex-only skill roster.
3. **Delete the dead TS tooling** (`package.json`, `bun.lock`, `tsconfig*.json`,
   `eslint.config.js`) **and drop `bun test` from `hex dev validate`** → the validation gate stops
   running a command that cannot pass.
