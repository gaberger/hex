//! Executive responder background task.
//!
//! Polls `agent_messages` for unanswered DMs addressed to executive personas
//! (cto, cpo, coo, ciso, chief-visionary). For each unanswered DM, generates
//! a reply via the local `/api/inference/complete` endpoint using a
//! persona-flavoured system prompt and writes the reply back as a DM to the
//! original sender. Marks the source DM as read so it isn't re-processed.
//!
//! "Unanswered" is determined by `read_by NOT contains role` — the responder
//! always calls `mark_read(role, msg_id)` after replying, which doubles as
//! the processed-marker.
//!
//! Phase 2 (ADR follow-on): after every successful reply, fires a tokio task
//! that prompts the persona for a one-line reasoning summary and writes a
//! `kind=decision` row to chat-relay.agent_thought via SpacetimePersonaSupervisor.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;

use crate::adapters::spacetime_agent_comm::SpacetimeAgentCommAdapter;
use crate::adapters::spacetime_persona::SpacetimePersonaSupervisor;
use hex_core::ports::agent_comm::IAgentCommPort;

const POLL_INTERVAL_SECS: u64 = 4;
const MAX_RECENT_DMS: u32 = 25;
const REPLY_MAX_TOKENS: u32 = 512;
/// Token cap for the secondary "why this reply" prompt. Kept short — these
/// thoughts are journal entries, not analyses.
const THOUGHT_MAX_TOKENS: u32 = 96;

/// Roles the responder will reply on behalf of. Matches the personas under
/// `hex-cli/assets/agents/hex/hex/`. Add a role here to enable auto-replies.
const RESPONDER_ROLES: &[&str] = &[
    "cto",
    "cpo",
    "coo",
    "ciso",
    "chief-visionary",
];

/// Persona-flavoured system prompt for replies.
fn persona_prompt(role: &str) -> String {
    let role_title = match role {
        "cto" => "Chief Technology Officer",
        "cpo" => "Chief Product Officer",
        "coo" => "Chief Operating Officer",
        "ciso" => "Chief Information Security Officer",
        "chief-visionary" => "Chief Visionary",
        "engineering-lead" => "Engineering Lead",
        "product-lead" => "Product Lead",
        "sre-lead" => "SRE Lead",
        _ => "Executive",
    };
    format!(
        "You are the {role_title} ({role}) in a hexagonal AIOS organization. \
         The CEO is messaging you directly. Reply concisely (1-3 sentences) \
         from your role's perspective. Address the CEO as 'CEO'. Speak in \
         first person. Do not preface with 'as the {role}' — just answer."
    )
}

/// In-process dedup of (role, msg_id) pairs we've already replied to this
/// session. STDB `mark_read` is best-effort + eventually-consistent; the
/// poll interval (4s) is shorter than the inference round-trip in many
/// cases, so without this set the same DM gets answered 2–3 times before
/// `read_by` propagates back. Cleared per-tick to bound memory.
#[derive(Default)]
struct RepliedTracker {
    set: HashSet<(String, u64)>,
}

impl RepliedTracker {
    fn mark(&mut self, role: &str, msg_id: u64) {
        self.set.insert((role.to_string(), msg_id));
    }
    fn contains(&self, role: &str, msg_id: u64) -> bool {
        self.set.contains(&(role.to_string(), msg_id))
    }
    /// Cap memory. Keep the most-recent 4096 entries (~ 64 KB).
    fn maybe_trim(&mut self) {
        if self.set.len() > 4096 {
            self.set.clear();
        }
    }
}

pub fn spawn(
    comm: Arc<SpacetimeAgentCommAdapter>,
    persona: Arc<SpacetimePersonaSupervisor>,
    port: u16,
) {
    let replied = Arc::new(Mutex::new(RepliedTracker::default()));
    tokio::spawn(async move {
        let http = match reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "org_responder: failed to build http client; disabled");
                return;
            }
        };
        let inference_url = format!("http://127.0.0.1:{}/api/inference/complete", port);
        tracing::info!(url = %inference_url, "org_responder: started");

        // Give the HTTP server time to bind before we start hitting the
        // self-loopback inference endpoint.
        tokio::time::sleep(Duration::from_secs(8)).await;

        let mut ticker = tokio::time::interval(Duration::from_secs(POLL_INTERVAL_SECS));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            ticker.tick().await;
            for role in RESPONDER_ROLES {
                if let Err(e) = process_role(role, &comm, &persona, &http, &inference_url, &replied).await {
                    tracing::debug!(role = %role, error = %e, "org_responder: tick error");
                }
            }
            replied.lock().await.maybe_trim();
        }
    });
}

async fn process_role(
    role: &str,
    comm: &Arc<SpacetimeAgentCommAdapter>,
    persona: &Arc<SpacetimePersonaSupervisor>,
    http: &reqwest::Client,
    inference_url: &str,
    replied: &Arc<Mutex<RepliedTracker>>,
) -> Result<(), String> {
    let role_string = role.to_string();
    let messages = comm
        .query_messages(role_string.clone(), Some(MAX_RECENT_DMS))
        .await
        .map_err(|e| format!("query_messages: {}", e))?;

    for msg in &messages {
        if msg.to_agent.as_deref() != Some(role) {
            continue;
        }
        if msg.read_by.iter().any(|r| r == role) {
            continue;
        }
        let msg_id = match msg.id {
            Some(id) => id,
            None => continue,
        };

        // In-process dedup. STDB read_by takes a few seconds to propagate
        // so without this the next 4s tick will re-answer the same DM
        // (we caught the COO sending the same reply 3× to a single ping).
        // Reserve the slot BEFORE inference so concurrent ticks short-circuit.
        {
            let mut g = replied.lock().await;
            if g.contains(role, msg_id) {
                continue;
            }
            g.mark(role, msg_id);
        }

        // Circuit-breaker check (STDB persona_health). If banned, skip this
        // role entirely — the ban will lift on its own. Crucially DON'T
        // mark_read here, so when the ban lifts the message gets answered.
        if let Some(banned_until) = persona.is_banned(role).await {
            tracing::info!(
                role = %role, msg_id, banned_until = %banned_until,
                "org_responder: persona banned, skipping inference"
            );
            continue;
        }

        let from = msg.from_agent.clone();
        let content = msg.message.clone();
        let thread_id = msg.thread_id.clone();

        tracing::info!(role = %role, msg_id, from = %from, "org_responder: replying to unanswered DM");

        // Build conversation history with `from`. Without this the persona
        // sees only the current DM and can't resolve back-references like
        // "where is it" → which "it"?  Pull every prior message between
        // these two parties (already in `messages`), order ascending by id,
        // exclude the current message, take the last HISTORY_TURNS.
        let history = build_conversation_history(&messages, role, &from, msg_id);

        // Persona's recent thoughts — the audit trail of WHY it said what
        // it said in past replies. Injecting these as a system-prompt
        // appendix lets the persona reference its own prior reasoning.
        let thoughts = persona.recent_thoughts(role, 5).await;

        let reply = match generate_reply(http, inference_url, role, &content, &history, &thoughts).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(role = %role, msg_id, error = %e, "org_responder: inference failed; will retry");
                let (status_code, model_id) = parse_inference_error(&e);
                persona.record_failure(role, &model_id, status_code).await;
                continue;
            }
        };

        if reply.trim().is_empty() {
            tracing::debug!(role = %role, msg_id, "org_responder: empty inference reply; will retry");
            continue;
        }

        // Inference succeeded — clear any pending ban + failure counter.
        persona.record_success(role).await;

        let reply_for_send = reply.clone();
        let reply_for_journal = reply.clone();

        if let Err(e) = comm
            .send_dm(role_string.clone(), from.clone(), reply_for_send, thread_id)
            .await
        {
            tracing::warn!(role = %role, msg_id, to = %from, error = %e, "org_responder: send_dm failed; will retry");
            continue;
        }

        // Only mark as read after a successful reply was committed.
        if let Err(e) = comm.mark_read(role_string.clone(), msg_id).await {
            tracing::warn!(role = %role, msg_id, error = %e, "org_responder: mark_read after reply failed");
        }

        // Journal a thought (Phase 2) — best-effort, in a detached task so
        // it can't block the responder loop.
        let persona_for_journal = persona.clone();
        let role_for_journal = role.to_string();
        let content_for_journal = content.clone();
        let inference_url_for_journal = inference_url.to_string();
        let http_for_journal = http.clone();
        tokio::spawn(async move {
            match generate_thought_summary(
                &http_for_journal,
                &inference_url_for_journal,
                &role_for_journal,
                &content_for_journal,
                &reply_for_journal,
            )
            .await
            {
                Ok(summary) if !summary.trim().is_empty() => {
                    persona_for_journal
                        .journal_thought(
                            &role_for_journal,
                            "decision",
                            summary.trim(),
                            "",
                            msg_id,
                            0.0,
                        )
                        .await;
                }
                Ok(_) => {
                    tracing::debug!(role = %role_for_journal, msg_id, "thought summary empty; skipped");
                }
                Err(e) => {
                    tracing::debug!(role = %role_for_journal, msg_id, error = %e, "thought summary failed");
                }
            }
        });
    }

    Ok(())
}

async fn generate_reply(
    http: &reqwest::Client,
    inference_url: &str,
    role: &str,
    content: &str,
    history: &[ChatTurn],
    thoughts: &[(String, String)],
) -> Result<String, String> {
    // Assemble messages array: [history…, current user turn].
    // History entries already encode role=user|assistant for the persona's POV.
    let mut chat_messages: Vec<serde_json::Value> = history
        .iter()
        .map(|t| {
            serde_json::json!({
                "role": t.role,
                "content": truncate(&t.content, 400),
            })
        })
        .collect();
    chat_messages.push(serde_json::json!({
        "role": "user",
        "content": content,
    }));

    // Augment system prompt with the persona's recent thoughts so it
    // can reference its own prior reasoning ("why did I say X earlier").
    let mut system = persona_prompt(role);
    if !thoughts.is_empty() {
        system.push_str("\n\nYour recent reasoning (newest first):\n");
        for (kind, body) in thoughts.iter().take(5) {
            let line = truncate(body, 240);
            system.push_str(&format!("- [{}] {}\n", kind, line));
        }
    }

    // Inject real repo grounding (ADR list, dashboard URLs, anti-fabrication
    // rule) so the persona stops claiming it sent docs to a "secure channel".
    let grounding = crate::orchestration::repo_grounding::grounding_block(20);
    if !grounding.is_empty() {
        system.push_str(&grounding);
    }

    let body = serde_json::json!({
        "messages": chat_messages,
        "system": system,
        "max_tokens": REPLY_MAX_TOKENS,
    });

    let resp = http
        .post(inference_url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("http: {}", e))?;
    let status = resp.status();
    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("json: {}", e))?;
    if !status.is_success() {
        return Err(format!("inference HTTP {}: {}", status, json));
    }
    json.get("content")
        .and_then(|c| c.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| format!("missing content in response: {}", json))
}

/// One turn of conversation history. `role` is "user" (the counterparty)
/// or "assistant" (the persona itself).
struct ChatTurn {
    role: String,
    content: String,
}

const HISTORY_TURNS: usize = 10;

/// Build conversation history between `me` (the persona) and `peer`,
/// excluding the current inbound message so the responder doesn't echo
/// it as both context and current turn.
fn build_conversation_history(
    all_messages: &[hex_core::ports::agent_comm::AgentMessage],
    me: &str,
    peer: &str,
    current_msg_id: u64,
) -> Vec<ChatTurn> {
    let mut filtered: Vec<&hex_core::ports::agent_comm::AgentMessage> = all_messages
        .iter()
        .filter(|m| m.id.is_some() && m.id != Some(current_msg_id))
        .filter(|m| {
            // Either side of the conversation between `me` and `peer`.
            (m.from_agent == me && m.to_agent.as_deref() == Some(peer))
                || (m.from_agent == peer && m.to_agent.as_deref() == Some(me))
        })
        .collect();
    filtered.sort_by_key(|m| m.id.unwrap_or(0));
    let n = filtered.len();
    let start = n.saturating_sub(HISTORY_TURNS);
    filtered[start..]
        .iter()
        .map(|m| ChatTurn {
            role: if m.from_agent == me {
                "assistant".to_string()
            } else {
                "user".to_string()
            },
            content: m.message.clone(),
        })
        .collect()
}

/// Ask the persona for a 1-line reasoning summary. Returns the raw content;
/// the caller persists it as kind=decision. Failures here do NOT block the
/// reply pipeline.
async fn generate_thought_summary(
    http: &reqwest::Client,
    inference_url: &str,
    role: &str,
    inbound_content: &str,
    reply_content: &str,
) -> Result<String, String> {
    let body = serde_json::json!({
        "messages": [{
            "role": "user",
            "content": format!(
                "You ({role}) just received this message: \"{}\"\n\n\
                 You replied: \"{}\"\n\n\
                 In ONE sentence (max 30 words), summarize WHY you replied that way \
                 — the assumption you made or the next step you committed to. \
                 No preamble, just the reason.",
                truncate(inbound_content, 800),
                truncate(reply_content, 800),
            ),
        }],
        "system": format!(
            "You are the {role}. Reply with a single short sentence — your \
             internal reasoning, not user-facing prose. No quotes, no preamble."
        ),
        "max_tokens": THOUGHT_MAX_TOKENS,
    });

    let resp = http
        .post(inference_url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("http: {}", e))?;
    let status = resp.status();
    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("json: {}", e))?;
    if !status.is_success() {
        return Err(format!("HTTP {}: {}", status, json));
    }
    json.get("content")
        .and_then(|c| c.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| format!("missing content in response: {}", json))
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n {
        return s.to_string();
    }
    let mut end = n.min(s.len());
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

/// Pull HTTP status + model id out of generate_reply's error string.
/// `format!("inference HTTP {}: {}", status, json)`. Returns
/// (status, model_id) — caller passes these to persona_health so the
/// STDB-side detector can attribute the failure to a specific model.
fn parse_inference_error(err: &str) -> (u16, String) {
    let status: u16 = err
        .split_whitespace()
        .find_map(|tok| tok.trim_end_matches(':').parse::<u16>().ok())
        .filter(|s| (100..600).contains(s))
        .unwrap_or(0);
    let model_id = err
        .split('"')
        .find(|tok| {
            tok.starts_with("claude-")
                || tok.starts_with("anthropic/")
                || tok.starts_with("openai/")
                || tok.starts_with("google/")
        })
        .unwrap_or("")
        .to_string();
    (status, model_id)
}
