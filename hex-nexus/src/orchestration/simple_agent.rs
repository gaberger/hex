//! Simple agent loop — the deliberately-flat alternative to the SOP path.
//!
//! Replaces the persona-rephrasing-rejoin loop (org_responder →
//! commitment_parser → drafter → twin → executor) with one straight
//! line: operator intent → LLM (with typed-tool function calling) →
//! ToolRegistry::execute() → tool_result → loop until LLM is done OR
//! the iteration budget hits. No personas, no Confirm:/Silent: contract,
//! no atomic-claim, no dual registry.
//!
//! Same gates apply as elsewhere — every write that goes through
//! `code_patch` / `adr_draft` / `spec_draft` / `workplan_emit` /
//! `adr_status_set` is tagged proposed_by="tool:<name>" by those tools,
//! so the twin's auto-approve fast path fires and the executor's
//! cargo_check + autonomous commit step land the artifact. The simple
//! agent loop never bypasses safety; it bypasses ceremony.

use crate::tools::ToolRegistry;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::{Duration, Instant};

const DEFAULT_MAX_ITERATIONS: u32 = 10;
const DEFAULT_MAX_TOKENS: u32 = 4096;

/// Configuration knobs for a single agent run.
pub struct RunConfig {
    pub intent: String,
    pub max_iterations: u32,
    pub max_tokens: u32,
    pub model: Option<String>,
}

/// Per-iteration step the agent took. Returned in the run summary so the
/// caller can audit what happened.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AgentStep {
    pub iteration: u32,
    pub tool: String,
    pub input: Value,
    pub ok: bool,
    pub output: Value,
    pub error: Option<String>,
    pub elapsed_ms: u64,
}

/// Final outcome of an agent run.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RunSummary {
    pub iterations: u32,
    pub steps: Vec<AgentStep>,
    pub final_text: String,
    pub stop_reason: String, // "finished" | "max_iterations" | "no_tool_use" | "error"
    pub elapsed_ms: u64,
}

/// Run one agent loop end-to-end. Returns when the LLM emits a turn
/// with no tool_use blocks (it's done explaining), when the iteration
/// budget is exhausted, or on transport error.
///
/// The inference path here is the same /api/anthropic-messages-compatible
/// endpoint the sop_executor uses, but with NO persona system prompt,
/// NO SOP phase scaffolding, and NO single-action emit constraint. The
/// LLM is just told what it can do and is left to drive.
pub async fn run(
    cfg: RunConfig,
    registry: Arc<ToolRegistry>,
    inference_url: String,
) -> Result<RunSummary, String> {
    let started = Instant::now();
    let max_iterations = if cfg.max_iterations == 0 {
        DEFAULT_MAX_ITERATIONS
    } else {
        cfg.max_iterations
    };
    let max_tokens = if cfg.max_tokens == 0 {
        DEFAULT_MAX_TOKENS
    } else {
        cfg.max_tokens
    };
    let model = cfg
        .model
        .clone()
        .or_else(|| std::env::var("HEX_AGENT_MODEL").ok())
        .unwrap_or_else(|| "qwen2.5-coder:14b".to_string());

    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(180))
        .build()
        .map_err(|e| format!("http build: {}", e))?;

    let system_prompt = build_system_prompt(&registry);
    let mut messages: Vec<Value> = vec![json!({
        "role": "user",
        "content": cfg.intent,
    })];

    let tools_schema = registry.anthropic_schema();

    let mut steps: Vec<AgentStep> = Vec::new();
    let mut final_text = String::new();

    for iteration in 0..max_iterations {
        let req_body = json!({
            "model": model,
            "max_tokens": max_tokens,
            "system": system_prompt,
            "tools": tools_schema,
            "messages": messages,
        });

        let resp = http
            .post(&inference_url)
            .json(&req_body)
            .send()
            .await
            .map_err(|e| format!("inference http (iter {}): {}", iteration, e))?;
        let status = resp.status();
        let body: Value = resp
            .json()
            .await
            .map_err(|e| format!("inference json (iter {}): {}", iteration, e))?;
        if !status.is_success() {
            return Ok(RunSummary {
                iterations: iteration,
                steps,
                final_text,
                stop_reason: format!("error: inference HTTP {}", status),
                elapsed_ms: started.elapsed().as_millis() as u64,
            });
        }

        // The inference adapter normalises whatever the upstream provider
        // emits (Anthropic content[] blocks, OpenAI choices[].message.tool_calls,
        // Ollama text-tool-call parse) into a single "content" string + an
        // optional "tool_calls" array on the top-level body. Try Anthropic
        // shape first (content blocks), then fall back to OpenAI shape.
        let (assistant_text, tool_uses) = extract_tool_uses(&body);

        if !assistant_text.is_empty() {
            if !final_text.is_empty() {
                final_text.push_str("\n\n");
            }
            final_text.push_str(&assistant_text);
        }

        if tool_uses.is_empty() {
            // No more tool calls — the agent decided it's done.
            return Ok(RunSummary {
                iterations: iteration + 1,
                steps,
                final_text,
                stop_reason: "no_tool_use".to_string(),
                elapsed_ms: started.elapsed().as_millis() as u64,
            });
        }

        // Mirror the assistant turn into history so the next iteration's
        // tool_result references resolve.
        messages.push(json!({
            "role": "assistant",
            "content": assistant_turn_content(&assistant_text, &tool_uses),
        }));

        let mut tool_results: Vec<Value> = Vec::new();
        let mut saw_finish = false;
        for tu in &tool_uses {
            if tu.name == "finish" {
                saw_finish = true;
                if let Some(s) = tu.input.get("summary").and_then(|v| v.as_str()) {
                    if !final_text.is_empty() {
                        final_text.push_str("\n\n");
                    }
                    final_text.push_str(s);
                }
                continue;
            }
            let exec_start = Instant::now();
            let normalized_input = normalize_tool_input(&tu.name, tu.input.clone());
            let res = registry.execute(&tu.name, normalized_input.clone()).await;
            let elapsed = exec_start.elapsed().as_millis() as u64;
            steps.push(AgentStep {
                iteration,
                tool: tu.name.clone(),
                input: normalized_input,
                ok: res.ok,
                output: res.output.clone(),
                error: res.error.clone(),
                elapsed_ms: elapsed,
            });
            let payload = json!({
                "ok": res.ok,
                "output": res.output,
                "error": res.error,
                "elapsed_ms": res.elapsed_ms,
                "truncated": res.truncated,
            });
            tool_results.push(json!({
                "type": "tool_result",
                "tool_use_id": tu.id,
                "content": serde_json::to_string(&payload).unwrap_or_default(),
                "is_error": !res.ok,
            }));
        }

        if saw_finish {
            return Ok(RunSummary {
                iterations: iteration + 1,
                steps,
                final_text,
                stop_reason: "finished".to_string(),
                elapsed_ms: started.elapsed().as_millis() as u64,
            });
        }

        if !tool_results.is_empty() {
            messages.push(json!({
                "role": "user",
                "content": tool_results,
            }));
        }
    }

    Ok(RunSummary {
        iterations: max_iterations,
        steps,
        final_text,
        stop_reason: "max_iterations".to_string(),
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

/// A parsed tool_use block from an LLM turn.
struct ToolUse {
    id: String,
    name: String,
    input: Value,
}

/// Build the system prompt enumerating tools. Intentionally terse —
/// the LLM's job is to pick a tool, not to recite the org chart.
///
/// Two response formats are accepted (extract_tool_uses tries each):
///   1. Native Anthropic content[] blocks with type=tool_use (when the
///      provider supports function-calling natively).
///   2. OpenAI choices[].message.tool_calls (when routed through an
///      OpenAI-compatible adapter).
///   3. Text-mode JSON envelope as a fallback for local models without
///      tool-use support: emit fenced ```json { "tool": "<name>",
///      "args": { ... } } ``` or fenced ```json { "finish": "<summary>" }
///      ```. The parser scans for these envelopes in the response text.
///
/// The system prompt requires #3 explicitly so the LLM doesn't drift
/// into prose-describing-what-it-would-do.
fn build_system_prompt(registry: &ToolRegistry) -> String {
    let mut s = String::from(
        "You are a focused hex agent. Use the typed tools below to fulfill the operator's intent.\n\n\
         RESPONSE FORMAT — emit ONE OR MORE fenced JSON blocks; nothing else.\n\
         For each tool you want to call:\n\
         ```json\n\
         { \"tool\": \"<tool-name>\", \"args\": { ...exact keys from the tool's input_schema below... } }\n\
         ```\n\
         When the intent is fully satisfied, end with:\n\
         ```json\n\
         { \"finish\": \"<one-sentence summary>\" }\n\
         ```\n\n\
         Rules:\n\
         - The fence delimiter is exactly three backticks + the word `json`.\n\
         - Each block is a single JSON object — `{ \"tool\": ..., \"args\": ... }` OR `{ \"finish\": ... }`.\n\
         - Use EXACTLY the key names from each tool's input_schema. Do NOT rename `path` to `file_path`, `content` to `body`, etc.\n\
         - Do NOT wrap the JSON in any other structure; do NOT add commentary text.\n\
         - Do NOT echo the operator intent back; act on it.\n\
         - You may emit multiple tool blocks in one response — they execute in order.\n\
         - cargo_check is REQUIRED after any .rs write before finish.\n\n\
         === Tools available (each with input_schema you must follow exactly) ===\n\n",
    );
    let mut names = registry.names();
    names.sort();
    for n in &names {
        if let Some(tool) = registry.get(n) {
            let schema_compact = serde_json::to_string(&tool.input_schema())
                .unwrap_or_else(|_| "{}".to_string());
            s.push_str(&format!(
                "- {} — {}\n  input_schema: {}\n\n",
                n,
                tool.description(),
                schema_compact
            ));
        }
    }
    s.push_str(
        "- finish — signal that the intent is complete.\n  \
         input_schema: {\"type\":\"object\",\"properties\":{\"summary\":{\"type\":\"string\"}},\"required\":[\"summary\"]}\n",
    );
    s
}

/// Lenient input-key normalization. LLMs drift on argument names even
/// when the schema is in the prompt — observed today: `file_path` for
/// `path`. Rather than failing the dispatch ("missing path"), normalize
/// the known cross-tool drift before handing off to
/// ToolRegistry::execute. The canonical key wins if both are present.
///
/// IMPORTANT: do NOT alias schema fields that are tool-specific. The
/// 2026-05-14 first-iteration normalizer tried to map `new_content` →
/// `content` for "all tools" and broke code_patch (whose schema
/// canonical field IS `new_content`). Lesson: only alias what's
/// genuinely cross-tool synonym (path-like keys, search patterns).
/// Per-tool schema variations belong in the schema-aware prompt, not
/// in this layer.
fn normalize_tool_input(_tool: &str, input: Value) -> Value {
    let mut obj = match input {
        Value::Object(m) => m,
        other => return other,
    };

    // Cross-tool path-like keys.
    for alias in ["file_path", "filepath", "filename", "file"] {
        if !obj.contains_key("path") {
            if let Some(v) = obj.remove(alias) {
                obj.insert("path".to_string(), v);
            }
        } else {
            obj.remove(alias);
        }
    }

    // cargo_check expects `crate`; LLM sometimes emits `cratename`.
    for alias in ["crate_name", "cratename", "package"] {
        if !obj.contains_key("crate") {
            if let Some(v) = obj.remove(alias) {
                obj.insert("crate".to_string(), v);
            }
        } else {
            obj.remove(alias);
        }
    }

    // repo_grep expects `pattern`; LLM sometimes emits `query`/`regex`.
    for alias in ["query", "regex", "search"] {
        if !obj.contains_key("pattern") {
            if let Some(v) = obj.remove(alias) {
                obj.insert("pattern".to_string(), v);
            }
        } else {
            obj.remove(alias);
        }
    }

    Value::Object(obj)
}

/// Text-mode parser: scans the assistant text for fenced ```json blocks
/// matching the contract documented in build_system_prompt. Returns
/// (text_outside_blocks, tool_uses).
///
/// Triggered when neither the Anthropic block shape nor the OpenAI
/// tool_calls shape is present in the response — the case for local
/// LLMs (qwen2.5-coder:14b, ollama) that ignore the `tools` schema in
/// the request body and reply in plain text.
fn extract_text_mode_tool_uses(text: &str) -> Vec<ToolUse> {
    let mut uses: Vec<ToolUse> = Vec::new();
    let mut rest = text;
    let mut counter: u32 = 0;
    while let Some(start) = rest.find("```json") {
        let after = &rest[start + 7..];
        let body_start = match after.find('\n') {
            Some(i) => i + 1,
            None => break,
        };
        let body = &after[body_start..];
        let end = match body.find("\n```") {
            Some(i) => i,
            None => break,
        };
        let raw_json = body[..end].trim();
        rest = &body[end + 4..];
        let parsed: Result<serde_json::Value, _> = serde_json::from_str(raw_json);
        let v = match parsed {
            Ok(v) => v,
            Err(_) => continue,
        };
        // Two shapes accepted: { tool, args } and { finish }.
        if let Some(name) = v.get("tool").and_then(|x| x.as_str()) {
            let args = v.get("args").cloned().unwrap_or(Value::Null);
            uses.push(ToolUse {
                id: format!("textmode_{}", counter),
                name: name.to_string(),
                input: args,
            });
            counter += 1;
        } else if let Some(summary) = v.get("finish").and_then(|x| x.as_str()) {
            uses.push(ToolUse {
                id: format!("textmode_{}", counter),
                name: "finish".to_string(),
                input: serde_json::json!({ "summary": summary }),
            });
            counter += 1;
        }
    }
    uses
}

/// Reconstruct what to put in the assistant message's `content` field so
/// the next iteration's tool_result blocks refer to the right tool_use ids.
fn assistant_turn_content(text: &str, tool_uses: &[ToolUse]) -> Value {
    let mut blocks: Vec<Value> = Vec::new();
    if !text.is_empty() {
        blocks.push(json!({ "type": "text", "text": text }));
    }
    for tu in tool_uses {
        blocks.push(json!({
            "type": "tool_use",
            "id": tu.id,
            "name": tu.name,
            "input": tu.input,
        }));
    }
    Value::Array(blocks)
}

/// Try the Anthropic content-block shape, then the OpenAI tool_calls
/// shape, then give up and return the body as plain text. Returns
/// (assistant_text, tool_uses).
fn extract_tool_uses(body: &Value) -> (String, Vec<ToolUse>) {
    let mut text = String::new();
    let mut uses: Vec<ToolUse> = Vec::new();

    // Anthropic shape: { content: [{type: text|tool_use, ...}, ...] }
    if let Some(blocks) = body.get("content").and_then(|v| v.as_array()) {
        // If the content is a single string rather than blocks, it's
        // probably the normalised "text completion" path — fall through.
        let block_shape = blocks
            .first()
            .map(|b| b.is_object())
            .unwrap_or(false);
        if block_shape {
            for block in blocks {
                match block.get("type").and_then(|v| v.as_str()) {
                    Some("text") => {
                        if let Some(t) = block.get("text").and_then(|v| v.as_str()) {
                            if !text.is_empty() {
                                text.push('\n');
                            }
                            text.push_str(t);
                        }
                    }
                    Some("tool_use") => {
                        uses.push(ToolUse {
                            id: block
                                .get("id")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string(),
                            name: block
                                .get("name")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string(),
                            input: block.get("input").cloned().unwrap_or(Value::Null),
                        });
                    }
                    _ => {}
                }
            }
            return (text, uses);
        }
    }

    // OpenAI shape: { choices: [{message: {content, tool_calls: [...]}}] }
    if let Some(msg) = body.pointer("/choices/0/message") {
        if let Some(c) = msg.get("content").and_then(|v| v.as_str()) {
            text.push_str(c);
        }
        if let Some(calls) = msg.get("tool_calls").and_then(|v| v.as_array()) {
            for (i, call) in calls.iter().enumerate() {
                let id = call
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| format!("call_{}", i));
                let func = call.get("function").cloned().unwrap_or(Value::Null);
                let name = func
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                let args_str = func
                    .get("arguments")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let input: Value =
                    serde_json::from_str(args_str).unwrap_or(Value::Null);
                uses.push(ToolUse { id, name, input });
            }
        }
        return (text, uses);
    }

    // Top-level normalised "content" string fallback (some adapters
    // collapse everything to a single string field).
    if let Some(s) = body.get("content").and_then(|v| v.as_str()) {
        text.push_str(s);
    }

    // Text-mode fallback: local LLMs without tool-use support reply
    // with fenced ```json envelopes in plain text per the contract in
    // build_system_prompt. Scan for those and convert to ToolUse if
    // the structured shapes above produced nothing.
    if uses.is_empty() && !text.is_empty() {
        uses = extract_text_mode_tool_uses(&text);
    }
    (text, uses)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extract_anthropic_blocks() {
        let body = json!({
            "content": [
                {"type": "text", "text": "I'll grep first."},
                {"type": "tool_use", "id": "abc", "name": "repo_grep", "input": {"pattern": "fizzbuzz"}},
            ]
        });
        let (t, uses) = extract_tool_uses(&body);
        assert_eq!(t, "I'll grep first.");
        assert_eq!(uses.len(), 1);
        assert_eq!(uses[0].name, "repo_grep");
        assert_eq!(uses[0].id, "abc");
    }

    #[test]
    fn extract_openai_tool_calls() {
        let body = json!({
            "choices": [{
                "message": {
                    "content": "Calling.",
                    "tool_calls": [{
                        "id": "call_0",
                        "function": {
                            "name": "cargo_check",
                            "arguments": "{\"crate\":\"hex-cli\"}"
                        }
                    }]
                }
            }]
        });
        let (t, uses) = extract_tool_uses(&body);
        assert_eq!(t, "Calling.");
        assert_eq!(uses.len(), 1);
        assert_eq!(uses[0].name, "cargo_check");
        assert_eq!(uses[0].input.get("crate").and_then(|v| v.as_str()), Some("hex-cli"));
    }

    #[test]
    fn no_tool_use_terminates() {
        let body = json!({
            "content": [{"type": "text", "text": "Done."}]
        });
        let (t, uses) = extract_tool_uses(&body);
        assert_eq!(t, "Done.");
        assert!(uses.is_empty());
    }

    #[test]
    fn text_mode_fenced_json_tool() {
        // The qwen2.5-coder:14b shape from the 2026-05-14 fire-it demo:
        // plain text response wrapping fenced JSON envelopes.
        let resp = "```json\n{\"tool\":\"repo_grep\",\"args\":{\"pattern\":\"fizzbuzz\"}}\n```";
        let body = json!({"content": resp});
        let (_t, uses) = extract_tool_uses(&body);
        assert_eq!(uses.len(), 1);
        assert_eq!(uses[0].name, "repo_grep");
        assert_eq!(
            uses[0].input.get("pattern").and_then(|v| v.as_str()),
            Some("fizzbuzz")
        );
    }

    #[test]
    fn text_mode_fenced_json_finish() {
        let resp = "```json\n{\"finish\":\"all done\"}\n```";
        let body = json!({"content": resp});
        let (_t, uses) = extract_tool_uses(&body);
        assert_eq!(uses.len(), 1);
        assert_eq!(uses[0].name, "finish");
        assert_eq!(
            uses[0].input.get("summary").and_then(|v| v.as_str()),
            Some("all done")
        );
    }

    #[test]
    fn normalize_aliases_file_path_to_path() {
        let input = json!({"file_path": "src/foo.rs", "content": "fn main() {}"});
        let out = normalize_tool_input("code_patch", input);
        assert_eq!(out.get("path").and_then(|v| v.as_str()), Some("src/foo.rs"));
        assert_eq!(out.get("content").and_then(|v| v.as_str()), Some("fn main() {}"));
        assert!(out.get("file_path").is_none());
    }

    #[test]
    fn normalize_preserves_new_content_for_code_patch() {
        // 2026-05-14 lesson: code_patch's schema canonical field is
        // `new_content`, not `content`. The earlier normalizer mapped
        // patch/body/text/data → content for "all tools" and broke
        // code_patch by draining the real schema field. Don't touch
        // tool-specific schema fields here.
        let input = json!({
            "file_path": "Cargo.toml",
            "mode": "create",
            "new_content": "[package]\nname=\"x\"",
            "rationale": "create"
        });
        let out = normalize_tool_input("code_patch", input);
        assert_eq!(out.get("path").and_then(|v| v.as_str()), Some("Cargo.toml"));
        assert_eq!(out.get("new_content").and_then(|v| v.as_str()), Some("[package]\nname=\"x\""));
        assert_eq!(out.get("mode").and_then(|v| v.as_str()), Some("create"));
        // No 'content' key — we did NOT clobber the schema canonical field.
        assert!(out.get("content").is_none());
    }

    #[test]
    fn normalize_canonical_key_wins_when_both_present() {
        // If LLM emits BOTH path and file_path, the canonical one wins
        // and the alias gets dropped (no silent override).
        let input = json!({"path": "good.rs", "file_path": "bad.rs"});
        let out = normalize_tool_input("code_patch", input);
        assert_eq!(out.get("path").and_then(|v| v.as_str()), Some("good.rs"));
        assert!(out.get("file_path").is_none());
    }

    #[test]
    fn normalize_passthrough_on_no_aliases() {
        let input = json!({"path": "x", "content": "y"});
        let out = normalize_tool_input("code_patch", input.clone());
        assert_eq!(out, input);
    }

    #[test]
    fn text_mode_multiple_blocks() {
        let resp = "first call:\n```json\n{\"tool\":\"repo_read\",\"args\":{\"path\":\"a\"}}\n```\nsecond:\n```json\n{\"tool\":\"repo_read\",\"args\":{\"path\":\"b\"}}\n```\nfinish:\n```json\n{\"finish\":\"read both\"}\n```";
        let body = json!({"content": resp});
        let (_t, uses) = extract_tool_uses(&body);
        assert_eq!(uses.len(), 3);
        assert_eq!(uses[0].input.get("path").and_then(|v| v.as_str()), Some("a"));
        assert_eq!(uses[1].input.get("path").and_then(|v| v.as_str()), Some("b"));
        assert_eq!(uses[2].name, "finish");
    }
}
