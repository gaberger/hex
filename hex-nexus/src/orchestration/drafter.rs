//! Drafter — turn open commitments into proposed_action(file_write) rows.
//!
//! Polls STDB every 30 s for commitments whose `artifact_kind = verifiable_path`
//! and `status = open`, asks the proposing persona to actually produce
//! the content of the named artifact, and writes a proposed_action row
//! the digital twin can then review.

use std::time::Duration;

const POLL_INTERVAL_SECS: u64 = 30;
// CPO cost-spec 2026-05-09 — halved from 4096 to 2048; truncation already handled below.
const DRAFT_MAX_TOKENS: u32 = 2048;
// CTO ADR-2605082600 — halved from 50KB to 24KB; staying under upstream BSATN
// `len too long` panic threshold (websocket_building.rs:180:57). Watchdog
// recovers if the cap is breached, but this prevents the crash entirely.
const CONTENT_CAP_BYTES: usize = 24 * 1024;

pub fn spawn(stdb_host: String, hex_db: String, port: u16) {
    if std::env::var("HEX_DISABLE_DRAFTER").is_ok() {
        tracing::info!("drafter disabled via HEX_DISABLE_DRAFTER");
        return;
    }
    tokio::spawn(async move {
        let http = match reqwest::Client::builder()
            .timeout(Duration::from_secs(180))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "drafter: http client build failed; disabled");
                return;
            }
        };
        let inference_url = format!("http://127.0.0.1:{}/api/inference/complete", port);
        tracing::info!(stdb_host = %stdb_host, db = %hex_db, "drafter: started");

        // Wait so STDB is up + the responder has had a chance to seed
        // some commitments before we poll.
        tokio::time::sleep(Duration::from_secs(30)).await;

        let mut ticker = tokio::time::interval(Duration::from_secs(POLL_INTERVAL_SECS));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            ticker.tick().await;
            if let Err(e) = run_one(&http, &stdb_host, &hex_db, &inference_url).await {
                tracing::debug!(error = %e, "drafter: tick error");
            }
        }
    });
}

async fn run_one(
    http: &reqwest::Client,
    stdb_host: &str,
    hex_db: &str,
    inference_url: &str,
) -> Result<(), String> {
    let commitments = fetch_open_path_commitments(http, stdb_host, hex_db).await?;
    if commitments.is_empty() {
        return Ok(());
    }
    let existing = fetch_pending_action_commitment_ids(http, stdb_host, hex_db).await?;

    for c in commitments {
        if existing.contains(&c.id) {
            continue; // drafter already ran for this commitment
        }
        // Bound concurrency by handling one per tick; LLM calls are slow.
        if let Err(e) = draft_one(http, stdb_host, hex_db, inference_url, &c).await {
            tracing::warn!(commitment_id = c.id, error = %e, "drafter: draft_one failed");
        }
        return Ok(());
    }
    Ok(())
}

#[derive(Debug)]
struct OpenCommitment {
    id: u64,
    role: String,
    action: String,
    success_artifact: String,
    thread_id: String,
}

async fn fetch_open_path_commitments(
    http: &reqwest::Client,
    stdb_host: &str,
    hex_db: &str,
) -> Result<Vec<OpenCommitment>, String> {
    let url = format!("{}/v1/database/{}/sql", stdb_host, hex_db);
    let body = "SELECT id, role, action, success_artifact, artifact_kind, status, thread_id FROM commitment";
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
        if cols.len() < 7 {
            continue;
        }
        let kind = cols.get(4).and_then(|x| x.as_str()).unwrap_or("");
        let status = cols.get(5).and_then(|x| x.as_str()).unwrap_or("");
        if kind != "verifiable_path" || status != "open" {
            continue;
        }
        let path = cols.get(3).and_then(|x| x.as_str()).unwrap_or("");
        if !is_safe_repo_path(path) {
            continue;
        }
        out.push(OpenCommitment {
            id: cols.first().and_then(|x| x.as_u64()).unwrap_or(0),
            role: cols.get(1).and_then(|x| x.as_str()).unwrap_or("").to_string(),
            action: cols.get(2).and_then(|x| x.as_str()).unwrap_or("").to_string(),
            success_artifact: extract_path(path),
            thread_id: cols.get(6).and_then(|x| x.as_str()).unwrap_or("").to_string(),
        });
    }
    Ok(out)
}

async fn fetch_pending_action_commitment_ids(
    http: &reqwest::Client,
    stdb_host: &str,
    hex_db: &str,
) -> Result<std::collections::HashSet<u64>, String> {
    let url = format!("{}/v1/database/{}/sql", stdb_host, hex_db);
    let body =
        "SELECT related_commitment_id, status FROM proposed_action";
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
    let mut out = std::collections::HashSet::new();
    for r in rows {
        let cols = match r.as_array() {
            Some(c) => c,
            None => continue,
        };
        let id = cols.first().and_then(|x| x.as_u64()).unwrap_or(0);
        let status = cols.get(1).and_then(|x| x.as_str()).unwrap_or("");
        if id > 0 && (status == "pending" || status == "approved" || status == "executed") {
            out.insert(id);
        }
    }
    Ok(out)
}

async fn draft_one(
    http: &reqwest::Client,
    stdb_host: &str,
    hex_db: &str,
    inference_url: &str,
    c: &OpenCommitment,
) -> Result<(), String> {
    // Pull the originating CEO message so the persona drafts the actual
    // requested artifact, not a generic doc that matches the path pattern.
    let ceo_ask = if c.thread_id.is_empty() {
        String::new()
    } else {
        fetch_originating_ask(http, &c.thread_id)
            .await
            .unwrap_or_default()
    };
    let ceo_ask_block = if ceo_ask.is_empty() {
        String::new()
    } else {
        format!("\n\nOriginal CEO request (this is what the file must answer):\n>>> {}\n", ceo_ask.trim())
    };

    let system = format!(
        "You are the {role} persona. The CEO asked you for a specific artifact and you committed to producing it.\n\
         Your committed action: {action}\n\
         Required success artifact: {artifact}{ceo_ask}\n\n\
         Produce the ACTUAL FULL CONTENTS of `{artifact}` NOW.\n\n\
         Rules:\n\
         - The file MUST directly answer the CEO request above. Do NOT drift to a generic 'enterprise tooling' \
           or off-topic document — match the SPECIFIC question the CEO asked.\n\
         - Output ONLY the file body — no preamble, no markdown code fence, no explanation about what you are doing.\n\
         - Aim for a one-pager (under 10 KB).\n\
         - Use Markdown if the path ends in .md, the appropriate language syntax otherwise.\n\
         - Reference real repo paths and concrete entities. Do not invent.\n\
         - If you genuinely cannot produce a useful draft (the CEO's request is ambiguous or requires \
           information you do not have), output ONLY the literal string `INSUFFICIENT_CONTEXT: <one-line reason>` \
           and nothing else.",
        role = c.role,
        action = c.action,
        artifact = c.success_artifact,
        ceo_ask = ceo_ask_block,
    );

    let body = serde_json::json!({
        "messages": [{
            "role": "user",
            "content": format!(
                "Write the contents of `{}` per your earlier commitment, answering the CEO request above.",
                c.success_artifact
            ),
        }],
        "system": system,
        "max_tokens": DRAFT_MAX_TOKENS,
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
    let mut content = json
        .get("content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if content.trim().is_empty() {
        return Err("empty draft".to_string());
    }
    if content.trim_start().starts_with("INSUFFICIENT_CONTEXT") {
        // Persona refused to invent. Don't write a proposed_action; the
        // commitment will go overdue and the operator sees the gap. This
        // is the correct failure mode (silent honest abstain).
        tracing::info!(
            commitment_id = c.id,
            role = %c.role,
            "drafter: persona returned INSUFFICIENT_CONTEXT — leaving commitment open"
        );
        return Ok(());
    }
    if content.len() > CONTENT_CAP_BYTES {
        // CTO ADR-2605082600 — surface truncation so operator can detect
        // patterns + coach personas to produce shorter drafts upfront.
        tracing::warn!(
            commitment_id = c.id,
            role = %c.role,
            original_len = content.len(),
            cap = CONTENT_CAP_BYTES,
            "drafter: content truncated — persona may need to produce a shorter draft"
        );
        content.truncate(CONTENT_CAP_BYTES);
        content.push_str("\n\n[truncated by drafter — CONTENT_CAP_BYTES]\n");
    }

    let payload = serde_json::json!({
        "path": c.success_artifact,
        "content": content,
    });
    let url = format!("{}/v1/database/{}/call/proposed_action_open", stdb_host, hex_db);
    let body = serde_json::json!([
        "file_write",
        payload.to_string(),
        c.role,
        c.id,
    ]);
    let resp = http
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("open http: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!(
            "proposed_action_open HTTP {}: {}",
            resp.status(),
            resp.text().await.unwrap_or_default()
        ));
    }
    tracing::info!(
        commitment_id = c.id,
        role = %c.role,
        path = %c.success_artifact,
        bytes = content.len(),
        "drafter: queued proposed_action(file_write)"
    );
    Ok(())
}

/// Fetch the FIRST message in a thread (typically CEO's broadcast / DM
/// that started the conversation). Used to give the drafter the original
/// ask so it doesn't drift into generic content matching only the path
/// pattern.
async fn fetch_originating_ask(
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
    // The first message in a thread is the originating ask. Pick the
    // smallest id (oldest) where from_agent == "ceo" (or fall back to
    // the smallest id regardless).
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
    Ok(from_ceo
        .or(any_oldest)
        .map(|(_, m)| m)
        .unwrap_or_default())
}

/// Extract a bare path from artifact text. Personas write things like
/// "located at `docs/specs/foo.md`" or "the file docs/specs/foo.md".
fn extract_path(s: &str) -> String {
    // Look for backtick-wrapped first.
    if let Some(start) = s.find('`') {
        if let Some(end) = s[start + 1..].find('`') {
            return s[start + 1..start + 1 + end].trim().to_string();
        }
    }
    // Otherwise scan tokens, pick the first that looks like a path.
    for tok in s.split(|c: char| c.is_ascii_whitespace() || c == ',' || c == '"') {
        let t = tok.trim_matches(|c: char| matches!(c, '.' | ':' | ';'));
        if (t.contains('/') || t.contains('\\'))
            && (t.ends_with(".md")
                || t.ends_with(".rs")
                || t.ends_with(".ts")
                || t.ends_with(".tsx")
                || t.ends_with(".json")
                || t.ends_with(".yml")
                || t.ends_with(".yaml")
                || t.ends_with(".toml"))
        {
            return t.to_string();
        }
    }
    s.to_string()
}

/// Defensive guard before we even propose. Twin + executor enforce
/// stricter rules; this just stops the drafter from generating drafts
/// for obviously-bad paths.
fn is_safe_repo_path(path_field: &str) -> bool {
    let p = extract_path(path_field);
    !p.starts_with('/')
        && !p.starts_with("..")
        && !p.is_empty()
        && p.len() < 256
}
