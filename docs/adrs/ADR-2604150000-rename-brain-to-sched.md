# ADR-2604150000: Rename `brain` subsystem to `sched`

**Status:** Accepted
**Date:** 2026-04-14
**Decision type:** restructure (rename with backward-compat aliases)
**Drivers:** Naming clarity, AIOS framing, operator ergonomics

---

## Context

hex-intf is an AI Operating System (AIOS). The `brain` subsystem is the
autonomous scheduler/supervisor that drains a persistent task queue, runs
periodic validation, auto-fixes architectural drift, and keeps agents fed with
work. Its job is **scheduling**, not reasoning.

The name `brain` is anthropomorphic and actively misleading in an AIOS where
AI agents *are* the compute workload:

- New operators read `hex brain status` and reasonably assume it's the LLM
  inference layer — it is not.
- The real "brains" in the system are the tiered inference adapters
  (`OllamaInferenceAdapter`, `ClaudeCodeInferenceAdapter`), the
  ReasoningBank memory, and the agents themselves.
- The subsystem's actual shape — queue drain + heartbeat + validation cycle —
  is a textbook scheduler / supervisor daemon (`systemd`, `launchd`, `cron`).

The statusline label `brain` triggered this ADR: it shows queue depth next to
other OS-level counters (inference QPS, active swarms) and the name is the
odd one out.

## Decision

Rename the subsystem to **`sched`** (daemon binary/crate-suffix: `hex-sched`).

- **CLI surface:** `hex brain *` → `hex sched *` (e.g. `hex sched daemon`,
  `hex sched enqueue`, `hex sched queue list`).
- **Statusline segment label:** `brain` → `sched`.
- **REST API:** `/api/brain/*` → `/api/sched/*`.
- **Pidfile / state paths:** `~/.hex/brain-daemon.pid` → `~/.hex/sched.pid`
  (with migration on first run).
- **Hook names, crate module paths, doc references:** all follow.

### Backward-compat posture

- `hex brain <subcmd>` stays as a hidden alias that forwards to `hex sched` and
  emits a one-line deprecation warning to stderr.
- `/api/brain/*` stays for one release with an `X-Hex-Deprecated` header; log
  a warn-level line on every hit.
- Historical ADRs and workplans that contain `brain` in their titles/bodies
  are **not rewritten** (that would forge history). Only live CLAUDE.md and
  active docs are updated.
- Pidfile migration: on startup, if `~/.hex/brain-daemon.pid` exists and
  `~/.hex/sched.pid` does not, rename it.

## Impact Analysis

### Consumer Dependency Map

Workspace grep (`\bbrain\b|hex_brain|hex-brain` across `.rs .ts .toml .yml .yaml
.json .md .cjs`, excluding `target/`, `node_modules/`, `.git/`): **401 files**.

Top live-code hotspots (excluding `.claude/worktrees/` agent scratch dirs):

| File | Hits | Role |
|---|---:|---|
| `hex-cli/src/commands/brain.rs` | 62 | Primary CLI module — rename + module path |
| `hex-cli/src/commands/hook.rs` | 13 | Hook dispatch — `brain enqueue` call sites |
| `docs/adrs/ADR-2604132330-brain-inbox-queue.md` | 21 | Historical (keep) |
| `docs/workplans/wp-brain-self-consistency.json` | 17 | Live workplan — rename refs |
| `docs/adrs/ADR-2604140000-hey-hex-natural-language.md` | 10 | Historical (keep) |
| `README.md` | 9 | User-facing — rename |
| `docs/workplans/wp-brain-*.json` (3 files) | 9 each | Live workplans — rename refs |
| `docs/adrs/ADR-2604141100-brain-updates-to-operator.md` | 9 | Historical (keep) |
| `docs/adrs/ADR-2604142200-hex-chat-conversational.md` | 9 | Historical (keep) |
| `CLAUDE.md` | ~5 (rules 1, 2, 5) | Live — rename |

**Code consumers (CRITICAL):**
- `hex-cli/src/commands/brain.rs` — CLI module, source of truth for `hex brain *`
- `hex-cli/src/commands/hook.rs` — emits `brain enqueue` on specific hook events
- `hex-nexus/src/routes/*` — `/api/brain/status`, `/api/brain/test` endpoints
  (see recent commits `b41ae86d`, `e619d88a`)
- `hex-cli/assets/helpers/hex-statusline.cjs` + `scripts/hex-statusline.cjs`
  — statusline segment rendering (duplicate; ADR-044 bakes one in)
- `hex-cli/assets/hooks/hex/*.yml` — hook YAMLs referencing brain commands
- `hex-nexus/src/coordination/cleanup.rs` / brain-daemon entry

**Config references (HIGH):**
- `~/.hex/brain-daemon.pid` — pidfile path (runtime migration on startup)
- `~/.hex/hub.db` (SQLite) — if any table/column names embed `brain`, they get
  renamed via migration (to verify in workplan P0)

**Documentation references (MEDIUM):**
- `CLAUDE.md` rules 1, 2, 5 — rename to `hex sched *`
- `README.md` — rename
- Live workplans `wp-brain-*.json` — rename filenames + internal refs
- Historical ADRs — **do not rewrite**; annotate only if cross-referenced

**Test references (MEDIUM):**
- `hex-nexus/tests/*` — any integration test hitting `/api/brain/*`
- CLI snapshot tests referencing `hex brain --help`

### Cross-Crate Analysis

```bash
grep -r 'pub use.*brain' --include='*.rs'           # re-exports: <none expected>
grep -r 'cfg.*feature.*brain' --include='*.rs'      # feature gates: <none expected>
grep -r 'features.*=.*"brain"' --include='*.toml'   # Cargo features: <none expected>
```

(Verification performed during P0 of the workplan.)

### Blast Radius

| Artifact | Consumers | Impact | Mitigation |
|---|---|---|---|
| `hex brain` CLI command tree | Hooks, scripts, docs, muscle memory | CRITICAL | Hidden alias + stderr deprecation for 1 release |
| `/api/brain/*` REST endpoints | Statusline helper, external scripts | CRITICAL | Route both prefixes for 1 release; header tag |
| `hex-cli/src/commands/brain.rs` (module) | hex-cli lib internal | CRITICAL | Rename module + `pub use brain as sched` shim for internal callers during transition |
| `~/.hex/brain-daemon.pid` | brain-daemon itself | HIGH | Startup migration: rename if present |
| Statusline segment label | `hex-statusline.cjs` | MEDIUM | Swap string; rebuild nexus binary |
| Live workplan filenames `wp-brain-*.json` | Planner, brain itself | MEDIUM | Rename files + update internal refs in one commit |
| Historical ADR titles | None (read-only) | LOW | Do not rewrite |

### Build Verification Gates

| Gate | Command | When |
|---|---|---|
| Workspace compile | `cargo check --workspace` | After each phase that touches Rust |
| Rust tests | `cargo test -p hex-cli -p hex-nexus` | After P2, P3 |
| TypeScript typecheck | `bun run check` | After P4 (if TS touched) |
| Rust lint | `cargo clippy --workspace -- -D warnings` | Before merge |
| Smoke: help text | `hex sched --help && hex brain --help 2>&1 \| grep -i deprecated` | After P2 |
| Smoke: statusline | `node hex-cli/assets/helpers/hex-statusline.cjs` shows `sched` | After P4 |
| Smoke: daemon roundtrip | `hex sched daemon --background && hex sched daemon-status && hex sched enqueue shell -- "echo ok"` | After P5 |
| Embedded-assets-generic | `hex doctor` passes `embedded-assets-generic` | Before merge |

## Consequences

**Positive:**
- Name matches function; AIOS framing strengthened.
- Clearer onboarding — new operators don't confuse the scheduler with inference.
- Statusline segment fits conventions (`sched`, `infer`, `nexus`).

**Negative:**
- 401-file change; non-trivial review load even if mostly mechanical.
- Muscle memory: existing operators will type `hex brain` for months.
- Short-term docs/URL churn; any external blog posts linking `/api/brain/*` break
  after the deprecation window.

**Mitigations:**
- Hidden alias + stderr deprecation softens the muscle-memory cost.
- Dual-routed REST endpoints for one release give external consumers a grace
  window.
- Mechanical rename executed as a single bundled PR per
  `feedback_agent_scope_and_reporting` (prefer one bundled PR for renames);
  reviewer only checks the diff pattern, not every site.

## Implementation (Workplan Phases)

Workplan to be created as `docs/workplans/wp-rename-brain-to-sched.json`.

| Phase | Description | Validation Gate |
|---|---|---|
| **P0** | Audit sweep — regenerate grep, inventory SQLite schema, confirm no Cargo feature gates | `cargo check --workspace` green (baseline) |
| **P1** | ADR + workplan accepted; status → Accepted | n/a (docs) |
| **P2** | Rename `hex-cli/src/commands/brain.rs` → `sched.rs`; add `brain` command as hidden alias that calls into `sched` with stderr deprecation | `cargo check --workspace` + `hex sched --help` + `hex brain --help` both work |
| **P3** | Rename nexus routes `/api/brain/*` → `/api/sched/*`; keep `brain` routes with deprecation header for 1 release | `cargo test -p hex-nexus` + curl smoke |
| **P4** | Update statusline helpers + embedded hook YAMLs + CLAUDE.md + README | `bun run check` + `hex doctor` passes `embedded-assets-generic` |
| **P5** | Pidfile/state migration on daemon startup (rename legacy paths); rename live workplans `wp-brain-*.json` → `wp-sched-*.json` | Daemon roundtrip smoke test |
| **P6** | Rebuild release binaries, commit, cleanup agent worktree scratch copies | `cargo build --release` + `hex doctor --verbose` green |

**BLOCKING:** No phase can start until the previous phase's validation gate is green.
Per `feedback_workplan_mandatory`, P1 must land before P2 starts.

## References

- `ADR-2604132330-brain-inbox-queue` — original brain-inbox design
- `ADR-2604141100-brain-updates-to-operator` — brain status reporting
- `ADR-2604142300-brain-auto-cleanup-stale-swarms` — active, rename covered in P4
- `CLAUDE.md` "Autonomous Operation" rules 1, 2, 5
- `feedback_brain_autonomous_drain` (memory) — semantics unchanged, label only

## Anti-Pattern Guard (lessons from ADR-2604050900)

- Workspace grep performed across ALL crates, not just `hex-cli/`.
- Every phase has a validation gate; no phase is marked done without it.
- `.claude/worktrees/agent-*/` copies are EXCLUDED from the rename (those are
  agent scratch spaces and will naturally pick up the new names on next
  worktree creation). P6 optionally prunes stale worktrees via
  `hex worktree cleanup --force`.
