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

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use crate::tools::ToolRegistry;

const POLL_INTERVAL_SECS: u64 = 60;
const MAX_PER_TICK: usize = 1;

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
    stdb_host: &str,
    hex_db: &str,
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
                if end >= 8 {
                    covered_adr_ids.insert(tail[..end].to_string());
                }
                i = abs + 1;
            }
        }
    }

    // 3. Pick the FIRST uncovered ADR (one per tick to keep inference cost bounded).
    let mut to_process: Vec<(String, String, std::path::PathBuf)> = Vec::new();
    for (id, name, path) in adr_entries {
        if covered_adr_ids.contains(&id) {
            continue;
        }
        to_process.push((id, name, path));
        if to_process.len() >= MAX_PER_TICK {
            break;
        }
    }
    if to_process.is_empty() {
        return Ok(());
    }

    for (adr_id, adr_name, adr_path) in to_process {
        if let Err(e) = derive_one(http, inference_url, registry, &adr_id, &adr_name, &adr_path).await {
            tracing::warn!(adr = %adr_name, error = %e, "workplan_auto_emitter: derive_one failed");
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

    let system = format!(
        "You are the workplan-derivation persona. Read this ADR and emit a hex \
         workplan via the workplan_emit tool. Rules:\n\
         - The workplan slug should be a kebab-case summary of the ADR title (max 60 chars)\n\
         - The 'feature' field is a one-line human description\n\
         - The 'adr' field MUST be exactly: ADR-{adr_id}\n\
         - Phases are dependency-ordered. Use P0 for domain/ports work, P1 for \
           secondary adapters, P2 for primary, P3 for usecases, P4 for integration\n\
         - Each phase has 1-5 concrete tasks; each task has id (e.g. P0.1), name \
           (concrete deliverable), layer (domain|ports|usecases|primary|secondary|infrastructure|integration), \
           AND files[] (1+ repo-relative paths the task creates or modifies — REQUIRED for hex plan reconcile).\n\
         - Map the ADR's Decision section into 1-3 phases minimum. Don't over-decompose.\n\
         - For files[]: extract concrete paths from the ADR text. Examples:\n\
           * 'modify hex-nexus/src/orchestration/drafter.rs' → files: ['hex-nexus/src/orchestration/drafter.rs']\n\
           * 'add new tool foo' → files: ['hex-nexus/src/tools/foo.rs']\n\
           * 'doc-only ADR with no code' → files: ['docs/adrs/ADR-{adr_id}-*.md'] for the ADR itself\n\
         - If the ADR is a pure-doc decision (no code work), emit ONE phase 'P0 Documentation' \
           with one task layer=infrastructure files=['docs/adrs/ADR-{adr_id}-*.md']. Reconciler marks done immediately.\n\
         You MUST call workplan_emit exactly once. Do not chat. Do not call other tools.",
        adr_id = adr_id,
    );
    let user_msg = format!("ADR file: docs/adrs/{}\n\n{}", adr_name, trimmed);

    // Use the OpenRouter / Anthropic path same as sop_executor — but simpler:
    // single tool call, no multi-round-trip loop. We emulate by sending the
    // whole tool registry but expecting workplan_emit only.
    let api_key = std::env::var("ANTHROPIC_API_KEY").ok().or_else(|| std::env::var("OPENROUTER_API_KEY").ok());
    let api_key = match api_key {
        Some(k) => k,
        None => return Err("no ANTHROPIC_API_KEY or OPENROUTER_API_KEY".to_string()),
    };

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

    let model = std::env::var("HEX_AUTOEMITTER_MODEL").unwrap_or_else(|_| "anthropic/claude-sonnet-4.5".to_string());
    let req_body = json!({
        "model": model,
        "messages": [
            { "role": "system", "content": system },
            { "role": "user",   "content": user_msg },
        ],
        "tools": openai_tools,
        "tool_choice": { "type": "function", "function": { "name": "workplan_emit" } },
        "max_tokens": 2048,
    });
    let resp = http
        .post("https://openrouter.ai/api/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("HTTP-Referer", "https://hex-aios.local")
        .header("X-Title", "hex workplan-auto-emitter")
        .json(&req_body)
        .send()
        .await
        .map_err(|e| format!("openrouter: {}", e))?;
    let status = resp.status();
    let v: Value = resp.json().await.map_err(|e| format!("openrouter json: {}", e))?;
    if !status.is_success() {
        return Err(format!("openrouter HTTP {}: {}", status, v));
    }

    let tc = v
        .pointer("/choices/0/message/tool_calls/0")
        .ok_or("no tool_calls in response")?;
    let args_str = tc
        .pointer("/function/arguments")
        .and_then(|x| x.as_str())
        .ok_or("no function.arguments")?;
    let args: Value = serde_json::from_str(args_str).map_err(|e| format!("args parse: {}", e))?;

    // Execute via registry (writes proposed_action under tool:workplan_emit).
    let result = registry.execute("workplan_emit", args).await;
    if !result.ok {
        return Err(format!("workplan_emit tool failed: {}", result.error.unwrap_or_default()));
    }
    let _ = inference_url; // silence unused — we went direct to OpenRouter for tighter control
    tracing::info!(
        adr_id = %adr_id,
        adr = %adr_name,
        wp_path = ?result.output.get("target_path"),
        "workplan_auto_emitter: derived workplan from ADR"
    );
    Ok(())
}
