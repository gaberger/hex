//! Drafter — turn open commitments into proposed_action(file_write) rows.
//!
//! Polls STDB every 30 s for commitments whose `artifact_kind = verifiable_path`
//! and `status = open`, asks the proposing persona to actually produce
//! the content of the named artifact, and writes a proposed_action row
//! the digital twin can then review.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

const POLL_INTERVAL_SECS: u64 = 30;
// CPO cost-spec 2026-05-09 — halved from 4096 to 2048; truncation already handled below.
const DRAFT_MAX_TOKENS: u32 = 2048;
// CTO ADR-2026-05-08-2600 — halved from 50KB to 24KB; staying under upstream BSATN
// `len too long` panic threshold (websocket_building.rs:180:57). Watchdog
// recovers if the cap is breached, but this prevents the crash entirely.
const CONTENT_CAP_BYTES: usize = 24 * 1024;
/// After N INSUFFICIENT_CONTEXT or empty-draft results, write a stub
/// artifact so the commitment closes and the operator can triage.
/// Without this, commitments where the persona over-committed (e.g.
/// promised a standup spec when CEO just asked "what's your priority")
/// loop forever and starve the queue.
const STUB_AFTER_FAILURES: u32 = 2;

pub fn spawn(stdb_host: String, hex_db: String, port: u16, repo_root: PathBuf) {
    if std::env::var("HEX_DISABLE_DRAFTER").is_ok() {
        tracing::info!("drafter disabled via HEX_DISABLE_DRAFTER");
        return;
    }
    let failures = Arc::new(Mutex::new(HashMap::<u64, u32>::new()));
    let repo_root = Arc::new(repo_root);
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
            if let Err(e) = run_one(&http, &stdb_host, &hex_db, &inference_url, &failures, &repo_root).await {
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
    failures: &Arc<Mutex<HashMap<u64, u32>>>,
    repo_root: &PathBuf,
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
        match draft_one(http, stdb_host, hex_db, inference_url, &c).await {
            Ok(DraftOutcome::ProposedAction) => {
                failures.lock().await.remove(&c.id);
            }
            Ok(DraftOutcome::PersonaAbstained) => {
                let mut g = failures.lock().await;
                let n = g.entry(c.id).or_insert(0);
                *n += 1;
                let count = *n;
                drop(g);
                if count >= STUB_AFTER_FAILURES {
                    tracing::warn!(
                        commitment_id = c.id, role = %c.role, fails = count,
                        "drafter: circuit-breaker — writing stub artifact so commitment can close"
                    );
                    if let Err(e) = write_stub_artifact(http, stdb_host, hex_db, &c, repo_root).await {
                        tracing::warn!(commitment_id = c.id, error = %e, "drafter: stub write failed");
                    } else {
                        failures.lock().await.remove(&c.id);
                    }
                }
            }
            Err(e) => {
                tracing::warn!(commitment_id = c.id, error = %e, "drafter: draft_one failed");
                // Transient errors also count toward the stub threshold so
                // repeated inference failures don't leave the commitment
                // open indefinitely.
                let mut g = failures.lock().await;
                let n = g.entry(c.id).or_insert(0);
                *n += 1;
            }
        }
        return Ok(());
    }
    Ok(())
}

/// Outcome of attempting to draft a commitment's artifact.
enum DraftOutcome {
    /// A proposed_action(file_write) was queued for twin review.
    ProposedAction,
    /// Persona returned INSUFFICIENT_CONTEXT (or empty) — no action queued.
    PersonaAbstained,
}

/// When a persona has refused N times to draft an artifact, write a stub
/// directly to disk and abandon the commitment. Bypasses twin review on
/// purpose — the stub is an operator-triage marker, not a persona-produced
/// artifact. Twin would (correctly) reject it as off-topic / fabrication.
async fn write_stub_artifact(
    http: &reqwest::Client,
    stdb_host: &str,
    hex_db: &str,
    c: &OpenCommitment,
    repo_root: &PathBuf,
) -> Result<(), String> {
    let ceo_ask = if c.thread_id.is_empty() {
        String::new()
    } else {
        fetch_originating_ask(http, &c.thread_id).await.unwrap_or_default()
    };

    let now = chrono::Utc::now().to_rfc3339();
    let stub = format!(
        "# {artifact} — STUB (operator triage required)\n\n\
         **Status:** stub — auto-generated after {n} drafter attempts\n\
         **Generated:** {now}\n\
         **Committed by:** `{role}`\n\
         **Commitment:** {action}\n\n\
         ## Why this is a stub\n\n\
         The persona `{role}` committed to producing this artifact, but on \
         {n} drafter attempts returned `INSUFFICIENT_CONTEXT` or an empty \
         draft. Most likely the persona over-committed during a \
         conversational reply — promising a structured artifact when the \
         CEO's original ask was open-ended.\n\n\
         ## Originating ask\n\n\
         ```\n{ask}\n```\n\n\
         ## What to do\n\n\
         One of:\n\n\
         1. **Fill it in by hand** — edit this file with the actual content \
            you want for `{artifact}`.\n\
         2. **Delete this stub** — the commitment is already marked abandoned \
            in STDB so nothing will retry.\n\
         3. **Re-ask with more context** — DM `@{role}` with a more specific \
            prompt and let the responder + drafter pipeline try again.\n\n\
         ---\n\n\
         *Stub written directly by the drafter circuit-breaker. Bypassed \
         twin review because stubs are an operator-triage signal, not a \
         persona artifact. Commitment_id `{cid}` was abandoned with the \
         abandon reason pointing here. See `hex-nexus/src/orchestration/drafter.rs`.*\n",
        artifact = c.success_artifact,
        n = STUB_AFTER_FAILURES,
        now = now,
        role = c.role,
        action = c.action,
        ask = if ceo_ask.is_empty() { "(no thread linkage — DM had no thread_id)".to_string() } else { ceo_ask.trim().to_string() },
        cid = c.id,
    );

    // Resolve target path safely against repo_root — refuse anything that
    // escapes the tree via .. or symlinks.
    let target = repo_root.join(&c.success_artifact);
    let canonical_root = repo_root
        .canonicalize()
        .map_err(|e| format!("canonicalise repo_root: {}", e))?;
    let parent = target.parent().ok_or("target has no parent")?;
    std::fs::create_dir_all(parent)
        .map_err(|e| format!("create parent dir {}: {}", parent.display(), e))?;
    let canonical_parent = parent
        .canonicalize()
        .map_err(|e| format!("canonicalise parent: {}", e))?;
    if !canonical_parent.starts_with(&canonical_root) {
        return Err(format!(
            "stub refused: {} resolves outside repo root",
            target.display()
        ));
    }

    // Atomic write via temp + rename.
    let tmp = target.with_extension("stubwrite-tmp");
    std::fs::write(&tmp, &stub).map_err(|e| format!("tmp write: {}", e))?;
    std::fs::rename(&tmp, &target).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("rename to target: {}", e)
    })?;

    // Mark the commitment abandoned in STDB with a clear evidence pointer.
    let abandon_reason = format!(
        "auto-stub after {} drafter attempts — see {} for operator triage",
        STUB_AFTER_FAILURES, c.success_artifact
    );
    let url = format!("{}/v1/database/{}/call/commitment_abandon", stdb_host, hex_db);
    let body = serde_json::json!([c.id, abandon_reason]);
    let resp = http
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("abandon http: {}", e))?;
    if !resp.status().is_success() {
        // The file is on disk regardless — don't lose that. Log but treat
        // success since the operator-visible side ran.
        tracing::warn!(
            commitment_id = c.id,
            status = %resp.status(),
            "drafter: commitment_abandon HTTP non-2xx (stub still on disk)"
        );
    }
    tracing::info!(
        commitment_id = c.id,
        role = %c.role,
        path = %c.success_artifact,
        "drafter: stub written directly + commitment abandoned (twin bypassed)"
    );
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
) -> Result<DraftOutcome, String> {
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

    // Pin a small fast model. Drafter writes file content; for the kinds
    // of artifacts personas commit to (specs, runbooks, short markdown)
    // qwen3:4b is fine. Override with HEX_DRAFTER_MODEL when a beefier
    // model is genuinely needed.
    let drafter_model = std::env::var("HEX_DRAFTER_MODEL").unwrap_or_else(|_| "qwen3:4b".to_string());
    let body = serde_json::json!({
        "model": drafter_model,
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
        // Treat as abstain so the circuit-breaker can promote to stub
        // after N attempts. Previously this errored, looping the commitment
        // forever without progress.
        tracing::info!(
            commitment_id = c.id, role = %c.role,
            "drafter: empty draft — treating as abstain"
        );
        return Ok(DraftOutcome::PersonaAbstained);
    }
    if content.trim_start().starts_with("INSUFFICIENT_CONTEXT") {
        tracing::info!(
            commitment_id = c.id,
            role = %c.role,
            "drafter: persona returned INSUFFICIENT_CONTEXT — abstain"
        );
        return Ok(DraftOutcome::PersonaAbstained);
    }
    if content.len() > CONTENT_CAP_BYTES {
        // CTO ADR-2026-05-08-2600 — surface truncation so operator can detect
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
    Ok(DraftOutcome::ProposedAction)
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
