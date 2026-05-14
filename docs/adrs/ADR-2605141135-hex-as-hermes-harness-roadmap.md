# ADR-2605141135: hex-as-hermes-harness — phased roadmap

**Status:** Accepted (Phase 0 proven by commit `f33c7a37` — first truly autonomous commit on main. Phases 1-6 are work-in-progress with their own per-phase ADRs to come.)
**Date:** 2026-05-14
**Drivers:** Operator wants hex to expose the **fully-featured operator ergonomics of Hermes Agent (Nous Research)** while preserving hex's distinguishing architectural primitives — typed-tool SOP, c-suite persona topology with atomic-claim mediation, twin reviewer with content-grounding gate, tiered inference routing, hexagonal-architecture enforcement. The 2026-05-13 session diagnosed (a) the autonomous artifact-authoring half works end-to-end (proven by `8e929b58` agentic-dev-roundtrip), (b) the autonomous *commit* step was missing and just shipped in `11169fb1`, but (c) the autonomous *construction* loop (workplan dispatch → hex-coder IC workers) is structurally broken (phantom cc_agent UUIDs, no IC workers register in `hex agent list`, `/api/workplan/execute/<id>/status` returns 404). The user request — *"i want a fully featured hermes agent harness but based on hex"* — is a multi-week port, not single-session work. This ADR is the master index that breaks it into phases, each shippable as its own workplan, each measurable against a concrete deliverable.

**Authors:** Operator. Roadmap-shaped ADRs typically sit with the operator because they sequence multiple downstream ADRs/workplans.

**References:**
- Hermes Agent docs — https://hermes-agent.nousresearch.com/docs/llms.txt and `/llms-full.txt`
- ADR-2026-05-08-2500 — Typed-tool SOP foundation
- ADR-2026-05-08-2300 — Digital-twin reviewer
- ADR-2026-05-13-1500 — Fail-open twin judge + `hex goal` verb (Hermes Ralph-loop port)
- ADR-2605131849 — User-defined SOUL personas alongside c-suite (Hermes profile port)
- ADR-2026-04-18-0001 — Workplan inference task stalling (the construction-loop bug)
- ADR-027 — HexFlo swarm coordination (the Kanban analog)
- Commits this session: `488e1503`, `c1450b58`, `f336930a`, `e305fc21`, `04bf854a`, `8e929b58`, `11169fb1`
- Memory: `project_typed_tool_sop_proven`, `project_self_managing_loop_2605091200`, `project_audit_autonomous_dev_2026_05_12`

## Context

### What Hermes is

Hermes Agent is a terminal-native autonomous agent built on a single `AIAgent` class (`run_agent.py`, ~15k LOC) that handles prompt assembly, tool dispatch, provider routing, compression, fallback, and callbacks. Around that engine sit ~20 operator surfaces:

| Hermes surface | One-liner |
|---|---|
| `SOUL.md` | Markdown identity in slot 1 of system prompt |
| Profiles | Separate `$HERMES_HOME` directories = independent agents, with command aliases at `~/.local/bin/<name>` |
| Skills | Procedural markdown loaded on-demand with progressive disclosure (list → view → path) |
| Curator | Background pass on agent-created skills with snapshot+rollback |
| Persistent memory | `MEMORY.md` (2.2 KB cap, frozen snapshot) + `USER.md` (1.4 KB cap) |
| `/goal` | Ralph loop: standing objective survives across turns until aux-model judge says done |
| `delegate_task` | Fork-join subagent: fresh context, restricted toolset, optional cheaper model |
| Kanban | Durable named-profile work board, dispatcher reclaims stale claims, peers coordinate via comments |
| Cron | Natural-language scheduled jobs with delivery to any gateway |
| `execute_code` | Programmatic Tool Calling — Python script over Unix socket; intermediate results never enter context |
| 70+ built-in tools | terminal, file, web, vision, browser, image-gen, TTS |
| MCP integration | Connect to external MCP servers, filter their tools |
| Voice mode | Real-time across CLI, Telegram, Discord (DMs + voice channels) |
| Messaging gateway | 20+ platforms (Telegram, Discord, Slack, WhatsApp, Signal, SMS, Matrix, Email, …) |
| Terminal backends | 6 (local, Docker, SSH, Daytona, Modal, Singularity) |
| Plugin system | Custom tools + lifecycle hooks loaded at runtime |
| Honcho dialectic | Two-layer adaptive memory with cost/depth knobs |
| ACP integration | Use Hermes inside VS Code, Zed, JetBrains |
| API server | OpenAI-compatible front-door |

### What hex already has

| hex primitive | Hermes equivalent | State |
|---|---|---|
| SOP executor (5-phase GROUND/REASON/ACT/VERIFY/REPORT) | `AIAgent.run_conversation` | Shipped, working |
| Typed-tool library (`cargo_check`, `repo_grep`, `repo_read`, `adr_draft`, `spec_draft`, `code_patch`, `workplan_emit`, `escalate_to_operator`, `memory_search`) | Hermes tools | 9 of 70+ |
| c-suite + IC persona YAMLs (`hex-cli/assets/agents/hex/hex/`) | Multiple profiles | Shipped but baked-in (rust-embed); no runtime SOUL until ADR-2605131849 lands |
| Twin reviewer with content-grounding gate | Command approval callback | Shipped, calibration in progress |
| HexFlo coordination (STDB-backed swarms + tasks + memory) | Kanban | Shipped, dispatcher mostly working |
| Tiered inference (T1/T2/T2.5/T3) | Auxiliary model slots | Shipped, per-task-shape routing |
| Mission Control dashboard | Hermes TUI | Shipped |
| Autonomous commit step (commit `11169fb1`) | Implicit in Hermes' commit flow | Shipped this session |
| ADR + workplan + reconciler system | None — hex unique | Shipped, reconciler has 70% FP per autonomy audit |
| Hexagonal-architecture enforcement (`hex analyze`) | None — hex unique | Shipped |

### The gap

What hex must add to reach "fully-featured Hermes harness" parity, ordered roughly by operator-leverage:

1. **Working code-construction loop** — workplan executor → hex-coder IC workers. Today: broken. Without this, no multi-task feature can build itself; operator-Opus has to step through tasks. This is the single biggest blocker for the user's "i need to be confident we can autonomously build a non-trivial system" question.
2. **`hex goal "<intent>"` Ralph loop** — ADR-2026-05-13-1500 §3 specs it; not built. The lightest possible primitive between `hex chat` (one turn) and `hex plan` (decomposed workplan).
3. **`hex persona create` user-defined SOUL personas** — ADR-2605131849 in flight; P1 (STDB schema + reducers) done in `04bf854a`; P2-P8 (~1030 LOC) pending.
4. **Bounded memory + curator** — Hermes' MEMORY.md (2.2 KB cap, frozen snapshot, character-budget enforced) + USER.md (1.4 KB) + weekly curator pass with snapshot/rollback. hex's STDB `hexflo_memory_*` is unbounded and never curated.
5. **`delegate_task` typed tool** — fork-join sub-SOP with isolated context, restricted toolset, optional cheaper model. Today the persona must inline every reasoning step; `delegate_task` lets it spawn focused child agents.
6. **`execute_code` Programmatic Tool Calling** — current 5-phase SOP per persona round; PTC lets a persona run a script that calls multiple tools and emits one summary, slashing token cost on data-processing workflows.
7. **Messaging gateway adapters** — start with Telegram (ADR-2026-05-08-2650 already Accepted: `telegram_notifier` stub shipped), then Discord, then Slack. Each unlocks operator-from-anywhere ergonomics.
8. **Cron with NL scheduling** — hex `sched` daemon exists but doesn't accept natural-language schedule grammars. Hermes' "every weekday at 9 send me a research brief" → `cron(...)` is the missing layer.
9. **Plugin runtime + lifecycle hooks** — today's `hex-cli/assets/` is compiled into the binary via `rust-embed`. A runtime plugin loader lets third-party tools/hooks ship without forking hex.
10. **OpenAI-compatible API server** — Hermes' `hermes api` exposes the agent as an OpenAI `/v1/chat/completions` server so any client (Cursor, Continue, OpenAI SDK, ...) can drive it. hex's nexus REST is structurally close; adapter shim is small.

What we **don't** copy from Hermes:
- Voice mode / browser / vision / TTS / image-gen (consumer-agent surface area; not a dev AIOS need).
- 6 terminal backends — local is enough until proven otherwise.
- ACP integration — nice-to-have, not foundational.
- Honcho dialectic — `hexflo_memory` plus the curator (item 4) covers the same ground at less complexity.

## Decision

Adopt a **6-phase port** spanning ~6 weeks (calendar; less for engineering hours). Each phase is its own ADR + workplan; this document is the index. Phases run mostly sequentially because each builds on the previous, with two cross-phase parallel tracks (gateway adapters can land any time; calibration fixes happen continuously).

### Phase 0 — Authoring loop end-to-end (THIS SESSION, mostly done)

Goal: prove the SOP authoring loop closes from operator board ask → on-disk artifact → commit on main, no operator-Claude in the loop after the initial ask.

| Deliverable | State | Commit |
|---|---|---|
| Twin Hermes-style fail-open after 5 parse failures | done | `488e1503` |
| Drafter placeholder rejection + grounding gate | done | `c1450b58` |
| Drafter stub-detection gate + path-based model routing | done | `f336930a` |
| Drafter sanitize placeholder paths in stub-write | done | `e305fc21` |
| ADR-2605131849 user-defined SOUL personas | done | `623afbbc` |
| wp-user-defined-soul-personas P1 (STDB schema) | done | `04bf854a` |
| Autonomous commit step (executor → git commit) | done | `11169fb1` |
| Drafter operator-passthrough bypass (literal briefs skip gates) | done | this commit |
| Autonomous commit step flag-order bugfix (`-m` before `--`) | done | this commit |
| **Smoke proof: board ask → file → autonomous commit** | **PROVEN** | **`f33c7a37`** — first truly autonomous commit on main, attributed `Co-Authored-By: hex-autonomous`. Operator typed one curl POST to `/api/org/send-message`; the entire chain (drafter literal-content detection → twin operator-passthrough auto-approve → executor write → cargo_check no-op for non-Rust → mark_executed → commitment_satisfy → autonomous git commit) ran with no operator-Claude in the loop. File: `docs/specs/autonomous-commit-smoke-v2.md` (46 bytes). |
| THIS roadmap ADR | done | this commit |

Exit criteria: a single curl invocation against `/api/org/send-message` results in a new commit on main with `Co-Authored-By: hex-autonomous` and no operator-Claude commit wrapping it. Already proven for `docs/specs/`; needs proof for `docs/adrs/` once the grounding gate calibration lands (Phase 1).

### Phase 1 — Construction loop (next 1-2 weeks)

Goal: fix the workplan executor → hex-coder IC worker dispatch so non-trivial multi-task features can ship without operator-Opus per-task intervention. This is the **biggest open blocker** for the user's confidence question.

| Deliverable | Owner | LOC est |
|---|---|---|
| Diagnose why `hex pool list` shows `1/1 alive` but `hex agent list` has zero hex-coder workers | sre-lead | n/a |
| Fix `/api/workplan/execute/<id>/status` REST endpoint 404 | engineering-lead | ~30 |
| Spawn hex-coder workers that actually register in `hex_agent` table at startup | engineering-lead | ~150 |
| Replace cc_agent phantom UUID with real lookup against agent registry | engineering-lead | ~50 |
| Per-task 120s → adaptive timeout (cargo check on big workspaces alone can take 90s+) | engineering-lead | ~30 |
| Workplan reconciler false-positive fix (70% FP per `project_audit_autonomous_dev_2026_05_12`) — likely aux-model judge per ADR-2026-05-13-1500 §2 | engineering-lead | ~200 |
| `hex plan run-local <wp>` synchronous fallback for when IC dispatch wedges | engineering-lead | ~200 |
| Twin grounding gate calibration (accept bare module names like `org_responder`, lowercase `adr-...`, common `~/.hex/` paths) | engineering-lead | ~50 |
| End-to-end demo: build `examples/hex-as-hermes-smoke/` — tiny Rust binary with domain fn + CLI adapter + 1 test, all driven by SOP | demo, n/a | n/a |

Exit criteria: `hex plan execute wp-foo` dispatches all tasks to hex-coder workers, each task fires `code_patch` + `cargo_check` inline, lands a commit per task via the Phase 0 autonomous commit step. Demo workplan completes 100% without operator intervention beyond the initial dispatch.

### Phase 2 — Operator surfaces (1-2 weeks)

Goal: ship the two Hermes primitives that are most-asked for in this session, both already specced.

| Deliverable | Spec | LOC est |
|---|---|---|
| `hex goal "<intent>"` Ralph loop (set / status / pause / resume / clear, STDB-persisted, fail-open aux judge, 20-turn default budget) | ADR-2026-05-13-1500 §3 | ~250 |
| `hex persona create <name> --soul <path>` user-defined SOUL personas — P2 (CLI verb), P3 (SOUL security scan), P4 (org-comms DM routing), P5 (responder), P6 (dashboard panel), P7 (distribution), P8 (smoke) | ADR-2605131849 + wp-user-defined-soul-personas | ~980 remaining |

Exit criteria: operator can `hex goal "fix every failing test in tests/foo"` and Hermes-style continuation works; operator can `hex persona create coding-buddy --soul ./my-soul.md` and DM that persona via `POST /api/org/send-message`.

### Phase 3 — Memory + curator (~1 week)

Goal: port Hermes' bounded-memory discipline so the lesson catalog stops growing without consolidation.

| Deliverable | LOC est |
|---|---|
| Add `memory_cap_bytes` enforcement to `hexflo_memory_store` reducer (2.2 KB default per scope) | ~80 |
| Frozen-snapshot pattern: capture memory state at session start, inject into system prompt verbatim, allow runtime writes that don't appear until next session | ~150 |
| `hex curator` weekly tick: snapshot memory + lesson catalog, run aux-model consolidation pass (merge near-duplicates, archive 30-day-stale to `~/.hex/memory/.archive/`, never auto-delete) | ~300 |
| `hex curator status / pin <key> / unpin <key> / restore <key> / rollback` subcommands | ~100 |

Exit criteria: `hex memory store lesson:foo "..."` rejects with a clear error when its scope is at cap; `hex curator status` shows last-run / next-run / pinned list / LRU candidates; `hex curator run --dry-run` produces a deterministic plan without mutating disk.

### Phase 4 — Gateway adapters (~2 weeks, cross-phase parallel)

Goal: operator-from-anywhere. Start small, prove the contract, then add.

| Deliverable | State | LOC est |
|---|---|---|
| `telegram_notifier` adapter (escalate_to_operator hook) | shipped 2026-05-08 via `d327a266` + `dc08f6f5` | done |
| `hex gateway telegram start/stop` long-running bot with `/status`, `/ack <id>`, `/queue <intent>`, `/abort <task>` commands | proposed (ADR-2026-05-08-2650) | ~400 |
| Discord adapter (mirror Telegram contract) | future | ~300 |
| Slack adapter (mirror) | future | ~300 |
| Email adapter (IMAP/SMTP) | future | ~250 |

Exit criteria: operator can answer escalations from phone via Telegram; `/status` from the bot returns the same data as `hex pulse`; `/queue write a spec for foo` enqueues a board ask end-to-end.

### Phase 5 — Power tools (~1 week)

Goal: ship the two Hermes primitives that have the biggest leverage for cost + reasoning depth.

| Deliverable | LOC est |
|---|---|
| `delegate_task(goal, context, toolsets, max_iterations)` typed tool — fork-join sub-SOP, restricted toolset, optional cheaper model, summary-only return | ~300 |
| `execute_code(script)` typed tool — Rust child process running Python/JS over Unix socket; persona-emitted script uses `from hex_tools import code_patch, repo_grep, cargo_check, ...` to compose; only `print()` output returns to caller | ~500 |

Exit criteria: a persona can `delegate_task` to a research sub-agent for a parallel web-fetch + summary, gets back a 3-bullet summary; a persona can `execute_code` a 20-line Python that iterates over 100 files, calling `repo_read` + `code_patch` in a loop, and only the final report enters the context window.

### Phase 6 — Plugin runtime + cron NL + OpenAI shim (~1-2 weeks)

Goal: extensibility for third parties + frontend compatibility.

| Deliverable | LOC est |
|---|---|
| `hex plugin install <git-url>` + lifecycle hooks (`pre_tool_call`, `post_tool_call`, `pre_commitment`, `post_commitment`, `session_end`) loaded at nexus boot | ~400 |
| `hex sched` NL grammar: "every weekday at 9 send me a research brief to telegram" → cron + delivery target | ~200 |
| `hex api start --port 11434` — OpenAI-compatible `/v1/chat/completions` + `/v1/models` shim over the SOP loop, so any OpenAI SDK client can drive a hex persona | ~300 |
| ACP minimal probe — Hermes uses ACP for VS Code / Zed integration; nice-to-have, not foundational | optional |

Exit criteria: third-party plugin installed from git extends a hex persona's tool surface without forking; cron NL "every morning post a status update to #ops on slack" round-trips; an OpenAI SDK client connecting to `localhost:11434` drives the c-suite responder.

## Consequences

### Positive

- **Operator gets the Hermes ergonomics layered on top of hex's architectural primitives.** The c-suite atomic-claim + typed-tool SOP + twin reviewer + hexagonal-architecture enforcement remain load-bearing; the Hermes surfaces ride on top.
- **The autonomous loop closes end-to-end.** Operator fires one board ask, the system goes spec → ADR → workplan → code → cargo_check → commit → satisfy commitment → notify operator if it escalated. No human in the per-step loop.
- **Cost discipline by design.** Bounded memory (Phase 3) prevents lesson catalog drift; `delegate_task` + `execute_code` (Phase 5) slash per-feature token cost; tiered inference + longform-model routing (already shipped) keeps the right model on the right work.
- **Extensibility.** Plugins + profile distribution (Phase 4 + 6 + ADR-2605131849 D5) let community-authored personas + tools land without forking.

### Negative

- **6-8 weeks of focused engineering.** Each phase is a workplan + ADR + multiple commits. Not a single-session shipment.
- **Surface-area explosion.** Each new operator surface is more code to keep correct + documented + reviewed when grounding/permission rules change.
- **Increased blast radius if any of the autonomous loops misfires.** The autonomous commit step (`11169fb1`) makes hex's autonomous loop able to write to git history without operator review. Counterbalanced by the gates (placeholder, grounding, stub-detection, fail-open judge, executor cargo_check rollback, no-op detection in the commit step). Still: operator should `hex inbox list` regularly + watch the dashboard.
- **Cross-phase dependencies.** Phase 2 (`hex goal`) depends on Phase 1's reconciler fix to know when the goal is satisfied; Phase 3 (curator) depends on Phase 1's aux-model judge. Phasing can flex but not all paths can compress.

### Neutral

- The `examples/` directory becomes the de-facto demonstration ground for the autonomous loop. SafeFileWriter (commit `0346eff8`) blocks hex-infra writes by autonomous agents; `examples/` is exempt. Future demos go there.
- Hermes API endpoints (`/llms.txt`, `/llms-full.txt`, `/api/v1/chat/completions`) become the conventions hex's REST surface targets where they exist.

## Implementation

### Phase 0 is in flight this session

Already-shipped commits (chronological from session start): `488e1503` (twin fail-open), `efcc9e5f` (ADR triage), `c1450b58` (drafter placeholder+grounding), `f336930a` (drafter stub-detection), `e305fc21` (sanitize placeholder), `623afbbc` (ADR-2605131849), `93e13d99` (wp-user-defined-soul-personas), `4939bf84` (wp lint fix), `04bf854a` (P1 STDB), `8e929b58` (overnight roundtrip), `11169fb1` (autonomous commit step).

Pending this turn: drafter operator-passthrough bypass, smoke proof on commitment #16417, this roadmap ADR.

### Per-phase rollout pattern

Each phase ships its own ADR (`docs/adrs/`), its own workplan (`docs/workplans/wp-<phase>-<feature>.json`), 5-20 commits, and exits with a small operator-checkable demo. Phase ADRs reference back to this roadmap by ID.

### Migration / backward compatibility

Every phase is additive. The c-suite topology, typed-tool SOP, twin reviewer, and hexagonal-architecture enforcement remain unchanged. New surfaces (e.g. `hex goal`, `hex persona`, `hex curator`, `hex gateway`, `hex plugin`) are opt-in CLI verbs and `/api/...` routes. No verb gets renamed, no STDB table gets a breaking-change column rename.

### Risk + mitigation

| Risk | Mitigation |
|---|---|
| Phase 1's IC-dispatch fix turns out to be ADR-2026-04-18-0001's already-tracked but unsolved bug, ie. harder than estimated | The synchronous `hex plan run-local` fallback ships as part of Phase 1 anyway; if IC-dispatch can't be fixed cleanly, fallback covers the construction loop |
| Bounded memory (Phase 3) drops important lessons during curator consolidation | Curator never auto-deletes — only archives to `~/.hex/memory/.archive/`. Pre-run tar.gz snapshot, one-command rollback per Hermes pattern |
| Messaging gateway (Phase 4) exposes a new attack surface | Bot-token gate, chat_id allowlist per `ADR-2026-05-08-2650`, rate limits per chat_id, secret-grant for token storage |
| Plugin runtime (Phase 6) introduces RCE via untrusted plugin | Plugin signing + SOUL.md security-scan pattern (P3 of wp-user-defined-soul-personas) extended to plugin manifest; default-deny outside `hex plugin allow <name>` |
| 6-week timeline slips | Each phase is independently valuable. Phase 0 + 1 alone closes the autonomous-development-of-non-trivial-systems gap; Phases 2-6 are quality-of-life on top |

## What this ADR does NOT commit to

- An OpenAI Operator / Computer-Use clone. Hex stays terminal- and code-shaped.
- Replacing the c-suite topology with flat profiles. ADR-2605131849 makes user-personas a sidecar, not a replacement.
- A complete port of every Hermes verb. We borrow the operator-facing primitives that earn their keep; we skip voice + browser + image-gen + the 6 terminal backends until evidence shows we need them.
- A specific completion date. Each phase's workplan will have its own dates; this roadmap orders them.

## Open questions (resolved in subsequent ADRs)

- Phase 1 IC-dispatch fix vs. `hex plan run-local` fallback — which carries the autonomous-construction-loop load long-term? Decided by the diagnostic in Phase 1 §1.
- Memory cap defaults (Phase 3) — Hermes uses 2.2 KB MEMORY + 1.4 KB USER. Right for hex? Tunable, but starting values matter for the first curator run.
- Cron NL grammar (Phase 6) — adopt Hermes' format verbatim or design our own? Hermes' is documented and battle-tested; default to copying unless we find a hex-specific reason to diverge.
