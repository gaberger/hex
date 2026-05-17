//! Digital-twin reviewer (ADR-2605082300).
//!
//! Polls proposed_action.pending, reviews each against the operator's
//! documented standards (memory dir + repo grounding), emits a verdict
//! via STDB. The twin is itself an inference call — uses the operator's
//! own memory as authority so the system stays aligned without the
//! operator being the synchronous gate.

use std::collections::HashMap;
use std::time::Duration;

const POLL_INTERVAL_SECS: u64 = 20;
const MEMORY_CAP_BYTES: usize = 32 * 1024;
const PAYLOAD_PREVIEW_BYTES: usize = 4 * 1024;
const TWIN_MAX_TOKENS: u32 = 512;
/// Hermes /goal-style fail-open: after this many consecutive parse failures
/// on a single action_id, escalate to the operator instead of retrying
/// forever. Inference / HTTP errors do NOT count toward this budget —
/// only structured-output parse failures (no JSON, missing verdict, etc.)
/// where the model is producing prose instead of the expected schema.
const MAX_PARSE_FAILURES: u32 = 5;
/// Default location of operator memory. Override via HEX_OPERATOR_MEMORY_DIR.
const DEFAULT_MEMORY_DIR: &str =
    "/home/gary/.claude/projects/-var-home-gary-hex-intf/memory";

pub fn spawn(stdb_host: String, hex_db: String, port: u16) {
    if std::env::var("HEX_DISABLE_TWIN").is_ok() {
        tracing::info!("twin_reviewer disabled via HEX_DISABLE_TWIN");
        return;
    }
    tokio::spawn(async move {
        let http = match reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "twin_reviewer: http client build failed; disabled");
                return;
            }
        };
        let inference_url = format!("http://127.0.0.1:{}/api/inference/complete", port);
        let memory_dir = std::env::var("HEX_OPERATOR_MEMORY_DIR")
            .unwrap_or_else(|_| DEFAULT_MEMORY_DIR.to_string());
        tracing::info!(
            stdb_host = %stdb_host,
            db = %hex_db,
            memory_dir = %memory_dir,
            "twin_reviewer: started"
        );

        // Wait for STDB + drafter to seed something.
        tokio::time::sleep(Duration::from_secs(40)).await;

        let mut ticker = tokio::time::interval(Duration::from_secs(POLL_INTERVAL_SECS));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        // Per-action parse-failure counter. Hermes /goal pattern: a broken
        // judge must never wedge progress; turn budget is the real backstop.
        let mut parse_failures: HashMap<u64, u32> = HashMap::new();

        loop {
            ticker.tick().await;
            if let Err(e) = run_one(
                &http,
                &stdb_host,
                &hex_db,
                &inference_url,
                &memory_dir,
                &mut parse_failures,
            )
            .await
            {
                tracing::debug!(error = %e, "twin_reviewer: tick error");
            }
        }
    });
}

#[derive(Debug)]
struct PendingAction {
    id: u64,
    kind: String,
    payload_json: String,
    proposed_by: String,
    related_commitment_id: u64,
    /// Looked up via the commitment's thread_id — the CEO message that
    /// started the chain. Empty if not derivable.
    originating_ceo_ask: String,
}

async fn run_one(
    http: &reqwest::Client,
    stdb_host: &str,
    hex_db: &str,
    inference_url: &str,
    memory_dir: &str,
    parse_failures: &mut HashMap<u64, u32>,
) -> Result<(), String> {
    let pending = fetch_pending(http, stdb_host, hex_db).await?;
    if pending.is_empty() {
        parse_failures.clear();
        return Ok(());
    }
    // Snapshot operator memory once per tick.
    let memory = load_operator_memory(memory_dir);

    // GC counters for actions no longer pending (decided / removed).
    let still_pending: std::collections::HashSet<u64> =
        pending.iter().map(|a| a.id).collect();
    parse_failures.retain(|k, _| still_pending.contains(k));

    for action in pending.into_iter().take(3) {
        let attempts = parse_failures.get(&action.id).copied().unwrap_or(0);
        if attempts >= MAX_PARSE_FAILURES {
            let rationale = format!(
                "twin parse-failure budget exhausted: {} attempts produced no valid JSON; \
                 routing to operator for manual review (Hermes /goal-style fail-open)",
                attempts
            );
            tracing::warn!(
                action_id = action.id,
                attempts,
                "twin_reviewer: escalating after parse-failure budget"
            );
            match decide(
                http,
                stdb_host,
                hex_db,
                action.id,
                "escalate",
                &rationale,
                &rationale,
            )
            .await
            {
                Ok(()) => {
                    parse_failures.remove(&action.id);
                }
                Err(e) => {
                    tracing::warn!(
                        action_id = action.id,
                        error = %e,
                        "twin_reviewer: escalate decide failed"
                    );
                }
            }
            continue;
        }

        match review_one(http, stdb_host, hex_db, inference_url, &memory, &action).await {
            Ok(()) => {
                parse_failures.remove(&action.id);
            }
            Err(e) => {
                let is_parse_failure = e.contains("no JSON")
                    || e.contains("json parse")
                    || e.contains("missing verdict")
                    || e.contains("invalid verdict");
                if is_parse_failure {
                    *parse_failures.entry(action.id).or_insert(0) += 1;
                }
                tracing::warn!(
                    action_id = action.id,
                    error = %e,
                    parse_failure = is_parse_failure,
                    "twin_reviewer: review_one failed"
                );
            }
        }
    }
    Ok(())
}

async fn fetch_pending(
    http: &reqwest::Client,
    stdb_host: &str,
    hex_db: &str,
) -> Result<Vec<PendingAction>, String> {
    let url = format!("{}/v1/database/{}/sql", stdb_host, hex_db);
    let body = "SELECT id, kind, payload_json, proposed_by, related_commitment_id, status FROM proposed_action";
    let resp = http
        .post(&url)
        .header("Content-Type", "text/plain")
        .body(body)
        .send()
        .await
        .map_err(|e| format!("http: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let v: serde_json::Value = resp.json().await.map_err(|e| format!("json: {}", e))?;
    let rows = v
        .as_array()
        .and_then(|a| a.first())
        .and_then(|t| t.get("rows"))
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();
    let mut out = Vec::new();
    for r in rows {
        let cols = match r.as_array() {
            Some(c) => c,
            None => continue,
        };
        if cols.len() < 6 {
            continue;
        }
        let status = cols.get(5).and_then(|x| x.as_str()).unwrap_or("");
        if status != "pending" {
            continue;
        }
        out.push(PendingAction {
            id: cols.first().and_then(|x| x.as_u64()).unwrap_or(0),
            kind: cols.get(1).and_then(|x| x.as_str()).unwrap_or("").to_string(),
            payload_json: cols.get(2).and_then(|x| x.as_str()).unwrap_or("").to_string(),
            proposed_by: cols.get(3).and_then(|x| x.as_str()).unwrap_or("").to_string(),
            related_commitment_id: cols.get(4).and_then(|x| x.as_u64()).unwrap_or(0),
            originating_ceo_ask: String::new(),
        });
    }
    // Backfill originating_ceo_ask for each pending action via its
    // commitment's thread_id — needs the chat-relay db, so it's a
    // separate hop. Best-effort: empty on any failure.
    for action in &mut out {
        if action.related_commitment_id == 0 {
            continue;
        }
        if let Ok(thread_id) =
            fetch_commitment_thread(http, stdb_host, hex_db, action.related_commitment_id).await
        {
            if !thread_id.is_empty() {
                if let Ok(ask) = fetch_originating_ask_for_twin(http, &thread_id).await {
                    action.originating_ceo_ask = ask;
                }
            }
        }
    }
    Ok(out)
}

async fn fetch_commitment_thread(
    http: &reqwest::Client,
    stdb_host: &str,
    hex_db: &str,
    commitment_id: u64,
) -> Result<String, String> {
    let url = format!("{}/v1/database/{}/sql", stdb_host, hex_db);
    let q = format!(
        "SELECT thread_id FROM commitment WHERE id = {}",
        commitment_id
    );
    let resp = http
        .post(&url)
        .header("Content-Type", "text/plain")
        .body(q)
        .send()
        .await
        .map_err(|e| format!("http: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let v: serde_json::Value = resp.json().await.map_err(|e| format!("json: {}", e))?;
    let rows = v
        .as_array()
        .and_then(|a| a.first())
        .and_then(|t| t.get("rows"))
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();
    Ok(rows
        .first()
        .and_then(|r| r.as_array())
        .and_then(|c| c.first())
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string())
}

async fn fetch_originating_ask_for_twin(
    http: &reqwest::Client,
    thread_id: &str,
) -> Result<String, String> {
    let host = std::env::var("HEX_SPACETIMEDB_HOST")
        .unwrap_or_else(|_| "http://127.0.0.1:3033".to_string());
    let chat_db = std::env::var("HEX_AGENT_COMMS_DATABASE")
        .unwrap_or_else(|_| hex_core::stdb_database_for_module("agent-comms").to_string());
    let url = format!("{}/v1/database/{}/sql", host, chat_db);
    let safe = thread_id.replace('\'', "''");
    let q = format!(
        "SELECT id, from_agent, message FROM agent_messages WHERE thread_id = '{}'",
        safe
    );
    let resp = http
        .post(&url)
        .header("Content-Type", "text/plain")
        .body(q)
        .send()
        .await
        .map_err(|e| format!("http: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let v: serde_json::Value = resp.json().await.map_err(|e| format!("json: {}", e))?;
    let rows = v
        .as_array()
        .and_then(|a| a.first())
        .and_then(|t| t.get("rows"))
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();
    let mut from_ceo: Option<(u64, String)> = None;
    let mut any_oldest: Option<(u64, String)> = None;
    for r in rows {
        let cols = match r.as_array() {
            Some(c) => c,
            None => continue,
        };
        let id = cols.first().and_then(|x| x.as_u64()).unwrap_or(0);
        let from = cols.get(1).and_then(|x| x.as_str()).unwrap_or("");
        let msg = cols.get(2).and_then(|x| x.as_str()).unwrap_or("");
        if msg.is_empty() || id == 0 {
            continue;
        }
        match &any_oldest {
            None => any_oldest = Some((id, msg.to_string())),
            Some((cid, _)) if id < *cid => any_oldest = Some((id, msg.to_string())),
            _ => {}
        }
        if from == "ceo" {
            match &from_ceo {
                None => from_ceo = Some((id, msg.to_string())),
                Some((cid, _)) if id < *cid => from_ceo = Some((id, msg.to_string())),
                _ => {}
            }
        }
    }
    Ok(from_ceo.or(any_oldest).map(|(_, m)| m).unwrap_or_default())
}

fn load_operator_memory(dir: &str) -> String {
    let mut buf = String::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => {
            tracing::debug!(dir = %dir, "twin_reviewer: memory dir unreadable");
            return buf;
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("md") {
            continue;
        }
        let content = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let header = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("(unknown)");
        buf.push_str(&format!("\n### {}\n", header));
        buf.push_str(&content);
        if buf.len() > MEMORY_CAP_BYTES {
            buf.truncate(MEMORY_CAP_BYTES);
            buf.push_str("\n[truncated]\n");
            break;
        }
    }
    buf
}

async fn review_one(
    http: &reqwest::Client,
    stdb_host: &str,
    hex_db: &str,
    inference_url: &str,
    memory: &str,
    action: &PendingAction,
) -> Result<(), String> {
    // ADR-2605121505 — adr_status_set is a typed, schema-bounded
    // mutation (one-line file edit + reason insertion). Twin gates it on
    // payload shape and target sanity, NOT through the LLM judge. The
    // executor re-validates against the on-disk file (status==Proposed,
    // path under docs/adrs/) so this arm is purely structural.
    if action.kind == "adr_status_set" {
        return review_adr_status_set(http, stdb_host, hex_db, action).await;
    }

    // SOURCE-CODE GUARD (post-mortem of 2026-05-10 17:0x runaway):
    // Only the typed `code_patch` tool (proposed_by="tool:code_patch")
    // OR operator-explicit content (proposed_by="operator-passthrough"
    // — the operator literally typed the bytes in a board ask via the
    // drafter's literal-content shortcut) may write to hex-*/src/ or
    // spacetime-modules/*/src/. The drafter's LLM-generated stubs are
    // still banned: they clobber real source files when the persona
    // hallucinates. operator-passthrough is fine because (a) the
    // operator typed the content explicitly and (b) the executor's
    // inline cargo_check gate (ADR-2605110700 R1) still rolls
    // back any .rs write that breaks the build, so a typo doesn't ship.
    if action.kind == "file_write" {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&action.payload_json) {
            let path = v.get("path").and_then(|x| x.as_str()).unwrap_or("");
            let touches_source = path.starts_with("hex-nexus/src/")
                || path.starts_with("hex-cli/src/")
                || path.starts_with("hex-core/src/")
                || path.starts_with("hex-agent/src/")
                || path.starts_with("hex-parser/src/")
                || path.starts_with("hex-analyzer/src/")
                || path.starts_with("hex-desktop/src/")
                || (path.starts_with("spacetime-modules/") && path.contains("/src/"));
            let permitted_source_writer = action.proposed_by == "tool:code_patch"
                || action.proposed_by == "operator-passthrough";
            if touches_source && !permitted_source_writer {
                return decide(
                    http,
                    stdb_host,
                    hex_db,
                    action.id,
                    "reject",
                    &format!(
                        "hard deny: only code_patch tool or operator-passthrough may write source files; \
                         got proposed_by='{}' for path '{}'",
                        action.proposed_by, path
                    ),
                    "",
                )
                .await;
            }
        }
    }

    // ADR-2605082500: actions from the SOP path (proposed_by="tool:*") are
    // already verified by the SOP's own Phase 4 + the tool's input schema.
    // The twin would just be a redundant LLM-judges-LLM gate that the ADR
    // explicitly killed. Auto-approve and let the executor consume it.
    //
    // Moved BEFORE hard_deny (was after) so tool:code_patch can write
    // Cargo.toml when its own allowlist permits — hard_deny's
    // trunk_blocklist was the wrong abstraction for the post-source-guard
    // era (the source-guard above already gates non-tool drafter writes).
    //
    // operator-passthrough: drafter's literal-content shortcut (operator
    // explicitly named the bytes in the board ask, e.g. "containing only
    // one line: X"). Not persona generation → no hallucination risk →
    // skip the grounding gate and LLM judge. Persona attribution stays in
    // the commitment row for audit.
    if action.proposed_by.starts_with("tool:")
        || action.proposed_by == "operator-passthrough"
    {
        return decide(
            http,
            stdb_host,
            hex_db,
            action.id,
            "approve",
            "auto-approved: SOP-emitted action (already passed typed Phase 4 verifier)",
            "",
        )
        .await;
    }

    // PERSONA CONTENT-GROUNDING GATE — added 2026-05-13 after CISO's first
    // attempt at the fail-open-goal-judge ADR produced 2 KB of OWASP-DRP
    // arbitration platitudes citing zero repo facts and dated 2023-04-28. The
    // existing CONTENT-VS-ASK prompt to the LLM judge is prose-comparison and
    // best-effort; this is a deterministic structural gate.
    //
    // Drafter-emitted (non-tool:*) file_writes to docs/* MUST cite at least
    // one of: an ADR ID, a repo path under a known hex crate / docs dir, a
    // git short-SHA, or a `hex <verb>` CLI invocation. Pure prose with no
    // grounding markers is overwhelmingly diagnostic of an ungrounded
    // REASON phase. Verdict is `escalate` (not `reject`) — surface to the
    // operator who can decide whether to approve, redirect, or scrap.
    if action.kind == "file_write" {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&action.payload_json) {
            let path = v.get("path").and_then(|x| x.as_str()).unwrap_or("");
            let content = v.get("content").and_then(|x| x.as_str()).unwrap_or("");
            if path.starts_with("docs/") && !content_has_grounding(content) {
                return decide(
                    http,
                    stdb_host,
                    hex_db,
                    action.id,
                    "escalate",
                    &format!(
                        "content-grounding gate: persona '{}' produced {} bytes to '{}' \
                         with no repo paths, ADR IDs, commit SHAs, or hex verbs cited. \
                         Likely hallucinated — routing to operator for review.",
                        action.proposed_by,
                        content.len(),
                        path
                    ),
                    "no-grounding",
                )
                .await;
            }
        }
    }

    // Hard deny-list — applies to drafter / non-tool writes that fall through.
    if let Some(reason) = hard_deny(&action.kind, &action.payload_json) {
        return decide(
            http,
            stdb_host,
            hex_db,
            action.id,
            "reject",
            &format!("hard deny: {}", reason),
            "",
        )
        .await;
    }

    let payload_preview = if action.payload_json.len() > PAYLOAD_PREVIEW_BYTES {
        format!(
            "{}\n[truncated — {} bytes total]",
            &action.payload_json[..PAYLOAD_PREVIEW_BYTES],
            action.payload_json.len()
        )
    } else {
        action.payload_json.clone()
    };

    let system = format!(
        "You are CEO-TWIN, the operator's automated stand-in. Your job is to approve, reject, or escalate \
         a proposed_action that a persona wants to execute on behalf of the operator. \
         The operator is asleep. Decide as if you ARE the operator, applying THEIR documented standards.\n\n\
         === OPERATOR MEMORY (their standards manual — these are LOAD-BEARING preferences) ===\n{memory}\n\n\
         === DECISION RULES ===\n\
         APPROVE when: (1) action matches a documented pattern in operator memory; AND (2) the proposed \
         file path is in docs/specs/, docs/adrs/, docs/workplans/, docs/analysis/, src/, or \
         hex-nexus/assets/src/; AND (3) the CONTENT directly answers the originating CEO request below \
         (not just a generic doc that happens to match the path pattern); AND (4) under 50 KB; AND (5) \
         no destructive operations.\n\
         REJECT when: violates a hard rule from memory (writes SQLite hub.db, edits trunk Cargo.toml \
         during freeze, touches /etc, contains secrets/keys, mass-overwrites unrelated files, conflicts \
         with explicit operator preference) OR the content is OFF-TOPIC for the originating CEO request \
         (e.g. CEO asked about persona tools and the file is about generic enterprise CI/CD).\n\
         ESCALATE when: novel domain not covered by memory, content > 30 KB, security-sensitive, the \
         proposing persona has fabricated facts (claims a 'secure channel', invents roles), or you are \
         genuinely uncertain whether the operator would approve.\n\n\
         CONTENT-VS-ASK CHECK: read the CEO's ORIGINATING REQUEST and the action PAYLOAD content. Ask \
         yourself: would the operator look at this file and say 'yes, this is what I asked for'? If \
         the file is a generic doc that doesn't match the SPECIFIC question the CEO asked, REJECT \
         with rationale 'off-topic: <one-line gap>'. The persona drifting into vague enterprise \
         platitudes is worse than no file at all.\n\n\
         You MUST respond as JSON ONLY in this exact shape:\n\
         {{\"verdict\":\"approve|reject|escalate\",\"rationale\":\"<one-line reason citing the relevant \
         memory rule, pattern, or content-vs-ask gap>\",\"escalate_reason\":\"<empty unless verdict=escalate>\"}}\n\
         No prose, no markdown, no preamble. Just the JSON object."
    );

    let ask_block = if action.originating_ceo_ask.is_empty() {
        "(could not fetch originating CEO request — judge based on payload + path only)".to_string()
    } else {
        format!(">>> {}", action.originating_ceo_ask.trim())
    };

    let user_msg = format!(
        "Proposed action #{id}\nkind: {kind}\nproposed_by: {by}\nrelated_commitment_id: {cid}\n\n\
         === ORIGINATING CEO REQUEST ===\n{ask}\n\n\
         === PROPOSED PAYLOAD ===\n{payload}",
        id = action.id,
        kind = action.kind,
        by = action.proposed_by,
        cid = action.related_commitment_id,
        ask = ask_block,
        payload = payload_preview,
    );

    // Pin a small fast model so we don't pile onto the 32B path the default
    // router picks. Twin verdicts are short structured output — qwen3:4b is
    // plenty. Override with HEX_TWIN_MODEL.
    let twin_model = std::env::var("HEX_TWIN_MODEL").unwrap_or_else(|_| "qwen3:4b".to_string());
    let body = serde_json::json!({
        "model": twin_model,
        "messages": [{ "role": "user", "content": user_msg }],
        "system": system,
        "max_tokens": TWIN_MAX_TOKENS,
    });
    let resp = http
        .post(inference_url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("inference http: {}", e))?;
    let status = resp.status();
    let json: serde_json::Value =
        resp.json().await.map_err(|e| format!("inference json: {}", e))?;
    if !status.is_success() {
        return Err(format!("inference HTTP {}: {}", status, json));
    }
    let raw = json
        .get("content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let (verdict, rationale, escalate_reason) = parse_verdict(&raw)?;
    decide(
        http,
        stdb_host,
        hex_db,
        action.id,
        &verdict,
        &rationale,
        &escalate_reason,
    )
    .await
}

fn parse_verdict(raw: &str) -> Result<(String, String, String), String> {
    // Strip code-fence if model wrapped it.
    let s = raw
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    // First {...} block.
    let start = s.find('{').ok_or_else(|| format!("no JSON in: {}", raw))?;
    let end = s.rfind('}').ok_or_else(|| format!("no JSON close in: {}", raw))?;
    let json: serde_json::Value = serde_json::from_str(&s[start..=end])
        .map_err(|e| format!("json parse: {} :: {}", e, &s[start..=end]))?;
    let verdict = json
        .get("verdict")
        .and_then(|v| v.as_str())
        .ok_or("missing verdict")?
        .to_lowercase();
    let rationale = json
        .get("rationale")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let escalate = json
        .get("escalate_reason")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if !matches!(verdict.as_str(), "approve" | "reject" | "escalate") {
        return Err(format!("invalid verdict: {}", verdict));
    }
    Ok((verdict, rationale, escalate))
}

/// Deterministic structural grounding gate for persona-drafted content.
/// Returns true if `content` cites at least one concrete repo fact: an ADR
/// ID, a path under a known hex crate / docs dir, a git short-SHA, or a
/// `hex <verb>` CLI invocation. Used by the twin to surface drafts that
/// look like hallucinated prose instead of grounded engineering writeups.
fn content_has_grounding(content: &str) -> bool {
    // ADR ID — both hyphen-date `ADR-YYYY-MM-DD-HHMM` and timestamp
    // `ADR-YYMMDDHHMM` forms.
    static ADR_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let adr_re = ADR_RE.get_or_init(|| {
        regex::Regex::new(r"ADR-(?:\d{4}-\d{2}-\d{2}-\d{4}|\d{10})").unwrap()
    });
    if adr_re.is_match(content) {
        return true;
    }

    // Repo paths — any known crate prefix or docs subdirectory.
    const REPO_PREFIXES: &[&str] = &[
        "hex-nexus/", "hex-cli/", "hex-core/", "hex-agent/", "hex-parser/",
        "hex-desktop/", "hex-analyzer/", "spacetime-modules/",
        "docs/adrs/", "docs/specs/", "docs/workplans/", "docs/analysis/",
        "scripts/", ".hex/project.json", "CLAUDE.md",
    ];
    if REPO_PREFIXES.iter().any(|p| content.contains(p)) {
        return true;
    }

    // Git short-SHA — 7-40 lowercase hex chars in a word boundary, with at
    // least one digit AND one letter so we don't false-match repeated
    // alphabetic tokens. Anchor on non-hex chars to keep `commit abc1234` in
    // and `aaaaaaa` (style-guide example) out.
    static SHA_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let sha_re = SHA_RE.get_or_init(|| {
        regex::Regex::new(r"(?:^|[^0-9a-f])([0-9a-f]{7,40})(?:[^0-9a-f]|$)").unwrap()
    });
    for cap in sha_re.captures_iter(content) {
        let s = &cap[1];
        if s.chars().any(|c| c.is_ascii_digit()) && s.chars().any(|c| c.is_ascii_alphabetic()) {
            return true;
        }
    }

    // Recognised `hex <verb>` CLI invocations — cheap heuristic that the
    // doc engages the system rather than abstract platitudes.
    const HEX_VERBS: &[&str] = &[
        "hex nexus", "hex adr", "hex plan", "hex chat", "hex memory",
        "hex go", "hex hey", "hex pulse", "hex status", "hex doctor",
        "hex sched", "hex pool", "hex stdb", "hex inbox", "hex swarm",
        "hex task", "hex dev", "hex worktree", "hex analyze", "hex spec",
        "hex brain", "hex agent", "hex goal", "hex verify",
    ];
    if HEX_VERBS.iter().any(|v| content.contains(v)) {
        return true;
    }

    false
}

#[cfg(test)]
mod grounding_tests {
    use super::content_has_grounding;

    #[test]
    fn detects_adr_id_hyphenated() {
        assert!(content_has_grounding("see ADR-2605131500 for context"));
    }

    #[test]
    fn detects_adr_id_timestamp() {
        assert!(content_has_grounding("supersedes ADR-2605082500"));
    }

    #[test]
    fn detects_repo_path() {
        assert!(content_has_grounding("modifies hex-nexus/src/orchestration/twin_reviewer.rs"));
    }

    #[test]
    fn detects_short_sha() {
        assert!(content_has_grounding("regressed in commit 4bb427ad"));
    }

    #[test]
    fn detects_hex_verb() {
        assert!(content_has_grounding("operator runs `hex nexus start` to recover"));
    }

    #[test]
    fn rejects_pure_prose() {
        // The hallucinated CISO draft from 2026-05-13 contained no markers.
        let hallucinated = "This document describes a goal judge for an arbitration dispute. \
                            The implementation must adhere to open-source principles vetted by \
                            recognized organizations such as OWASP DRP. Implementation will be \
                            evaluated by a test suite provided by the CEO upon request.";
        assert!(!content_has_grounding(hallucinated));
    }

    #[test]
    fn rejects_alphabetic_token_as_sha() {
        // `feedfeed` is hex but all-letters — not a real commit SHA.
        assert!(!content_has_grounding("the token feedfeed has all hex characters"));
    }
}

fn hard_deny(kind: &str, payload: &str) -> Option<String> {
    if kind == "file_write" {
        let v: serde_json::Value = match serde_json::from_str(payload) {
            Ok(v) => v,
            Err(_) => return Some("payload not JSON".to_string()),
        };
        let path = v.get("path").and_then(|x| x.as_str()).unwrap_or("");
        if path.is_empty() {
            return Some("empty path".to_string());
        }
        if path.starts_with('/') {
            return Some("absolute path outside repo".to_string());
        }
        if path.contains("..") {
            return Some("path traversal".to_string());
        }
        // Trunk infrastructure files — these are the recurring hijacker
        // damage paths. Operator must touch them by hand.
        let trunk_blocklist = [
            "Cargo.toml",
            "Cargo.lock",
            "package.json",
            "package-lock.json",
            "rust-toolchain.toml",
            ".gitignore",
            "hex-nexus/Cargo.toml",
            "hex-cli/Cargo.toml",
            "hex-core/Cargo.toml",
        ];
        for bad in trunk_blocklist {
            if path == bad {
                return Some(format!("trunk infra file: {}", bad));
            }
        }
        // Outside known doc / asset / src directories.
        // Aligned with code_patch tool allowlist (hex-nexus/src/tools/code_patch.rs)
        // so the twin doesn't double-reject paths the typed tool already accepts.
        let allowed_prefixes = [
            "docs/",
            "src/",
            "tests/",
            "examples/",
            "scripts/",
            "hex-nexus/src/",
            "hex-cli/src/",
            "hex-core/src/",
            "hex-agent/src/",
            "hex-parser/src/",
            "hex-analyzer/src/",
            "hex-desktop/src/",
            "hex-nexus/assets/src/",
            "hex-cli/assets/",
            "spacetime-modules/",
        ];
        if !allowed_prefixes.iter().any(|p| path.starts_with(p)) {
            return Some(format!("path outside allowed prefixes: {}", path));
        }
    }
    None
}

async fn decide(
    http: &reqwest::Client,
    stdb_host: &str,
    hex_db: &str,
    id: u64,
    verdict: &str,
    rationale: &str,
    escalate_reason: &str,
) -> Result<(), String> {
    let url = format!(
        "{}/v1/database/{}/call/proposed_action_twin_decide",
        stdb_host, hex_db
    );
    let body = serde_json::json!([id, verdict, rationale, escalate_reason]);
    let resp = http
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("decide http: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!(
            "twin_decide HTTP {}: {}",
            resp.status(),
            resp.text().await.unwrap_or_default()
        ));
    }
    tracing::info!(action_id = id, verdict, rationale, "twin_reviewer: decided");
    Ok(())
}

/// ADR-2605121505 — twin gate for `adr_status_set` actions.
/// Pure payload-shape validation. The on-disk preconditions (file exists,
/// current status is Proposed, path under docs/adrs/) are enforced by the
/// executor at apply time so this gate can run without filesystem access.
async fn review_adr_status_set(
    http: &reqwest::Client,
    stdb_host: &str,
    hex_db: &str,
    action: &PendingAction,
) -> Result<(), String> {
    match validate_adr_status_payload(&action.payload_json) {
        Ok(rationale) => {
            decide(http, stdb_host, hex_db, action.id, "approve", &rationale, "").await
        }
        Err(reason) => {
            decide(
                http,
                stdb_host,
                hex_db,
                action.id,
                "reject",
                &format!("adr_status_set: {}", reason),
                "",
            )
            .await
        }
    }
}

fn validate_adr_status_payload(payload_json: &str) -> Result<String, String> {
    let v: serde_json::Value = serde_json::from_str(payload_json)
        .map_err(|e| format!("payload not JSON: {}", e))?;
    let adr_id = v
        .get("adr_id")
        .and_then(|x| x.as_str())
        .ok_or_else(|| "missing adr_id".to_string())?;
    let new_status = v
        .get("new_status")
        .and_then(|x| x.as_str())
        .ok_or_else(|| "missing new_status".to_string())?;
    let reason = v
        .get("reason")
        .and_then(|x| x.as_str())
        .ok_or_else(|| "missing reason".to_string())?;
    if !adr_id.starts_with("ADR-") || adr_id.len() < 8 || adr_id.len() > 80 {
        return Err(format!("malformed adr_id '{}'", adr_id));
    }
    if !matches!(new_status, "Accepted" | "Abandoned" | "Superseded") {
        return Err(format!(
            "new_status must be Accepted|Abandoned|Superseded, got '{}'",
            new_status
        ));
    }
    if reason.trim().is_empty() {
        return Err("reason must be non-empty".to_string());
    }
    if reason.len() > 500 {
        return Err(format!("reason too long ({} bytes, max 500)", reason.len()));
    }
    Ok(format!(
        "adr_status_set approved: {} → {} (reason: {} chars)",
        adr_id,
        new_status,
        reason.len()
    ))
}

#[cfg(test)]
mod adr_status_review_tests {
    use super::validate_adr_status_payload;

    #[test]
    fn approves_well_formed() {
        let p = serde_json::json!({
            "adr_id": "ADR-2605090100",
            "new_status": "Accepted",
            "reason": "unblocks CTO code_patch legacy-ADR recognition",
        });
        assert!(validate_adr_status_payload(&p.to_string()).is_ok());
    }

    #[test]
    fn rejects_bad_json() {
        assert!(validate_adr_status_payload("not json").is_err());
    }

    #[test]
    fn rejects_missing_fields() {
        let p = serde_json::json!({"adr_id":"ADR-x","new_status":"Accepted"});
        assert!(validate_adr_status_payload(&p.to_string()).is_err());
    }

    #[test]
    fn rejects_bad_status() {
        let p = serde_json::json!({
            "adr_id": "ADR-2605090100",
            "new_status": "Approved",
            "reason": "x",
        });
        assert!(validate_adr_status_payload(&p.to_string()).is_err());
    }

    #[test]
    fn rejects_empty_reason() {
        let p = serde_json::json!({
            "adr_id": "ADR-2605090100",
            "new_status": "Abandoned",
            "reason": "   ",
        });
        assert!(validate_adr_status_payload(&p.to_string()).is_err());
    }

    #[test]
    fn rejects_oversize_reason() {
        let p = serde_json::json!({
            "adr_id": "ADR-2605090100",
            "new_status": "Accepted",
            "reason": "x".repeat(501),
        });
        assert!(validate_adr_status_payload(&p.to_string()).is_err());
    }
}
