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
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

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
    /// Max edit→verify attempts before giving up (default 3).
    #[serde(default)]
    pub max_attempts: Option<u32>,
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
#[derive(Debug, Clone, Serialize)]
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
    pub evidence_passed: bool,
    pub committed: Option<String>,
    pub duration_ms: u64,
    pub error: Option<String>,
}

/// Record a run from any in-nexus agent (not just the direct executor) into the
/// shared feed served by GET /api/direct/runs. `detail` becomes the instruction
/// line shown in the dashboard.
pub fn record_agent_run(
    agent: &str,
    started_at: String,
    detail: String,
    ok: bool,
    committed: Option<String>,
    duration_ms: u64,
    error: Option<String>,
) {
    let run = DirectRun {
        id: RUN_ID.fetch_add(1, Ordering::Relaxed),
        agent: agent.to_string(),
        started_at,
        instruction: detail.chars().take(240).collect(),
        file: String::new(),
        model: String::new(),
        ok,
        attempts: 1,
        evidence_passed: ok,
        committed,
        duration_ms,
        error,
    };
    store_run(run);
}

/// Push a run into the in-memory feed (fast path for the API) AND persist it to
/// SpacetimeDB (so the feed survives nexus restarts).
fn store_run(run: DirectRun) {
    persist_run_async(run.clone());
    if let Ok(mut q) = RUNS.lock() {
        q.push_front(run);
        while q.len() > RUN_HISTORY {
            q.pop_back();
        }
    }
}

static RUNS: LazyLock<Mutex<VecDeque<DirectRun>>> = LazyLock::new(|| Mutex::new(VecDeque::new()));
static RUN_ID: AtomicU64 = AtomicU64::new(1);

// Serialize the read→edit→evidence→commit critical section. Two concurrent runs
// touching the working tree / git index race and can false-positive (one reports
// ok=true + a commit while another's edit interleaves) — found by the 2026-06-04
// review swarm. Global (not per-file) because git add/commit is process-global.
static EXEC_LOCK: LazyLock<tokio::sync::Mutex<()>> = LazyLock::new(|| tokio::sync::Mutex::new(()));

/// A `cargo test <filter>` matching zero tests exits 0 with "running 0 tests" /
/// "0 passed; 0 failed" — a vacuous pass. The gate must require the change to be
/// actually exercised, so treat these as NOT satisfied.
fn evidence_is_vacuous(output: &str) -> bool {
    output.contains("running 0 tests")
        || output.contains("0 passed; 0 failed; 0 ignored")
        || output.contains("0 passed; 0 failed; 0 measured")
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
        evidence_passed: r.evidence_passed,
        committed: r.committed.clone(),
        duration_ms,
        error: r.error.clone(),
    };
    store_run(run);
}

// ── SpacetimeDB persistence (survives nexus restarts) ────────────────────────

fn stdb_host() -> String {
    std::env::var("HEX_STDB_HOST").unwrap_or_else(|_| hex_core::SPACETIMEDB_DEFAULT_HOST.to_string())
}

/// Fire-and-forget persist of a run to STDB. The in-memory feed is the fast path;
/// STDB is the durable backing. Never blocks or fails a recorder.
fn persist_run_async(run: DirectRun) {
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        handle.spawn(async move {
            if let Err(e) = persist_run(&run).await {
                tracing::debug!(error = %e, "agent-run STDB persist failed (non-fatal)");
            }
        });
    }
}

async fn persist_run(run: &DirectRun) -> Result<(), String> {
    // Globally-unique key — `<started_at>#<seq>` stays unique even though the
    // in-memory RUN_ID resets to 1 on each restart (started_at differs).
    let id = format!("{}#{}", run.started_at, run.id);
    let url = format!("{}/v1/database/hex/call/record_agent_run", stdb_host());
    let args = json!([
        id,
        run.agent,
        run.started_at,
        run.instruction,
        run.file,
        run.model,
        run.ok,
        run.attempts,
        run.evidence_passed,
        run.committed.clone().unwrap_or_default(),
        run.duration_ms,
        run.error.clone().unwrap_or_default(),
    ]);
    let res = reqwest::Client::new()
        .post(&url)
        .json(&args)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        return Err(format!("reducer {}: {}", res.status(), res.text().await.unwrap_or_default()));
    }
    Ok(())
}

/// Hydrate the in-memory feed from STDB at startup (newest `RUN_HISTORY`). Called
/// once after SpacetimeDB is up; safe to fail (empty feed) if the table is absent.
pub async fn hydrate_from_stdb() {
    let url = format!("{}/v1/database/hex/sql", stdb_host());
    // SpacetimeDB SQL has no ORDER BY — fetch (bounded) and sort newest-first in Rust.
    let q = "SELECT id, agent, started_at, instruction, file, model, ok, attempts, evidence_passed, committed, duration_ms, error FROM agent_run LIMIT 2000".to_string();
    let res = match reqwest::Client::new()
        .post(&url)
        .header("Content-Type", "text/plain")
        .body(q)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, "agent-run hydrate: query failed");
            return;
        }
    };
    let text = match res.text().await {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(error = %e, "agent-run hydrate: body read failed");
            return;
        }
    };
    let body: Value = match serde_json::from_str(&text) {
        Ok(b) => b,
        Err(_) => {
            tracing::warn!(body = %text.chars().take(160).collect::<String>(), "agent-run hydrate: non-JSON response");
            return;
        }
    };
    let rows = body
        .as_array()
        .and_then(|a| a.first())
        .and_then(|f| f.get("rows"))
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();

    // Rows come newest-first; rebuild the deque oldest-last and re-number for display.
    let mut loaded: Vec<DirectRun> = Vec::new();
    for row in &rows {
        let c = match row.as_array() {
            Some(c) if c.len() >= 12 => c,
            _ => continue,
        };
        let s = |i: usize| c.get(i).and_then(|v| v.as_str()).unwrap_or("").to_string();
        let opt = |i: usize| {
            let v = s(i);
            if v.is_empty() { None } else { Some(v) }
        };
        loaded.push(DirectRun {
            id: 0, // reassigned below
            agent: s(1),
            started_at: s(2),
            instruction: s(3),
            file: s(4),
            model: s(5),
            ok: c.get(6).and_then(|v| v.as_bool()).unwrap_or(false),
            attempts: c.get(7).and_then(|v| v.as_u64()).unwrap_or(1) as u32,
            evidence_passed: c.get(8).and_then(|v| v.as_bool()).unwrap_or(false),
            committed: opt(9),
            duration_ms: c.get(10).and_then(|v| v.as_u64()).unwrap_or(0),
            error: opt(11),
        });
    }
    // Newest-first (RFC3339 UTC strings sort lexically = chronologically), capped.
    loaded.sort_by(|a, b| b.started_at.cmp(&a.started_at));
    loaded.truncate(RUN_HISTORY);
    let n = loaded.len();
    if n == 0 {
        return;
    }
    // Display ids: highest number = most recent (loaded[0]).
    for (idx, run) in loaded.iter_mut().enumerate() {
        run.id = (n - idx) as u64;
    }
    RUN_ID.store((n as u64) + 1, Ordering::Relaxed);
    if let Ok(mut q) = RUNS.lock() {
        for run in loaded {
            q.push_back(run);
        }
    }
    tracing::info!(count = n, "agent-run feed hydrated from SpacetimeDB");
}

/// Newest-first snapshot of recorded runs for the API / CLI / dashboard.
pub fn runs_snapshot() -> Vec<DirectRun> {
    RUNS.lock().map(|q| q.iter().cloned().collect()).unwrap_or_default()
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
    let model = task.model.clone().unwrap_or_else(|| {
        std::env::var("HEX_DIRECT_MODEL").unwrap_or_else(|_| "qwen2.5-coder:32b".to_string())
    });
    let result = execute_direct_inner(task.clone()).await;
    record_run(started_at, &task, &model, &result, started.elapsed().as_millis() as u64);
    result
}

async fn execute_direct_inner(task: DirectTask) -> DirectResult {
    // Serialize the whole read→edit→evidence→commit section against other runs.
    let _exec_guard = EXEC_LOCK.lock().await;
    let max_attempts = task.max_attempts.unwrap_or(3).clamp(1, 6);
    let model = task.model.clone().unwrap_or_else(|| {
        std::env::var("HEX_DIRECT_MODEL").unwrap_or_else(|_| "qwen2.5-coder:32b".to_string())
    });

    let repo_root = repo_root();
    let abs_path = repo_root.join(&task.file);

    // Phase 2 (ADR-2606061359): assemble graph-context + lessons once and prepend
    // it to every edit prompt — the single agent reasons with structural context
    // + memory (Hermes/OpenClaw), not from the file slice alone.
    let context_block = gather_context(&task).await;

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
        let edit = match request_edit(&model, &task, &grounded, &context_block, last_error.as_deref()).await {
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
        let (passed, output) = run_evidence(&task.evidence, &repo_root).await;
        result.evidence_output = output.chars().take(4000).collect();
        // A `cargo test <filter>` that matches ZERO tests prints "running 0 tests"
        // / "0 passed; 0 failed" and still exits 0 — a vacuous pass. The whole point
        // of the gate is that the change is *verified*, so reject it (found by the
        // 2026-06-04 review swarm).
        let vacuous = passed && evidence_is_vacuous(&output);
        if passed && !vacuous {
            result.evidence_passed = true;
            // 5. Commit (scoped to the edited file only).
            match commit(&repo_root, &task.file, &task.instruction).await {
                Ok(hash) => {
                    result.committed = Some(hash);
                    result.ok = true;
                    tracing::info!(attempt, file = %task.file, "direct_exec: evidence passed, committed");
                    return result;
                }
                Err(e) => {
                    result.error = Some(format!("commit: {}", e));
                    return result; // edit good + evidence passed but commit failed — surface it
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
fn ground_window(content: &str, instruction: &str) -> String {
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
async fn gather_context(task: &DirectTask) -> String {
    let mut out = String::new();

    // (a) Graph neighbourhood for the target file, from graphify-out/graph.json.
    let graph_path = repo_root().join("graphify-out").join("graph.json");
    if let Ok(raw) = std::fs::read_to_string(&graph_path) {
        if let Ok(g) = hex_graph::model::KnowledgeGraph::from_json(&raw) {
            let opts = hex_graph::context::ContextOpts { max_each: 15 };
            if let Some(bundle) = hex_graph::context::context_for(&g, &task.file, opts) {
                out.push_str(&hex_graph::context::render_markdown(&bundle));
            }
        }
    }

    // (b) Learned lessons from memory (the minimal self-improvement loop).
    let lessons = fetch_lessons(6).await;
    if !lessons.is_empty() {
        out.push_str("\n## Lessons (from memory)\n");
        for l in &lessons {
            out.push_str("- ");
            out.push_str(l);
            out.push('\n');
        }
    }

    out
}

/// Best-effort pull of `lesson:` entries from the hexflo_memory table over the
/// STDB HTTP SQL endpoint. Columns are mapped by name (schema.elements).
async fn fetch_lessons(limit: usize) -> Vec<String> {
    let url = format!("{}/v1/database/hex/sql", stdb_host());
    let http = match reqwest::Client::builder().timeout(Duration::from_secs(3)).build() {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let resp = match http
        .post(&url)
        .header("Content-Type", "text/plain")
        .body("SELECT key, value FROM hexflo_memory")
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => r,
        _ => return Vec::new(),
    };
    let body: Value = match resp.json().await {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let mut lessons = Vec::new();
    let Some(tables) = body.as_array() else {
        return lessons;
    };
    for table in tables {
        let cols: Vec<&str> = table
            .get("schema")
            .and_then(|s| s.get("elements"))
            .and_then(|e| e.as_array())
            .map(|els| {
                els.iter()
                    .filter_map(|el| {
                        el.get("name").and_then(|n| n.get("some")).and_then(|s| s.as_str())
                    })
                    .collect()
            })
            .unwrap_or_default();
        let ki = cols.iter().position(|c| *c == "key");
        let vi = cols.iter().position(|c| *c == "value");
        if let Some(rows) = table.get("rows").and_then(|r| r.as_array()) {
            for row in rows {
                if let Some(vals) = row.as_array() {
                    let key = ki.and_then(|i| vals.get(i)).and_then(|v| v.as_str()).unwrap_or("");
                    let val = vi.and_then(|i| vals.get(i)).and_then(|v| v.as_str()).unwrap_or("");
                    if key.starts_with("lesson:") && !val.is_empty() {
                        lessons.push(val.to_string());
                        if lessons.len() >= limit {
                            return lessons;
                        }
                    }
                }
            }
        }
    }
    lessons
}

// ─── the one inference call ───────────────────────────────────────────────────

struct Edit {
    mode: String, // replace_string | append | create
    old_string: String,
    new_string: String,
}

async fn request_edit(
    model: &str,
    task: &DirectTask,
    grounded: &str,
    context: &str,
    prior_error: Option<&str>,
) -> Result<Edit, String> {
    let port = std::env::var("HEX_NEXUS_PORT").unwrap_or_else(|_| "5555".to_string());
    let url = format!("http://127.0.0.1:{}/api/inference/complete", port);

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

    let body = json!({
        "model": model,
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": user},
        ],
        "max_tokens": std::env::var("HEX_DIRECT_MAX_TOKENS").ok().and_then(|v| v.parse::<u32>().ok()).unwrap_or(4096),
    });

    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(600))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = http.post(&url).json(&body).send().await.map_err(|e| e.to_string())?;
    let status = resp.status();
    let rb: Value = resp.json().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("HTTP {}: {}", status, rb));
    }
    let content = rb.get("content").and_then(|v| v.as_str()).unwrap_or("").to_string();
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

fn apply_edit(abs_path: &std::path::Path, content: &str, edit: &Edit) -> Result<(), String> {
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

async fn run_evidence(cmd: &str, repo_root: &std::path::Path) -> (bool, String) {
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

async fn commit(repo_root: &std::path::Path, file: &str, instruction: &str) -> Result<String, String> {
    let add = tokio::process::Command::new("git")
        .args(["add", file])
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
    // Scope the commit to ONLY the edited file (pathspec) so a pre-staged or
    // concurrently-changed file can't get swept into the executor's commit
    // (found by the 2026-06-04 review swarm — bare `git commit` committed all
    // staged changes).
    let c = tokio::process::Command::new("git")
        .args(["commit", "-m", &msg, "--", file])
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

fn repo_root() -> std::path::PathBuf {
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
