# Pipeline Failure Report: hex-fixer Never Converges

**Investigation Date:** 2026-03-25
**SWARM:** investigate-dev-pipeline-failures (6a4f27aa-c58f-45b5-89d0-01e688716b47)
**Symptom:** hex-fixer runs 5 iterations in 3 separate rounds (15 total fix attempts) without resolving compile errors. Last task result: "hex-fixer: failed — fix agent failed".

---

## Architecture Overview: Two Separate Fix Paths

There are **two independent fix loops** in the codebase that are NOT the same:

| Path | Location | Called By | Iterations |
|------|----------|-----------|------------|
| **Quality loop** | `validate_phase.rs::run_quality_loop()` | TUI (Quick mode only) | up to 3 or 5 |
| **Objective loop** | `supervisor.rs::run_tier()` | TUI (Auto/Interactive mode), supervisor | up to 5 (hardcoded `MAX_ITERATIONS`) |

The "three separate rounds of 5 iterations" pattern indicates the **objective loop** in `supervisor.rs::run_tier()` is running, not the quality loop. This loop is called once per tier (Tier 0, Tier 1, Tier 2, etc.), and each tier gets its own 5 iterations. If 3 tiers each fail 5 times = 15 total fix attempts.

---

## Finding 1: The Fixer Does NOT Carry Error Context Between Iterations

**Root cause #1.**

In `supervisor.rs::run_tier()` (line 983–1076), each iteration calls `evaluate_all()` to re-evaluate all objectives fresh, producing a new `ObjectiveState` with `blocking_issues`. These blocking_issues are the error strings passed to the fixer.

The fixer (`FixAgent::execute()` in `fix_agent.rs`) receives `error_context` built from the current iteration's `blocking_issues` only. There is no accumulation of error history across iterations. Each iteration:
1. Re-runs `cargo check` / `tsc --noEmit`
2. Gets fresh error list from that run
3. Passes only that list to the fixer

This is correct behavior for compile errors specifically (errors may change between runs). However, the critical gap is in **what happens when the fixer produces `status: "unchanged"`** — the loop doesn't detect this and just tries again with the exact same input on the next iteration.

---

## Finding 2: The Fixer Targets Only ONE File Per Iteration — Likely the Wrong One

**Root cause #2 — most likely primary cause.**

`infer_fix_target()` in `supervisor.rs` (line 2136–2205) uses a fragile heuristic to select which file to fix:

```rust
// Look for patterns like "path/to/file.ts:42:" or "path/to/file.ts: message"
let parts: Vec<&str> = issue.splitn(2, ':').collect();
let candidate = parts.first().unwrap_or(&"").trim();
```

It takes the **first blocking issue** only and splits on `:` to extract a file path. For Rust compile errors, the format from `evaluate_code_compiles()` is `"file:line: message"` — this parse works only if the path has no colon. But there is a deeper problem:

**The fixer fixes one file per iteration, but Rust compile errors span multiple files.** After fixing file A, the next iteration may still fail because file B has errors. The fixer then picks a target from the new error list (which may be file B, or still file A if it introduced new errors). With 5 iterations and N files with errors, convergence only happens if N ≤ 5 AND each fix doesn't break something else.

Additionally, `call_fix_inference()` in `validate_phase.rs` (the quality loop path) has an even weaker fallback: if no file can be extracted from the error text, it calls `find_first_source_file()` which walks the directory and returns **the first file found alphabetically**. This is almost certainly the wrong file for the errors being reported.

---

## Finding 3: "fix agent failed" Is an Anyhow Context Wrap on `FixAgent::execute()`

`supervisor.rs` line 2092–2095:
```rust
let result = agent
    .execute(input, model_override, provider_pref)
    .await
    .context("fix agent failed")?;
```

The `?` propagates the error up through `execute_agent_tracked()` → `run_tier()`. The error causes `run_tier()` to return `Err(...)` for non-reviewer objectives (reviewer errors are silently skipped; line 1048–1065). For `CodeCompiles`, this is a hard failure that aborts the entire tier.

The underlying error from `FixAgent::execute()` can be:
- `POST /api/inference/complete failed for fix` — nexus network error or inference API error
- `model selection failed for fix` — free-tier quota exhausted, no fallback model available
- `loading fix-compile prompt template` — embedded asset not found
- `writing fix to <path>` — filesystem write permission error

**Most likely cause of the hard failure:** free-tier model quota exhaustion during the fix call. If `select_model(TaskType::CodeEdit, ...)` fails because all free-tier models are exhausted and no paid fallback is configured, the entire fix chain fails immediately rather than gracefully degrading.

---

## Finding 4: The Quality Loop (`validate_phase.rs`) Has the Same Single-File Problem

In `call_fix_inference()` (line 814–916), the method:
1. Tries to extract the error file from the error text via `extract_error_file()`
2. Falls back to `find_first_source_file()` — the alphabetically-first source file

When it succeeds in extracting a file, it **only sends that one file's content** to the fixer and **writes the fix only to that one file**. For projects with compile errors spanning multiple files, this is structurally incapable of converging unless every error happens to be in the same file.

The Rust error parser in `extract_error_file()` (line 964–979) parses lines matching `--> path:line:col`. This works, but only selects the **most-mentioned file** — which is the file with the most error lines, not necessarily the one where the fix is simplest or most impactful.

---

## Finding 5: When max_iterations Is Reached — No Escalation, Just Soft Failure

`run_tier()` returns `TierResult::MaxIterations` (not `Err`). The calling loop at line 953–958:

```rust
let passed = matches!(&result, TierResult::AllPassed { .. });
tier_results.push((tier, result));

if !passed {
    info!(tier, "tier did not fully pass — continuing to next tier");
}
```

The pipeline **continues to the next tier** regardless. There is no escalation, no human notification, no stopping. This means a project with persistent compile errors in Tier 0 will still have Tier 1, Tier 2, etc. attempt to run — which are likely to also fail because the base code doesn't compile.

The `on_max_iterations: escalate` field exists in the swarm YAML config and `FeedbackLoopConfig` struct, but **it is only used by `workflow_engine.rs`** (the YAML-driven workflow engine). The supervisor's `run_tier()` has a hardcoded `MAX_ITERATIONS = 5` constant and no escalation logic.

---

## Finding 6: The Fixer Never Updates Blocking Issues Before Retry

After `execute_agent_tracked()` returns successfully, the loop **does not re-evaluate immediately** — it goes back to the top of the `for iteration` loop, which calls `evaluate_all()` again. This is correct. However, if `FixAgent::execute()` returns `status: "unchanged"` (model produced identical content), the supervisor logs it but treats it as success and proceeds to re-evaluate. On re-evaluation, the same errors are found, and the fixer is called again with the same input — infinite loop within the 5-iteration budget.

There is no detection of `"unchanged"` status in the supervisor's `execute_agent_tracked()` path (lines 2048–2111). The status is logged but not used to break or escalate.

---

## Root Cause Summary

| # | Root Cause | Severity |
|---|------------|----------|
| 1 | **Single-file targeting**: fixer patches one file per iteration; multi-file compile errors cannot converge | Critical |
| 2 | **No "unchanged" detection**: fixer returning identical content re-triggers the same fix next iteration | High |
| 3 | **Inference failure propagates as hard error**: free-tier quota exhaustion causes `?` to abort the tier, reported as "fix agent failed" | High |
| 4 | **No cross-iteration error context**: each iteration starts fresh; fixer cannot learn from previous failed attempts | Medium |
| 5 | **No escalation at max_iterations**: pipeline silently moves on to next tier rather than stopping or notifying | Medium |

---

## Specific Answers

**Does the fixer carry error context between iterations?**
No. Each iteration calls `evaluate_all()` fresh. The only state carried between iterations is `prior_results` (a `HashMap<Objective, bool>`) which only tracks whether an agent has run before, not what errors it saw.

**Does the fixer correctly write fixed code back to the right path?**
Partially. `FixAgent::execute()` uses `input.target_file` (an absolute path resolved by `infer_fix_target()`) and writes directly to it. The path logic is correct when `infer_fix_target()` successfully parses a file path from the error text. The failure mode is when no path is parsed — it falls back to `first_source_file_for_tier()`, which returns the first file found for the tier (not necessarily where the errors are).

**What happens when max_iterations is reached?**
`run_tier()` returns `TierResult::MaxIterations`. The supervisor logs "tier did not fully pass — continuing to next tier" and moves on. No escalation, no hard failure, no human notification.

**Is "fix agent failed" a network error, parsing error, or logic error?**
It is an anyhow-wrapped error from `FixAgent::execute()`. The most likely causes in order of probability: (1) inference API failure / quota exhaustion during `POST /api/inference/complete`, (2) model selection failure when all free-tier models are exhausted, (3) filesystem write error.

**Root cause: why does the fixer never converge?**
The primary structural reason is that the fixer operates on one file per iteration, but real-world Rust/TypeScript projects with generated code have compile errors across multiple files. Even with 5 iterations, if 3+ files have errors and each fix introduces new errors (which LLMs frequently do), convergence requires more iterations than the budget allows. The secondary reason is that if inference itself fails (quota), the entire tier aborts immediately rather than gracefully using a fallback or skipping.

---

## Recommended Fixes

1. **Multi-file fix pass**: Collect ALL error files from blocking_issues, not just the first. Run a fix pass per file in a single iteration, or batch all errors into one inference call with all affected file contents.

2. **Detect "unchanged" and escalate**: If `FixAgent` returns `status: "unchanged"`, the supervisor should immediately try a stronger model or break the loop rather than burning remaining iterations on identical input.

3. **Harden inference error handling**: Wrap `agent.execute()` so quota/network errors don't abort the tier — log the failure, mark the objective as `"blocked"`, and continue to next tier rather than propagating `Err`.

4. **Add iteration context to fixer**: Pass the previous iteration's blocking_issues alongside the current ones so the fixer prompt includes "these errors persisted after your last attempt".

5. **Wire escalation in `run_tier()`**: When `MaxIterations` is reached, call `hex inbox notify` to alert the operator rather than silently continuing.
