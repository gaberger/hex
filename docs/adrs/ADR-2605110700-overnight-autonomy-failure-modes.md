# ADR-2605110700: Overnight Autonomy Failure Modes + Remediation

Status: **Accepted** (shipped 2026-05; R1 in commit fe77411d `fix(executor): cargo_check becomes hard-gate in action_executor`; R2/R3/R4 in commit 2ac57f07 `fix(cycler): per-file commit, no stash, wall-clock watchdog`)
Date: 2026-05-11
Drivers: Overnight autonomous run 2026-05-10 22:25 EDT → 2026-05-11 05:44 EDT delivered 0 commits of autonomous code despite 61 SOP runs across 6 personas. Stash-on-build-fail safety worked exactly as designed but produced no shippable code value. Operator woke to a missing report + no observable progress.

## Context

The first multi-hour unattended overnight run exposed four failure modes that the synchronous (operator-present) loop had hidden:

### F1 — Verifier-loop divergence
The cargo_check chain inside `code_patch` (commit `47c6867e`) embeds compile errors in the tool_result the LLM receives. In synchronous use, the LLM sees the errors in the next REASON round and emits a corrective replace_string patch. Overnight observation: the LLM **does not reliably self-correct on cargo_check failures within a single SOP run**. Patches compound the brokenness rather than fixing it.

Evidence: every cycle's `cargo check` failed; cycler stashed and rolled back. 5 stashes contain pyramided broken edits.

### F2 — Stash-on-fail loses learning
The cycler script's safety net (`git stash` on cargo_check fail) successfully prevented broken commits but ALSO prevented partial progress from accumulating. Each cycle started fresh from the last known-good commit. The team never saw their prior cycle's attempt — every cycle is effectively a cold start.

### F3 — Build failure poisons doc-only work
The cycler treated the cycle as atomic: stage everything → cargo check → all-or-nothing commit. When Rust patches broke build, doc-only specs (which can't break anything) were stashed alongside the Rust. 8 valid specs sat in stashes overnight; recovered manually at 06:52 EDT.

### F4 — Wake-up trigger missed target
The "wait for cycler PID to exit" watcher only fires when the cycler process terminates. But the cycler can be stuck inside `sleep 4500` — process is "alive" but doing nothing. Plus the cycler ran 48 min past its 06:00 EDT target because the final sleep wasn't time-bounded against target. Operator woke at 06:48 to silence.

## Decision

Four remediations, in priority order:

### R1 — Verifier hook becomes a HARD GATE inside code_patch
**Today**: code_patch returns the cargo_check output as part of its result; LLM consumes it via tool_result; LLM optionally retries.
**Change**: when cargo_check reports errors, code_patch returns `ToolResult::err` with the cargo errors as the error message — the file_write is **rolled back inside the tool** before the proposed_action_open even happens. The LLM cannot accumulate broken state.

Implementation: in `hex-nexus/src/tools/code_patch.rs`, after the cargo_check chain runs, if `check_result.output.errors.len() > 0`: `std::fs::write(target, original_content_backup)` to undo the write, then return `ToolResult::err(format!("cargo_check rejected patch: {} errors\n{}", count, errors_summary), elapsed)`.

This is more invasive than the current advisory pattern but matches the principle: a tool should never report success when it left the system in a worse state.

### R2 — Per-file commit, not per-cycle commit
**Today**: cycler `git add` accumulates all changed paths, runs ONE cargo check, commits all-or-nothing.
**Change**: cycler iterates changed paths, runs `cargo check` on the affected crate per change. Docs: commit immediately (no verification needed). Rust: only commit the file if its crate's `cargo check` passes. Files that fail go to a dated `stash@docs/` for forensic review but don't block the rest.

### R3 — Persistent partial-progress between cycles
**Today**: stash erases everything between cycles.
**Change**: do NOT stash on failure. Leave broken changes in working tree; the next cycle's persona sees them via repo_read and can attempt fixes. The verifier hard-gate from R1 prevents further damage. If the working tree becomes truly unrecoverable, escalate to operator inbox.

### R4 — Wall-clock watchdog independent of process state
**Today**: "wait for cycler PID to exit" hangs if cycler is stuck mid-sleep.
**Change**: two parallel watchers — (a) PID exit, (b) `until [ $(date +%s) -ge $TARGET ]; do sleep 60; done`. EITHER firing wakes the report-compiler. Plus: cycler's main loop checks `date +%s` against target at the TOP of each iteration so it exits cleanly at boundary instead of mid-sleep.

## Consequences

**Accept:**
- R1 makes individual code_patch calls more conservative — some legitimate "land it broken, fix in next round" flows get blocked. Acceptable trade for not accumulating brokenness.
- R3 leaves a possibly-broken working tree for the next cycle to inherit. The verifier hard-gate ensures it doesn't get worse.

**Defer:**
- A true "rebase-style" autonomy where each cycle's failed work is preserved as a branch for inspection — needs git-worktree infrastructure already in place but not wired for this use case.

**Revisit:**
- After the next overnight run, audit: did cycles converge or diverge? How many cycles produced shippable commits? Compare to this run's 0-of-6.

## Workplan

Three code_patches need to land before the next overnight run:
1. `hex-nexus/src/tools/code_patch.rs` — hard-gate on cargo_check errors (R1)
2. `scripts/overnight-cycler.sh` — per-file commit + wall-clock watchdog + skip stash (R2, R3, R4)
3. Move the cycler script from `/tmp/` to `scripts/` so it's version-controlled and reviewable

Test plan: dry-run the cycler for 1 hour with deliberate broken-patch injection; verify that doc files commit but broken Rust does not, and verify wall-clock watchdog fires at the target even if a sleep is running.

## References

- 8 specs recovered from overnight stashes — committed in `971ab896`
- 5 stashes pending forensics (cycles 1-5)
- ADR-2605082500 typed-tool library + SOP execution (the foundation this overnight exercised)
- commit `47c6867e` — cargo_check inline chain (the verifier this ADR proposes hardening)
