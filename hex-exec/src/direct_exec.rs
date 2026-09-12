//! Minimal "cut the pipeline" executor — ADR-2026-06-04-1740 Path A.
//!
//! The whole factory's doing-path (org_responder → SOP phases → personas →
//! twin approval → commitments → 4 duplicate reason loops → conductor) is, for
//! execution, accidental complexity: ~10 independently-failing stages
//! coordinating through mutable STDB claims/leases. Across two sessions it never
//! autonomously produced a working composed artifact.
//!
//! This is the irreducible loop that actually ships code:
//!
//!   task {instruction, file, evidence} →
//!     read the file (deterministic, NO model-driven exploration) →
//!     ONE inference call asking for a precise {mode, old_string, new_string} edit →
//!     apply the edit → run the evidence command (must exit 0) →
//!     pass → commit.  fail → feed the error + current content back, retry (≤ max).
//!     still failing → return failed (visible, not a silent escalation).
//!
//! No personas. No board. No twin. No commitments. No claims. The
//! over-exploration failure can't occur: the file is pre-grounded and there are
//! no exploration tools to loop on.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::LazyLock;

#[derive(Debug, Clone, Deserialize)]
pub struct DirectTask {
    /// What to do, in plain language.
    pub instruction: String,
    /// Repo-relative path of the file to edit.
    pub file: String,
    /// Shell command that must exit 0 for the change to count as done
    /// (e.g. "cargo test -p hex-nexus test_foo").
    pub evidence: String,
    /// Override the reasoning model (default: a calibrated code model).
    #[serde(default)]
    pub model: Option<String>,
    /// Max edit→verify attempts before giving up (default 3). Single-shot path.
    #[serde(default)]
    pub max_attempts: Option<u32>,
    /// Use the single-shot path (read → one edit → evidence → retry) instead of
    /// the default multi-step ReAct tool-use loop (ADR-2606071XXX).
    #[serde(default)]
    pub fast: bool,
    /// Max ReAct loop steps (tool calls) before giving up (default 12). Ignored
    /// in `fast` mode.
    #[serde(default)]
    pub max_steps: Option<u32>,
    /// Run in a dedicated `hex/auto/<id>` worktree instead of the operator's
    /// checked-out branch (ADR-2606071323). Defaults to TRUE (safe-by-default):
    /// only the interactive operator path (`hex do`) opts out with `isolate:false`
    /// to commit on its own branch. Absent ⇒ isolated.
    #[serde(default)]
    pub isolate: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct DirectResult {
    pub ok: bool,
    pub attempts: u32,
    pub edit_applied: bool,
    pub committed: Option<String>,
    pub evidence_passed: bool,
    pub evidence_output: String,
    pub error: Option<String>,
}

/// Hard cap on grounded lines. The local code model is loaded with a 4096-token
/// context; keep the prompt well under that or the input gets silently truncated
/// and the model produces garbage (measured 2026-06-04: input_tokens=4095).
const MAX_GROUND_LINES: usize = 200;
const WINDOW: usize = 24;

// ─── observability: recorded runs ─────────────────────────────────────────────
//
// The new execution model's unit of work is a direct run, not a persona
// conversation. Every run is recorded here so `GET /api/direct/runs`, the CLI,
// and the dashboard can show what the agents actually DID — task, evidence
// verdict, commit — instead of the retired liveness signals (personas/swarms/
// commitments). In-memory ring buffer (last RUN_HISTORY); STDB persistence is a
// follow-up.

const RUN_HISTORY: usize = 200;

/// One recorded agent run — the monitorable unit of the new model. Shared by the
/// direct executor and any other in-nexus agent (e.g. adr-steward) so they all
/// surface in one dashboard feed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectRun {
    pub id: u64,
    /// Which agent produced this run ("direct-executor", "adr-steward", ...).
    pub agent: String,
    pub started_at: String,
    pub instruction: String,
    pub file: String,
    pub model: String,
    pub ok: bool,
    pub attempts: u32,
    /// ReAct loop steps (tool calls). 1 for non-loop runs; == attempts for the
    /// single-shot path. Surfaced in the dashboard so operators see exploration depth.
    #[serde(default)]
    pub steps: u32,
    pub evidence_passed: bool,
    pub committed: Option<String>,
    pub duration_ms: u64,
    pub error: Option<String>,
}

/// Record a ReAct-loop run (multi-step tool-use, ADR-2606071XXX) into the shared
/// feed — keeps the run-buffer internals private to this module.
#[allow(clippy::too_many_arguments)]
pub(crate) fn record_react_run(
    started_at: String,
    task: &DirectTask,
    model: &str,
    ok: bool,
    evidence_passed: bool,
    committed: Option<String>,
    steps: u32,
    duration_ms: u64,
    error: Option<String>,
) {
    let run = DirectRun {
        id: RUN_ID.fetch_add(1, Ordering::Relaxed),
        agent: "direct-react".to_string(),
        started_at,
        instruction: task.instruction.chars().take(240).collect(),
        file: task.file.clone(),
        model: model.to_string(),
        ok,
        attempts: steps,
        steps,
        evidence_passed,
        committed,
        duration_ms,
        error,
    };
    store_run(run);
}

/// Persist a run to the local store.
///
/// There is no in-memory ring buffer any more. It was the fast path for a
/// daemon's HTTP API, and a cache in front of a file that only a short-lived
/// process reads is not a cache — it is a way to report zero runs while the
/// file holds every one of them. Never fails a run: losing a feed entry must
/// not lose an edit.
fn store_run(run: DirectRun) {
    persist_run_async(run);
}

static RUN_ID: AtomicU64 = AtomicU64::new(1);

// Serialize the read→edit→evidence→commit critical section. Two concurrent runs
// touching the working tree / git index race and can false-positive (one reports
// ok=true + a commit while another's edit interleaves) — found by the 2026-06-04
// review swarm. Global (not per-file) because git add/commit is process-global.
static EXEC_LOCK: LazyLock<tokio::sync::Mutex<()>> = LazyLock::new(|| tokio::sync::Mutex::new(()));

/// Did the evidence command actually exercise anything?
///
/// A `cargo test <filter>` matching zero tests exits 0 and prints "running 0
/// tests" — a pass that verified nothing. The gate exists to require the change
/// be *exercised*, so that is not satisfied.
///
/// **This counts across suites rather than matching one line.** The first
/// version asked `output.contains("0 passed; 0 failed; 0 ignored")`, which is
/// true of any multi-binary cargo run that happens to contain one empty target
/// — a lib with no inline tests, a binary whose tests live in `tests/`. A real
/// run of a scaffolded project, 39 tests passing across four binaries, was
/// judged vacuous by that check and its commit would have been rejected. A gate
/// that fails for a reason unrelated to what it gates is indistinguishable from
/// the gated thing being broken.
///
/// Recognises cargo, `go test`, and node's TAP output. When it recognises
/// nothing — a `make check`, a shell script — it returns `false`: we cannot
/// judge, and rejecting every unrecognised runner would break more gates than
/// it protects. That limit is real and is why this is a guard against the
/// crudest case, not a proof of coverage.
pub(crate) fn evidence_is_vacuous(output: &str) -> bool {
    match tests_observed(output) {
        Some(0) => true,
        _ => false,
    }
}

/// How many tests the output reports having run, or `None` if no runner we
/// know about is recognisable in it.
pub fn tests_observed(output: &str) -> Option<u64> {
    let mut total: u64 = 0;
    let mut recognised = false;

    for line in output.lines() {
        let t = line.trim();

        // cargo: "test result: ok. 39 passed; 0 failed; …"
        if let Some(rest) = t.strip_prefix("test result:") {
            recognised = true;
            if let Some(n) = rest.split_whitespace().find_map(|w| w.parse::<u64>().ok()) {
                total = total.saturating_add(n);
            }
            continue;
        }
        // cargo: "running 12 tests"
        if let Some(rest) = t.strip_prefix("running ") {
            if rest.ends_with(" tests") || rest.ends_with(" test") {
                recognised = true;
            }
            continue;
        }
        // node --test TAP: "# pass 88"
        if let Some(rest) = t.strip_prefix("# pass ") {
            recognised = true;
            if let Ok(n) = rest.trim().parse::<u64>() {
                total = total.saturating_add(n);
            }
            continue;
        }
        // go: "ok  \tpkg\t0.01s" means the package's tests ran and passed;
        // "?   \tpkg\t[no test files]" means it had none.
        if t.starts_with("ok  \t") || t.starts_with("ok\t") {
            recognised = true;
            total = total.saturating_add(1);
            continue;
        }
        if t.contains("[no test files]") {
            recognised = true;
            continue;
        }
        if t.starts_with("--- PASS") || t.starts_with("=== RUN") {
            recognised = true;
            total = total.saturating_add(1);
            continue;
        }
    }

    recognised.then_some(total)
}

#[cfg(test)]
mod vacuous_tests {
    use super::{evidence_is_vacuous, tests_observed};

    #[test]
    fn a_cargo_run_with_no_tests_is_vacuous() {
        let out = "running 0 tests\n\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out";
        assert!(evidence_is_vacuous(out));
        assert_eq!(tests_observed(out), Some(0));
    }

    /// The regression. Four cargo binaries, one of them empty, 39 tests in
    /// total. The old substring check called this vacuous and would have
    /// rejected a correct, fully-gated commit.
    #[test]
    fn one_empty_binary_among_several_is_not_vacuous() {
        let out = "\
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 33 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out";
        assert_eq!(tests_observed(out), Some(39));
        assert!(!evidence_is_vacuous(out));
    }

    #[test]
    fn node_tap_output_is_counted() {
        assert_eq!(tests_observed("# tests 88\n# pass 88\n# fail 0"), Some(88));
        assert!(!evidence_is_vacuous("# pass 88"));
        assert!(evidence_is_vacuous("# pass 0"));
    }

    #[test]
    fn a_go_run_with_only_empty_packages_is_vacuous() {
        assert!(evidence_is_vacuous("?   \tdemo/internal/ports\t[no test files]"));
        assert!(!evidence_is_vacuous(
            "?   \tdemo/internal/ports\t[no test files]\nok  \tdemo\t0.004s"
        ));
    }

    /// An unrecognised runner is not judged. Rejecting every `make check`
    /// would break more gates than the guard protects.
    #[test]
    fn an_unrecognised_runner_is_not_called_vacuous() {
        assert_eq!(tests_observed("Build succeeded.\nAll checks passed."), None);
        assert!(!evidence_is_vacuous("Build succeeded.\nAll checks passed."));
    }
}

fn record_run(started_at: String, task: &DirectTask, model: &str, r: &DirectResult, duration_ms: u64) {
    let run = DirectRun {
        id: RUN_ID.fetch_add(1, Ordering::Relaxed),
        agent: "direct-executor".to_string(),
        started_at,
        instruction: task.instruction.chars().take(240).collect(),
        file: task.file.clone(),
        model: model.to_string(),
        ok: r.ok,
        attempts: r.attempts,
        steps: r.attempts,
        evidence_passed: r.evidence_passed,
        committed: r.committed.clone(),
        duration_ms,
        error: r.error.clone(),
    };
    store_run(run);
}

// ── local persistence (survives a restart, needs no database) ────────────────

/// Fire-and-forget persist of a run. The in-memory ring is the fast path; this is the copy that
/// outlives the process. Never blocks or fails a recorder.
///
/// Was a `record_agent_run` reducer call to SpacetimeDB — so a feed that exists to be READ needed a
/// database WRITE to a service the daemon owned, and the agent loop carried that dependency purely
/// to leave a trace of itself.
fn persist_run_async(run: DirectRun) {
    // `<started_at>#<seq>` stays unique across restarts even though RUN_ID resets to 1, because
    // started_at differs. Kept from the STDB key for exactly that reason.
    let id = format!("{}#{}", run.started_at, run.id);
    crate::local_store::persist_run(&json!({
        "id": id,
        "agent": run.agent,
        "started_at": run.started_at,
        "instruction": run.instruction,
        "file": run.file,
        "model": run.model,
        "ok": run.ok,
        "attempts": run.attempts,
        "evidence_passed": run.evidence_passed,
        "committed": run.committed,
        "duration_ms": run.duration_ms,
        "error": run.error,
    }));
}

/// Newest-first snapshot of recorded runs.
///
/// Reads the local store directly. It used to read an in-memory ring buffer
/// that `hydrate_feed()` filled at daemon startup — and when the daemon went,
/// nothing called it. `hex do` is a short-lived process, so the buffer was
/// empty on every read and `hex do runs` reported 0 while
/// `~/.hex/agent-runs.jsonl` held every run that had ever happened.
///
/// The rows are mapped field by field rather than through
/// `serde_json::from_value::<DirectRun>`, because the two shapes disagree and
/// always have: the persisted `id` is the string `<started_at>#<seq>` — unique
/// across restarts, which is why it is written that way — while `DirectRun.id`
/// is a `u64` display number. A whole-struct deserialize fails on every row,
/// and `hydrate_feed` swallowed that with `.ok()`. So the feed was broken
/// twice over: never called, and wrong if it had been.
///
/// The display id is assigned here instead, newest highest.
pub fn runs_snapshot() -> Vec<DirectRun> {
    let rows = crate::local_store::recent_runs(RUN_HISTORY);
    let n = rows.len() as u64;
    rows.into_iter()
        .enumerate()
        .map(|(idx, v)| {
            let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
            let opt = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_string);
            let u = |k: &str| v.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
            let b = |k: &str| v.get(k).and_then(|x| x.as_bool()).unwrap_or(false);
            DirectRun {
                id: n - idx as u64,
                agent: s("agent"),
                started_at: s("started_at"),
                instruction: s("instruction"),
                file: s("file"),
                model: s("model"),
                ok: b("ok"),
                attempts: u32::try_from(u("attempts")).unwrap_or(u32::MAX),
                steps: u32::try_from(u("steps")).unwrap_or(u32::MAX),
                evidence_passed: b("evidence_passed"),
                committed: opt("committed"),
                duration_ms: u("duration_ms"),
                error: opt("error"),
            }
        })
        .collect()
}

/// Aggregate counters for an at-a-glance monitor header.
pub fn runs_summary() -> Value {
    let runs = runs_snapshot();
    let total = runs.len();
    let passed = runs.iter().filter(|r| r.ok).count();
    let committed = runs.iter().filter(|r| r.committed.is_some()).count();
    json!({
        "total": total,
        "passed": passed,
        "failed": total - passed,
        "committed": committed,
        "pass_rate": if total > 0 { passed as f64 / total as f64 } else { 0.0 },
    })
}

/// Run one task end-to-end and record it. Returns a structured, honest result —
/// `ok` is true ONLY if the evidence command exited 0 and the change committed.
pub async fn execute_direct(task: DirectTask) -> DirectResult {
    let started = std::time::Instant::now();
    let started_at = chrono::Utc::now().to_rfc3339();
    let Some(model) = resolve_model(&task) else {
        return DirectResult::err(NO_MODEL_CONFIGURED.to_string());
    };

    // Default (ADR-2606071XXX): the multi-step ReAct tool-use loop — the agent
    // explores (grep/read/cargo_check) before editing. `--fast` keeps the
    // single-shot path (read → one edit → evidence → retry) for trivial edits.
    if task.fast {
        let result = execute_direct_inner(task.clone()).await;
        record_run(started_at, &task, &model, &result, started.elapsed().as_millis() as u64);
        result
    } else {
        // Evidence-gated best-of-N across candidate models (ADR-2606072044): try
        // each in order, commit the first that passes. Single-model configs resolve
        // to a one-element list, so this is a no-op for them.
        let (result, steps, used_model) = crate::direct_react::react_execute_best_of_n(task.clone()).await;
        record_react_run(
            started_at,
            &task,
            &used_model,
            result.ok,
            result.evidence_passed,
            result.committed.clone(),
            steps,
            started.elapsed().as_millis() as u64,
            result.error.clone(),
        );
        result
    }
}

/// The model this task runs on: the task's own choice, then the environment
/// override, then the project's configured tier.
///
/// The last step used to be a hardcoded model id. That is the founding-goal G1
/// failure the tier module exists to remove — a caller that names a model
/// cannot be re-pointed by editing configuration, which is the entire content
/// of "model independence". `hex_infer::tier_model` returns `None` rather than
/// guessing, on purpose, so an unconfigured project fails with a message that
/// names the missing key instead of silently running on a model nobody chose.
pub(crate) fn resolve_model(task: &DirectTask) -> Option<String> {
    task.model
        .clone()
        .or_else(|| std::env::var("HEX_DIRECT_MODEL").ok())
        .or_else(|| hex_infer::tier_model("t2"))
}

/// What to tell an operator whose project configures no model for the do-loop.
pub(crate) const NO_MODEL_CONFIGURED: &str =
    "no model configured — set inference.tier_models.t2 in .hex/project.json, \
     pass --model, or set HEX_DIRECT_MODEL";

/// Should this run be isolated to its own worktree? Default TRUE (ADR-2606071323)
/// — only the operator path opts out via `isolate:false`.
pub(crate) fn want_isolation(task: &DirectTask) -> bool {
    task.isolate.unwrap_or(true)
}

impl DirectResult {
    pub(crate) fn err(msg: String) -> Self {
        DirectResult {
            ok: false,
            attempts: 0,
            edit_applied: false,
            committed: None,
            evidence_passed: false,
            evidence_output: String::new(),
            error: Some(msg),
        }
    }
}

async fn execute_direct_inner(task: DirectTask) -> DirectResult {
    // Serialize the whole acquire→read→edit→evidence→commit→finish section.
    let _exec_guard = EXEC_LOCK.lock().await;

    // ADR-2606071323: confine the run to its own worktree unless the operator
    // explicitly opted out. Never silently fall back to the operator's tree.
    let isolate = want_isolation(&task);
    let slug = crate::direct_workspace::next_run_slug();
    let workspace = match crate::direct_workspace::RunWorkspace::acquire(&slug, isolate) {
        Ok(w) => w,
        Err(e) => return DirectResult::err(format!("workspace: {e}")),
    };
    if let Err(e) = workspace.assert_off_operator_tree() {
        workspace.finish(false);
        return DirectResult::err(e);
    }
    let repo_root = workspace.workdir().to_path_buf();
    let factory = workspace.is_isolated();
    let result = exec_attempts(&task, &repo_root, factory).await;
    workspace.finish(result.ok);
    result
}

async fn exec_attempts(task: &DirectTask, repo_root: &std::path::Path, factory: bool) -> DirectResult {
    // Snapshot pre-run dirty files so the commit includes the supporting files the
    // evidence depends on (ADR-2606080915 follow-up — do-loop commit-gap fix).
    let start_dirty = dirty_paths(repo_root).await;
    let max_attempts = task.max_attempts.unwrap_or(3).clamp(1, 6);
    let Some(model) = resolve_model(task) else {
        return DirectResult::err(NO_MODEL_CONFIGURED.to_string());
    };

    let abs_path = repo_root.join(&task.file);

    // Phase 2 (ADR-2606061359): assemble graph-context + lessons once and prepend
    // it to every edit prompt — the single agent reasons with structural context
    // + memory (Hermes/OpenClaw), not from the file slice alone.
    let context_block = gather_context(task).await;

    let mut result = DirectResult {
        ok: false,
        attempts: 0,
        edit_applied: false,
        committed: None,
        evidence_passed: false,
        evidence_output: String::new(),
        error: None,
    };

    let mut last_error: Option<String> = None;

    for attempt in 1..=max_attempts {
        result.attempts = attempt;

        // 1. Read the current file (re-read each attempt: prior attempt may have edited it).
        let content = match std::fs::read_to_string(&abs_path) {
            Ok(c) => c,
            Err(e) => {
                result.error = Some(format!("read {}: {}", task.file, e));
                return result;
            }
        };
        let grounded = ground_window(&content, &task.instruction);

        // 2. ONE inference call for a precise edit.
        let edit = match request_edit(&model, task, &grounded, &context_block, last_error.as_deref()).await {
            Ok(e) => e,
            Err(e) => {
                last_error = Some(format!("inference: {}", e));
                tracing::warn!(attempt, error = %e, "direct_exec: inference failed");
                continue;
            }
        };

        // 3. Apply the edit to the real file.
        if let Err(e) = apply_edit(&abs_path, &content, &edit) {
            last_error = Some(format!("apply: {}", e));
            tracing::warn!(attempt, error = %e, "direct_exec: edit apply failed");
            continue;
        }
        result.edit_applied = true;
        tracing::info!(attempt, file = %task.file, mode = %edit.mode, "direct_exec: edit applied");

        // 4. Run the evidence command.
        let (passed, output) = run_evidence(&task.evidence, repo_root).await;
        result.evidence_output = output.chars().take(4000).collect();
        // A `cargo test <filter>` that matches ZERO tests prints "running 0 tests"
        // / "0 passed; 0 failed" and still exits 0 — a vacuous pass. The whole point
        // of the gate is that the change is *verified*, so reject it (found by the
        // 2026-06-04 review swarm).
        let vacuous = passed && evidence_is_vacuous(&output);
        if passed && !vacuous {
            result.evidence_passed = true;
            // 5. Commit the target + the pre-run supporting files (start-snapshot).
            match commit(repo_root, &task.file, &task.instruction, factory, &start_dirty).await {
                Ok(hash) => {
                    result.committed = Some(hash);
                    result.ok = true;
                    tracing::info!(attempt, file = %task.file, "direct_exec: evidence passed, committed");
                    return result;
                }
                Err(e) => {
                    // The edit is good and the gate passed; only git failed. Keep
                    // the change, unstage it, and name the half that broke.
                    let _ = std::process::Command::new("git")
                        .args(["reset", "-q", "--"])
                        .arg(&task.file)
                        .current_dir(repo_root)
                        .output();
                    result.error = Some(crate::direct_react::commit_failure_hint(&e));
                    return result;
                }
            }
        } else {
            // Feed the failure back for the next attempt.
            last_error = Some(if vacuous {
                format!(
                    "evidence `{}` PASSED VACUOUSLY — it ran 0 tests (exit 0 but nothing executed). \
                     The named test must actually EXIST and RUN. Output:\n{}",
                    task.evidence,
                    output.chars().take(2000).collect::<String>()
                )
            } else {
                format!(
                    "evidence `{}` FAILED. Output:\n{}",
                    task.evidence,
                    output.chars().take(2500).collect::<String>()
                )
            });
            tracing::warn!(attempt, vacuous, "direct_exec: evidence not satisfied, retrying with error fed back");
        }
    }

    result.error = Some(format!(
        "exhausted {} attempts without passing evidence. last: {}",
        max_attempts,
        last_error.unwrap_or_default()
    ));
    result
}

// ─── grounding ──────────────────────────────────────────────────────────────

/// Feed the model a focused window when the file is large: the region around the
/// instruction's keywords plus any `#[cfg(test)]` module, with real content so
/// `replace_string` edits match the actual file. Whole file if small enough.
pub(crate) fn ground_window(content: &str, instruction: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let render = |keep: &[bool]| -> String {
        let mut out = String::new();
        let mut eliding = false;
        for (i, line) in lines.iter().enumerate() {
            if keep[i] {
                if eliding {
                    out.push_str("// … (unchanged code elided) …\n");
                    eliding = false;
                }
                out.push_str(&format!("{:>5}  {}\n", i + 1, line));
            } else {
                eliding = true;
            }
        }
        out
    };

    if lines.len() <= MAX_GROUND_LINES {
        return render(&vec![true; lines.len()]);
    }

    // Specific symbols only (contain '_' or len>=8) so generic words like
    // "module"/"assert"/"existing" don't blow the window up.
    let keywords: Vec<String> = instruction
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|w| w.len() >= 8 || w.contains('_'))
        .map(|w| w.to_lowercase())
        .collect();

    let mut keep = vec![false; lines.len()];
    for (i, line) in lines.iter().enumerate() {
        let lc = line.to_lowercase();
        if keywords.iter().any(|k| !k.is_empty() && lc.contains(k.as_str())) {
            let lo = i.saturating_sub(WINDOW);
            let hi = (i + WINDOW).min(lines.len() - 1);
            (lo..=hi).for_each(|j| keep[j] = true);
        }
    }
    // a slice of the test module for style
    if let Some(t) = lines.iter().position(|l| l.contains("#[cfg(test)]") || l.contains("mod tests")) {
        let hi = (t + 50).min(lines.len() - 1);
        (t..=hi).for_each(|j| keep[j] = true);
    }
    // always include the file tail (closing braces / where append lands)
    let tail = lines.len().saturating_sub(40);
    (tail..lines.len()).for_each(|j| keep[j] = true);

    // Hard upper bound (tail-biased): if we kept too much, drop the EARLIEST
    // kept lines until under cap — the test module + append point live at the end.
    let mut budget = keep.iter().filter(|&&k| k).count();
    for i in 0..lines.len() {
        if budget <= MAX_GROUND_LINES {
            break;
        }
        if keep[i] {
            keep[i] = false;
            budget -= 1;
        }
    }
    render(&keep)
}

// ─── Phase 2 (ADR-2606061359): context + memory for the single-agent loop ─────

/// Assemble the Hermes/OpenClaw-style PROJECT CONTEXT block prepended to every
/// edit prompt: the target file's graph neighbourhood (hex-graph engine — hex's
/// structural-context differentiator) plus relevant learned lessons. Best-effort:
/// any failure yields "" and never breaks the edit loop.
pub(crate) async fn gather_context(task: &DirectTask) -> String {
    let mut out = String::new();

    // (a) Graph neighbourhood for the target file, from graph-out/graph.json.
    let bundle = {
        let graph_path = repo_root().join("graph-out").join("graph.json");
        std::fs::read_to_string(&graph_path)
            .ok()
            .and_then(|raw| hex_graph::model::KnowledgeGraph::from_json(&raw).ok())
            .and_then(|g| {
                hex_graph::context::context_for(&g, &task.file, hex_graph::context::ContextOpts { max_each: 15 })
            })
    };
    if let Some(ref b) = bundle {
        out.push_str(&hex_graph::context::render_markdown(b));
    }

    // (b) Learned lessons — GRAPH-RELEVANT first. Rank all lessons by how many of
    // the target file's neighbourhood labels (path/symbols) they mention, so the
    // agent gets the lessons about THIS code, not 6 arbitrary recent ones. Falls
    // back to recency when there's no graph/anchors (ADR-2606061359 memory loop).
    let all = fetch_lessons().await;
    if !all.is_empty() {
        let chosen: Vec<(String, String)> = match &bundle {
            Some(b) => {
                let ranked = hex_graph::context::rank_lessons(b, &all, 6);
                if ranked.is_empty() {
                    all.iter().take(4).cloned().collect()
                } else {
                    ranked.into_iter().map(|s| (s.key, s.value)).collect()
                }
            }
            None => all.iter().take(4).cloned().collect(),
        };
        out.push_str("\n## Lessons (from memory — most relevant to this file)\n");
        for (k, v) in &chosen {
            out.push_str(&format!("- [{}] {}\n", k, v));
        }
    }

    out
}

/// Best-effort pull of `lesson:`/`gap:` entries from the local memory file.
///
/// Was a SQL query against the `hexflo_memory` table over SpacetimeDB's HTTP endpoint, so the
/// agent could not recall a lesson without a database up — for a read of key/value pairs it never
/// writes here. Returned unranked; callers rank by graph relevance
/// (`hex_graph::context::rank_lessons`). Capped to keep the pull bounded.
///
/// One JSON object per line, `{"key": "lesson:…", "value": "…"}`, at `~/.hex/memory.jsonl`. An
/// absent file is an empty memory, not an error — a fresh install has learned nothing yet.
pub async fn fetch_lessons() -> Vec<(String, String)> {
    const CAP: usize = 200;
    crate::local_store::memory_entries(CAP)
}

// ─── the one inference call ───────────────────────────────────────────────────

pub(crate) struct Edit {
    pub(crate) mode: String, // replace_string | append | create
    pub(crate) old_string: String,
    pub(crate) new_string: String,
}

async fn request_edit(
    model: &str,
    task: &DirectTask,
    grounded: &str,
    context: &str,
    prior_error: Option<&str>,
) -> Result<Edit, String> {

    let system = "You are a precise Rust code editor. Reply in EXACTLY this format and nothing \
        else (no prose before or after):\n\
        First line: `MODE: append` to add code to the END of the file, or `MODE: replace` to \
        replace an existing snippet.\n\
        For MODE: append — then ONE fenced code block containing the code to append:\n\
        ```\n<code to append>\n```\n\
        For MODE: replace — then TWO fenced code blocks: first the EXACT existing snippet copied \
        verbatim from the file (it MUST occur exactly once), then its replacement:\n\
        ```\n<exact existing snippet>\n```\n\
        ```\n<replacement>\n```\n\
        Never include the leading line numbers shown in the file. Make the SMALLEST change that \
        satisfies the task and keep surrounding code byte-for-byte identical.";

    let mut user = String::new();
    if !context.is_empty() {
        // Phase 2: structural neighbourhood + lessons. Read-only grounding —
        // the agent edits only the FILE below, but reasons with this context.
        user.push_str(
            "PROJECT CONTEXT (read-only grounding — shows how the target file connects \
             and lessons learned; do NOT edit anything here):\n",
        );
        user.push_str(context);
        user.push_str("\n\n");
    }
    user.push_str(&format!(
        "TASK: {}\n\nFILE {} (current content; the left-margin numbers are line references — \
         do NOT copy them into your code blocks):\n----------\n{}\n----------\n\n\
         Reply now in the MODE + fenced-block format.",
        task.instruction, task.file, grounded
    ));
    if let Some(err) = prior_error {
        user.push_str(&format!(
            "\n\nYOUR PREVIOUS EDIT DID NOT WORK. Fix it. Error:\n{}",
            err
        ));
    }

    // A LIBRARY CALL, not a POST to 127.0.0.1.
    //
    // This used to go to `http://127.0.0.1:$HEX_NEXUS_PORT/api/inference/complete`, so `hex do`
    // could not run unless a daemon was up — for a call that already knew its own model. Same
    // inputs, same reply, one process (Phase 1 of the solo refactor).
    let max_tokens = std::env::var("HEX_DIRECT_MAX_TOKENS")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(4096);

    let content = hex_infer::complete_text(model, system, &user, max_tokens).await?;
    parse_edit(&content)
}

/// Parse the MODE + fenced-block reply. Robust for code (no JSON escaping).
fn parse_edit(s: &str) -> Result<Edit, String> {
    let mode = if s.to_lowercase().contains("mode: append") {
        "append"
    } else if s.to_lowercase().contains("mode: replace") {
        "replace"
    } else {
        // no explicit MODE — infer: two blocks ⇒ replace, one ⇒ append
        ""
    };

    let blocks = extract_fenced_blocks(s);
    if blocks.is_empty() {
        return Err("no fenced code block in reply".into());
    }

    let (mode, old_string, new_string) = match mode {
        "append" => ("append".to_string(), String::new(), blocks[0].clone()),
        "replace" => {
            if blocks.len() < 2 {
                return Err("MODE: replace requires two fenced blocks (old, then new)".into());
            }
            ("replace".to_string(), blocks[0].clone(), blocks[1].clone())
        }
        _ => {
            if blocks.len() >= 2 {
                ("replace".to_string(), blocks[0].clone(), blocks[1].clone())
            } else {
                ("append".to_string(), String::new(), blocks[0].clone())
            }
        }
    };

    if mode == "replace" && old_string.trim().is_empty() {
        return Err("replace requires a non-empty old snippet".into());
    }
    if new_string.trim().is_empty() {
        return Err("empty replacement/append body".into());
    }
    Ok(Edit { mode, old_string, new_string })
}

/// Extract the contents of ```...``` fences, dropping an optional language tag
/// on the opening fence (```rust). Returns blocks in order.
fn extract_fenced_blocks(s: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut in_block = false;
    let mut cur = String::new();
    for line in s.lines() {
        if line.trim_start().starts_with("```") {
            if in_block {
                blocks.push(cur.trim_end_matches('\n').to_string());
                cur.clear();
                in_block = false;
            } else {
                in_block = true; // opening fence (drop the ```lang line)
            }
        } else if in_block {
            cur.push_str(line);
            cur.push('\n');
        }
    }
    blocks
}

// ─── apply / verify / commit ──────────────────────────────────────────────────

pub(crate) fn apply_edit(abs_path: &std::path::Path, content: &str, edit: &Edit) -> Result<(), String> {
    let new_content = match edit.mode.as_str() {
        "append" => {
            let mut c = content.to_string();
            if !c.ends_with('\n') {
                c.push('\n');
            }
            c.push_str(&edit.new_string);
            if !c.ends_with('\n') {
                c.push('\n');
            }
            c
        }
        "replace" | "replace_string" => {
            let n = content.matches(&edit.old_string).count();
            if n == 0 {
                return Err("old snippet not found in file (copy it verbatim)".into());
            }
            if n > 1 {
                return Err(format!("old snippet occurs {} times (must be unique)", n));
            }
            content.replace(&edit.old_string, &edit.new_string)
        }
        other => return Err(format!("unsupported mode '{}'", other)),
    };
    std::fs::write(abs_path, new_content).map_err(|e| e.to_string())
}

pub(crate) async fn run_evidence(cmd: &str, repo_root: &std::path::Path) -> (bool, String) {
    // CRITICAL: run under bash with `pipefail` so the exit code reflects the FIRST
    // failing command in a pipe, not the last. Without this, an evidence command
    // like `cargo test … | tail` returns tail's 0 and a FAILING test reads as
    // passed — defeating the entire evidence gate (measured 2026-06-04: a failing
    // test got committed because of exactly this).
    let wrapped = format!("set -o pipefail; {}", cmd);
    let out = tokio::process::Command::new("bash")
        .arg("-c")
        .arg(&wrapped)
        .current_dir(repo_root)
        .output()
        .await;
    match out {
        Ok(o) => {
            let mut s = String::from_utf8_lossy(&o.stdout).into_owned();
            s.push_str(&String::from_utf8_lossy(&o.stderr));
            (o.status.success(), s)
        }
        Err(e) => (false, format!("spawn evidence: {}", e)),
    }
}

/// Paths already changed/untracked in the working tree (porcelain). Captured at the
/// START of a run so the commit can include the operator's pre-run supporting files
/// (a spec, a module registration) that the evidence depended on — without sweeping in
/// anything that appears *concurrently* after the run begins.
pub(crate) async fn dirty_paths(repo_root: &std::path::Path) -> Vec<String> {
    let out = match tokio::process::Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=all"])
        .current_dir(repo_root)
        .output()
        .await
    {
        Ok(o) if o.status.success() => o.stdout,
        _ => return Vec::new(),
    };
    String::from_utf8_lossy(&out)
        .lines()
        .filter_map(|l| {
            let path = l.get(3..)?; // drop the "XY " porcelain status prefix
            let path = path.rsplit(" -> ").next().unwrap_or(path); // renames: keep new path
            let p = path.trim().trim_matches('"');
            (!p.is_empty()).then(|| p.to_string())
        })
        .collect()
}

/// The target file's package directory (relative, trailing `/`): the nearest ancestor
/// containing a `Cargo.toml`/`package.json`, else the file's own directory. Used to
/// scope the start-snapshot so a feature commit only sweeps in supporting files from the
/// *same package* — never unrelated dirty files elsewhere in the repo.
fn package_prefix(repo_root: &std::path::Path, file: &str) -> String {
    let mut dir = std::path::Path::new(file).parent();
    while let Some(d) = dir {
        if !d.as_os_str().is_empty()
            && (repo_root.join(d).join("Cargo.toml").exists()
                || repo_root.join(d).join("package.json").exists())
        {
            return format!("{}/", d.to_string_lossy());
        }
        dir = d.parent();
    }
    match std::path::Path::new(file).parent() {
        Some(p) if !p.as_os_str().is_empty() => format!("{}/", p.to_string_lossy()),
        _ => String::new(),
    }
}

pub(crate) async fn commit(
    repo_root: &std::path::Path,
    file: &str,
    instruction: &str,
    factory: bool,
    start_dirty: &[String],
) -> Result<String, String> {
    // Commit the target PLUS the pre-run supporting files (spec / module-registration
    // the evidence needed) so the commit reproduces the green state — not just the
    // target (which left supporting files uncommitted, a broken committed state). Scope
    // to the target's PACKAGE and exclude anything dirtied concurrently, so neither
    // unrelated dirty files nor concurrent changes get swept (preserves the 2026-06-04
    // review-swarm fix).
    let pkg = package_prefix(repo_root, file);
    let mut paths: Vec<String> = vec![file.to_string()];
    for p in start_dirty {
        if p != file
            && !paths.contains(p)
            && p.starts_with(&pkg)
            && repo_root.join(p).exists()
        {
            paths.push(p.clone());
        }
    }
    let add = tokio::process::Command::new("git")
        .arg("add")
        .arg("--")
        .args(&paths)
        .current_dir(repo_root)
        .output()
        .await
        .map_err(|e| e.to_string())?;
    if !add.status.success() {
        return Err(format!("git add: {}", String::from_utf8_lossy(&add.stderr)));
    }
    let subject = instruction.lines().next().unwrap_or("direct edit");
    let msg = format!(
        "feat(direct): {}\n\nProduced by the direct executor (ADR-2026-06-04-1740 Path A): \
         one agent, one evidence-gated edit, no SOP pipeline.\n\n\
         Co-Authored-By: hex-direct <noreply@hex.local>",
        subject.chars().take(72).collect::<String>()
    );
    // Isolated (autonomous) commits are authored by a distinct factory identity
    // (ADR-2606071323 §4) so they are attributable and never masquerade as the
    // operator's `hex-coder`. `-c` keeps it commit-local (no global config change).
    let mut cmd = tokio::process::Command::new("git");
    if factory {
        cmd.args([
            "-c",
            "user.name=hex-factory",
            "-c",
            "user.email=factory@hex.local",
        ]);
    }
    // Scope the commit to the run's footprint (target + start-snapshot pathspec) so a
    // concurrently-changed file can't get swept in (the 2026-06-04 review swarm's fix,
    // generalized from one file to the captured set).
    let c = cmd
        .args(["commit", "-m", &msg, "--"])
        .args(&paths)
        .current_dir(repo_root)
        .output()
        .await
        .map_err(|e| e.to_string())?;
    if !c.status.success() {
        return Err(format!("git commit: {}", String::from_utf8_lossy(&c.stderr)));
    }
    let rev = tokio::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .current_dir(repo_root)
        .output()
        .await
        .map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&rev.stdout).trim().to_string())
}

pub(crate) fn repo_root() -> std::path::PathBuf {
    // Honor explicit override; else walk up from CWD to the nearest .git.
    if let Ok(p) = std::env::var("HEX_PROJECT_ROOT") {
        return std::path::PathBuf::from(p);
    }
    let mut dir = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    loop {
        if dir.join(".git").exists() {
            return dir;
        }
        if !dir.pop() {
            return std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        }
    }
}

#[cfg(test)]
mod commit_snapshot_tests {
    use super::{commit, dirty_paths};
    use std::process::Command as Git;

    fn git(dir: &std::path::Path, args: &[&str]) {
        let out = Git::new("git").args(args).current_dir(dir).output().unwrap();
        assert!(out.status.success(), "git {:?}: {}", args, String::from_utf8_lossy(&out.stderr));
    }

    fn init_repo(dir: &std::path::Path) {
        git(dir, &["init", "-q", "-b", "main"]);
        git(dir, &["config", "user.email", "t@t"]);
        git(dir, &["config", "user.name", "t"]);
        std::fs::write(dir.join("base.txt"), "base").unwrap();
        git(dir, &["add", "."]);
        git(dir, &["commit", "-qm", "init"]);
    }

    // The bug: the do-loop committed only its target, leaving the pre-run supporting
    // files (spec / module registration) the evidence depended on uncommitted.
    #[tokio::test]
    async fn commit_includes_pre_run_supporting_files() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        init_repo(dir);
        std::fs::write(dir.join("support.rs"), "// spec the evidence needs").unwrap();
        let start_dirty = dirty_paths(dir).await;
        assert!(start_dirty.iter().any(|p| p == "support.rs"),
            "start snapshot must see the supporting file, got {start_dirty:?}");
        std::fs::write(dir.join("target.rs"), "fn t() {}").unwrap();
        commit(dir, "target.rs", "add target", false, &start_dirty).await.unwrap();
        let st = Git::new("git").args(["status", "--porcelain"]).current_dir(dir).output().unwrap();
        assert!(String::from_utf8_lossy(&st.stdout).trim().is_empty(),
            "tree must be clean — support.rs should have been committed with the target");
        let files = Git::new("git").args(["show", "--name-only", "--format=", "HEAD"]).current_dir(dir).output().unwrap();
        let names = String::from_utf8_lossy(&files.stdout);
        assert!(names.contains("target.rs") && names.contains("support.rs"),
            "commit must contain both target and support, got: {names}");
    }

    // The review-swarm fix preserved: a change that appears AFTER the run starts must
    // NOT be swept into the commit.
    #[tokio::test]
    async fn commit_excludes_concurrent_changes() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        init_repo(dir);
        let start_dirty = dirty_paths(dir).await; // empty: clean tree at start
        std::fs::write(dir.join("concurrent.rs"), "// not ours").unwrap();
        std::fs::write(dir.join("target.rs"), "fn t() {}").unwrap();
        commit(dir, "target.rs", "add target", false, &start_dirty).await.unwrap();
        let st = Git::new("git").args(["status", "--porcelain"]).current_dir(dir).output().unwrap();
        assert!(String::from_utf8_lossy(&st.stdout).contains("concurrent.rs"),
            "a change appearing after run-start must NOT be swept into the commit");
    }

    // The snapshot is scoped to the target's PACKAGE: a pre-run dirty file in a
    // DIFFERENT package (unrelated WIP elsewhere in the repo) must not be swept in.
    #[tokio::test]
    async fn commit_scopes_snapshot_to_target_package() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        init_repo(dir);
        std::fs::create_dir_all(dir.join("mypkg/src")).unwrap();
        std::fs::write(dir.join("mypkg/Cargo.toml"), "[package]\nname=\"m\"\nversion=\"0.0.0\"").unwrap();
        git(dir, &["add", "."]);
        git(dir, &["commit", "-qm", "pkg"]);
        // pre-run dirty: a supporting file IN the package + unrelated WIP OUTSIDE it
        std::fs::write(dir.join("mypkg/spec.rs"), "// in-package support").unwrap();
        std::fs::write(dir.join("unrelated.rs"), "// different package WIP").unwrap();
        let start_dirty = dirty_paths(dir).await;
        std::fs::write(dir.join("mypkg/src/target.rs"), "fn t() {}").unwrap();
        commit(dir, "mypkg/src/target.rs", "add", false, &start_dirty).await.unwrap();
        let files = Git::new("git").args(["show", "--name-only", "--format=", "HEAD"]).current_dir(dir).output().unwrap();
        let names = String::from_utf8_lossy(&files.stdout);
        assert!(names.contains("mypkg/src/target.rs"), "target committed");
        assert!(names.contains("mypkg/spec.rs"), "in-package supporting file committed");
        assert!(!names.contains("unrelated.rs"), "out-of-package WIP must NOT be swept, got: {names}");
        let st = Git::new("git").args(["status", "--porcelain"]).current_dir(dir).output().unwrap();
        assert!(String::from_utf8_lossy(&st.stdout).contains("unrelated.rs"),
            "unrelated WIP must remain uncommitted");
    }
}
