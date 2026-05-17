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
/// Maximum twin rejections per commitment before the drafter abandons.
/// Twin rejections mean the persona DID produce content but twin_reviewer
/// judged it unfit (path-not-allowed, content-grounding gate, etc.). Unlike
/// PersonaAbstained, the content exists — so retrying with the same persona
/// and same prompt is unlikely to improve. Observed 2026-05-17: commitment
/// 24578 retried 54 times against the same content-grounding rejection,
/// commitment 12293 retried 323 times — neither incremented the existing
/// failure counter because that only watched PersonaAbstained outcomes. ROI:
/// the 6 worst loopers in the 9-day log account for ~75% of all drafter
/// work, all of them >5 rejections. Capping at 5 truncates them with zero
/// loss of legitimate attempts.
const REJECT_BUDGET: u32 = 5;
/// Maximum bytes of the existing file we feed back into the patch prompt
/// before truncating. Keeps the prompt under the model's context budget
/// even when editing a 100 KB doc.
const PATCH_CONTEXT_CAP_BYTES: usize = 16 * 1024;
/// Minimum fraction of the existing file's significant lines that must
/// survive in the new draft for it to count as a real patch (vs. a
/// hallucinated rewrite). 0.40 catches full-file replacements while still
/// allowing substantial restructuring with the same anchors preserved.
const PATCH_MIN_PRESERVATION_RATIO: f32 = 0.40;

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
    let reject_counts = fetch_twin_reject_counts(http, stdb_host, hex_db).await.unwrap_or_default();

    for c in commitments {
        if existing.contains(&c.id) {
            continue; // drafter already ran for this commitment
        }
        // Back-pressure from twin_reviewer: if this commitment has already
        // accumulated >= REJECT_BUDGET twin rejections, abandon it instead of
        // re-drafting. Without this check the drafter loops forever — every
        // reject simply produces a fresh draft attempt with no learning.
        // Observed loops: commitment 12293 (323 retries), 12292 (256), 24578 (54).
        let prior_rejects = reject_counts.get(&c.id).copied().unwrap_or(0);
        if prior_rejects >= REJECT_BUDGET {
            tracing::warn!(
                commitment_id = c.id,
                role = %c.role,
                artifact = %c.success_artifact,
                rejects = prior_rejects,
                budget = REJECT_BUDGET,
                "drafter: twin-reject budget exhausted — abandoning commitment without re-draft"
            );
            let abandon_url = format!("{}/v1/database/{}/call/commitment_abandon", stdb_host, hex_db);
            let abandon_body = serde_json::json!([
                c.id,
                format!("drafter: twin_reviewer rejected {} drafts; budget {} exhausted — persona content unfit for this artifact, no retry will help", prior_rejects, REJECT_BUDGET),
            ]);
            if let Err(e) = http.post(&abandon_url).json(&abandon_body).send().await {
                tracing::warn!(commitment_id = c.id, error = %e, "drafter: abandon http failed");
            }
            failures.lock().await.remove(&c.id);
            return Ok(());
        }
        // Bound concurrency by handling one per tick; LLM calls are slow.
        match draft_one(http, stdb_host, hex_db, inference_url, &c, repo_root).await {
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
    // Stubs are markdown operator-triage notes. Writing one at a source-file
    // path clobbers real code with markdown and breaks the build. Refuse
    // outright — the abandon path below still marks the commitment failed,
    // so the persona loop doesn't spin. Observed 2026-05-17: a cto commitment
    // targeting hex-nexus/src/orchestration/drafter.rs landed a stub directly
    // over the source file after STUB_AFTER_FAILURES abstains.
    if is_source_file_path(&c.success_artifact) {
        tracing::warn!(
            commitment_id = c.id,
            role = %c.role,
            artifact = %c.success_artifact,
            "drafter: refusing to write stub at source-file path — would clobber real code; abandoning commitment without stub"
        );
        return Err(format!(
            "stub refused: source-file path '{}' cannot receive a markdown stub",
            c.success_artifact
        ));
    }

    let ceo_ask = if c.thread_id.is_empty() {
        String::new()
    } else {
        fetch_originating_ask(http, &c.thread_id).await.unwrap_or_default()
    };

    // Sanitize the artifact path before using it as a filename. If the
    // persona emitted unresolved template placeholders like `<auto-id>`,
    // writing them literally to disk produces filenames the operator (and
    // ls/grep/glob) can't sanely handle. Substitute the placeholder with
    // a timestamp suffix so the stub still lands at a triagable path.
    // Observed 2026-05-13: CTO committed to `ADR-<auto-id>-...md`; the
    // placeholder gate caught the file_write attempts but the
    // circuit-breaker still wrote the stub at the literal path.
    let sanitized_artifact = sanitize_artifact_path(&c.success_artifact);

    let now = chrono::Utc::now().to_rfc3339();
    let stub = format!(
        "# {artifact} — STUB (operator triage required)\n\n\
         **Status:** stub — auto-generated after {n} drafter attempts\n\
         **Generated:** {now}\n\
         **Committed by:** `{role}`\n\
         **Original committed path:** `{original}`\n\
         **Commitment:** {action}\n\n\
         ## Why this is a stub\n\n\
         The persona `{role}` committed to producing this artifact, but on \
         {n} drafter attempts the drafter could not produce a usable \
         draft. Causes include: persona returned `INSUFFICIENT_CONTEXT`, \
         persona returned an empty draft, content was too short for the \
         long-form artifact type (e.g. ADR / spec), or the artifact path \
         contained unresolved template placeholders like `<auto-id>` that \
         the persona forgot to substitute.\n\n\
         ## Originating ask\n\n\
         ```\n{ask}\n```\n\n\
         ## What to do\n\n\
         One of:\n\n\
         1. **Fill it in by hand** — edit this file with the actual content \
            you want for `{artifact}`.\n\
         2. **Delete this stub** — the commitment is already marked abandoned \
            in STDB so nothing will retry.\n\
         3. **Re-ask with more context** — DM `@{role}` with a more specific \
            prompt (and an explicit concrete artifact path/ID if the prior \
            failure was a placeholder) and let the responder + drafter \
            pipeline try again. Consider pinning `HEX_DRAFTER_MODEL_LONGFORM` \
            to a stronger model for ADR/spec asks.\n\n\
         ---\n\n\
         *Stub written directly by the drafter circuit-breaker. Bypassed \
         twin review because stubs are an operator-triage signal, not a \
         persona artifact. Commitment_id `{cid}` was abandoned with the \
         abandon reason pointing here. See `hex-nexus/src/orchestration/drafter.rs`.*\n",
        artifact = sanitized_artifact,
        original = c.success_artifact,
        n = STUB_AFTER_FAILURES,
        now = now,
        role = c.role,
        action = c.action,
        ask = if ceo_ask.is_empty() { "(no thread linkage — DM had no thread_id)".to_string() } else { ceo_ask.trim().to_string() },
        cid = c.id,
    );

    // Resolve target path safely against repo_root — refuse anything that
    // escapes the tree via .. or symlinks.
    let target = repo_root.join(&sanitized_artifact);
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
        STUB_AFTER_FAILURES, sanitized_artifact
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
        path = %sanitized_artifact,
        original_path = %c.success_artifact,
        "drafter: stub written directly + commitment abandoned (twin bypassed)"
    );
    Ok(())
}

/// Sanitizes an artifact path by substituting any unresolved `<token>`
/// placeholders with a UTC timestamp suffix, so a placeholder-bearing
/// commitment can still close to a triagable filename instead of writing
/// a literal `<auto-id>` to disk. Format: each `<token>` becomes
/// `placeholder-YYYYMMDDHHMMSS` so multiple placeholders in one path get
/// distinct-enough names within a single run. Idempotent for paths with
/// no placeholders.
fn sanitize_artifact_path(path: &str) -> String {
    if find_unresolved_placeholder(path).is_none() {
        return path.to_string();
    }
    let ts = chrono::Utc::now().format("%Y%m%d%H%M%S").to_string();
    let mut out = String::with_capacity(path.len());
    let mut chars = path.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if c != '<' {
            out.push(c);
            continue;
        }
        let rest = &path[i + 1..];
        let end = match rest.find('>') {
            Some(e) => e,
            None => {
                out.push(c);
                continue;
            }
        };
        let inner = &rest[..end];
        let alnum_only = !inner.is_empty()
            && !inner.contains('<')
            && !inner.contains(' ')
            && !inner.contains('/')
            && !inner.contains('\n')
            && inner
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-');
        if alnum_only {
            out.push_str(&format!("placeholder-{}", ts));
            // Skip ahead past the closing `>`.
            for _ in 0..(end + 1) {
                chars.next();
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[derive(Debug)]
struct OpenCommitment {
    id: u64,
    role: String,
    action: String,
    success_artifact: String,
    artifact_kind: String,
    thread_id: String,
    related_msg_id: u64,
}

async fn fetch_open_path_commitments(
    http: &reqwest::Client,
    stdb_host: &str,
    hex_db: &str,
) -> Result<Vec<OpenCommitment>, String> {
    let url = format!("{}/v1/database/{}/sql", stdb_host, hex_db);
    let body = "SELECT id, role, action, success_artifact, artifact_kind, status, thread_id, related_msg_id FROM commitment";
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
        if status != "open" {
            continue;
        }
        // ADR-2026-05-12-1505 — accept the new adr_status_flip kind as well as
        // legacy verifiable_path. Path-safety check only applies to file-write
        // kinds; status-flip payloads use ADR-<id>:<status> format that the
        // existing path validator would reject.
        let artifact_raw = cols.get(3).and_then(|x| x.as_str()).unwrap_or("");
        let (artifact, ok) = match kind {
            "verifiable_path" => (extract_path(artifact_raw), is_safe_repo_path(artifact_raw)),
            "adr_status_flip" => (artifact_raw.trim().to_string(), is_adr_status_flip_target(artifact_raw)),
            _ => continue,
        };
        if !ok {
            continue;
        }
        out.push(OpenCommitment {
            id: cols.first().and_then(|x| x.as_u64()).unwrap_or(0),
            role: cols.get(1).and_then(|x| x.as_str()).unwrap_or("").to_string(),
            action: cols.get(2).and_then(|x| x.as_str()).unwrap_or("").to_string(),
            success_artifact: artifact,
            artifact_kind: kind.to_string(),
            thread_id: cols.get(6).and_then(|x| x.as_str()).unwrap_or("").to_string(),
            related_msg_id: cols.get(7).and_then(|x| x.as_u64()).unwrap_or(0),
        });
    }
    Ok(out)
}

/// Count rejected proposed_actions per commitment. Used by the spawn loop
/// to enforce REJECT_BUDGET back-pressure: a commitment that has accumulated
/// too many twin rejections gets abandoned instead of looping.
async fn fetch_twin_reject_counts(
    http: &reqwest::Client,
    stdb_host: &str,
    hex_db: &str,
) -> Result<std::collections::HashMap<u64, u32>, String> {
    let url = format!("{}/v1/database/{}/sql", stdb_host, hex_db);
    let body = "SELECT related_commitment_id, status FROM proposed_action";
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
    let mut out: std::collections::HashMap<u64, u32> = std::collections::HashMap::new();
    for r in rows {
        let cols = match r.as_array() {
            Some(c) => c,
            None => continue,
        };
        let id = cols.first().and_then(|x| x.as_u64()).unwrap_or(0);
        let status = cols.get(1).and_then(|x| x.as_str()).unwrap_or("");
        if id > 0 && status == "rejected" {
            *out.entry(id).or_insert(0) += 1;
        }
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
    repo_root: &PathBuf,
) -> Result<DraftOutcome, String> {
    // ADR-2026-05-12-1505 — adr_status_flip bypasses the LLM entirely. The
    // persona's decision is already encoded in success_artifact (`ADR-X:Y`);
    // the drafter just assembles the typed payload + queues the action.
    if c.artifact_kind == "adr_status_flip" {
        return draft_adr_status_flip(http, stdb_host, hex_db, c).await;
    }

    // Reject unresolved template placeholders (e.g. `<turn>`, `<id>`,
    // `<persona>`) in the artifact path. Personas occasionally emit
    // commitment lines like "draft ADR-2026-05-12-<turn>-foo.md" expecting
    // the system to substitute — there is no substitution layer; literal
    // angle-bracket tokens leak straight to disk. Treat as an abstain so
    // the circuit-breaker can re-ask or promote to a stub, rather than
    // queueing a known-broken proposed_action that the twin and executor
    // then both have to special-case. Observed 2026-05-13 on CISO's first
    // attempt at the fail-open-goal-judge ADR.
    if let Some(token) = find_unresolved_placeholder(&c.success_artifact) {
        tracing::warn!(
            commitment_id = c.id,
            role = %c.role,
            artifact = %c.success_artifact,
            placeholder = %token,
            "drafter: rejecting commitment — artifact path contains unresolved template placeholder; abstaining"
        );
        return Ok(DraftOutcome::PersonaAbstained);
    }

    // Source-file guard: twin_reviewer hard-denies any file_write to
    // hex-*/src/ or spacetime-modules/*/src/ unless proposed_by is
    // `tool:code_patch` or `operator-passthrough` (twin_reviewer.rs:402-432).
    // The drafter only emits proposed_by=<persona-role>, so source-file
    // commitments here will ALWAYS reject — and the open-commitment poller
    // will retry indefinitely, burning inference budget on a loop the
    // persona literally cannot escape (observed 2026-05-17 on commitment
    // 24581 against hex-nexus/src/analysis/boundary_checker.rs). Abstain
    // immediately so the circuit-breaker can promote to stub or operator.
    if is_source_file_path(&c.success_artifact) {
        tracing::warn!(
            commitment_id = c.id,
            role = %c.role,
            artifact = %c.success_artifact,
            "drafter: rejecting commitment — source-file target requires SOP code_patch tool, not drafter LLM; abstaining"
        );
        return Ok(DraftOutcome::PersonaAbstained);
    }

    // Pull the originating CEO message — prefer thread, fall back to the
    // related_msg_id lookup so DM-style commitments (no thread_id) still
    // get the original ask passed through to the drafter.
    let ceo_ask = if !c.thread_id.is_empty() {
        fetch_originating_ask(http, &c.thread_id).await.unwrap_or_default()
    } else if c.related_msg_id > 0 {
        fetch_message_by_id(http, c.related_msg_id).await.unwrap_or_default()
    } else {
        String::new()
    };

    // SHORT-CIRCUIT: if the CEO ask contains an explicit literal-content
    // brief (e.g. "containing only one line: X" or "with content: X"),
    // write X directly and skip the LLM. Deterministic, no rambling.
    if !ceo_ask.is_empty() {
        if let Some(literal) = extract_literal_content(&ceo_ask) {
            tracing::info!(
                commitment_id = c.id,
                role = %c.role,
                bytes = literal.len(),
                "drafter: literal-content brief detected — bypassing LLM"
            );
            return queue_file_write_action(http, stdb_host, hex_db, c, &literal).await;
        }
    }

    let ceo_ask_block = if ceo_ask.is_empty() {
        String::new()
    } else {
        format!("\n\nOriginal CEO request (this is what the file must answer):\n>>> {}\n", ceo_ask.trim())
    };

    // Patch-mode context (2026-05-17 simplification — replaces the planned
    // twin-side patch-fidelity gate). If the target file already exists on
    // disk, the persona is supposed to be EDITING it, not regenerating from
    // scratch. Fetch the current bytes and bind them into the prompt so the
    // model has the actual content to preserve. Without this the drafter
    // hallucinates a new doc every time and the twin escalates everything
    // as ungrounded. Cap at PATCH_CONTEXT_CAP_BYTES so a 50 KB ADR doesn't
    // blow the prompt budget.
    let target_existing: Option<String> = {
        let p = repo_root.join(&c.success_artifact);
        if p.is_file() {
            match std::fs::read_to_string(&p) {
                Ok(s) if !s.trim().is_empty() => Some(s),
                _ => None,
            }
        } else {
            None
        }
    };
    let existing_block = match target_existing.as_ref() {
        Some(s) => {
            let body = if s.len() > PATCH_CONTEXT_CAP_BYTES {
                format!("{}\n[truncated — {} bytes total]", &s[..PATCH_CONTEXT_CAP_BYTES], s.len())
            } else {
                s.clone()
            };
            format!(
                "\n\nEXISTING FILE CONTENT — this file already exists. You are EDITING it.\n\
                 Preserve every line verbatim EXCEPT for the specific change the CEO asked for.\n\
                 Do NOT rewrite the document, do NOT change unrelated sections, do NOT alter framing.\n\
                 Output the FULL updated file body with the targeted change applied.\n\
                 ---BEGIN EXISTING---\n{}\n---END EXISTING---\n",
                body
            )
        }
        None => String::new(),
    };

    let system = format!(
        "You are the {role} persona. The CEO asked you for a specific artifact and you committed to producing it.\n\
         Your committed action: {action}\n\
         Required success artifact: {artifact}{ceo_ask}{existing}\n\n\
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
        existing = existing_block,
    );

    // Pin nemotron-mini by default. Reason same as the responder commit-mode
    // switch: qwen3:4b is a thinking model — it produces stream-of-
    // consciousness rambling instead of the actual file content (verified
    // in 02:17 test where 4601 bytes of "Wait, the user said…" shipped to
    // disk instead of the requested one-liner). nemotron-mini doesn't think,
    // doesn't ramble, follows the "produce file body now" instruction.
    //
    // BUT: nemotron-mini is too small for long-form artifacts (ADRs, specs).
    // Observed 2026-05-13: CTO produced a 53-byte stub for an ADR ask —
    // grounded gate caught it, but the right structural fix is to route
    // long-form artifacts through a stronger model when the operator opts in
    // via HEX_DRAFTER_MODEL_LONGFORM. Generic override is still HEX_DRAFTER_MODEL.
    let drafter_model = pick_drafter_model(&c.success_artifact);
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
    // Long-form artifact stub-detection gate. ADRs / specs / analysis docs
    // all need substantive content; a persona that returns 50 bytes for an
    // ADR is producing a stub, not an artifact. Treat as abstain so the
    // circuit-breaker can re-ask (ideally with a stronger drafter model
    // pinned via HEX_DRAFTER_MODEL_LONGFORM). Observed 2026-05-13: CTO
    // wrote `# soul personas alongside c-suite for adr-2026-05-13` and
    // nothing else for an ADR ask that included 5+ KB of grounding context.
    if let Some(min_bytes) = min_content_bytes_for_path(&c.success_artifact) {
        let actual = content.trim().len();
        if actual < min_bytes {
            tracing::warn!(
                commitment_id = c.id,
                role = %c.role,
                path = %c.success_artifact,
                actual,
                min = min_bytes,
                model = %drafter_model,
                "drafter: stub-detection gate — content too short for long-form artifact; abstaining (consider HEX_DRAFTER_MODEL_LONGFORM)"
            );
            return Ok(DraftOutcome::PersonaAbstained);
        }
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

    // Patch-fidelity check (2026-05-17 — replaces the planned twin-side
    // gate). If the target file existed before this draft, the persona is
    // editing — preservation is mandatory. Compute line-set overlap on
    // significant lines (trimmed, ≥20 chars) between existing and new
    // content. If <40% of existing significant lines survive, the drafter
    // rewrote the doc instead of patching — abstain so the circuit-breaker
    // re-asks (and the off-disk artifact does not get clobbered with
    // hallucinated content as it did on 2026-05-17 with action 45090).
    if let Some(existing) = target_existing.as_ref() {
        let preserved_ratio = significant_line_overlap_ratio(existing, &content);
        if preserved_ratio < PATCH_MIN_PRESERVATION_RATIO {
            tracing::warn!(
                commitment_id = c.id,
                role = %c.role,
                path = %c.success_artifact,
                preserved_ratio = preserved_ratio,
                threshold = PATCH_MIN_PRESERVATION_RATIO,
                existing_bytes = existing.len(),
                new_bytes = content.len(),
                "drafter: patch-fidelity gate — preserved <40% of existing lines; abstaining (likely full-file rewrite)"
            );
            return Ok(DraftOutcome::PersonaAbstained);
        }
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

/// Extract a literal content brief from a CEO ask. Returns Some(content)
/// when the ask contains a pattern that names exact file contents — e.g.
///   "containing only one line: Hello from the pipeline"
///   "with content: foo bar baz"
///   "with body: ..."
/// Avoids the LLM ramble entirely when the operator has been specific.
fn extract_literal_content(ask: &str) -> Option<String> {
    // Triggers we recognise (case-insensitive prefix scan).
    const TRIGGERS: &[&str] = &[
        "containing only one line:",
        "containing only the line:",
        "containing only:",
        "containing the literal text:",
        "containing the text:",
        "with content:",
        "with body:",
        "exactly:",
        "the file should contain:",
        "file body:",
    ];
    let lower = ask.to_lowercase();
    for trig in TRIGGERS {
        if let Some(start) = lower.find(trig) {
            let after = &ask[start + trig.len()..];
            let after_trimmed = after.trim_start();

            // Multi-line fenced content: ```[lang]\n...\n```
            // Lets operator paste a whole file body in the board ask.
            // Cap raised to 32 KB to fit Rust source files; the executor
            // and STDB payload caps still apply downstream.
            if after_trimmed.starts_with("```") {
                let after_fence_open = &after_trimmed[3..];
                // Skip optional language tag + the newline that closes it.
                let body_start = after_fence_open
                    .find('\n')
                    .map(|i| i + 1)
                    .unwrap_or(0);
                let body = &after_fence_open[body_start..];
                if let Some(end) = body.find("\n```") {
                    // Well-formed fence — respect its content boundary
                    // even if oversized (don't fall through to single-line
                    // and accidentally capture the fence delimiter).
                    let content = body[..end].to_string();
                    if content.is_empty() || content.len() > 32 * 1024 {
                        return None;
                    }
                    return Some(content);
                }
                // Malformed fence (no close) — fall through to single-line
                // semantics rather than swallowing the trigger.
            }

            // Take until end-of-message or a clear terminator. Strip
            // surrounding whitespace and any wrapping quotes/backticks.
            let mut content = after.trim().to_string();
            // If wrapped in matching quotes/backticks, peel them.
            for delim in ['"', '\'', '`'] {
                if content.starts_with(delim) && content.ends_with(delim) && content.len() > 1 {
                    content = content[1..content.len() - 1].to_string();
                    break;
                }
            }
            // Stop at obvious sentence terminators when the brief looks
            // like a single-line request.
            if let Some(idx) = content.find('\n') {
                content.truncate(idx);
            }
            let content = content.trim().to_string();
            if content.is_empty() || content.len() > 8192 {
                return None;
            }
            return Some(content);
        }
    }
    None
}

/// Look up a single agent_messages row by id. Used by drafter when a
/// commitment lacks a thread_id but has a related_msg_id (DM mode).
async fn fetch_message_by_id(http: &reqwest::Client, msg_id: u64) -> Result<String, String> {
    let stdb_host_agent_comms = std::env::var("HEX_STDB_HOST")
        .unwrap_or_else(|_| "http://127.0.0.1:3033".to_string());
    let url = format!("{}/v1/database/agent-comms/sql", stdb_host_agent_comms);
    let body = format!("SELECT message FROM agent_messages WHERE id = {}", msg_id);
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
    let msg = v
        .as_array()
        .and_then(|a| a.first())
        .and_then(|t| t.get("rows"))
        .and_then(|r| r.as_array())
        .and_then(|rows| rows.first())
        .and_then(|r| r.as_array())
        .and_then(|cols| cols.first())
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    Ok(msg)
}

/// Queue a file_write proposed_action for a literal content payload.
/// Extracted so the LLM and short-circuit paths share the same submission code.
async fn queue_file_write_action(
    http: &reqwest::Client,
    stdb_host: &str,
    hex_db: &str,
    c: &OpenCommitment,
    content: &str,
) -> Result<DraftOutcome, String> {
    let payload = serde_json::json!({
        "path": c.success_artifact,
        "content": content,
    });
    let url = format!("{}/v1/database/{}/call/proposed_action_open", stdb_host, hex_db);
    // Literal-content briefs are operator's words transcribed verbatim by
    // the drafter — NOT persona LLM generation. The content-grounding
    // gate exists to catch persona hallucination; it doesn't apply when
    // the operator explicitly named the bytes. Tag the proposed_action
    // with proposed_by="operator-passthrough" so the twin auto-approves
    // (mirroring its tool:* fast path) instead of running the LLM judge
    // and the structural grounding gate. Persona attribution is preserved
    // in the commitment row (c.role) for audit.
    let body = serde_json::json!(["file_write", payload.to_string(), "operator-passthrough", c.id]);
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
        "drafter: queued literal-content proposed_action(file_write)"
    );
    Ok(DraftOutcome::ProposedAction)
}

/// Selects the drafter model based on the artifact path. Long-form
/// artifacts (ADRs, specs, analysis docs) get a model sized for
/// ≥800-byte coherent prose; short-form (confirm/abstain replies,
/// code patches) use the small format-follower.
///
/// Defaults match the 2026-05-13 re-bench (memory:
/// project_t2_5_bench_results) — qwen2.5-coder:14b ties 32B quality
/// at 2× the speed and is the current T2/T2.5 pin. nemotron-mini is
/// retained for short-form because it follows commit-format contracts
/// reliably under a small token budget.
///
/// Overrides: HEX_DRAFTER_MODEL_LONGFORM (long-form only) and
/// HEX_DRAFTER_MODEL (any path).
const DRAFTER_MODEL_LONGFORM_DEFAULT: &str = "qwen2.5-coder:14b";
const DRAFTER_MODEL_SHORTFORM_DEFAULT: &str = "nemotron-mini";

fn pick_drafter_model(path: &str) -> String {
    let is_longform = path.starts_with("docs/adrs/")
        || path.starts_with("docs/specs/")
        || path.starts_with("docs/analysis/");
    if is_longform {
        if let Ok(m) = std::env::var("HEX_DRAFTER_MODEL_LONGFORM") {
            if !m.trim().is_empty() {
                return m;
            }
        }
    }
    if let Ok(m) = std::env::var("HEX_DRAFTER_MODEL") {
        if !m.trim().is_empty() {
            return m;
        }
    }
    if is_longform {
        DRAFTER_MODEL_LONGFORM_DEFAULT.to_string()
    } else {
        DRAFTER_MODEL_SHORTFORM_DEFAULT.to_string()
    }
}

/// Returns the minimum-content-byte threshold for a given artifact path,
/// or None if no minimum applies. Long-form documents (ADRs, specs,
/// analysis) MUST be substantive; a 50-byte "ADR" is always a stub.
fn min_content_bytes_for_path(path: &str) -> Option<usize> {
    if path.starts_with("docs/adrs/") {
        Some(1000)
    } else if path.starts_with("docs/specs/") {
        Some(800)
    } else if path.starts_with("docs/analysis/") {
        Some(800)
    } else {
        None
    }
}

/// Returns the first unresolved `<token>` placeholder in the path, if any.
/// Examples: `<turn>`, `<id>`, `<persona>`. Used by draft_one to refuse
/// committing a proposed_action whose path contains literal template
/// markers a persona forgot to substitute.
/// Mirror of twin_reviewer.rs:405-412 source-path detection. Kept in sync so
/// the drafter abstains on the same paths the twin would hard-deny.
fn is_source_file_path(path: &str) -> bool {
    path.starts_with("hex-nexus/src/")
        || path.starts_with("hex-cli/src/")
        || path.starts_with("hex-core/src/")
        || path.starts_with("hex-agent/src/")
        || path.starts_with("hex-parser/src/")
        || path.starts_with("hex-analyzer/src/")
        || path.starts_with("hex-desktop/src/")
        || (path.starts_with("spacetime-modules/") && path.contains("/src/"))
}

fn find_unresolved_placeholder(path: &str) -> Option<String> {
    let mut chars = path.char_indices();
    while let Some((i, c)) = chars.next() {
        if c != '<' {
            continue;
        }
        // Find matching '>' on the same line, no whitespace, no other '<'.
        let rest = &path[i + 1..];
        let end = rest.find('>')?;
        let inner = &rest[..end];
        if inner.is_empty()
            || inner.contains('<')
            || inner.contains(' ')
            || inner.contains('/')
            || inner.contains('\n')
        {
            continue;
        }
        // Heuristic: an alnum/underscore-only token between angle brackets
        // is almost certainly a template placeholder, not a legitimate path
        // component. Real filesystems don't put `<foo>` in paths.
        if inner.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-') {
            return Some(format!("<{}>", inner));
        }
    }
    None
}

#[cfg(test)]
mod literal_tests {
    use super::{extract_literal_content, find_unresolved_placeholder, min_content_bytes_for_path, pick_drafter_model};

    #[test]
    fn min_bytes_adr() {
        assert_eq!(min_content_bytes_for_path("docs/adrs/ADR-x.md"), Some(1000));
    }

    #[test]
    fn min_bytes_spec() {
        assert_eq!(min_content_bytes_for_path("docs/specs/foo.md"), Some(800));
    }

    #[test]
    fn min_bytes_analysis() {
        assert_eq!(min_content_bytes_for_path("docs/analysis/report.md"), Some(800));
    }

    #[test]
    fn min_bytes_workplan_none() {
        // Workplans are JSON and the structural validator handles short
        // ones — drafter doesn't gate on size for these.
        assert_eq!(min_content_bytes_for_path("docs/workplans/wp-x.json"), None);
    }

    #[test]
    fn min_bytes_source_code_none() {
        assert_eq!(min_content_bytes_for_path("hex-nexus/src/foo.rs"), None);
    }

    #[test]
    fn sanitize_no_placeholder_is_identity() {
        use super::sanitize_artifact_path;
        let p = "docs/adrs/ADR-2026-05-13-2300-foo.md";
        assert_eq!(sanitize_artifact_path(p), p);
    }

    #[test]
    fn sanitize_replaces_auto_id_placeholder() {
        use super::sanitize_artifact_path;
        let p = "docs/adrs/ADR-<auto-id>-user-defined-soul-personas.md";
        let s = sanitize_artifact_path(p);
        assert!(s.starts_with("docs/adrs/ADR-placeholder-"), "got {}", s);
        assert!(s.ends_with("-user-defined-soul-personas.md"), "got {}", s);
        assert!(!s.contains('<'), "got {}", s);
        assert!(!s.contains('>'), "got {}", s);
    }

    #[test]
    fn sanitize_skips_non_placeholder_brackets() {
        use super::sanitize_artifact_path;
        // Space + bang inside → not a placeholder, leave alone.
        let p = "docs/x <hello world!>.md";
        assert_eq!(sanitize_artifact_path(p), p);
    }

    #[test]
    fn pick_model_routing() {
        // Combined test: env vars are process-global and tests run in
        // parallel by default, so we sequence all env-var manipulation
        // inside one test rather than risking races between two.
        let prev_lf = std::env::var("HEX_DRAFTER_MODEL_LONGFORM").ok();
        let prev = std::env::var("HEX_DRAFTER_MODEL").ok();

        // Case 1: both env vars unset → defaults per pick_drafter_model
        // (commit 8e929b58 — longform paths default to qwen2.5-coder:14b
        // per the 2026-05-13 T2/T2.5 re-bench; short-form to nemotron-mini).
        std::env::remove_var("HEX_DRAFTER_MODEL_LONGFORM");
        std::env::remove_var("HEX_DRAFTER_MODEL");
        assert_eq!(pick_drafter_model("docs/notes.md"), "nemotron-mini");
        assert_eq!(pick_drafter_model("docs/adrs/foo.md"), "qwen2.5-coder:14b");
        assert_eq!(pick_drafter_model("docs/specs/foo.md"), "qwen2.5-coder:14b");

        // Case 2: HEX_DRAFTER_MODEL_LONGFORM override beats the longform default.
        std::env::set_var("HEX_DRAFTER_MODEL_LONGFORM", "test-strong-model");
        assert_eq!(pick_drafter_model("docs/adrs/foo.md"), "test-strong-model");
        assert_eq!(pick_drafter_model("docs/specs/foo.md"), "test-strong-model");
        assert_eq!(pick_drafter_model("docs/analysis/r.md"), "test-strong-model");
        assert_eq!(pick_drafter_model("hex-cli/src/foo.rs"), "nemotron-mini");

        // Case 3: HEX_DRAFTER_MODEL also set → non-longform paths use it,
        // longform still wins via LONGFORM.
        std::env::set_var("HEX_DRAFTER_MODEL", "test-default");
        assert_eq!(pick_drafter_model("hex-cli/src/foo.rs"), "test-default");
        assert_eq!(pick_drafter_model("docs/adrs/foo.md"), "test-strong-model");

        // Restore prior env so other tests in the binary aren't disturbed.
        match prev_lf {
            Some(v) => std::env::set_var("HEX_DRAFTER_MODEL_LONGFORM", v),
            None => std::env::remove_var("HEX_DRAFTER_MODEL_LONGFORM"),
        }
        match prev {
            Some(v) => std::env::set_var("HEX_DRAFTER_MODEL", v),
            None => std::env::remove_var("HEX_DRAFTER_MODEL"),
        }
    }
    #[test] fn one_line() {
        let r = extract_literal_content("@cto write docs/x.md containing only one line: Hello from the pipeline.");
        assert_eq!(r.as_deref(), Some("Hello from the pipeline."));
    }
    #[test] fn quoted() {
        let r = extract_literal_content("write file with content: \"ok done\"");
        assert_eq!(r.as_deref(), Some("ok done"));
    }
    #[test] fn no_trigger() {
        assert!(extract_literal_content("write a status doc").is_none());
    }

    #[test]
    fn multi_line_fence_with_lang() {
        let ask = "@cto write src/foo.rs with content:\n\
                   ```rust\n\
                   pub fn hello() -> &'static str { \"hi\" }\n\
                   ```";
        let r = extract_literal_content(ask).unwrap();
        assert!(r.contains("pub fn hello"));
        assert!(r.contains("\"hi\""));
        assert!(!r.contains("```"));
    }

    #[test]
    fn multi_line_fence_no_lang() {
        let ask = "with body:\n```\nline1\nline2\nline3\n```";
        let r = extract_literal_content(ask).unwrap();
        assert_eq!(r, "line1\nline2\nline3");
    }

    #[test]
    fn fence_without_close_falls_through_to_singleline() {
        // Malformed (no closing fence) → drop back to single-line capture.
        let ask = "with content: ```rust\npub fn x() {}";
        // Single-line capture picks up "```rust" until newline.
        let r = extract_literal_content(ask).unwrap();
        assert_eq!(r, "```rust");
    }

    #[test]
    fn fence_caps_at_32kb() {
        let huge = "a".repeat(33 * 1024);
        let ask = format!("with content:\n```\n{}\n```", huge);
        // Content exceeds 32 KB cap → fall through to single-line which
        // also bails on size; total result is None.
        assert!(extract_literal_content(&ask).is_none());
    }
    #[test] fn placeholder_turn() {
        assert_eq!(
            find_unresolved_placeholder("docs/adrs/ADR-2026-05-12-<turn>-fail-open-goal-judge.md").as_deref(),
            Some("<turn>")
        );
    }
    #[test] fn placeholder_id() {
        assert_eq!(
            find_unresolved_placeholder("docs/specs/<id>.md").as_deref(),
            Some("<id>")
        );
    }
    #[test] fn placeholder_none() {
        assert!(find_unresolved_placeholder("docs/adrs/ADR-2026-05-13-1500-x.md").is_none());
    }
    #[test] fn placeholder_real_brackets_skipped() {
        // Spaces, slashes, nested angle brackets → not a placeholder.
        assert!(find_unresolved_placeholder("hello <world!> foo.md").is_none());
        assert!(find_unresolved_placeholder("a < b > c").is_none());
    }
}

#[cfg(test)]
mod adr_status_tests {
    use super::{is_adr_status_flip_target, parse_adr_status_target};

    #[test]
    fn parses_accepted() {
        let (id, st) = parse_adr_status_target("ADR-2026-05-09-0100:Accepted").unwrap();
        assert_eq!(id, "ADR-2026-05-09-0100");
        assert_eq!(st, "Accepted");
    }

    #[test]
    fn parses_with_whitespace() {
        let (id, st) = parse_adr_status_target("  ADR-2026-05-09-0100 : Abandoned  ").unwrap();
        assert_eq!(id, "ADR-2026-05-09-0100");
        assert_eq!(st, "Abandoned");
    }

    #[test]
    fn rejects_missing_separator() {
        assert!(parse_adr_status_target("ADR-2026-05-09-0100").is_err());
    }

    #[test]
    fn rejects_unknown_status() {
        assert!(parse_adr_status_target("ADR-2026-05-09-0100:Approved").is_err());
        assert!(parse_adr_status_target("ADR-2026-05-09-0100:proposed").is_err());
    }

    #[test]
    fn rejects_bad_prefix() {
        assert!(parse_adr_status_target("X-123:Accepted").is_err());
    }

    #[test]
    fn validator_matches_parser() {
        assert!(is_adr_status_flip_target("ADR-2026-05-09-0100:Accepted"));
        assert!(!is_adr_status_flip_target("docs/specs/foo.md"));
        assert!(!is_adr_status_flip_target(""));
    }
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

/// ADR-2026-05-12-1505 — validate adr_status_flip targets at fetch time.
/// Format: `ADR-<id>:<NewStatus>` (e.g. `ADR-2026-05-09-0100:Accepted`).
/// Twin + executor re-validate — this is the cheap drafter-side filter.
fn is_adr_status_flip_target(s: &str) -> bool {
    parse_adr_status_target(s).is_ok()
}

fn parse_adr_status_target(s: &str) -> Result<(String, String), String> {
    let s = s.trim();
    let (left, right) = s
        .split_once(':')
        .ok_or_else(|| format!("missing ':' separator in '{}'", s))?;
    let adr_id = left.trim();
    let new_status = right.trim();
    if !adr_id.starts_with("ADR-") || adr_id.len() < 8 || adr_id.len() > 80 {
        return Err(format!("invalid adr_id prefix in '{}'", adr_id));
    }
    if !matches!(new_status, "Accepted" | "Abandoned" | "Superseded") {
        return Err(format!(
            "new_status must be one of Accepted|Abandoned|Superseded, got '{}'",
            new_status
        ));
    }
    Ok((adr_id.to_string(), new_status.to_string()))
}

/// ADR-2026-05-12-1505 — emit an `adr_status_set` proposed_action without
/// invoking the LLM. The persona's commitment already encoded the decision
/// (target ADR + new status); we just assemble the typed payload here.
async fn draft_adr_status_flip(
    http: &reqwest::Client,
    stdb_host: &str,
    hex_db: &str,
    c: &OpenCommitment,
) -> Result<DraftOutcome, String> {
    let (adr_id, new_status) = parse_adr_status_target(&c.success_artifact)
        .map_err(|e| format!("draft_adr_status_flip parse: {}", e))?;

    // Reason is pulled directly from the commitment.action field. The persona
    // wrote it when committing — twin uses it to verify rationale exists.
    let reason: String = c.action.chars().take(500).collect();
    if reason.trim().is_empty() {
        return Err("draft_adr_status_flip: commitment.action empty (no reason)".to_string());
    }

    let payload = serde_json::json!({
        "adr_id": adr_id,
        "new_status": new_status,
        "reason": reason,
        "commitment_id": c.id,
    });

    let url = format!("{}/v1/database/{}/call/proposed_action_open", stdb_host, hex_db);
    let body = serde_json::json!(["adr_status_set", payload.to_string(), c.role, c.id]);
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
        adr_id = %payload.get("adr_id").and_then(|v| v.as_str()).unwrap_or(""),
        new_status = %payload.get("new_status").and_then(|v| v.as_str()).unwrap_or(""),
        "drafter: queued adr_status_set proposed_action"
    );
    Ok(DraftOutcome::ProposedAction)
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

/// Fraction of `existing`'s significant lines that appear (verbatim, trimmed)
/// in `new_content`. A "significant" line is a trimmed line of at least 20
/// characters — short markup like `---` or blank lines are ignored. Used by
/// the patch-fidelity gate to detect drafts that rewrote the doc instead of
/// editing it. Returns 1.0 when `existing` has no significant lines (cannot
/// fail-open on an effectively empty target — falls through to other gates).
fn significant_line_overlap_ratio(existing: &str, new_content: &str) -> f32 {
    let new_lines: std::collections::HashSet<&str> = new_content
        .lines()
        .map(|l| l.trim())
        .filter(|l| l.len() >= 20)
        .collect();
    let mut total = 0u32;
    let mut preserved = 0u32;
    for line in existing.lines().map(|l| l.trim()).filter(|l| l.len() >= 20) {
        total += 1;
        if new_lines.contains(line) {
            preserved += 1;
        }
    }
    if total == 0 {
        return 1.0;
    }
    preserved as f32 / total as f32
}

#[cfg(test)]
mod patch_fidelity_tests {
    use super::significant_line_overlap_ratio;

    #[test]
    fn full_rewrite_with_no_overlap_returns_zero() {
        let existing = "## Status: Proposed\n\nThis ADR describes the IC-responder gap in hex-nexus/src/orchestration/org_responder.rs lines 80-85.\nThe daemon polls only executive personas, leaving 26 ICs silent.\n";
        let new = "## Incident Response Framework\n\nWe will use machine learning to detect incidents and train staff in response protocols.\nThe new system will track all events with full traceability.\n";
        let ratio = significant_line_overlap_ratio(existing, new);
        assert!(ratio < 0.25, "expected near-zero overlap, got {}", ratio);
    }

    #[test]
    fn real_patch_preserves_most_lines() {
        let existing = "# Title\n\nThis ADR describes the IC-responder gap in hex-nexus/src/orchestration/org_responder.rs lines 80-85.\nThe daemon polls only executive personas, leaving 26 ICs silent.\nWe propose a sister daemon to address the gap.\n";
        let new = "# Title\n\n## Status: Proposed\n\nThis ADR describes the IC-responder gap in hex-nexus/src/orchestration/org_responder.rs lines 80-85.\nThe daemon polls only executive personas, leaving 26 ICs silent.\nWe propose a sister daemon to address the gap.\n\nDrafted by: cto\n";
        let ratio = significant_line_overlap_ratio(existing, new);
        assert!(ratio >= 0.99, "expected full preservation, got {}", ratio);
    }

    #[test]
    fn short_lines_are_ignored() {
        let existing = "## A\n---\n   \n";
        let new = "different entirely";
        // No significant lines in existing → ratio is 1.0 (fall-open).
        assert_eq!(significant_line_overlap_ratio(existing, new), 1.0);
    }
}
