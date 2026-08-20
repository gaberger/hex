//! workplan_auto_emitter — closes the ADR → workplan auto-bridge.
//!
//! Polls executed_action for new file_writes under docs/adrs/. For each
//! ADR-* file that doesn't already have a matching wp-<slug>.json, fires
//! one inference call asking the LLM to derive a workplan stub from the
//! ADR's Decision section, and calls workplan_emit to land the JSON.
//!
//! This is what makes the agent factory self-managing: a persona writes
//! an ADR via adr_draft → the executor lands it → THIS task derives the
//! workplan → `hex swarm init wp-<slug>` dispatches it → agents call
//! code_patch to apply changes → cargo_check verifies → reconcile flips
//! ADR Proposed → Accepted via adr_status_set. Zero operator hands.
//!
//! Disabled with HEX_DISABLE_WORKPLAN_AUTO_EMITTER=1.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::tools::ToolRegistry;

const POLL_INTERVAL_SECS: u64 = 60;
const MAX_PER_TICK: usize = 1;
/// Cooldown for an ADR that failed derivation. Skip it for this long
/// before retrying. Without this gate, the auto-emitter retries the
/// same FIRST uncovered ADR every tick — observed 2026-05-21: ADR-035
/// failed "no tool_calls in response" once per minute for an hour,
/// burning inference calls and feeding the nexus-CPU runaway.
const RETRY_COOLDOWN_SECS: u64 = 3600;

/// Per-ADR last-failure timestamp. Tick skips any ADR whose last
/// failure is within `RETRY_COOLDOWN_SECS`.
fn failure_cache() -> &'static Mutex<HashMap<String, Instant>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn spawn(stdb_host: String, hex_db: String) {
    if std::env::var("HEX_DISABLE_WORKPLAN_AUTO_EMITTER").is_ok() {
        tracing::info!("workplan_auto_emitter disabled via HEX_DISABLE_WORKPLAN_AUTO_EMITTER");
        return;
    }
    tokio::spawn(async move {
        let http = match reqwest::Client::builder()
            .timeout(Duration::from_secs(180))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "workplan_auto_emitter: http client build failed");
                return;
            }
        };
        let registry = Arc::new(ToolRegistry::default());
        tracing::info!(
            stdb_host = %stdb_host,
            db = %hex_db,
            interval_secs = POLL_INTERVAL_SECS,
            "workplan_auto_emitter: started"
        );

        // Wait so other init bits settle.
        tokio::time::sleep(Duration::from_secs(60)).await;

        // Ports/paths used by the inference loopback call.
        let inference_url = format!(
            "http://127.0.0.1:{}/api/inference/complete",
            std::env::var("HEX_NEXUS_PORT").unwrap_or_else(|_| "5555".to_string())
        );

        let mut ticker = tokio::time::interval(Duration::from_secs(POLL_INTERVAL_SECS));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            ticker.tick().await;
            if let Err(e) = run_one(&http, &stdb_host, &hex_db, &inference_url, &registry).await {
                tracing::debug!(error = %e, "workplan_auto_emitter: tick error");
            }
        }
    });
}

async fn run_one(
    http: &reqwest::Client,
    _stdb_host: &str,
    _hex_db: &str,
    inference_url: &str,
    registry: &Arc<ToolRegistry>,
) -> Result<(), String> {
    // 1. List ADR files on disk.
    let repo_root = std::env::var("HEX_REPO_ROOT").unwrap_or_else(|_| "/home/gary/hex-intf".to_string());
    let adrs_dir = std::path::Path::new(&repo_root).join("docs/adrs");
    let workplans_dir = std::path::Path::new(&repo_root).join("docs/workplans");

    let adr_entries: Vec<(String, String, std::path::PathBuf)> = match std::fs::read_dir(&adrs_dir) {
        Ok(rd) => rd
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_str()?.to_string();
                if !name.starts_with("ADR-") || !name.ends_with(".md") {
                    return None;
                }
                // Extract id (digits between "ADR-" and the next "-")
                let after = name.strip_prefix("ADR-")?;
                let id = after.split_once('-').map(|(a, _)| a)?.to_string();
                if !id.chars().all(|c| c.is_ascii_digit()) {
                    return None;
                }
                Some((id, name, e.path()))
            })
            .collect(),
        Err(e) => return Err(format!("read_dir adrs: {}", e)),
    };

    // 2. Existing workplan slugs (so we can dedup by ADR id reference).
    let existing_workplans: HashSet<String> = match std::fs::read_dir(&workplans_dir) {
        Ok(rd) => rd
            .flatten()
            .filter_map(|e| e.file_name().to_str().map(|s| s.to_string()))
            .filter(|n| n.starts_with("wp-") && n.ends_with(".json"))
            .collect(),
        Err(_) => HashSet::new(),
    };
    // Walk existing workplans, collect any ADR ids already referenced.
    let mut covered_adr_ids: HashSet<String> = HashSet::new();
    for wp_name in &existing_workplans {
        if let Ok(content) = std::fs::read_to_string(workplans_dir.join(wp_name)) {
            // Cheap scan: any "ADR-<digits>" token marks coverage.
            let mut i = 0;
            while let Some(idx) = content[i..].find("ADR-") {
                let abs = i + idx + 4;
                let tail = &content[abs..];
                let end = tail.find(|c: char| !c.is_ascii_digit()).unwrap_or(tail.len());
                if end >= 3 {
                    covered_adr_ids.insert(tail[..end].to_string());
                }
                i = abs + 1;
            }
        }
    }

    // 3. Pick the FIRST uncovered ADR not in failure cooldown
    //    (one per tick to keep inference cost bounded).
    let now = Instant::now();
    let cooldown = Duration::from_secs(RETRY_COOLDOWN_SECS);
    let mut to_process: Vec<(String, String, std::path::PathBuf)> = Vec::new();
    {
        let cache = failure_cache().lock().unwrap();
        for (id, name, path) in adr_entries {
            if covered_adr_ids.contains(&id) {
                continue;
            }
            if let Some(last_fail) = cache.get(&id) {
                if now.saturating_duration_since(*last_fail) < cooldown {
                    // Recently failed — skip until cooldown expires.
                    continue;
                }
            }
            to_process.push((id, name, path));
            if to_process.len() >= MAX_PER_TICK {
                break;
            }
        }
    }
    if to_process.is_empty() {
        return Ok(());
    }

    for (adr_id, adr_name, adr_path) in to_process {
        match derive_one(http, inference_url, registry, &adr_id, &adr_name, &adr_path).await {
            Ok(()) => {
                // Successful derive — clear any prior failure record.
                failure_cache().lock().unwrap().remove(&adr_id);
            }
            Err(e) => {
                tracing::warn!(
                    adr = %adr_name,
                    error = %e,
                    cooldown_secs = RETRY_COOLDOWN_SECS,
                    "workplan_auto_emitter: derive_one failed — backing off"
                );
                failure_cache().lock().unwrap().insert(adr_id, now);
            }
        }
    }
    Ok(())
}

async fn derive_one(
    http: &reqwest::Client,
    inference_url: &str,
    registry: &Arc<ToolRegistry>,
    adr_id: &str,
    adr_name: &str,
    adr_path: &std::path::Path,
) -> Result<(), String> {
    let body_text = std::fs::read_to_string(adr_path).map_err(|e| format!("read {}: {}", adr_name, e))?;
    if body_text.len() > 24 * 1024 {
        // Trim to the Decision + Consequences sections to fit context.
        // Cheap: take the first 24KB.
    }
    let trimmed = body_text.chars().take(24 * 1024).collect::<String>();

    // System prompt with a one-shot example. The example is the most-
    // load-bearing part: empirically observed 2026-05-23 that abstract
    // rules alone produced ~30% schema-error rate (model emitting empty
    // phase ids, missing slug, empty phases array). A concrete example
    // showing the exact shape — every required field populated — cuts
    // that rate sharply. Keep the example minimal (1-2 phases, 1-2 tasks
    // each) so the model doesn't try to over-mimic structure.
    let system = format!(
        "You are the workplan-derivation persona. Read this ADR and emit a hex \
         workplan by calling the workplan_emit tool exactly ONCE. Do not chat. \
         Do not call other tools.\n\n\
         REQUIRED FIELDS (all 4 mandatory; missing any = rejection):\n\
           slug    — kebab-case, ≤60 chars, derived from ADR title\n\
           feature — one-line human description\n\
           adr     — MUST be exactly: ADR-{adr_id}\n\
           phases  — non-empty array; each phase has id, name, tier, tasks[]\n\n\
         PER-PHASE REQUIRED FIELDS:\n\
           id    — MUST match ^P\\d+$ (e.g. \"P0\", \"P1\", \"P2\")\n\
           name  — what this phase delivers\n\
           tier  — integer 0..5 (0=domain/ports, 1=secondary, 2=primary, 3=usecases, 4=integration)\n\
           tasks — non-empty array\n\n\
         PER-TASK REQUIRED FIELDS:\n\
           id    — e.g. \"P0.1\" (phase-id . sequence)\n\
           name  — concrete deliverable\n\
           layer — one of: domain | ports | usecases | primary | secondary | infrastructure | integration\n\
           files — non-empty array of repo-relative paths the task creates or modifies\n\n\
         === ONE-SHOT EXAMPLE — call workplan_emit with arguments shaped EXACTLY like this ===\n\
         {{\n\
           \"slug\":    \"example-feature-name\",\n\
           \"feature\": \"Short human description of what ships\",\n\
           \"adr\":     \"ADR-{adr_id}\",\n\
           \"phases\": [\n\
             {{\n\
               \"id\": \"P0\",\n\
               \"name\": \"Domain + ports scaffolding\",\n\
               \"tier\": 0,\n\
               \"tasks\": [\n\
                 {{\n\
                   \"id\": \"P0.1\",\n\
                   \"name\": \"Define the domain type\",\n\
                   \"layer\": \"domain\",\n\
                   \"files\": [\"hex-core/src/domain/feature.rs\"]\n\
                 }},\n\
                 {{\n\
                   \"id\": \"P0.2\",\n\
                   \"name\": \"Define the port interface\",\n\
                   \"layer\": \"ports\",\n\
                   \"files\": [\"hex-core/src/ports/feature_port.rs\"]\n\
                 }}\n\
               ]\n\
             }},\n\
             {{\n\
               \"id\": \"P1\",\n\
               \"name\": \"Secondary adapter implementation\",\n\
               \"tier\": 1,\n\
               \"tasks\": [\n\
                 {{\n\
                   \"id\": \"P1.1\",\n\
                   \"name\": \"Implement the adapter\",\n\
                   \"layer\": \"secondary\",\n\
                   \"files\": [\"hex-nexus/src/adapters/feature_adapter.rs\"]\n\
                 }}\n\
               ]\n\
             }}\n\
           ]\n\
         }}\n\
         === END EXAMPLE ===\n\n\
         DERIVATION RULES:\n\
         - Map the ADR's Decision section into 1-3 phases minimum (don't over-decompose).\n\
         - For files[]: extract concrete paths from the ADR text where possible:\n\
             'modify hex-nexus/src/orchestration/drafter.rs' → files: [\"hex-nexus/src/orchestration/drafter.rs\"]\n\
             'add new tool foo'                              → files: [\"hex-nexus/src/tools/foo.rs\"]\n\
         - DOC-ONLY ADR (no code work in the Decision section): emit ONE phase exactly:\n\
             {{\"id\":\"P0\",\"name\":\"Documentation\",\"tier\":0,\n\
               \"tasks\":[{{\"id\":\"P0.1\",\"name\":\"Document the decision\",\n\
                          \"layer\":\"infrastructure\",\n\
                          \"files\":[\"docs/adrs/{adr_file_glob}\"]}}]}}\n\
           Reconciler will mark this done immediately because the ADR file already exists.\n\n\
         REMINDER: call workplan_emit ONCE with the arguments shaped like the example above. \
         Every required field present. No commentary.",
        adr_id = adr_id,
        adr_file_glob = format!("ADR-{}-*.md", adr_id),
    );
    let user_msg = format!("ADR file: docs/adrs/{}\n\n{}", adr_name, trimmed);

    // Route through nexus `/api/inference/complete` — it already has the
    // tools fast-path (provider chain: local Ollama → OpenAI-compat →
    // OpenRouter, sorted by `priority_for_tools`, with credit-exhaustion
    // handling baked in). This used to go direct to OpenRouter with a
    // hardcoded model slug — that wedged for hours every time the slug
    // went stale or the OpenRouter Anthropic credit ran out. The proxy
    // fixes both: stale slug auto-routes, credit exhaustion auto-falls-
    // through to the next provider.

    // Pull just the workplan_emit schema for a focused single-tool call.
    let wp_emit = registry.get("workplan_emit").ok_or("workplan_emit tool missing from registry")?;
    let openai_tools = serde_json::json!([{
        "type": "function",
        "function": {
            "name": wp_emit.name(),
            "description": wp_emit.description(),
            "parameters": wp_emit.input_schema(),
        }
    }]);

    // Default to the nexus-resolved model name (the inference layer maps
    // this to whatever Anthropic provider is healthy + has credits). The
    // OpenRouter direct-call path used `anthropic/claude-sonnet-4.5`
    // which started returning null content circa 2026-05 — the nexus
    // resolver routes `claude-sonnet-4-6` to a working endpoint.
    let model = std::env::var("HEX_AUTOEMITTER_MODEL")
        .unwrap_or_else(|_| "claude-sonnet-4-6".to_string());
    let req_body = json!({
        "model": model,
        "messages": [
            { "role": "system", "content": system },
            { "role": "user",   "content": user_msg },
        ],
        "tools": openai_tools,
        "tool_choice": { "type": "function", "function": { "name": "workplan_emit" } },
        // 4096 covers a typical multi-phase workplan (~2-3k tokens). The
        // earlier 1024 was set when we called OpenRouter direct and were
        // budget-constrained ("can only afford 372 tokens" errors); now
        // the proxy prefers local Ollama (no credit cap), so we can give
        // the model enough room to emit a complete JSON payload. If the
        // proxy falls all the way through to a credit-bound provider,
        // operator can override down via HEX_AUTO_EMITTER_MAX_TOKENS.
        "max_tokens": std::env::var("HEX_AUTO_EMITTER_MAX_TOKENS")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(4096),
    });
    let resp = http
        .post(inference_url)
        .json(&req_body)
        .send()
        .await
        .map_err(|e| format!("inference loopback: {}", e))?;
    let status = resp.status();
    let v: Value = resp.json().await.map_err(|e| format!("inference json: {}", e))?;
    if !status.is_success() {
        return Err(format!("inference HTTP {}: {}", status, v));
    }

    // The nexus tools fast-path returns either
    //   { content, tool_calls: [{id, type: "function", function: {name, arguments}}, ...] }
    // OR (when the underlying provider returned the OpenAI shape unchanged)
    //   { choices: [{message: {content, tool_calls: [...]}}] }
    // Try the fast-path first, fall through to the OpenAI shape, fall
    // through one more time to "tool call emitted as content JSON" (local
    // Ollama tool-capable models do this — e.g. qwen2.5-coder:14b returns
    //   content = "{\"name\":\"workplan_emit\",\"arguments\":{...}}"
    // without lifting it into a structured tool_calls field).
    let args: Value = if let Some(tc) = v
        .pointer("/tool_calls/0")
        .or_else(|| v.pointer("/choices/0/message/tool_calls/0"))
    {
        let args_str = tc
            .pointer("/function/arguments")
            .and_then(|x| x.as_str())
            .ok_or("no function.arguments")?;
        serde_json::from_str(args_str).map_err(|e| format!("args parse: {}", e))?
    } else if let Some(parsed) = extract_tool_call_from_content(&v) {
        parsed
    } else {
        // Include the response shape in the error so the operator can
        // diagnose without re-running. Cap to 400 chars so logs stay
        // readable.
        let finish = v
            .pointer("/finish_reason")
            .or_else(|| v.pointer("/choices/0/finish_reason"))
            .and_then(|x| x.as_str())
            .unwrap_or("?");
        let content = v
            .pointer("/content")
            .or_else(|| v.pointer("/choices/0/message/content"))
            .and_then(|x| x.as_str())
            .unwrap_or("");
        let err_field = v.pointer("/error").map(|e| e.to_string()).unwrap_or_default();
        let snippet: String = content.chars().take(200).collect();
        return Err(format!(
            "no tool_calls in response (finish={}, err={}, content[:200]={:?})",
            finish, err_field, snippet
        ));
    };

    // Execute via registry (writes proposed_action under tool:workplan_emit).
    let result = registry.execute("workplan_emit", args).await;
    if !result.ok {
        return Err(format!("workplan_emit tool failed: {}", result.error.unwrap_or_default()));
    }
    tracing::info!(
        adr_id = %adr_id,
        adr = %adr_name,
        wp_path = ?result.output.get("target_path"),
        model = %model,
        "workplan_auto_emitter: derived workplan from ADR"
    );
    Ok(())
}

/// Some Ollama tool-capable models (qwen2.5-coder, qwen3, etc.) reliably
/// emit a tool call but in `content` as a JSON string rather than lifting
/// it into a structured `tool_calls[]` field. Recognise the common shape
///   {"name":"workplan_emit","arguments":{...}}
/// and return the arguments object. Returns None if no parseable inline
/// tool call is found — caller then surfaces the proper "no tool_calls"
/// error with diagnostic context.
fn extract_tool_call_from_content(v: &Value) -> Option<Value> {
    let content = v
        .pointer("/content")
        .or_else(|| v.pointer("/choices/0/message/content"))
        .and_then(|x| x.as_str())?;

    // Strip common markdown fence prefixes the model might wrap around the JSON.
    let trimmed = content.trim();
    let trimmed = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .unwrap_or(trimmed)
        .trim_end_matches("```")
        .trim();

    let parsed: Value = serde_json::from_str(trimmed).ok()?;

    // Shape A: { "name": "workplan_emit", "arguments": { ... } }
    if let (Some(name), Some(args)) = (parsed.get("name"), parsed.get("arguments")) {
        if name.as_str() == Some("workplan_emit") {
            return Some(args.clone());
        }
    }

    // Shape B: { "tool_calls": [{"name":..., "arguments":...}, ...] }
    if let Some(calls) = parsed.get("tool_calls").and_then(|v| v.as_array()) {
        for call in calls {
            let name = call
                .pointer("/name")
                .or_else(|| call.pointer("/function/name"))
                .and_then(|x| x.as_str());
            if name == Some("workplan_emit") {
                if let Some(args) = call
                    .pointer("/arguments")
                    .or_else(|| call.pointer("/function/arguments"))
                {
                    if let Some(args_str) = args.as_str() {
                        return serde_json::from_str::<Value>(args_str).ok();
                    }
                    return Some(args.clone());
                }
            }
        }
    }

    // Shape C: the model emitted the workplan_emit args directly — i.e. the
    // whole content IS the workplan JSON. Heuristic: it has the required
    // top-level fields the tool schema demands.
    if parsed.get("feature").is_some() && parsed.get("phases").is_some() {
        return Some(parsed);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extract_inline_name_arguments_shape() {
        let v = json!({
            "content": r#"{"name":"workplan_emit","arguments":{"slug":"x","feature":"f","phases":[]}}"#
        });
        let args = extract_tool_call_from_content(&v).unwrap();
        assert_eq!(args.get("slug").and_then(|x| x.as_str()), Some("x"));
    }

    #[test]
    fn extract_inline_tool_calls_array_shape() {
        let v = json!({
            "content": r#"{"tool_calls":[{"name":"workplan_emit","arguments":{"slug":"y","feature":"f","phases":[]}}]}"#
        });
        let args = extract_tool_call_from_content(&v).unwrap();
        assert_eq!(args.get("slug").and_then(|x| x.as_str()), Some("y"));
    }

    #[test]
    fn extract_inline_with_markdown_fence() {
        let v = json!({
            "content": "```json\n{\"name\":\"workplan_emit\",\"arguments\":{\"slug\":\"z\",\"feature\":\"f\",\"phases\":[]}}\n```"
        });
        let args = extract_tool_call_from_content(&v).unwrap();
        assert_eq!(args.get("slug").and_then(|x| x.as_str()), Some("z"));
    }

    #[test]
    fn extract_openai_choices_message_content_shape() {
        let v = json!({
            "choices": [{"message": {"content": r#"{"name":"workplan_emit","arguments":{"slug":"q","feature":"f","phases":[]}}"#}}]
        });
        let args = extract_tool_call_from_content(&v).unwrap();
        assert_eq!(args.get("slug").and_then(|x| x.as_str()), Some("q"));
    }

    #[test]
    fn extract_bare_args_shape_when_top_level_workplan_fields_present() {
        // Model skipped the {name,arguments} wrapper and just emitted the
        // workplan_emit arguments directly. Accept when feature+phases present.
        let v = json!({
            "content": r#"{"slug":"bare","feature":"direct","phases":[]}"#
        });
        let args = extract_tool_call_from_content(&v).unwrap();
        assert_eq!(args.get("slug").and_then(|x| x.as_str()), Some("bare"));
    }

    #[test]
    fn extract_returns_none_for_non_workplan_content() {
        let v = json!({"content": "Sorry, I cannot help with that."});
        assert!(extract_tool_call_from_content(&v).is_none());
    }

    #[test]
    fn extract_returns_none_for_wrong_tool_name() {
        let v = json!({
            "content": r#"{"name":"something_else","arguments":{"x":1}}"#
        });
        assert!(extract_tool_call_from_content(&v).is_none());
    }
}
