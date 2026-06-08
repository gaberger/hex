//! ReAct tool-use loop for the single-agent executor (ADR-2606071XXX).
//!
//! The default `hex do` path. Where `direct_exec` makes ONE edit per attempt,
//! this lets the agent *explore* before editing — grep callers, read neighbours,
//! `cargo_check` — then commit through the same evidence gate. It reuses the
//! proven `simple_agent` tool-call parsing (native function-calling with a
//! text-mode JSON fallback) + the curated, already-guarded `ToolRegistry`, and
//! keeps the transcript bounded via `compress` (ADR-2606071XXX, cf. RLM
//! arXiv:2512.24601). The terminal action `propose_edit` runs the task's evidence
//! command and only commits on exit 0 — the same guarantee as the single-shot path.
//!
//! Safeguards: curated read/verify tool allowlist (no arbitrary shell, no
//! persona/side-effecting tools), per-tool guards inherited from the registry
//! (path allowlist, critical-path block, subprocess timeouts, output caps),
//! `max_steps` + duplicate-call detection + a no-progress guard, and the evidence
//! gate as the ultimate authority on what commits.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use crate::compress::{cap_str, compress_messages, estimate_tokens, CompressOpts};
use crate::direct_exec::{self, DirectResult, DirectTask, Edit};
use crate::simple_agent::{
    assistant_turn_content, extract_tool_uses, normalize_tool_input, strip_metadata_fields,
};
use crate::tools::ToolRegistry;

const DEFAULT_MAX_STEPS: u32 = 12;
const MAX_TOKENS: u32 = 4096;
/// When the compressed transcript still exceeds this, summarize the older region
/// with one cheap-model call (the LLM half of the hybrid compression).
const MAX_CTX_TOKENS: usize = 12_000;
const NO_PROGRESS_LIMIT: u32 = 3;

/// Read/verify tools the loop may call. No arbitrary shell; no persona or
/// side-effecting tools (adr_draft/workplan_emit/delegate/escalate/web_search).
/// The terminal edit is `propose_edit`, handled specially below.
const ALLOWED_TOOLS: &[&str] =
    &["repo_read", "repo_grep", "cargo_check", "typescript_check", "dep_audit", "secret_scan"];

/// Resolve the model for the loop. The ReAct loop needs a strong tool-caller
/// (local code models are unreliable at multi-step function-calling), so it has
/// its OWN setting, distinct from the single-shot default. Precedence:
/// explicit `--model` → `HEX_REACT_MODEL` env → `.hex/project.json`
/// `inference.react_model` → the single-shot default.
pub(crate) fn resolve_react_model(task: &DirectTask) -> String {
    if let Some(m) = &task.model {
        if !m.is_empty() {
            return m.clone();
        }
    }
    if let Ok(m) = std::env::var("HEX_REACT_MODEL") {
        if !m.is_empty() {
            return m;
        }
    }
    if let Some(m) = react_model_from_config() {
        return m;
    }
    direct_exec::resolve_model(task)
}

fn react_model_from_config() -> Option<String> {
    let path = direct_exec::repo_root().join(".hex").join("project.json");
    let raw = std::fs::read_to_string(path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    v.get("inference")?
        .get("react_model")?
        .as_str()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// Run the ReAct loop end-to-end. Returns the result, the number of steps
/// (tool calls) taken, and the model used, for the run feed.
pub async fn react_execute(task: DirectTask) -> (DirectResult, u32, String) {
    // ADR-2606071323: confine the run to its own worktree unless the operator
    // opted out (`isolate:false`). Never silently fall back to the operator tree.
    let isolate = direct_exec::want_isolation(&task);
    let slug = crate::direct_workspace::next_run_slug();
    let workspace = match crate::direct_workspace::RunWorkspace::acquire(&slug, isolate) {
        Ok(w) => w,
        Err(e) => return (DirectResult::err(format!("workspace: {e}")), 0, resolve_react_model(&task)),
    };
    if let Err(e) = workspace.assert_off_operator_tree() {
        workspace.finish(false);
        return (DirectResult::err(e), 0, resolve_react_model(&task));
    }
    let repo_root = workspace.workdir().to_path_buf();
    let factory = workspace.is_isolated();
    let out = react_attempts(&task, &repo_root, factory).await;
    workspace.finish(out.0.ok);
    out
}

/// The ReAct loop body, run inside the resolved workspace (ADR-2606071323).
async fn react_attempts(
    task: &DirectTask,
    repo_root: &std::path::Path,
    factory: bool,
) -> (DirectResult, u32, String) {
    let model = resolve_react_model(task);
    let max_steps = task.max_steps.unwrap_or(DEFAULT_MAX_STEPS).clamp(1, 40);
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

    let context_block = direct_exec::gather_context(task).await;
    let registry = Arc::new(ToolRegistry::default());
    let tools_schema = curated_schema(&registry);
    let system_prompt = build_system_prompt(&tools_schema);
    let seed = build_seed(task, &context_block, &abs_path);
    let mut messages: Vec<Value> = vec![json!({ "role": "user", "content": seed })];

    let inference_url = {
        let port = std::env::var("HEX_NEXUS_PORT").unwrap_or_else(|_| "5555".to_string());
        format!("http://127.0.0.1:{}/api/inference/complete", port)
    };
    let http = match reqwest::Client::builder().timeout(Duration::from_secs(600)).build() {
        Ok(c) => c,
        Err(e) => {
            result.error = Some(format!("http build: {}", e));
            return (result, 0, model);
        }
    };

    let mut prior_successes: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut steps = 0u32;
    let mut no_progress = 0u32;
    let opts = CompressOpts::default();

    for _ in 0..max_steps {
        // Compress the transcript before each call; LLM-summarize on overflow.
        let mut sent = compress_messages(&messages, &opts);
        if estimate_tokens(&sent) > MAX_CTX_TOKENS {
            sent = summarize_overflow(&http, &inference_url, &model, sent).await;
        }

        let req = json!({
            "model": model,
            "max_tokens": MAX_TOKENS,
            "system": system_prompt,
            "tools": tools_schema,
            "messages": sent,
        });
        let body: Value = match http.post(&inference_url).json(&req).send().await {
            Ok(r) => match r.json().await {
                Ok(v) => v,
                Err(e) => {
                    result.error = Some(format!("inference json: {}", e));
                    break;
                }
            },
            Err(e) => {
                result.error = Some(format!("inference http: {}", e));
                break;
            }
        };

        let (assistant_text, tool_uses) = extract_tool_uses(&body);

        if tool_uses.is_empty() {
            // Model emitted no action — it's either done or stuck. Without a
            // committed edit that's a failure (nothing shipped).
            if !result.ok && result.error.is_none() {
                result.error = Some(format!(
                    "loop ended with no edit: {}",
                    assistant_text.chars().take(240).collect::<String>()
                ));
            }
            break;
        }

        messages.push(json!({
            "role": "assistant",
            "content": assistant_turn_content(&assistant_text, &tool_uses),
        }));

        let mut tool_results: Vec<Value> = Vec::new();
        let mut made_progress = false;

        for tu in &tool_uses {
            steps += 1;

            // Terminal action: apply the edit + evidence gate (+ commit on pass).
            if tu.name == "propose_edit" {
                made_progress = true;
                tracing::info!(step = steps, args = %serde_json::to_string(&tu.input).unwrap_or_default().chars().take(300).collect::<String>(), "react: propose_edit");
                match apply_and_verify(&abs_path, repo_root, task, &tu.input, factory).await {
                    EditOutcome::Committed(hash) => {
                        tracing::info!(step = steps, %hash, "react: propose_edit COMMITTED");
                        result.edit_applied = true;
                        result.evidence_passed = true;
                        result.committed = Some(hash);
                        result.ok = true;
                        result.attempts = steps;
                        return (result, steps, model);
                    }
                    EditOutcome::EvidenceFailed(msg) => {
                        tracing::warn!(step = steps, detail = %msg.chars().take(200).collect::<String>(), "react: propose_edit evidence FAILED (reverted)");
                        result.edit_applied = true;
                        result.evidence_output = msg.chars().take(4000).collect();
                        tool_results.push(tool_result_block(&tu.id, false, &json!({ "evidence": "failed", "detail": msg })));
                    }
                    EditOutcome::ApplyFailed(msg) => {
                        tracing::warn!(step = steps, detail = %msg, "react: propose_edit apply FAILED");
                        tool_results.push(tool_result_block(&tu.id, false, &json!({ "apply": "failed", "detail": msg })));
                    }
                }
                continue;
            }

            // Safeguard: only the curated read/verify tools are dispatchable.
            if !ALLOWED_TOOLS.contains(&tu.name.as_str()) {
                tool_results.push(tool_result_block(
                    &tu.id,
                    false,
                    &json!({ "error": format!("tool '{}' is not permitted in the edit loop; allowed: {:?} + propose_edit", tu.name, ALLOWED_TOOLS) }),
                ));
                continue;
            }

            let normalized = normalize_tool_input(&tu.name, tu.input.clone());
            let sig = format!(
                "{}:{}",
                tu.name,
                serde_json::to_string(&strip_metadata_fields(&normalized)).unwrap_or_default()
            );
            if prior_successes.contains(&sig) {
                tool_results.push(tool_result_block(&tu.id, true, &json!({ "skipped": "duplicate of a prior successful call — use the earlier result" })));
                continue;
            }

            made_progress = true;
            let res = registry.execute(&tu.name, normalized.clone()).await;
            tracing::info!(step = steps, tool = %tu.name, ok = res.ok, args = %serde_json::to_string(&normalized).unwrap_or_default().chars().take(160).collect::<String>(), "react: tool");
            if res.ok {
                prior_successes.insert(sig);
            }
            let payload = json!({
                "ok": res.ok,
                "output": res.output,
                "error": res.error,
                "truncated": res.truncated,
            });
            tool_results.push(tool_result_block(&tu.id, res.ok, &payload));
        }

        // No-progress guard: terminate if the model just churns duplicates.
        if made_progress {
            no_progress = 0;
        } else {
            no_progress += 1;
            if no_progress >= NO_PROGRESS_LIMIT {
                result.error = Some("no progress (repeated/duplicate tool calls)".into());
                break;
            }
        }

        if !tool_results.is_empty() {
            messages.push(json!({ "role": "user", "content": tool_results }));
        }
    }

    if !result.ok && result.error.is_none() {
        result.error = Some(format!("exhausted {} steps without a passing evidence-gated edit", max_steps));
    }
    result.attempts = steps;
    (result, steps, model)
}

enum EditOutcome {
    Committed(String),
    EvidenceFailed(String),
    ApplyFailed(String),
}

/// Apply a proposed edit, run the evidence command, commit on pass. On any
/// failure the edit is REVERTED so each `propose_edit` is atomic (the next one
/// matches against the original file, not a half-applied one).
async fn apply_and_verify(
    abs_path: &std::path::Path,
    repo_root: &std::path::Path,
    task: &DirectTask,
    input: &Value,
    factory: bool,
) -> EditOutcome {
    let edit = match parse_propose_edit(input) {
        Ok(e) => e,
        Err(e) => return EditOutcome::ApplyFailed(e),
    };
    let content = match std::fs::read_to_string(abs_path) {
        Ok(c) => c,
        Err(e) => return EditOutcome::ApplyFailed(format!("read {}: {}", task.file, e)),
    };
    if let Err(e) = direct_exec::apply_edit(abs_path, &content, &edit) {
        return EditOutcome::ApplyFailed(e);
    }
    let (passed, output) = direct_exec::run_evidence(&task.evidence, repo_root).await;
    let vacuous = passed && direct_exec::evidence_is_vacuous(&output);
    if passed && !vacuous {
        match direct_exec::commit(repo_root, &task.file, &task.instruction, factory).await {
            Ok(hash) => EditOutcome::Committed(hash),
            Err(e) => {
                let _ = std::fs::write(abs_path, &content); // revert
                EditOutcome::ApplyFailed(format!("commit: {}", e))
            }
        }
    } else {
        let _ = std::fs::write(abs_path, &content); // revert the failed edit
        let detail = if vacuous {
            format!(
                "evidence `{}` PASSED VACUOUSLY (ran 0 tests). The change must be actually exercised. Output:\n{}",
                task.evidence,
                output.chars().take(1800).collect::<String>()
            )
        } else {
            format!(
                "evidence `{}` FAILED (edit reverted). Read the error, then propose a corrected edit:\n{}",
                task.evidence,
                output.chars().take(1800).collect::<String>()
            )
        };
        EditOutcome::EvidenceFailed(detail)
    }
}

fn parse_propose_edit(input: &Value) -> Result<Edit, String> {
    let new_string = input
        .get("new_string")
        .or_else(|| input.get("content"))
        .or_else(|| input.get("code"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let old_string = input
        .get("old_string")
        .or_else(|| input.get("old"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let mode_in = input.get("mode").and_then(|v| v.as_str()).unwrap_or("");
    let mode = if mode_in == "append" || mode_in == "replace" {
        mode_in.to_string()
    } else if !old_string.trim().is_empty() {
        "replace".to_string()
    } else {
        "append".to_string()
    };
    if new_string.trim().is_empty() {
        return Err("propose_edit needs a non-empty new_string".into());
    }
    if mode == "replace" && old_string.trim().is_empty() {
        return Err("propose_edit mode=replace needs old_string (the exact existing snippet)".into());
    }
    Ok(Edit { mode, old_string, new_string })
}

fn tool_result_block(id: &str, ok: bool, payload: &Value) -> Value {
    json!({
        "type": "tool_result",
        "tool_use_id": id,
        "content": serde_json::to_string(payload).unwrap_or_default(),
        "is_error": !ok,
    })
}

/// The curated tool schema: the read/verify allowlist + the terminal propose_edit.
fn curated_schema(registry: &ToolRegistry) -> Value {
    let mut arr: Vec<Value> = Vec::new();
    for name in ALLOWED_TOOLS {
        if let Some(t) = registry.get(name) {
            arr.push(json!({
                "name": t.name(),
                "description": t.description(),
                "input_schema": t.input_schema(),
            }));
        }
    }
    arr.push(json!({
        "name": "propose_edit",
        "description": "Apply your edit to the target file and run the evidence command. On evidence pass it COMMITS and the task is done; on fail the edit is reverted and you get the error to correct. This is the ONLY way to finish — stop calling read tools and call this when ready.",
        "input_schema": {
            "type": "object",
            "properties": {
                "mode": { "type": "string", "enum": ["append", "replace"], "description": "append code to the file end, or replace an exact existing snippet" },
                "old_string": { "type": "string", "description": "mode=replace only: the exact existing snippet, copied verbatim (must occur exactly once)" },
                "new_string": { "type": "string", "description": "the code to append, or the replacement snippet" }
            },
            "required": ["mode", "new_string"]
        }
    }));
    Value::Array(arr)
}

fn build_system_prompt(tools_schema: &Value) -> String {
    let mut s = String::from(
        "You are a focused hex coding agent editing ONE file to satisfy a task, verified by an \
         evidence command. Work in a ReAct loop: explore with the read/verify tools, then apply \
         your change with propose_edit.\n\n\
         RESPONSE FORMAT — two protocols accepted:\n\
         1) NATIVE TOOL-USE (preferred): emit tool calls via your client's function-calling.\n\
         2) TEXT-MODE FALLBACK (local models without native tool-use): emit a fenced block \
         ```json\n{ \"tool\": \"<name>\", \"args\": { ... } }\n``` per call.\n\n\
         Rules:\n\
         - Use EXACTLY the key names from each tool's input_schema.\n\
         - Explore only as much as you need (grep consumers, read neighbours, cargo_check) — every \
         observation costs context.\n\
         - FINISH by calling propose_edit. It runs the evidence command and commits on pass; on \
         fail the edit is reverted and you correct it. Do not stop without a successful propose_edit.\n\
         - Make the SMALLEST change that satisfies the task; keep surrounding code byte-for-byte identical.\n\n\
         === Tools (use input_schema exactly) ===\n\n",
    );
    if let Some(arr) = tools_schema.as_array() {
        for t in arr {
            let name = t.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let desc = t.get("description").and_then(|v| v.as_str()).unwrap_or("");
            let schema = t.get("input_schema").map(|v| v.to_string()).unwrap_or_default();
            s.push_str(&format!("- {} — {}\n  input_schema: {}\n\n", name, desc, schema));
        }
    }
    s
}

fn build_seed(task: &DirectTask, context: &str, abs_path: &std::path::Path) -> String {
    let mut s = String::new();
    s.push_str(&format!("TASK: {}\n\nTARGET FILE: {}\n\n", task.instruction, task.file));
    if !context.is_empty() {
        s.push_str("PROJECT CONTEXT (read-only grounding — how the file connects + lessons learned):\n");
        s.push_str(context);
        s.push_str("\n\n");
    }
    // Seed the current file (windowed) so the agent doesn't have to spend a step
    // reading it; it can still repo_read others.
    if let Ok(content) = std::fs::read_to_string(abs_path) {
        let window = direct_exec::ground_window(&content, &task.instruction);
        s.push_str(&format!(
            "CURRENT CONTENT of {} (left-margin numbers are references; do NOT copy them into edits):\n----------\n{}\n----------\n\n",
            task.file, window
        ));
    } else {
        s.push_str(&format!("(The target file {} does not exist yet — propose_edit mode=append will create it.)\n\n", task.file));
    }
    s.push_str(&format!(
        "Evidence command that must pass: `{}`\nExplore if needed, then call propose_edit.",
        task.evidence
    ));
    s
}

/// LLM half of hybrid compression: when even the mechanical pass is over budget,
/// summarize the older region (everything but the seed + last 2 turns) into one
/// note via a single cheap-model call. Best-effort: on failure, return the input.
async fn summarize_overflow(
    http: &reqwest::Client,
    url: &str,
    model: &str,
    messages: Vec<Value>,
) -> Vec<Value> {
    if messages.len() <= 3 {
        return messages;
    }
    let keep_tail = 2;
    let seed = messages[0].clone();
    let older = &messages[1..messages.len() - keep_tail];
    let tail = &messages[messages.len() - keep_tail..];

    let blob: String = older.iter().map(flatten_text).collect::<Vec<_>>().join("\n");
    let prompt = format!(
        "Summarize the agent's earlier exploration into at most 6 terse, specific bullet findings \
         (files/symbols seen, key facts, errors hit). Keep only what's needed to finish the edit.\n\n{}",
        cap_str(&blob, 8000, 2000)
    );
    let req = json!({
        "model": model,
        "max_tokens": 512,
        "messages": [{ "role": "user", "content": prompt }],
    });
    let summary = match http.post(url).json(&req).send().await {
        Ok(r) => r
            .json::<Value>()
            .await
            .ok()
            .and_then(|b| b.get("content").and_then(|v| v.as_str()).map(|s| s.to_string())),
        Err(_) => None,
    };

    let mut out = vec![seed];
    if let Some(s) = summary {
        out.push(json!({ "role": "user", "content": format!("[Earlier exploration, summarized]\n{}", s) }));
    }
    out.extend_from_slice(tail);
    out
}

/// Flatten a message to readable text for summarization input.
fn flatten_text(m: &Value) -> String {
    match m.get("content") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter_map(|b| {
                b.get("text")
                    .and_then(|v| v.as_str())
                    .or_else(|| b.get("content").and_then(|v| v.as_str()))
                    .map(|s| s.to_string())
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_propose_edit_infers_mode() {
        let append = parse_propose_edit(&json!({ "new_string": "fn x() {}" })).unwrap();
        assert_eq!(append.mode, "append");
        let replace = parse_propose_edit(&json!({ "old_string": "a", "new_string": "b" })).unwrap();
        assert_eq!(replace.mode, "replace");
    }

    #[test]
    fn parse_propose_edit_rejects_empty() {
        assert!(parse_propose_edit(&json!({ "mode": "append", "new_string": "  " })).is_err());
        assert!(parse_propose_edit(&json!({ "mode": "replace", "new_string": "x" })).is_err());
    }

    #[test]
    fn curated_schema_includes_propose_edit_and_excludes_persona_tools() {
        let reg = ToolRegistry::default();
        let schema = curated_schema(&reg);
        let names: Vec<&str> = schema.as_array().unwrap().iter().filter_map(|t| t.get("name").and_then(|v| v.as_str())).collect();
        assert!(names.contains(&"propose_edit"));
        assert!(names.contains(&"repo_grep"));
        assert!(!names.contains(&"adr_draft"));
        assert!(!names.contains(&"delegate"));
    }
}

/// Resolve the ordered candidate-model list for the do-loop with precedence:
/// 1) explicit model always wins (list of one)
/// 2) configured list in order
/// 3) single fallback
/// 4) default pair
pub fn candidate_models(explicit: Option<&str>, configured: &[String], single: Option<&str>) -> Vec<String> {
    if let Some(m) = explicit {
        return vec![m.to_string()];
    }
    if !configured.is_empty() {
        return configured.to_vec();
    }
    if let Some(m) = single {
        return vec![m.to_string()];
    }
    vec!["devstral-small-2:24b".to_string(), "qwen2.5-coder:14b".to_string()]
}
/// Extract candidate model list from a parsed config JSON value
/// and delegate ordering/precedence to candidate_models.
pub fn react_models_from_config_value(cfg: &serde_json::Value, explicit: Option<&str>) -> Vec<String> {
    // Step (a): read configured = cfg["inference"]["react_models"] as array of strings
    let mut configured = Vec::<String>::new();
    if let Some(inference) = cfg.get("inference") {
        if let Some(react_models) = inference.get("react_models") {
            if let Some(arr) = react_models.as_array() {
                for item in arr {
                    if let Some(s) = item.as_str() {
                        configured.push(s.to_string());
                    }
                }
            }
        }
    }
    
    // Step (b): read single = cfg["inference"]["react_model"] as Option<&str>
    let single = if let Some(inference) = cfg.get("inference") {
        inference.get("react_model").and_then(|v| v.as_str())
    } else {
        None
    };
    
    // Step (c): return candidate_models(explicit, &configured, single)
    candidate_models(explicit, &configured, single)
}

/// Pick the best-of-N winner from per-candidate outcomes in priority order: the
/// first whose result passed, else the last attempt. Moves each item (DirectResult
/// is not Clone) by holding the last as it iterates. (ADR-2606072044.)
/// Hand-finished: both local models stalled on this ownership pattern.
pub fn select_best_of_n(
    outcomes: Vec<(String, DirectResult)>,
) -> (String, DirectResult) {
    let mut last = None;
    for (model, result) in outcomes {
        if result.ok {
            return (model, result);
        }
        last = Some((model, result));
    }
    last.expect("select_best_of_n: outcomes must be non-empty")
}

/// Resolve the ordered candidate-model list for a run: explicit `task.model` or
/// `HEX_REACT_MODEL` wins; else `.hex/project.json` `inference.react_models`; else
/// `inference.react_model`; else the default complementary pair. (ADR-2606072044.)
pub(crate) fn resolve_react_models(task: &DirectTask) -> Vec<String> {
    let explicit = task
        .model
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| std::env::var("HEX_REACT_MODEL").ok().filter(|s| !s.is_empty()));
    let cfg = std::fs::read_to_string(direct_exec::repo_root().join(".hex").join("project.json"))
        .ok()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    react_models_from_config_value(&cfg, explicit.as_deref())
}

/// Evidence-gated best-of-N: run the task on each candidate model in order and
/// return the first that passes (it has already committed via the evidence gate);
/// if none pass, return the last attempt. Per-candidate isolation/commit is handled
/// by `react_execute`. (ADR-2606072044.)
pub async fn react_execute_best_of_n(task: DirectTask) -> (DirectResult, u32, String) {
    let candidates = resolve_react_models(&task);
    let mut outcomes: Vec<(String, DirectResult)> = Vec::new();
    let mut total_steps = 0u32;
    for model in candidates {
        let mut t = task.clone();
        t.model = Some(model.clone());
        // A `claude-code` candidate is the frontier fallback: delegate the whole
        // task to `claude -p` (an agent, not a per-step completion) instead of the
        // local ReAct tool-loop. Same evidence gate + worktree isolation.
        let (result, steps, used) = if is_claude_model(&model) {
            claude_execute(t).await
        } else {
            react_execute(t).await
        };
        total_steps += steps;
        if result.ok {
            return (result, total_steps, used);
        }
        outcomes.push((used, result));
    }
    let (model, result) = select_best_of_n(outcomes);
    (result, total_steps, model)
}

/// A candidate that should be served by `claude -p` rather than the local loop.
pub(crate) fn is_claude_model(m: &str) -> bool {
    let m = m.to_ascii_lowercase();
    m == "claude-code" || m == "claude" || m.starts_with("claude-code")
}

/// Frontier fallback: delegate the whole task to `claude -p` inside an isolated
/// worktree, then gate the result with the SAME evidence command + commit as the
/// ReAct path. `claude -p` is itself an agent, so it slots in as a task delegate
/// (no per-step tool protocol). Uses the operator's logged-in `claude` CLI — no
/// API key, no VRAM ceiling. Mirrors `react_execute`'s workspace lifecycle.
pub async fn claude_execute(task: DirectTask) -> (DirectResult, u32, String) {
    let isolate = direct_exec::want_isolation(&task);
    let slug = crate::direct_workspace::next_run_slug();
    let workspace = match crate::direct_workspace::RunWorkspace::acquire(&slug, isolate) {
        Ok(w) => w,
        Err(e) => return (DirectResult::err(format!("workspace: {e}")), 0, "claude-code".to_string()),
    };
    if let Err(e) = workspace.assert_off_operator_tree() {
        workspace.finish(false);
        return (DirectResult::err(e), 0, "claude-code".to_string());
    }
    let repo_root = workspace.workdir().to_path_buf();
    let factory = workspace.is_isolated();
    let out = claude_attempts(&task, &repo_root, factory).await;
    workspace.finish(out.0.ok);
    out
}

async fn claude_attempts(
    task: &DirectTask,
    repo_root: &std::path::Path,
    factory: bool,
) -> (DirectResult, u32, String) {
    let model = "claude-code".to_string();
    let mut result = DirectResult {
        ok: false,
        attempts: 0,
        edit_applied: false,
        committed: None,
        evidence_passed: false,
        evidence_output: String::new(),
        error: None,
    };
    let abs_path = repo_root.join(&task.file);
    let snapshot = std::fs::read_to_string(&abs_path).unwrap_or_default();

    let binary = std::env::var("HEX_CLAUDE_BINARY").unwrap_or_else(|_| "claude".to_string());
    let prompt = format!(
        "Task: {}\n\nEdit ONLY the file `{}` in this repository so that the shell command \
         `{}` exits 0. Make the change directly to the file now. Do not ask questions and do \
         not explain — just apply the edit.",
        task.instruction, task.file, task.evidence
    );
    let timeout = Duration::from_secs(
        std::env::var("CLAUDE_TIMEOUT_SECS").ok().and_then(|s| s.parse().ok()).unwrap_or(300),
    );

    let spawn = tokio::process::Command::new(&binary)
        .arg("-p")
        .arg("--dangerously-skip-permissions")
        .arg(&prompt)
        .current_dir(repo_root)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output();

    match tokio::time::timeout(timeout, spawn).await {
        Ok(Ok(_out)) => {} // claude finished (or errored) — the evidence gate decides
        Ok(Err(e)) => {
            result.error = Some(format!("claude spawn failed ({}): {}", binary, e));
            return (result, 1, model);
        }
        Err(_) => {
            let _ = std::fs::write(&abs_path, &snapshot);
            result.error = Some(format!("claude -p timed out after {}s", timeout.as_secs()));
            return (result, 1, model);
        }
    }

    result.edit_applied = std::fs::read_to_string(&abs_path).map(|c| c != snapshot).unwrap_or(false);
    let (passed, output) = direct_exec::run_evidence(&task.evidence, repo_root).await;
    let vacuous = passed && direct_exec::evidence_is_vacuous(&output);
    result.evidence_output = output.chars().take(4000).collect();
    result.attempts = 1;

    if passed && !vacuous {
        match direct_exec::commit(repo_root, &task.file, &task.instruction, factory).await {
            Ok(hash) => {
                result.ok = true;
                result.evidence_passed = true;
                result.committed = Some(hash);
            }
            Err(e) => {
                let _ = std::fs::write(&abs_path, &snapshot);
                result.error = Some(format!("commit: {}", e));
            }
        }
    } else {
        let _ = std::fs::write(&abs_path, &snapshot);
        result.error = Some(if vacuous {
            "claude: evidence passed vacuously (0 tests) — reverted".to_string()
        } else {
            "claude: evidence did not pass — reverted".to_string()
        });
    }
    (result, 1, model)
}
