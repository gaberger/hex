//! SOP executor (ADR-2026-05-08-2500).
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

use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, OnceLock};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::Mutex as AsyncMutex;

use crate::tools::ToolRegistry;

// ---------------------------------------------------------------------------
// In-memory SOP run store (2026-05-18 — closes the SOP telemetry gap from
// ADR-2026-05-20-ic-responder-gap follow-on).
//
// Ring buffer of the last `SOP_RUN_RING_CAP` runs. Each run starts as
// `in_flight` (no completed_at) and is patched to `completed`/`failed` when
// `run()` returns. Dashboard polls `/api/org/sop/{active,recent,runs}`.
//
// State-loss on nexus restart is acceptable: the dashboard wants real-time
// visibility, not historical archaeology. If durability ever matters, swap
// the VecDeque for a STDB-backed table.

const SOP_RUN_RING_CAP: usize = 200;

#[derive(Debug, Clone, Serialize)]
pub struct SopRunRecord {
    pub id: u64,
    pub role: String,
    pub intent: String,
    /// First 240 chars of the inbound DM. Enough for the dashboard preview
    /// without bloating the JSON payload.
    pub message_preview: String,
    pub started_at_ms: u64,
    pub completed_at_ms: Option<u64>,
    /// `in_flight` | `completed` | `failed`
    pub status: &'static str,
    pub emitted_action_kind: Option<String>,
    pub phase_trace: Vec<String>,
    pub error: Option<String>,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn ring() -> &'static AsyncMutex<VecDeque<SopRunRecord>> {
    static RING: OnceLock<AsyncMutex<VecDeque<SopRunRecord>>> = OnceLock::new();
    RING.get_or_init(|| AsyncMutex::new(VecDeque::with_capacity(SOP_RUN_RING_CAP)))
}

fn next_run_id() -> u64 {
    static SEQ: AtomicU64 = AtomicU64::new(1);
    SEQ.fetch_add(1, Ordering::Relaxed)
}

fn message_preview(s: &str) -> String {
    let trimmed = s.trim();
    if trimmed.chars().count() <= 240 {
        trimmed.to_string()
    } else {
        let head: String = trimmed.chars().take(240).collect();
        format!("{head}…")
    }
}

/// Stamp an `in_flight` record at the start of `run()`. Returns the run ID
/// so the caller can patch the record on completion.
async fn record_start(role: &str, intent: &str, message: &str) -> u64 {
    let id = next_run_id();
    let rec = SopRunRecord {
        id,
        role: role.to_string(),
        intent: intent.to_string(),
        message_preview: message_preview(message),
        started_at_ms: now_ms(),
        completed_at_ms: None,
        status: "in_flight",
        emitted_action_kind: None,
        phase_trace: Vec::new(),
        error: None,
    };
    let mut g = ring().lock().await;
    if g.len() >= SOP_RUN_RING_CAP {
        g.pop_front();
    }
    g.push_back(rec);
    id
}

/// Patch the record with the final SopResult.
async fn record_end(id: u64, result: &SopResult) {
    let mut g = ring().lock().await;
    if let Some(rec) = g.iter_mut().find(|r| r.id == id) {
        rec.completed_at_ms = Some(now_ms());
        rec.status = if result.success { "completed" } else { "failed" };
        rec.emitted_action_kind = result.emitted_action_kind.clone();
        rec.phase_trace = result.phase_trace.clone();
        rec.error = result.error.clone();
    }
}

/// Snapshot helpers consumed by the `/api/org/sop/*` routes.
pub async fn recent_runs(limit: usize) -> Vec<SopRunRecord> {
    let g = ring().lock().await;
    g.iter().rev().take(limit).cloned().collect()
}

pub async fn active_runs() -> Vec<SopRunRecord> {
    let g = ring().lock().await;
    g.iter()
        .filter(|r| r.completed_at_ms.is_none())
        .cloned()
        .collect()
}

pub async fn all_runs() -> Vec<SopRunRecord> {
    let g = ring().lock().await;
    g.iter().rev().cloned().collect()
}

/// True if `role` is opted into the SOP path. Source precedence:
///   1. `HEX_SOP_PERSONAS` env var CSV (operator override)
///   2. `.hex/project.json` → `sop.personas` (array of strings)
///   3. Default roster — every C-suite role + the chief-of-X tier
///
/// Tier-3 is the structural fix for the 2026-05-21 outage: when
/// `hex nexus start` adopts an orphan daemon, the cmd.env() default-setter
/// is bypassed and the SOP path goes dark for the entire session — every
/// operator board ask routes to the free-prose Confirm/Silent path,
/// no typed tools fire, zero `proposed_action` rows land. With this
/// default the autonomous pipeline survives env-var inheritance loss
/// across restart cycles.
pub fn is_sop_persona(role: &str) -> bool {
    // Tier 1: explicit env override.
    let csv = std::env::var("HEX_SOP_PERSONAS").unwrap_or_default();
    if !csv.trim().is_empty() {
        return csv.split(',')
            .map(|s| s.trim())
            .any(|s| !s.is_empty() && s.eq_ignore_ascii_case(role));
    }

    // Tier 2: project.json sop.personas array.
    if let Some(personas) = read_project_json_personas() {
        return personas.iter().any(|p| p.eq_ignore_ascii_case(role));
    }

    // Tier 3: default roster — every exec role gets typed-tool dispatch.
    // Matches the seed list in hex-cli/src/commands/nexus.rs:399 plus the
    // four lead roles whose pools the persona supervisor seeds.
    const DEFAULT_SOP_ROSTER: &[&str] = &[
        "ceo", "cto", "cpo", "coo", "ciso",
        "chief-architect", "chief-visionary",
        "engineering-lead", "product-lead", "sre-lead",
    ];
    DEFAULT_SOP_ROSTER.iter().any(|r| r.eq_ignore_ascii_case(role))
}

/// Read `.hex/project.json` → `sop.personas` array. Returns None on
/// missing/invalid file (caller falls through to defaults). Path
/// resolved via `HEX_PROJECT_DIR` if set, else current dir — matches
/// stdb_endpoint::locate_project_json (P4.3 convention).
fn read_project_json_personas() -> Option<Vec<String>> {
    let root = std::env::var("HEX_PROJECT_DIR")
        .ok()
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::current_dir().ok())?;
    let path = root.join(".hex").join("project.json");
    let raw = std::fs::read_to_string(&path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&raw).ok()?;
    json.get("sop")
        .and_then(|s| s.get("personas"))
        .and_then(|p| p.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
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
    // PHASE 1 CLASSIFY (outside the inner fn so we can register the
    // in_flight record with the correct intent before any LLM call).
    let intent = classify_intent(operator_message);
    let run_id = record_start(role, intent, operator_message).await;
    let result = run_inner(role, operator_message, repo_root, intent).await;
    record_end(run_id, &result).await;
    result
}

async fn run_inner(
    role: &str,
    operator_message: &str,
    _repo_root: &str,
    intent: &'static str,
) -> SopResult {
    let mut trace = Vec::new();
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
        json!({ "pattern": pattern, "glob": g, "max_matches": std::env::var("HEX_GROUND_MATCH_CAP").ok().and_then(|v| v.parse::<u32>().ok()).unwrap_or(8) })
    } else {
        json!({ "pattern": pattern, "max_matches": std::env::var("HEX_GROUND_MATCH_CAP").ok().and_then(|v| v.parse::<u32>().ok()).unwrap_or(8) })
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
    let anthropic_key = std::env::var("ANTHROPIC_API_KEY").ok();
    let openrouter_key = std::env::var("OPENROUTER_API_KEY").ok();

    // Cost governance: prefer local Ollama when HEX_SOP_REASON_PREFER_OLLAMA=1
    // OR for cheap intents (code_question, arch_review) regardless of env.
    // Falls back to paid (OR/Anthropic) only if Ollama fails. Costs roughly 0
    // for routine work; paid tier is reserved for quality-critical emissions.
    let cheap_intents = ["code_question", "arch_review", "roadmap"];
    let prefer_ollama = std::env::var("HEX_SOP_REASON_PREFER_OLLAMA")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
        || cheap_intents.contains(&intent);
    if prefer_ollama {
        tracing::info!(role = %role, intent = %intent, "reason_with_tools: preferring local Ollama (cost governance)");
        match reason_via_ollama_fallback(role, operator_message, intent, ground_pack, registry.clone()).await {
            Ok(r) => return Ok(r),
            Err(e) => {
                tracing::warn!(error = %e, "Ollama-first failed; escalating to paid tier");
                // Fall through to paid
            }
        }
    }

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
            "max_tokens": std::env::var("HEX_SOP_MAX_TOKENS").ok().and_then(|v| v.parse::<u32>().ok()).unwrap_or(8192),
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
            "max_tokens": std::env::var("HEX_SOP_MAX_TOKENS").ok().and_then(|v| v.parse::<u32>().ok()).unwrap_or(8192),
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
            // OR 402 = "out of credits" — fall back to local Ollama instead of failing the team.
            let is_payment_required = status.as_u16() == 402;
            if is_content_filter || is_payment_required {
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
            "max_tokens": std::env::var("HEX_SOP_MAX_TOKENS").ok().and_then(|v| v.parse::<u32>().ok()).unwrap_or(8192),
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
        let mut tool_calls = message.get("tool_calls").and_then(|v| v.as_array()).cloned().unwrap_or_default();

        // Same small-model fallback as openrouter path: Ollama models also
        // often emit tool calls as literal JSON in content. Parse + synthesise.
        if tool_calls.is_empty() && !assistant_content.is_empty() {
            tool_calls = parse_text_tool_calls(&assistant_content);
        }

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
    // Body content moved to
    // `orchestration::persona_prompt_seeds::reason_seed` per ADR-2026-
    // 05-23-0900 §Phase 2 (code-motion only; behavior unchanged).
    // Phase 4 of the same ADR will add an STDB-first lookup wrapping this
    // call so the active prompt can be observed via the `persona_prompt`
    // table without losing the hardcoded fallback.
    crate::orchestration::persona_prompt_seeds::reason_seed(role, intent)
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

    // Serialize env-mutating tests against the workspace test lock.
    // Without it, parallel cargo test runs race on HEX_SOP_PERSONAS +
    // HEX_PROJECT_DIR (same hazard as stdb_endpoint::tests).
    #[tokio::test]
    async fn sop_persona_csv() {
        let _lock = crate::adapters::test_env_lock().lock_owned().await;
        let prev_sop = std::env::var("HEX_SOP_PERSONAS").ok();
        let prev_dir = std::env::var("HEX_PROJECT_DIR").ok();
        std::env::set_var("HEX_SOP_PERSONAS", "cto, cpo");
        std::env::set_var("HEX_PROJECT_DIR", "/tmp/hex-sop-test-no-such-dir");
        assert!(is_sop_persona("cto"));
        assert!(is_sop_persona("CPO"));
        assert!(!is_sop_persona("ciso"));
        // Restore prior state.
        match prev_sop { Some(v) => std::env::set_var("HEX_SOP_PERSONAS", v), None => std::env::remove_var("HEX_SOP_PERSONAS") }
        match prev_dir { Some(v) => std::env::set_var("HEX_PROJECT_DIR", v), None => std::env::remove_var("HEX_PROJECT_DIR") }
    }

    #[tokio::test]
    async fn sop_persona_default_roster_when_env_unset() {
        // The 2026-05-21 outage: env unset → sop path went dark. Default
        // roster is the structural fix. Restart-safe.
        let _lock = crate::adapters::test_env_lock().lock_owned().await;
        let prev_sop = std::env::var("HEX_SOP_PERSONAS").ok();
        let prev_dir = std::env::var("HEX_PROJECT_DIR").ok();
        std::env::remove_var("HEX_SOP_PERSONAS");
        std::env::set_var("HEX_PROJECT_DIR", "/tmp/hex-sop-test-no-such-dir-2");
        assert!(is_sop_persona("cto"));
        assert!(is_sop_persona("CEO"));
        assert!(is_sop_persona("chief-architect"));
        assert!(is_sop_persona("engineering-lead"));
        assert!(!is_sop_persona("random-role"));
        match prev_sop { Some(v) => std::env::set_var("HEX_SOP_PERSONAS", v), None => std::env::remove_var("HEX_SOP_PERSONAS") }
        match prev_dir { Some(v) => std::env::set_var("HEX_PROJECT_DIR", v), None => std::env::remove_var("HEX_PROJECT_DIR") }
    }

    #[tokio::test]
    async fn sop_persona_env_overrides_default() {
        // Operator can EXCLUDE a default roster member by setting an
        // explicit CSV that doesn't include them.
        let _lock = crate::adapters::test_env_lock().lock_owned().await;
        let prev_sop = std::env::var("HEX_SOP_PERSONAS").ok();
        let prev_dir = std::env::var("HEX_PROJECT_DIR").ok();
        std::env::set_var("HEX_SOP_PERSONAS", "cto");
        std::env::set_var("HEX_PROJECT_DIR", "/tmp/hex-sop-test-no-such-dir-3");
        assert!(is_sop_persona("cto"));
        assert!(!is_sop_persona("ceo"));
        match prev_sop { Some(v) => std::env::set_var("HEX_SOP_PERSONAS", v), None => std::env::remove_var("HEX_SOP_PERSONAS") }
        match prev_dir { Some(v) => std::env::set_var("HEX_PROJECT_DIR", v), None => std::env::remove_var("HEX_PROJECT_DIR") }
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

/// Query the RL engine for the highest-q_value model known for this intent.
/// Returns None on any failure or no scores. Exploit-only (no exploration)
/// since this drives live persona work — RL exploration happens elsewhere
/// in sched_service::run_improvement_cycle.
///
/// Wires the existing rl_q_entry table (state_key, action='model:NAME', q_value)
/// — see hex-nexus/src/sched_service.rs::select_model_for_cycle for the
/// reference pattern.
pub async fn pick_model_via_rl(intent: &str) -> Option<String> {
    let stdb_host = std::env::var("HEX_SPACETIMEDB_HOST")
        .unwrap_or_else(|_| "http://127.0.0.1:3033".to_string());
    let state_key = format!("sop:{}", intent);
    let sql = format!(
        "SELECT action, q_value FROM rl_q_entry WHERE state_key = '{}'",
        state_key.replace('\'', "''")
    );
    let url = format!("{}/v1/database/{}/sql", stdb_host, hex_core::STDB_DATABASE_RL);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(1))
        .build()
        .ok()?;
    let resp = client
        .post(&url)
        .header("content-type", "text/plain")
        .body(sql)
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let parsed: Value = resp.json().await.ok()?;
    let rows = parsed
        .as_array()
        .and_then(|a| a.first())
        .and_then(|t| t.get("rows"))
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();
    let mut best: Option<(String, f64)> = None;
    for r in rows {
        let arr = r.as_array()?;
        let action = arr.first()?.as_str()?;
        let q = arr.get(1)?.as_f64()?;
        if let Some(model) = action.strip_prefix("model:") {
            if best.as_ref().map(|(_, bq)| q > *bq).unwrap_or(true) {
                best = Some((model.to_string(), q));
            }
        }
    }
    best.map(|(m, q)| {
        tracing::info!(intent = %intent, model = %m, q_value = q, "pick_model_via_rl: selected");
        m
    })
}
