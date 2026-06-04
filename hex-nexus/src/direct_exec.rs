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
use std::time::Duration;

#[derive(Debug, Deserialize)]
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

/// Run one task end-to-end. Returns a structured, honest result — `ok` is true
/// ONLY if the evidence command exited 0 and the change was committed.
pub async fn execute_direct(task: DirectTask) -> DirectResult {
    let max_attempts = task.max_attempts.unwrap_or(3).clamp(1, 6);
    let model = task.model.clone().unwrap_or_else(|| {
        std::env::var("HEX_DIRECT_MODEL").unwrap_or_else(|_| "qwen2.5-coder:32b".to_string())
    });

    let repo_root = repo_root();
    let abs_path = repo_root.join(&task.file);

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
        let edit = match request_edit(&model, &task, &grounded, last_error.as_deref()).await {
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
        if passed {
            result.evidence_passed = true;
            // 5. Commit.
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
            last_error = Some(format!(
                "evidence `{}` FAILED. Output:\n{}",
                task.evidence,
                output.chars().take(2500).collect::<String>()
            ));
            tracing::warn!(attempt, "direct_exec: evidence failed, retrying with error fed back");
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

    let mut user = format!(
        "TASK: {}\n\nFILE {} (current content; the left-margin numbers are line references — \
         do NOT copy them into your code blocks):\n----------\n{}\n----------\n\n\
         Reply now in the MODE + fenced-block format.",
        task.instruction, task.file, grounded
    );
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
    let c = tokio::process::Command::new("git")
        .args(["commit", "-m", &msg])
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
