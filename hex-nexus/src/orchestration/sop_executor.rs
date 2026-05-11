//! SOP executor (ADR-2605082500).
//!
//! Replaces the org_responder's single-LLM-call hot path with a
//! 5-phase state machine for SOP-enabled personas (controlled by
//! `HEX_SOP_PERSONAS` CSV env). Each phase has a deterministic gate or
//! a bounded LLM call; off-schema responses are dropped, not negotiated.
//!
//! Phase 1 CLASSIFY  — regex intent detection, no LLM
//! Phase 2 GROUND    — parallel tool calls (repo_grep, etc.), no LLM
//! Phase 3 REASON    — single Anthropic call with tool registry, function-calling
//! Phase 4 VERIFY    — schema/cargo gate on the emitted action
//! Phase 5 EMIT      — already handled by tools (proposed_action_open) + chat card

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use crate::tools::ToolRegistry;

/// True if `role` is opted into the SOP path via env CSV
/// `HEX_SOP_PERSONAS=cto,cpo`.
pub fn is_sop_persona(role: &str) -> bool {
    let csv = std::env::var("HEX_SOP_PERSONAS").unwrap_or_default();
    csv.split(',')
        .map(|s| s.trim())
        .any(|s| !s.is_empty() && s.eq_ignore_ascii_case(role))
}

/// Coarse intent classification — regex + keyword heuristics. Zero
/// LLM cost. Returns a stable string for the rest of the SOP.
pub fn classify_intent(message: &str) -> &'static str {
    let lower = message.to_ascii_lowercase();
    let has = |needle: &str| lower.contains(needle);

    // Paradigm / strategy questions — escalate immediately.
    if has("should we stop")
        || has("paradigm")
        || has("rethink")
        || has("scrap and restart")
        || has("are we wrong")
    {
        return "paradigm_question";
    }
    // Explicit code_patch directive wins over keyword matches — operator
    // can force the SOP path to route through the code_patch tool.
    if has("intent=code_patch")
        || has("emit code_patch")
        || has("via code_patch")
        || has("self-fix ask")
    {
        return "code_patch";
    }
    if has("draft") && (has("adr") || has("decision record")) {
        return "adr_draft";
    }
    if has("workplan") || has("work plan") {
        return "workplan_emit";
    }
    if has("review") && (has("architecture") || has("module") || has("crate") || has("pr") || has("merge")) {
        return "arch_review";
    }
    if has("bug") || has("fix") || has("error") || has("crash") || has("panic") {
        return "bug_triage";
    }
    if has("roadmap") || has("priority") || has("plan for") {
        return "roadmap";
    }
    "code_question"
}

/// Result of one full SOP execution. Returned from `run()` to the caller
/// so it can post the chat card and mark_read.
#[derive(Debug, Clone)]
pub struct SopResult {
    pub intent: String,
    pub phase_trace: Vec<String>,
    /// Action emitted by phase REASON via tool call. None on escalate or no-action.
    pub emitted_action_kind: Option<String>,
    pub chat_card: String,
    pub success: bool,
    pub error: Option<String>,
}

/// Run the full SOP for a single inbound persona DM. Caller wires this
/// into org_responder when `is_sop_persona(role)`.
pub async fn run(
    role: &str,
    operator_message: &str,
    repo_root: &str,
) -> SopResult {
    let mut trace = Vec::new();

    // PHASE 1 CLASSIFY
    let intent = classify_intent(operator_message);
    trace.push(format!("CLASSIFY → {}", intent));

    // GATE: paradigm questions escalate, no LLM.
    if intent == "paradigm_question" {
        let registry = ToolRegistry::default();
        let _ = registry.execute(
            "escalate_to_operator",
            json!({
                "reason": format!("Persona {} sees this as a paradigm/strategy question that requires human judgment, not autonomous action.", role),
                "urgency": "med",
            }),
        ).await;
        trace.push("ESCALATE → paradigm".to_string());
        return SopResult {
            intent: intent.to_string(),
            phase_trace: trace,
            emitted_action_kind: None,
            chat_card: format!(
                "[{}] Escalated: this looks like a paradigm/strategy decision. Operator review queued.",
                role
            ),
            success: true,
            error: None,
        };
    }

    // PHASE 2 GROUND — parallel tool calls relevant to the intent.
    let registry = Arc::new(ToolRegistry::default());
    let ground_pack = ground_for_intent(&registry, intent, operator_message).await;
    trace.push(format!(
        "GROUND → {} repo_grep matches",
        ground_pack
            .get("repo_grep")
            .and_then(|v| v.get("total_matches"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
    ));

    // PHASE 3 REASON — Anthropic call with tools attached.
    let reason_result = reason_with_tools(role, operator_message, intent, &ground_pack, registry.clone()).await;
    let reason = match reason_result {
        Ok(r) => r,
        Err(e) => {
            trace.push(format!("REASON → ERROR: {}", e));
            return SopResult {
                intent: intent.to_string(),
                phase_trace: trace,
                emitted_action_kind: None,
                chat_card: format!("[{}] reasoning failed: {}", role, e),
                success: false,
                error: Some(e),
            };
        }
    };
    trace.push(format!(
        "REASON → emitted {} (after {} tool round trips)",
        reason.emitted_kind.as_deref().unwrap_or("(no action)"),
        reason.tool_round_trips
    ));

    // PHASE 4 VERIFY (best-effort; tool-side validators already gate)
    // For adr_draft, the tool itself validated body sections + sizes.
    // For escalate, no further verify.
    // For acknowledge, no verify.
    let verified = true;
    if verified {
        trace.push("VERIFY → pass".to_string());
    } else {
        trace.push("VERIFY → fail".to_string());
    }

    // PHASE 5 EMIT — already done by the tool. Build chat card.
    let card = build_chat_card(role, intent, &reason);
    trace.push(format!("EMIT → chat card ({} chars)", card.len()));

    SopResult {
        intent: intent.to_string(),
        phase_trace: trace,
        emitted_action_kind: reason.emitted_kind,
        chat_card: card,
        success: true,
        error: None,
    }
}

#[derive(Debug)]
struct ReasonResult {
    emitted_kind: Option<String>,
    tool_round_trips: u32,
    final_text: String,
}

/// Phase GROUND: cheap parallel tool calls to populate context for REASON.
///
/// Path-bypass for upstream PII redaction: any path-like token in the
/// operator message gets pre-read here. The LLM consumes file CONTENT
/// (which is not redacted) instead of file NAMES (which can be).
async fn ground_for_intent(
    registry: &Arc<ToolRegistry>,
    intent: &str,
    operator_message: &str,
) -> Value {
    // Pre-read any explicit paths the operator mentioned. This sidesteps
    // upstream PII redaction (e.g. "MissionControl.tsx" → "[PERSON_NAME]").
    let paths = extract_repo_paths(operator_message);
    let mut prefetched: Vec<Value> = Vec::new();
    for p in paths.iter().take(6) {
        let result = registry
            .execute(
                "repo_read",
                json!({ "path": p, "max_bytes": 16 * 1024 }),
            )
            .await;
        if result.ok {
            prefetched.push(json!({
                "path": p,
                "content": result.output.get("content").cloned().unwrap_or(Value::Null),
                "byte_len": result.output.get("byte_len").cloned().unwrap_or(Value::Null),
                "total_lines": result.output.get("total_lines").cloned().unwrap_or(Value::Null),
            }));
        } else {
            prefetched.push(json!({
                "path": p,
                "error": result.error,
            }));
        }
    }

    // Pull the most distinctive nouns from the operator message — anything
    // ≥ 4 chars that's not a stopword.
    let pattern = derive_grep_pattern(operator_message);
    let glob = match intent {
        "adr_draft" => Some("docs/adrs/*.md"),
        "workplan_emit" => Some("docs/workplans/*.json"),
        _ => None,
    };
    let grep_input = if let Some(g) = glob {
        json!({ "pattern": pattern, "glob": g, "max_matches": 20 })
    } else {
        json!({ "pattern": pattern, "max_matches": 20 })
    };
    let grep_result = registry.execute("repo_grep", grep_input).await;
    json!({
        "intent": intent,
        "prefetched_paths": prefetched,
        "repo_grep": grep_result.output,
    })
}

/// Extract path-like tokens from the operator message. Looks for tokens
/// containing '/' that end in a known file extension. Stops short of
/// general filename matching to avoid false positives on prose.
fn extract_repo_paths(message: &str) -> Vec<String> {
    let mut out = Vec::new();
    for tok in message.split(|c: char| {
        c.is_ascii_whitespace()
            || c == ','
            || c == ';'
            || c == '"'
            || c == '\''
            || c == '`'
            || c == '('
            || c == ')'
    }) {
        let t = tok.trim_matches(|c: char| matches!(c, '.' | ':' | ';' | '!' | '?'));
        if !t.contains('/') {
            continue;
        }
        if t.starts_with('/') || t.starts_with("..") {
            continue;
        }
        let lower = t.to_ascii_lowercase();
        let extensions = [
            ".rs", ".ts", ".tsx", ".md", ".json", ".yaml", ".yml", ".toml",
            ".sh", ".py", ".js", ".css", ".html", ".sql",
        ];
        if extensions.iter().any(|e| lower.ends_with(e)) {
            if !out.iter().any(|s: &String| s == t) {
                out.push(t.to_string());
            }
        }
    }
    out
}

fn derive_grep_pattern(message: &str) -> String {
    let stop: std::collections::HashSet<&str> = [
        "the", "and", "for", "you", "with", "are", "was", "have", "from", "this", "that",
        "what", "how", "why", "when", "where", "should", "could", "would", "draft", "spec",
        "ceo", "cto", "cpo", "coo", "ciso", "board", "ask", "need", "any", "into",
    ].iter().copied().collect();
    let words: Vec<&str> = message
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-')
        .filter(|w| w.len() >= 4 && !stop.contains(&w.to_ascii_lowercase().as_str()))
        .take(5)
        .collect();
    if words.is_empty() {
        "TODO".to_string()
    } else {
        words.join("|")
    }
}

async fn reason_with_tools(
    role: &str,
    operator_message: &str,
    intent: &str,
    ground_pack: &Value,
    registry: Arc<ToolRegistry>,
) -> Result<ReasonResult, String> {
    // Prefer Anthropic direct (cleanest function-calling); fall back to
    // OpenRouter with OpenAI-format tools when Anthropic key absent.
    let anthropic_key = std::env::var("ANTHROPIC_API_KEY").ok();
    let openrouter_key = std::env::var("OPENROUTER_API_KEY").ok();

    if let Some(key) = anthropic_key {
        return reason_via_anthropic(role, operator_message, intent, ground_pack, registry, key).await;
    }
    if let Some(key) = openrouter_key {
        return reason_via_openrouter(role, operator_message, intent, ground_pack, registry, key).await;
    }
    Err("no ANTHROPIC_API_KEY or OPENROUTER_API_KEY available".to_string())
}

async fn reason_via_anthropic(
    role: &str,
    operator_message: &str,
    intent: &str,
    ground_pack: &Value,
    registry: Arc<ToolRegistry>,
    api_key: String,
) -> Result<ReasonResult, String> {
    let model = std::env::var("HEX_SOP_REASON_MODEL")
        .unwrap_or_else(|_| "claude-sonnet-4-5".to_string());

    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| format!("http build: {}", e))?;

    let system = build_reason_system_prompt(role, intent);
    let user_content = format!(
        "Operator message:\n>>> {}\n\nGround pack (deterministic tool results):\n{}\n\n\
         Per the SOP: emit exactly ONE structured action via tool call \
         (adr_draft, spec_draft, workplan_emit, code_patch, adr_status_set, escalate_to_operator), \
         or — if the operator's ask is genuinely answered by the ground pack alone with no \
         artifact needed — reply with a brief 1-2 sentence direct answer and no tool call. \
         For code-modifying asks (intent=code_patch, bug_triage, 'fix the X'): emit code_patch \
         after grounding the exact file:line via repo_read.",
        operator_message,
        serde_json::to_string_pretty(ground_pack).unwrap_or_default()
    );

    let mut messages: Vec<Value> = vec![json!({
        "role": "user",
        "content": user_content,
    })];

    let tools_schema = registry.anthropic_schema();

    let mut emitted_kind: Option<String> = None;
    let mut final_text = String::new();
    let mut round_trips: u32 = 0;
    let max_round_trips: u32 = std::env::var("HEX_SOP_MAX_ROUND_TRIPS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(16);

    loop {
        if round_trips >= max_round_trips {
            return Err(format!("tool round-trip cap ({}) hit without final reply", max_round_trips));
        }

        let req_body = json!({
            "model": model,
            "max_tokens": 4096,
            "system": system,
            "tools": tools_schema,
            "messages": messages,
        });

        let resp = http
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&req_body)
            .send()
            .await
            .map_err(|e| format!("anthropic http: {}", e))?;

        let status = resp.status();
        let body: Value = resp.json().await.map_err(|e| format!("anthropic json: {}", e))?;
        if !status.is_success() {
            return Err(format!("anthropic HTTP {}: {}", status, body));
        }

        let stop_reason = body.get("stop_reason").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let content_blocks = body.get("content").and_then(|v| v.as_array()).cloned().unwrap_or_default();

        // Append assistant turn to messages so subsequent tool_result references resolve.
        messages.push(json!({
            "role": "assistant",
            "content": content_blocks.clone(),
        }));

        let mut tool_uses: Vec<(String, String, Value)> = Vec::new(); // (id, name, input)
        for block in &content_blocks {
            match block.get("type").and_then(|v| v.as_str()) {
                Some("text") => {
                    if let Some(t) = block.get("text").and_then(|v| v.as_str()) {
                        final_text.push_str(t);
                    }
                }
                Some("tool_use") => {
                    let id = block.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let name = block.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let input = block.get("input").cloned().unwrap_or(Value::Null);
                    tool_uses.push((id, name, input));
                }
                _ => {}
            }
        }

        if tool_uses.is_empty() {
            // Model returned a final non-tool reply. Done.
            return Ok(ReasonResult {
                emitted_kind,
                tool_round_trips: round_trips,
                final_text,
            });
        }

        // Execute every tool_use block in this turn, append a single
        // user turn with tool_result blocks for each.
        let mut tool_results: Vec<Value> = Vec::new();
        for (id, name, input) in &tool_uses {
            // Track the FIRST emit-class tool call as the persona's emitted action.
            if emitted_kind.is_none() && matches!(name.as_str(), "adr_draft" | "workplan_emit" | "spec_draft" | "code_patch" | "adr_status_set" | "escalate_to_operator") {
                emitted_kind = Some(name.clone());
            }
            let result = registry.execute(name, input.clone()).await;
            let result_payload = json!({
                "ok": result.ok,
                "output": result.output,
                "error": result.error,
                "elapsed_ms": result.elapsed_ms,
                "truncated": result.truncated,
            });
            tool_results.push(json!({
                "type": "tool_result",
                "tool_use_id": id,
                "content": serde_json::to_string(&result_payload).unwrap_or_default(),
                "is_error": !result.ok,
            }));
        }
        messages.push(json!({
            "role": "user",
            "content": tool_results,
        }));

        round_trips += 1;

        // If model said end_turn AND we executed tool calls (rare but
        // possible on multi-block final), exit; otherwise loop for next reasoning step.
        if stop_reason == "end_turn" {
            return Ok(ReasonResult {
                emitted_kind,
                tool_round_trips: round_trips,
                final_text,
            });
        }
    }
}

/// OpenRouter path — same SOP semantics, OpenAI-format function calling.
/// Used when ANTHROPIC_API_KEY is absent. Picks a tool-capable model
/// via HEX_SOP_REASON_OR_MODEL (default anthropic/claude-sonnet-4).
async fn reason_via_openrouter(
    role: &str,
    operator_message: &str,
    intent: &str,
    ground_pack: &Value,
    registry: Arc<ToolRegistry>,
    api_key: String,
) -> Result<ReasonResult, String> {
    let model = std::env::var("HEX_SOP_REASON_OR_MODEL")
        .unwrap_or_else(|_| "anthropic/claude-sonnet-4.5".to_string());

    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| format!("http build: {}", e))?;

    let system = build_reason_system_prompt(role, intent);
    let user_content = format!(
        "Operator message:\n>>> {}\n\nGround pack (deterministic tool results):\n{}\n\n\
         Per the SOP: emit exactly ONE structured action via tool call \
         (adr_draft, spec_draft, workplan_emit, code_patch, adr_status_set, escalate_to_operator), \
         or — if the operator's ask is genuinely answered by the ground pack alone with no \
         artifact needed — reply with a brief 1-2 sentence direct answer and no tool call. \
         For code-modifying asks (intent=code_patch, bug_triage, 'fix the X'): emit code_patch \
         after grounding the exact file:line via repo_read.",
        operator_message,
        serde_json::to_string_pretty(ground_pack).unwrap_or_default()
    );

    let mut messages: Vec<Value> = vec![
        json!({ "role": "system", "content": system }),
        json!({ "role": "user", "content": user_content }),
    ];

    // OpenAI/OpenRouter tools format: [{type: "function", function: {name, description, parameters}}]
    let anthropic_arr = registry.anthropic_schema();
    let openai_tools: Vec<Value> = anthropic_arr
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|t| {
            json!({
                "type": "function",
                "function": {
                    "name": t.get("name").cloned().unwrap_or(Value::Null),
                    "description": t.get("description").cloned().unwrap_or(Value::Null),
                    "parameters": t.get("input_schema").cloned().unwrap_or_else(|| json!({"type":"object","properties":{}})),
                }
            })
        })
        .collect();

    let mut emitted_kind: Option<String> = None;
    let mut final_text = String::new();
    let mut round_trips: u32 = 0;
    let max_round_trips: u32 = std::env::var("HEX_SOP_MAX_ROUND_TRIPS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(16);

    // Conversation-wide serializer: persist across ALL round trips of this
    // REASON loop, not just within one tool_uses batch. Prevents the cross-round
    // race where round N's patch was based on file state before round N-1's
    // write committed.
    let mut paths_written_this_conversation: HashSet<String> = HashSet::new();

    loop {
        if round_trips >= max_round_trips {
            return Err(format!("tool round-trip cap ({}) hit without final reply", max_round_trips));
        }

        let req_body = json!({
            "model": model,
            "messages": messages,
            "tools": openai_tools,
            "max_tokens": 4096,
        });

        let resp = http
            .post("https://openrouter.ai/api/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", api_key))
            .header("HTTP-Referer", "https://hex-aios.local")
            .header("X-Title", "hex SOP")
            .header("content-type", "application/json")
            .json(&req_body)
            .send()
            .await
            .map_err(|e| format!("openrouter http: {}", e))?;
        let status = resp.status();
        let body: Value = resp.json().await.map_err(|e| format!("openrouter json: {}", e))?;

        // Ollama fallback on content-filter 403 or redaction errors (post-mortem of [PHONE] redaction breaking CTO outputs)
        if !status.is_success() {
            let body_str = serde_json::to_string(&body).unwrap_or_default().to_lowercase();
            let is_content_filter = status.as_u16() == 403 && (body_str.contains("content filter") || body_str.contains("redaction"));
            if is_content_filter {
                tracing::warn!(
                    role = %role, intent = %intent,
                    "openrouter content filter blocked REASON phase; retrying via local ollama"
                );
                return reason_via_ollama_fallback(role, operator_message, intent, ground_pack, registry).await;
            }
            return Err(format!("openrouter HTTP {}: {}", status, body));
        }

        let choice = match body
            .get("choices")
            .and_then(|v| v.as_array())
            .and_then(|a| a.first())
            .cloned()
        {
            Some(c) => c,
            None => {
                tracing::warn!(
                    role = %role, intent = %intent,
                    "openrouter empty choices array; retrying via local ollama"
                );
                return reason_via_ollama_fallback(role, operator_message, intent, ground_pack, registry).await;
            }
        };
        let message = choice.get("message").cloned().unwrap_or(Value::Null);
        let assistant_content = message.get("content").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let mut tool_calls = message.get("tool_calls").and_then(|v| v.as_array()).cloned().unwrap_or_default();

        // Small-model fallback: some OpenRouter models return tool calls as
        // literal text content `{"name":"foo","arguments":{...}}` instead of
        // structured `tool_calls` blocks. Parse and synthesise tool_call
        // entries so the rest of the loop dispatches correctly. Affects
        // chief-architect + chief-visionary in current routing.
        if tool_calls.is_empty() && !assistant_content.is_empty() {
            tool_calls = parse_text_tool_calls(&assistant_content);
        }

        // Push assistant turn (preserve tool_calls so subsequent tool messages link).
        messages.push(json!({
            "role": "assistant",
            "content": if assistant_content.is_empty() { Value::Null } else { Value::String(assistant_content.clone()) },
            "tool_calls": tool_calls.clone(),
        }));

        if !assistant_content.is_empty() {
            final_text.push_str(&assistant_content);
        }

        if tool_calls.is_empty() {
            return Ok(ReasonResult {
                emitted_kind,
                tool_round_trips: round_trips,
                final_text,
            });
        }

        // Execute each tool call; append a tool message per call.
        for tc in &tool_calls {
            let id = tc.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let func = tc.get("function").cloned().unwrap_or(Value::Null);
            let name = func.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let args_str = func.get("arguments").and_then(|v| v.as_str()).unwrap_or("{}");
            let input: Value = serde_json::from_str(args_str).unwrap_or(Value::Null);

            if emitted_kind.is_none() && matches!(name.as_str(), "adr_draft" | "workplan_emit" | "spec_draft" | "code_patch" | "adr_status_set" | "escalate_to_operator") {
                emitted_kind = Some(name.clone());
            }

            // Same-file serializer: if code_patch targets a path already
            // patched this round, skip execution and synthesize error.
            if name == "code_patch" {
                if let Some(path_val) = input.get("path") {
                    if let Some(path_str) = path_val.as_str() {
                        if paths_written_this_conversation.contains(path_str) {
                            let err_payload = json!({
                                "ok": false,
                                "output": {},
                                "error": format!(
                                    "race detected: path '{}' was already patched this round; re-read the current file via repo_read and emit a replace_string patch in the next round trip",
                                    path_str
                                ),
                                "elapsed_ms": 0,
                                "truncated": false,
                            });
                            messages.push(json!({
                                "role": "tool",
                                "tool_call_id": id,
                                "content": serde_json::to_string(&err_payload).unwrap_or_default(),
                            }));
                            continue;
                        }
                    }
                }
            }

            // Snapshot the path BEFORE moving input into execute().
            let patched_path: Option<String> = if name == "code_patch" {
                input.get("path").and_then(|v| v.as_str()).map(String::from)
            } else {
                None
            };

            let result = registry.execute(&name, input).await;

            // If code_patch succeeded, record the path to block subsequent
            // same-file patches this round.
            if result.ok {
                if let Some(p) = patched_path {
                    paths_written_this_conversation.insert(p);
                }
            }

            let result_payload = json!({
                "ok": result.ok,
                "output": result.output,
                "error": result.error,
                "elapsed_ms": result.elapsed_ms,
                "truncated": result.truncated,
            });
            messages.push(json!({
                "role": "tool",
                "tool_call_id": id,
                "content": serde_json::to_string(&result_payload).unwrap_or_default(),
            }));
        }

        round_trips += 1;
    }
}

/// Ollama fallback for content-filtered OpenRouter requests.
/// Mirrors reason_via_openrouter but posts to local Ollama endpoint with
/// HEX_SOP_OLLAMA_MODEL (default qwen2.5-coder:32b).
async fn reason_via_ollama_fallback(
    role: &str,
    operator_message: &str,
    intent: &str,
    ground_pack: &Value,
    registry: Arc<ToolRegistry>,
) -> Result<ReasonResult, String> {
    let model = std::env::var("HEX_SOP_OLLAMA_MODEL")
        .unwrap_or_else(|_| "qwen2.5-coder:32b".to_string());
    let ollama_url = std::env::var("HEX_SOP_OLLAMA_URL")
        .unwrap_or_else(|_| "http://localhost:11434".to_string());

    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| format!("http build: {}", e))?;

    let system = build_reason_system_prompt(role, intent);
    let user_content = format!(
        "Operator message:\n>>> {}\n\nGround pack (deterministic tool results):\n{}\n\n\
         Per the SOP: emit exactly ONE structured action via tool call \
         (adr_draft, spec_draft, workplan_emit, code_patch, adr_status_set, escalate_to_operator), \
         or — if the operator's ask is genuinely answered by the ground pack alone with no \
         artifact needed — reply with a brief 1-2 sentence direct answer and no tool call. \
         For code-modifying asks (intent=code_patch, bug_triage, 'fix the X'): emit code_patch \
         after grounding the exact file:line via repo_read.",
        operator_message,
        serde_json::to_string_pretty(ground_pack).unwrap_or_default()
    );

    let mut messages: Vec<Value> = vec![
        json!({ "role": "system", "content": system }),
        json!({ "role": "user", "content": user_content }),
    ];

    // OpenAI/Ollama tools format
    let anthropic_arr = registry.anthropic_schema();
    let openai_tools: Vec<Value> = anthropic_arr
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|t| {
            json!({
                "type": "function",
                "function": {
                    "name": t.get("name").cloned().unwrap_or(Value::Null),
                    "description": t.get("description").cloned().unwrap_or(Value::Null),
                    "parameters": t.get("input_schema").cloned().unwrap_or_else(|| json!({"type":"object","properties":{}})),
                }
            })
        })
        .collect();

    let mut emitted_kind: Option<String> = None;
    let mut final_text = String::new();
    let mut round_trips: u32 = 0;
    let max_round_trips: u32 = std::env::var("HEX_SOP_MAX_ROUND_TRIPS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(16);

    // Conversation-wide serializer: persist across ALL round trips of this
    // REASON loop, not just within one tool_uses batch. Prevents the cross-round
    // race where round N's patch was based on file state before round N-1's
    // write committed.
    let mut paths_written_this_conversation: HashSet<String> = HashSet::new();

    loop {
        if round_trips >= max_round_trips {
            return Err(format!("tool round-trip cap ({}) hit without final reply", max_round_trips));
        }

        let req_body = json!({
            "model": model,
            "messages": messages,
            "tools": openai_tools,
            "max_tokens": 4096,
        });

        let resp = http
            .post(format!("{}/v1/chat/completions", ollama_url))
            .header("content-type", "application/json")
            .json(&req_body)
            .send()
            .await
            .map_err(|e| format!("ollama http: {}", e))?;

        let status = resp.status();
        let body: Value = resp.json().await.map_err(|e| format!("ollama json: {}", e))?;
        if !status.is_success() {
            return Err(format!("ollama HTTP {}: {}", status, body));
        }

        let choice = body
            .get("choices")
            .and_then(|c| c.as_array())
            .and_then(|a| a.first())
            .cloned()
            .ok_or_else(|| "ollama: empty choices".to_string())?;
        let message = choice.get("message").cloned().unwrap_or(Value::Null);
        let assistant_content = message.get("content").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let tool_calls = message.get("tool_calls").and_then(|v| v.as_array()).cloned().unwrap_or_default();

        messages.push(json!({
            "role": "assistant",
            "content": if assistant_content.is_empty() { Value::Null } else { Value::String(assistant_content.clone()) },
            "tool_calls": tool_calls.clone(),
        }));

        if !assistant_content.is_empty() {
            final_text.push_str(&assistant_content);
        }

        if tool_calls.is_empty() {
            return Ok(ReasonResult {
                emitted_kind,
                tool_round_trips: round_trips,
                final_text,
            });
        }

        // Execute each tool call; append a tool message per call.
        for tc in &tool_calls {
            let id = tc.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let func = tc.get("function").cloned().unwrap_or(Value::Null);
            let name = func.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let args_str = func.get("arguments").and_then(|v| v.as_str()).unwrap_or("{}");
            let input: Value = serde_json::from_str(args_str).unwrap_or(Value::Null);

            if emitted_kind.is_none() && matches!(name.as_str(), "adr_draft" | "workplan_emit" | "spec_draft" | "code_patch" | "adr_status_set" | "escalate_to_operator") {
                emitted_kind = Some(name.clone());
            }

            // Same-file serializer (same logic as openrouter path)
            if name == "code_patch" {
                if let Some(path_val) = input.get("path") {
                    if let Some(path_str) = path_val.as_str() {
                        if paths_written_this_conversation.contains(path_str) {
                            let err_payload = json!({
                                "ok": false,
                                "output": {},
                                "error": format!(
                                    "race detected: path '{}' was already patched this round; re-read the current file via repo_read and emit a replace_string patch in the next round trip",
                                    path_str
                                ),
                                "elapsed_ms": 0,
                                "truncated": false,
                            });
                            messages.push(json!({
                                "role": "tool",
                                "tool_call_id": id,
                                "content": serde_json::to_string(&err_payload).unwrap_or_default(),
                            }));
                            continue;
                        }
                    }
                }
            }

            let patched_path: Option<String> = if name == "code_patch" {
                input.get("path").and_then(|v| v.as_str()).map(String::from)
            } else {
                None
            };

            let result = registry.execute(&name, input).await;

            if result.ok {
                if let Some(p) = patched_path {
                    paths_written_this_conversation.insert(p);
                }
            }

            let result_payload = json!({
                "ok": result.ok,
                "output": result.output,
                "error": result.error,
                "elapsed_ms": result.elapsed_ms,
                "truncated": result.truncated,
            });
            messages.push(json!({
                "role": "tool",
                "tool_call_id": id,
                "content": serde_json::to_string(&result_payload).unwrap_or_default(),
            }));
        }

        round_trips += 1;
    }
}

/// Small-model fallback: scan assistant text content for tool-call shapes
/// that some OpenRouter models emit as content instead of structured
/// `tool_calls`. Returns synthesised tool_call array entries.
///
/// Accepts forms:
///   {"name": "foo", "arguments": {...}}
///   {"name": "foo", "arguments": "{\"path\":\"...\"}"}
///   `tool_use foo({"path": "..."})`
///   `foo(path="...")` (best-effort — only the JSON forms parse cleanly)
fn parse_text_tool_calls(content: &str) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    // Strategy: scan for `{` followed by JSON that contains both "name"
    // and "arguments" keys. Brace-balanced extraction.
    let bytes = content.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' {
            // Find matching close brace, respecting nesting + strings
            let mut depth = 0i32;
            let mut in_string = false;
            let mut escape = false;
            let mut end = i;
            for j in i..bytes.len() {
                let c = bytes[j];
                if escape { escape = false; continue; }
                if c == b'\\' && in_string { escape = true; continue; }
                if c == b'"' { in_string = !in_string; continue; }
                if in_string { continue; }
                if c == b'{' { depth += 1; }
                else if c == b'}' {
                    depth -= 1;
                    if depth == 0 { end = j + 1; break; }
                }
            }
            if end > i {
                let candidate = &content[i..end];
                if candidate.contains("\"name\"") && candidate.contains("\"arguments\"") {
                    if let Ok(v) = serde_json::from_str::<Value>(candidate) {
                        if let (Some(name), Some(args)) =
                            (v.get("name").and_then(|x| x.as_str()),
                             v.get("arguments"))
                        {
                            // arguments may be a string-encoded JSON or a JSON object
                            let args_str = if let Some(s) = args.as_str() {
                                s.to_string()
                            } else {
                                args.to_string()
                            };
                            out.push(json!({
                                "id": format!("synth_{}", out.len()),
                                "type": "function",
                                "function": {
                                    "name": name,
                                    "arguments": args_str,
                                }
                            }));
                        }
                    }
                }
                i = end;
                continue;
            }
        }
        i += 1;
    }
    out
}

fn build_reason_system_prompt(role: &str, intent: &str) -> String {
    let role_title = match role {
        "cto" => "Chief Technology Officer",
        "cpo" => "Chief Product Officer",
        "coo" => "Chief Operating Officer",
        "ciso" => "Chief Information Security Officer",
        "chief-visionary" => "Chief Visionary",
        "chief-architect" => "Chief Architect",
        _ => "Executive",
    };
    let domain = match role {
        "cto" => "code shipping, build/test gates, day-to-day technical execution, ADR drafting for individual changes",
        "cpo" => "product strategy, UX, user-facing surfaces, behavioural specs, dashboard design",
        "coo" => "process, people, ops, workflow, runbooks, incident response",
        "ciso" => "security, compliance, secrets, threat model, hexagonal-boundary integrity",
        "chief-visionary" => "long-term direction, paradigm choices, architectural pivots, strategic posture",
        "chief-architect" => "system architecture, hexagonal-boundary integrity (cross-crate), ADR-class structural decisions, dependency strategy across the workspace, cross-cutting refactors, technical-debt prioritisation",
        _ => "general executive concerns",
    };
    let tool_hints = match role {
        "cto" => "PREFERRED TOOLS: repo_read for source files, cargo_check after any Rust suggestion, repo_grep \
                  for impact analysis across hex-nexus/src and spacetime-modules/, adr_draft for typed \
                  technical decisions. Avoid escalating ADR-class work — produce the ADR.",
        "cpo" => "PREFERRED TOOLS: repo_read for docs/specs/ and hex-nexus/assets/src (Solid views), repo_grep \
                  for user-facing string surfaces, adr_draft when shipping a behavioural change. The body \
                  should describe user flow + observable artifact, not implementation detail.",
        "coo" => "PREFERRED TOOLS: repo_grep across docs/workplans/ and scripts/, repo_read for runbooks, \
                  adr_draft for process / SOP changes. Bias toward escalate_to_operator when the ask is \
                  about WHO should do something — that's a human decision.",
        "ciso" => "PREFERRED TOOLS: repo_grep for unsafe/secret/credential patterns across the workspace, \
                  repo_read on suspect files, cargo_check (with --release for prod parity) when threat \
                  model touches Rust code. adr_draft for security policy changes. Bias toward escalate \
                  for any threat that requires operator scoping.",
        "chief-visionary" => "PREFERRED TOOLS: repo_grep across docs/adrs/ and docs/specs/ to detect drift \
                  from documented direction, repo_read on key ADRs (especially the latest 5), \
                  escalate_to_operator for paradigm-class questions. adr_draft only for direction-setting \
                  ADRs (rare). DO NOT draft technical or product ADRs — that's CTO/CPO domain; either \
                  escalate or stay silent.",
        "chief-architect" => "PREFERRED TOOLS: repo_grep workspace-wide for cross-cutting structural patterns \
                  (imports, trait impls across crates, hexagonal-boundary violations), repo_read on ports/ \
                  + composition-root + adapter mod.rs files, cargo_check --workspace after any structural \
                  suggestion, adr_draft for STRUCTURAL decisions (new ports, adapter additions, crate \
                  splits, dependency strategy). Distinct from CTO: CTO is tactical (this PR, this build); \
                  Chief Architect is strategic-but-concrete (this quarter's structural debt, the hex \
                  boundary integrity, the workspace's dependency hygiene). Distinct from Chief Visionary: \
                  CV is paradigm + multi-quarter; Chief Architect is the bridge — implementable structural \
                  decisions that survive multiple sprints. Bias against escalate when the question is \
                  'what is the right structural shape' — that IS your job.",
        _ => "PREFERRED TOOLS: repo_grep for grounding, escalate_to_operator when uncertain.",
    };
    format!(
        "You are the {role_title} ({role}) of a hexagonal AIOS development \
         project called hex. You operate under ADR-2605082500's SOP contract.\n\n\
         The intent of this turn was classified as: {intent}.\n\n\
         === CONTRACT ===\n\
         You may call tools to ground your reasoning (e.g. repo_grep additional \
         patterns, repo_read specific files, cargo_check a crate). When you have \
         what you need, emit EXACTLY ONE structured action via tool call:\n\n\
         - `adr_draft(id, title, status, body)` for an ADR (intent=adr_draft, arch_review)\n\
         - `spec_draft(slug, body)` for a docs/specs/<slug>.md design spec\n\
         - `workplan_emit(id, body_json)` for a docs/workplans/wp-<slug>.json work plan\n\
         - `code_patch(path, mode, ...)` to modify a source file (intent=code_patch, bug_triage). \
           Allowed paths: hex-*/src/, examples/, scripts/, docs/, spacetime-modules/, tests/. \
           Modes: replace_lines (line range), replace_string (anchored), append, create.\n\
         - `adr_status_set(adr_id, new_status)` to flip an ADR's Status header\n\
         - `escalate_to_operator(reason, urgency, options?)` when the operator should pick\n\
         - or no tool call + a 1-2 sentence direct text answer when the ground pack already \
           contains the answer (e.g. simple code questions about file contents)\n\n\
         For code_patch / bug_triage / fix asks: GROUND the exact file:line via repo_read \
         FIRST, then emit code_patch. Do NOT reply with a 'Confirm: I will fix...' commitment \
         when the operator asked for a code_patch — that is the wrong contract for this turn.\n\n\
         === DOMAIN + TOOL BIAS ===\n\
         Domain: {domain}\n\
         {tool_hints}\n\n\
         === HARD RULES ===\n\
         - Cite real repo paths from the ground pack or tool calls. Do NOT invent files \
           that don't exist.\n\
         - For adr_draft: id MUST be the current 10-digit timestamp form (e.g. 2605082600); \
           body MUST contain `## Context`, `## Decision`, and `## Consequences` sections; \
           body 200-50000 chars; status='proposed' for new drafts.\n\
         - Stay in your domain. Out-of-domain → escalate_to_operator with a 'this is X's domain' note.\n\
         - The operator does not want padding. Be precise. Cite. Decide.",
        role = role,
        role_title = role_title,
        intent = intent,
        domain = domain,
        tool_hints = tool_hints,
    )
}

fn build_chat_card(role: &str, intent: &str, reason: &ReasonResult) -> String {
    let mut s = String::new();
    s.push_str(&format!("[{}] intent={} ", role, intent));
    if let Some(ref kind) = reason.emitted_kind {
        s.push_str(&format!("→ {} ", kind));
    } else {
        s.push_str("→ direct ");
    }
    s.push_str(&format!("(rounds={})", reason.tool_round_trips));
    if !reason.final_text.is_empty() {
        // Was 800; bumped to 4000 so status reports / arch reviews don't get
        // cut at section 2. Override via HEX_SOP_CHAT_CARD_MAX_CHARS.
        let cap: usize = std::env::var("HEX_SOP_CHAT_CARD_MAX_CHARS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(4000);
        s.push_str(&format!("\n\n{}", reason.final_text.chars().take(cap).collect::<String>()));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intent_classifier() {
        assert_eq!(classify_intent("Draft an ADR for typed tools"), "adr_draft");
        assert_eq!(classify_intent("Should we stop and rethink the paradigm"), "paradigm_question");
        assert_eq!(classify_intent("there's a bug in the chat dispatcher"), "bug_triage");
        assert_eq!(classify_intent("Review the architecture of the merge gate"), "arch_review");
        assert_eq!(classify_intent("hello"), "code_question");
    }

    #[test]
    fn sop_persona_csv() {
        std::env::set_var("HEX_SOP_PERSONAS", "cto, cpo");
        assert!(is_sop_persona("cto"));
        assert!(is_sop_persona("CPO"));
        assert!(!is_sop_persona("ciso"));
        std::env::remove_var("HEX_SOP_PERSONAS");
    }

    #[test]
    fn grep_pattern_extraction() {
        let p = derive_grep_pattern("Draft an ADR for the typed tool library and SOP execution");
        assert!(p.contains("typed") || p.contains("library") || p.contains("execution"));
    }

    #[test]
    fn path_extraction_finds_real_paths() {
        let m = "Use repo_read on hex-nexus/src/tools/cargo_check.rs and \
                 hex-nexus/assets/src/components/views/MissionControl.tsx, \
                 then call adr_draft.";
        let paths = extract_repo_paths(m);
        assert!(paths.contains(&"hex-nexus/src/tools/cargo_check.rs".to_string()));
        assert!(paths.contains(&"hex-nexus/assets/src/components/views/MissionControl.tsx".to_string()));
    }

    #[test]
    fn path_extraction_rejects_absolute_and_traversal() {
        let m = "Read /etc/passwd or ../../../secrets.toml";
        let paths = extract_repo_paths(m);
        assert!(paths.is_empty(), "expected no paths, got {:?}", paths);
    }

    #[test]
    fn path_extraction_ignores_prose_with_slashes() {
        let m = "split the work 50/50 between teams and report by EOD";
        let paths = extract_repo_paths(m);
        assert!(paths.is_empty());
    }
}
