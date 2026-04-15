# Brain String Audit — println!/eprintln!/log macro occurrences

Generated: 2026-04-15
Workplan: wp-brain-string-cleanup (P1.1)
Scope: hex-cli, hex-nexus, hex-agent, hex-core

## Classification Key

| Category | Meaning | In scope? |
|----------|---------|-----------|
| **(a)** | Reachable from `hex sched` command path | **YES** |
| **(b)** | Only reachable from `hex brain` alias path | No |
| **(c)** | Internal debug log / comment / test only | No |

---

## hex-cli/src/commands/sched.rs

These are all reachable via `hex sched <subcommand>` (the canonical path). The `hex brain` alias forwards to the same functions.

| Line | String | Category |
|------|--------|----------|
| 64 | `eprintln!("  {} brain-state dir: {}", ...)` | **(a)** |
| 71 | `eprintln!("  {} brain-state write: {}", ...)` | **(a)** |
| 74 | `eprintln!("  {} brain-state encode: {}", ...)` | **(a)** |
| 206 | `println!("⬡ enqueued brain task {id} ({kind}: {payload})")` | **(a)** |
| 239 | `println!("Brain service not configured. Run hex-nexus with brain service enabled.")` | **(a)** |
| 752 | `println!("⬡ hex brain validate")` | **(a)** |
| 1029 | `println!("⬡ hex brain prime")` | **(a)** |
| 1576 | `println!("⬡ brain daemon starting ...")` | **(a)** |
| 1588 | `println!("⬡ brain tick at ...")` | **(a)** |
| 1694 | `println!("  ⬡ leasing brain task {id} ({kind})")` | **(a)** |
| 1816 | `println!("⬡ brain daemon received ctrl-C, shutting down")` | **(a)** |
| 1832 | `println!("brain daemon already running ...")` | **(a)** |
| 1862 | `println!("⬡ brain daemon started in background ...")` | **(a)** |
| 1867 | `println!("  stop with: hex brain daemon-stop")` | **(a)** |
| 1878 | `println!("brain daemon not running ...")` | **(a)** |
| 1888 | `println!("brain daemon ..." )` (stale pid) | **(a)** |
| 1907 | `println!("⬡ brain daemon stopped ...")` | **(a)** |
| 1927 | `println!("⬡ brain daemon running ...")` | **(a)** |
| 1935 | `println!("brain daemon not running ...")` (stale pid) | **(a)** |
| 1941 | `println!("brain daemon not running")` (no pid file) | **(a)** |
| 2000 | `println!("⬡ watching brain_tick events ...")` | **(a)** |
| 2110 | `println!("  ...  brain_tick  session=... duration=...")` | **(a)** |
| 3009 | `println!("No pending brain tasks.")` | **(a)** |
| 3067 | `println!("No brain tasks in history...")` | **(a)** |
| 3160 | `println!("⬡ cleared {cleared} completed/failed brain tasks")` | **(a)** |
| 3167 | `println!("No pending brain tasks to drain.")` | **(a)** |
| 3170 | `println!("⬡ draining {} pending brain task(s)...")` | **(a)** |

**Subtotal: 27 occurrences, all category (a).**

### sched.rs — non-output strings containing "brain" (category c — internal/structural)

These are function names, variable names, comments, doc-comments, format strings used as API keys/memory keys, and test module names. They are not user-facing output but are structural references that a later phase will rename.

| Line(s) | Kind | Example |
|----------|------|---------|
| 24-74 | State struct + helpers | `brain_state_path()`, `brain-state.json` |
| 103-171 | Clap doc-strings | `"Show brain service status"`, `"List models available for brain selection"` |
| 205, 2312-2411 | Function names | `enqueue_brain_task`, `list_brain_tasks`, `drain_brain_tasks`, `update_brain_task` |
| 1357 | PID file path | `brain-daemon.pid` |
| 1431-1489 | Notify config event keys | `"brain.task.completed"`, `"brain.validate.regression"`, etc. |
| 1521-1563 | Notify helpers | `notify_operator("brain.validate.regression", ...)` |
| 1663-1700 | Drain loop comments | `brain-lease` swarm references |
| 2598-2822 | Dispatch/lease functions | `dispatch_brain_task`, `find_brain_lease_swarm`, `stamp_brain_task_lease` |
| 2925 | Execute function | `execute_brain_task` |
| 3041-3125 | History render | `/api/brain/queue/history`, `brain-task:<id>` |
| 3397-3871 | Tests | `brain::task_schema`, `brain::lease_durations`, `brain::inline_fallback` |

---

## hex-cli/src/commands/hook.rs

| Line | String | Category |
|------|--------|----------|
| 3017 | `eprintln!("⬡ brain: binary stale after commit — background rebuild spawned")` | **(a)** — runs in `PostToolUse` commit hook |
| 3024 | `eprintln!("⬡ brain: release binary missing — run cargo build --release")` | **(a)** — same hook path |
| 3036 | `eprintln!("⬡ brain: {} MCP tools without CLI commands: ...")` | **(a)** — same hook path |

### hook.rs — non-output (category c)

| Line(s) | Kind |
|----------|------|
| 298-307 | Comments: "brain notifications", "brain daemon" |
| 415-457 | `ensure_brain_daemon_running()` + doc-comment + `brain-daemon.pid` path + `.args(["brain", ...])` |

---

## hex-cli/src/commands/brain_alias.rs

| Line | String | Category |
|------|--------|----------|
| 26 | `eprintln!("warning: \`hex brain\` is deprecated; use \`hex sched\` instead")` | **(b)** — only reachable when user invokes `hex brain` directly |

### brain_alias.rs — non-output (category c)

| Line(s) | Kind |
|----------|------|
| 1-4 | Module doc-comment |
| 14-18 | Function doc-comment |

---

## hex-cli/src/commands/hey.rs

| Line | String | Category |
|------|--------|----------|
| 434 | `println!("    hex brain enqueue hex-command -- \"<your-command>\"")` | **(a)** — reachable from `hex hey` |
| 439 | `println!("    Try: hex brain enqueue hex-command -- \"<your-command>\"")` | **(a)** — reachable from `hex hey` |
| 466 | `println!("  ⬡ enqueued brain task {}", id)` | **(a)** — reachable from `hex hey` |

### hey.rs — non-output (category c)

| Line(s) | Kind |
|----------|------|
| 23 | Doc-comment: "brain task kind `remote-shell`" |
| 162, 306-308, 394 | Comments/struct fields referencing brain |
| 504 | Error message: `"... hex brain enqueue ..."` — **(a)** (user-facing timeout hint) |

**hey.rs line 504 reclassified as (a):** `"LLM classify timed out ... try manual: hex brain enqueue hex-command -- \"<cmd>\""` — user-facing error.

---

## hex-nexus/src/routes/brain.rs

No println!/eprintln!/log macros with "brain" in output strings. All occurrences are:

| Line(s) | Kind | Category |
|----------|------|----------|
| 1-5 | Module doc-comments | **(c)** |
| 26-369 | Struct names, doc-comments, function names, API paths | **(c)** |

---

## hex-nexus/src/brain_service.rs

| Line | String | Category |
|------|--------|----------|
| 25 | `"brain:model:selection".to_string()` — memory key constant | **(c)** — internal key, not user-facing |
| 28 | Doc-comment | **(c)** |

---

## hex-nexus/src/lib.rs

| Line(s) | Kind | Category |
|----------|------|----------|
| 43, 515-519 | `pub mod brain_service`, spawn call, comments | **(c)** |

---

## hex-nexus/src/state.rs

| Line(s) | Kind | Category |
|----------|------|----------|
| 74-76, 130 | `brain_last_test` field + comments | **(c)** |

---

## hex-nexus/src/routes/mod.rs

| Line(s) | Kind | Category |
|----------|------|----------|
| 39 | `pub mod brain;` | **(c)** |
| 461-462 | Route mount: `/api/brain/status`, `/api/brain/test` | **(c)** — structural, not user-facing output |

---

## hex-nexus/src/routes/classifier.rs

| Line(s) | Kind | Category |
|----------|------|----------|
| 11, 46 | Import + comment referencing `brain::INTENT_RULES` | **(c)** |

---

## hex-nexus/src/adapters/brain.rs

| Line | Kind | Category |
|------|------|----------|
| 1 | Import from `domain::brain` | **(c)** |

---

## hex-nexus/tests/sched_daemon_terminal_state.rs

All 45+ occurrences are test helper functions, assertions, and doc-comments. Category **(c)**.

---

## hex-agent/src/worker.rs

No println!/eprintln!/log macros with "brain" in user-visible output. All occurrences are:

| Line(s) | Kind | Category |
|----------|------|----------|
| 1-3, 86-87, 101, 137 | Doc-comments | **(c)** |
| 179 | API URL string: `"brain-task:"` in search query | **(c)** |
| 235 | Comment: "operators see ... in `hex brain queue list`" | **(c)** |
| 263 | Memory key: `format!("brain-task:{id}")` | **(c)** |

---

## hex-core/src/domain/mod.rs + hex-core/src/domain/brain_tests.rs + hex-core/src/ports/brain.rs + hex-core/src/ports/mod.rs

No println!/eprintln!/log macros. All are module declarations, imports, and test module names. Category **(c)**.

---

## spacetime-modules/hexflo-coordination/src/lib.rs

| Line(s) | Kind | Category |
|----------|------|----------|
| 3122-3141 | Block comment explaining why brain tasks use hexflo_memory not a dedicated table | **(c)** |

---

## Summary

| Category | Count | Description |
|----------|-------|-------------|
| **(a)** Reachable from `hex sched` | **31** | User-facing println!/eprintln! strings on the canonical sched path |
| **(b)** Only from `hex brain` alias | **1** | Deprecation warning in brain_alias.rs |
| **(c)** Internal / test / comment | ~120+ | Function names, doc-comments, API paths, memory keys, test code |

### Category (a) — full list for rename scope

1. `hex-cli/src/commands/sched.rs:64` — `"brain-state dir"`
2. `hex-cli/src/commands/sched.rs:71` — `"brain-state write"`
3. `hex-cli/src/commands/sched.rs:74` — `"brain-state encode"`
4. `hex-cli/src/commands/sched.rs:206` — `"enqueued brain task"`
5. `hex-cli/src/commands/sched.rs:239` — `"Brain service not configured"`
6. `hex-cli/src/commands/sched.rs:752` — `"⬡ hex brain validate"`
7. `hex-cli/src/commands/sched.rs:1029` — `"⬡ hex brain prime"`
8. `hex-cli/src/commands/sched.rs:1576` — `"⬡ brain daemon starting"`
9. `hex-cli/src/commands/sched.rs:1588` — `"⬡ brain tick at"`
10. `hex-cli/src/commands/sched.rs:1694` — `"leasing brain task"`
11. `hex-cli/src/commands/sched.rs:1816` — `"⬡ brain daemon received ctrl-C"`
12. `hex-cli/src/commands/sched.rs:1832` — `"brain daemon already running"`
13. `hex-cli/src/commands/sched.rs:1862` — `"⬡ brain daemon started in background"`
14. `hex-cli/src/commands/sched.rs:1867` — `"stop with: hex brain daemon-stop"`
15. `hex-cli/src/commands/sched.rs:1878` — `"brain daemon not running"` (no pid)
16. `hex-cli/src/commands/sched.rs:1888` — `"brain daemon"` (stale pid)
17. `hex-cli/src/commands/sched.rs:1907` — `"⬡ brain daemon stopped"`
18. `hex-cli/src/commands/sched.rs:1927` — `"⬡ brain daemon running"`
19. `hex-cli/src/commands/sched.rs:1935` — `"brain daemon not running"` (stale pid)
20. `hex-cli/src/commands/sched.rs:1941` — `"brain daemon not running"` (no file)
21. `hex-cli/src/commands/sched.rs:2000` — `"⬡ watching brain_tick events"`
22. `hex-cli/src/commands/sched.rs:2110` — `"brain_tick"` in event display
23. `hex-cli/src/commands/sched.rs:3009` — `"No pending brain tasks."`
24. `hex-cli/src/commands/sched.rs:3067` — `"No brain tasks in history"`
25. `hex-cli/src/commands/sched.rs:3160` — `"cleared ... brain tasks"`
26. `hex-cli/src/commands/sched.rs:3167` — `"No pending brain tasks to drain."`
27. `hex-cli/src/commands/sched.rs:3170` — `"draining ... pending brain task(s)"`
28. `hex-cli/src/commands/hook.rs:3017` — `"⬡ brain: binary stale after commit"`
29. `hex-cli/src/commands/hook.rs:3024` — `"⬡ brain: release binary missing"`
30. `hex-cli/src/commands/hook.rs:3036` — `"⬡ brain: ... MCP tools without CLI commands"`
31. `hex-cli/src/commands/hey.rs:434` — `"hex brain enqueue hex-command"`
32. `hex-cli/src/commands/hey.rs:439` — `"Try: hex brain enqueue hex-command"`
33. `hex-cli/src/commands/hey.rs:466` — `"enqueued brain task"`
34. `hex-cli/src/commands/hey.rs:504` — `"... hex brain enqueue hex-command"` (timeout error)
