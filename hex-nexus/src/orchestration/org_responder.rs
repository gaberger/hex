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

use tokio::sync::{Mutex, Semaphore};

use crate::adapters::spacetime_agent_comm::SpacetimeAgentCommAdapter;
use crate::adapters::spacetime_persona::SpacetimePersonaSupervisor;
use crate::routes::chat::strip_think_block;
use hex_core::ports::agent_comm::IAgentCommPort;

const POLL_INTERVAL_SECS: u64 = 4;
const MAX_RECENT_DMS: u32 = 25;
const REPLY_MAX_TOKENS: u32 = 512;
/// Token cap for the secondary "why this reply" prompt. Kept short — these
/// thoughts are journal entries, not analyses.
const THOUGHT_MAX_TOKENS: u32 = 96;
/// Default models for the two responder reply modes.
///
/// chat-mode = conversational asks (status, what/how/why/…) — qwen3:4b is
/// fine, rambling prose is what the operator wants.
///
/// commit-mode = directive asks — needs strict `Confirm: …` / `Silent`
/// format. qwen3:4b improvises and rambles past the contract; nemotron-mini
/// (2.5 GB, NVIDIA, no think-mode) reliably emits the format in ~30 tokens.
///
/// Both overrideable: HEX_RESPONDER_CHAT_MODEL, HEX_RESPONDER_COMMIT_MODEL.
/// Legacy HEX_RESPONDER_MODEL still wins for both if set.
// Benchmark-driven defaults (scripts/bench-persona-prompts.py, v2 prompts).
//   chat-mode   → qwen3.5:9b      avg 0.94, best at grounded status answers
//   commit-mode → nemotron-mini   avg 1.00 on Confirm: format with few-shot
//                                 prompt, ~1s latency, ~25 output tokens
// Overrides: HEX_RESPONDER_CHAT_MODEL, HEX_RESPONDER_COMMIT_MODEL.
// Re-run the bench when new models drop.
const REPLY_MODEL_CHAT_DEFAULT: &str = "qwen3.5:9b";
const REPLY_MODEL_COMMIT_DEFAULT: &str = "nemotron-mini";
/// Model for the secondary "why this reply" prompt. Pinned to a small
/// non-thinking format-follower so summaries don't burn THOUGHT_MAX_TOKENS=96
/// on `<think>` reasoning. Override: HEX_THOUGHT_SUMMARIZER_MODEL.
const THOUGHT_SUMMARIZER_MODEL_DEFAULT: &str = "nemotron-mini";
/// Cap concurrent inference calls so we don't queue 9 simultaneous
/// requests at Ollama for a 4B model. Override with HEX_RESPONDER_CONCURRENCY.
const REPLY_CONCURRENCY_DEFAULT: usize = 3;

/// Roles the responder will reply on behalf of. Matches the personas under
/// `hex-cli/assets/agents/hex/hex/`. Add a role here to enable auto-replies.
///
/// Lead-tier roles (engineering-lead, product-lead, sre-lead) were added
/// 2026-05-11 so the dashboard "Orchestrator" chat panel — which defaults
/// to @engineering-lead — actually gets replies. Without this, DMs to lead
/// personas landed in STDB but nothing picked them up.
const RESPONDER_ROLES: &[&str] = &[
    "cto",
    "cpo",
    "coo",
    "ciso",
    "chief-visionary",
    "chief-architect",
    "engineering-lead",
    "product-lead",
    "sre-lead",
];

fn role_title(role: &str) -> &'static str {
    match role {
        "cto" => "Chief Technology Officer",
        "cpo" => "Chief Product Officer",
        "coo" => "Chief Operating Officer",
        "ciso" => "Chief Information Security Officer",
        "chief-visionary" => "Chief Visionary",
        "chief-architect" => "Chief Architect",
        "engineering-lead" => "Engineering Lead",
        "product-lead" => "Product Lead",
        "sre-lead" => "SRE Lead",
        "sre-engineer" => "SRE Engineer",
        _ => "Executive",
    }
}

/// Detect operator-style conversational asks (questions, status requests,
/// explain/summary). When matched, the responder uses a free-form prompt
/// and bypasses the strict Confirm/Silent parser. This makes the Mission
/// Control "Orchestrator" chat panel actually conversational instead of
/// dropping every reply as off-contract.
fn is_conversational(content: &str) -> bool {
    // Strip leading @mention tokens so we look at the body.
    let trimmed = content.trim_start();
    let body = trimmed
        .strip_prefix('@')
        .map(|s| s.trim_start_matches(|c: char| c.is_alphanumeric() || c == '-' || c == '_'))
        .unwrap_or(trimmed)
        .trim_start();
    if body.is_empty() {
        return false;
    }
    if body.ends_with('?') || body.starts_with('?') {
        return true;
    }
    let lower = body.to_ascii_lowercase();
    const Q_PREFIXES: &[&str] = &[
        "what ", "how ", "why ", "where ", "who ", "when ", "which ",
        "is ", "are ", "can ", "could ", "does ", "do ", "did ",
        "tell me", "explain", "give me", "show me", "summary",
        "status", "describe", "list ",
    ];
    Q_PREFIXES.iter().any(|p| lower.starts_with(p))
}

/// Free-form conversational prompt for operator-originated chat asks.
/// No Confirm/Silent contract — the persona answers directly.
fn conversational_prompt(role: &str) -> String {
    let title = role_title(role);
    format!(
        "You are the {title} ({role}) in a hexagonal AIOS organization.\n\n\
         The operator (CEO) is asking you a question via the Mission Control \
         chat panel. This is a CONVERSATION, not a directive — answer directly.\n\n\
         === OUTPUT GUIDELINES ===\n\n\
         - HARD CAP: 120 words. 4 sentences max. Bullet points OK.\n\
         - Ground every claim in real artifacts: file paths under docs/, src/, \
           tests/, hex-nexus/, hex-cli/, spacetime-modules/; ADR IDs in the form \
           ADR-YYYY-MM-DD-HHMM or ADR-NNN; commit SHAs; dashboard hashroutes \
           (e.g. #/merge-gate, #/missions, #/resources).\n\
         - If you don't know, say so plainly in ONE sentence. Suggest who would \
           (peer name) or where the answer lives (file path, log, dashboard).\n\
         - NO fabrication. NO claims about systems you haven't grounded against.\n\
         - Be brief. Be specific. Stop when you've answered.\n\
         - DO NOT use the `Confirm:` format — that's for directives, not chats.\n\n\
         === BANNED PHRASES (start of reply) ===\n\
         Do NOT begin with: 'We are', 'The user', 'Let me', 'I'll respond', \
         'I will respond', 'First,', 'Looking at', 'Key points', 'Note:'. \
         These are pre-answer narration. Skip straight to the answer.\n\n\
         === EXAMPLE (the only valid shape) ===\n\
         Shipped: docs/specs/cost-runbook.md. In flight: ADR-2026-05-08-2500 \
         pipeline integration. Concern: 3 active persona rollbacks per \
         /persona-health.\n\n\
         Begin your reply with the first character of your answer. Now."
    )
}

/// Persona-flavoured system prompt for replies.
///
/// Per ADR-2026-05-08-2400, personas are commitment-creators, not artifact-
/// producers. Output is restricted to ONE Confirm: line OR the literal
/// string "Silent". Anything else is dropped at the parser.
///
/// v2 (2026-05-12): added few-shot examples after benchmark
/// (scripts/bench-persona-prompts.py) showed they lift nemotron-mini's
/// commit-mode score from 0.67 → 1.00. The two examples cost ~80 tokens
/// of system prompt but eliminate the "I will respond with…" rambling
/// path entirely.
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
        "sre-engineer" => "SRE Engineer",
        _ => "Executive",
    };
    format!(
        "You are the {role_title} ({role}) in a hexagonal AIOS organization. \
         The CEO sent a message addressed to you or your role tier.\n\n\
         === STRICT OUTPUT CONTRACT (HARD — replies that don't match are silently dropped) ===\n\n\
         Emit EXACTLY ONE of the following two outputs and nothing else:\n\n\
         (A) `Confirm: I (<role>) will <one-line concrete action> by <deadline> — success: <artifact>`\n\n\
            Where <artifact> is one of:\n\
              - a repo file path under docs/, src/, tests/, examples/, scripts/, hex-nexus/assets/src/\n\
                (the digital-twin loop will draft it, review it, and write it for you)\n\
              - a dashboard hashroute (e.g. #/merge-gate, #/resources, #/commitments)\n\
              - the literal string `requires-operator-action — <one-line of what the operator must do>`\n\
                (when the action genuinely cannot be automated)\n\n\
            Pick ONE concrete deliverable. Keep the action under 25 words. The deadline can be `EOD`, \
            `EOW`, `tomorrow`, `in 2h`, etc.\n\n\
         (B) `Silent`\n\n\
            Choose this when:\n\
              - You have no specific role in this request (a peer is better-fit)\n\
              - The CEO's ask is too vague to produce a concrete Confirm\n\
              - You would otherwise be padding (\"I'll facilitate\", \"I'll ensure\", etc.)\n\n\
         === FORBIDDEN ===\n\
         - Free prose, multi-line replies, paragraphs, bullet lists, PLAN: blocks\n\
         - Acknowledgments (\"Got it CEO\", \"Understood\")\n\
         - Status updates (\"shortly\", \"immediately\", \"I'll get back to you\")\n\
         - Proposing peer coordination (\"@cpo and I will sync\") — that's not your tool\n\
         - Multiple Confirm lines — pick ONE\n\
         - Any output whose first non-whitespace character is not `C` (for Confirm) or `S` (for Silent)\n\n\
         You have NO tools beyond emitting this Confirm row. The factory pipeline (drafter→twin→\
         executor) will consume your Confirm and produce the artifact. If you write Silent, no \
         artifact is produced — that is correct when you genuinely have nothing concrete to add.\n\n\
         === EXAMPLES (these are the ONLY valid output shapes) ===\n\
         Confirm: I ({role}) will write docs/specs/cost-runbook.md by EOD — success: docs/specs/cost-runbook.md\n\
         Confirm: I ({role}) will draft ADR-2026-05-12-0900-pool-rebalance by EOW — success: docs/adrs/ADR-2026-05-12-0900-pool-rebalance.md\n\
         Confirm: I ({role}) will patch hex-cli/src/commands/plan.rs in 2h — success: hex-cli/src/commands/plan.rs\n\
         Silent\n\n\
         Begin your reply with the first character now. No preface."
    )
}

/// In-process dedup of (role, msg_id) pairs we've already replied to this
/// session. STDB `mark_read` is best-effort + eventually-consistent; the
/// poll interval (4s) is shorter than the inference round-trip in many
/// cases, so without this set the same DM gets answered 2–3 times before
/// `read_by` propagates back. Cleared per-tick to bound memory.
/// Also tracks per-(role, msg_id) failure counts so a single stuck inference
/// can't loop forever and starve new asks from the same role.
const MAX_FAILURES_BEFORE_DROP: u32 = 3;

#[derive(Default)]
struct RepliedTracker {
    set: HashSet<(String, u64)>,
    failures: std::collections::HashMap<(String, u64), u32>,
}

impl RepliedTracker {
    fn mark(&mut self, role: &str, msg_id: u64) {
        self.set.insert((role.to_string(), msg_id));
    }
    fn contains(&self, role: &str, msg_id: u64) -> bool {
        self.set.contains(&(role.to_string(), msg_id))
    }
    /// Increment failure count; returns true if the message has hit the
    /// circuit-breaker threshold and should be dropped from further attempts.
    fn record_failure(&mut self, role: &str, msg_id: u64) -> bool {
        let entry = self.failures.entry((role.to_string(), msg_id)).or_insert(0);
        *entry += 1;
        *entry >= MAX_FAILURES_BEFORE_DROP
    }
    fn clear_failures(&mut self, role: &str, msg_id: u64) {
        self.failures.remove(&(role.to_string(), msg_id));
    }
    fn failure_count(&self, role: &str, msg_id: u64) -> u32 {
        self.failures.get(&(role.to_string(), msg_id)).copied().unwrap_or(0)
    }
    /// Cap memory. Keep the most-recent 4096 entries (~ 64 KB).
    fn maybe_trim(&mut self) {
        if self.set.len() > 4096 {
            self.set.clear();
        }
        if self.failures.len() > 4096 {
            self.failures.clear();
        }
    }
}

pub fn spawn(
    comm: Arc<SpacetimeAgentCommAdapter>,
    persona: Arc<SpacetimePersonaSupervisor>,
    port: u16,
) {
    let replied = Arc::new(Mutex::new(RepliedTracker::default()));
    let concurrency = std::env::var("HEX_RESPONDER_CONCURRENCY")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|n| *n > 0 && *n <= 32)
        .unwrap_or(REPLY_CONCURRENCY_DEFAULT);
    // Legacy single-model override still wins to keep ops contracts intact.
    let legacy = std::env::var("HEX_RESPONDER_MODEL").ok();
    let model_chat = legacy.clone().unwrap_or_else(|| {
        std::env::var("HEX_RESPONDER_CHAT_MODEL")
            .unwrap_or_else(|_| REPLY_MODEL_CHAT_DEFAULT.to_string())
    });
    let model_commit = legacy.unwrap_or_else(|| {
        std::env::var("HEX_RESPONDER_COMMIT_MODEL")
            .unwrap_or_else(|_| REPLY_MODEL_COMMIT_DEFAULT.to_string())
    });
    let sem = Arc::new(Semaphore::new(concurrency));
    tokio::spawn(async move {
        tracing::info!(
            concurrency = concurrency,
            chat_model = %model_chat,
            commit_model = %model_commit,
            "org_responder: parallelism + per-mode pinned models"
        );
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
            // Fan out roles in parallel — previously sequential, which meant
            // one slow inference (up to the 120s http timeout) pinned every
            // downstream role for that whole tick. Now each role processes
            // its own message queue concurrently. Messages within a single
            // role are still serial to keep that persona's conversation
            // linear.
            let mut handles = Vec::with_capacity(RESPONDER_ROLES.len());
            for role in RESPONDER_ROLES {
                let comm = comm.clone();
                let persona = persona.clone();
                let http = http.clone();
                let inference_url = inference_url.clone();
                let replied = replied.clone();
                let sem = sem.clone();
                let model_chat = model_chat.clone();
                let model_commit = model_commit.clone();
                handles.push(tokio::spawn(async move {
                    if let Err(e) = process_role(role, &comm, &persona, &http, &inference_url, &replied, &sem, &model_chat, &model_commit).await {
                        tracing::debug!(role = %role, error = %e, "org_responder: tick error");
                    }
                }));
            }
            for h in handles {
                let _ = h.await;
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
    sem: &Arc<Semaphore>,
    model_chat: &str,
    model_commit: &str,
) -> Result<(), String> {
    let role_string = role.to_string();
    let mut messages = comm
        .query_messages(role_string.clone(), Some(MAX_RECENT_DMS))
        .await
        .map_err(|e| format!("query_messages: {}", e))?;

    // Process NEWEST messages first within a role. Previously the iteration
    // order was query-order, which (combined with our 120s-timeout retry
    // loop) meant a single stuck old message could starve every new ask
    // because the role kept burning its tick budget on the same old DM.
    // Sorting newest-first ensures the operator's most recent ask gets the
    // first inference slot every tick.
    messages.sort_by(|a, b| b.id.cmp(&a.id));

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

        // Circuit-breaker: if this message has failed MAX_FAILURES_BEFORE_DROP
        // times, mark it read and stop attempting. Prevents a single bad
        // message (oversize history, content-filter trip, etc.) from looping
        // forever and starving fresh asks.
        let fails = replied.lock().await.failure_count(role, msg_id);
        if fails >= MAX_FAILURES_BEFORE_DROP {
            tracing::warn!(
                role = %role, msg_id, fails,
                "org_responder: circuit-breaker — dropping after repeated failures"
            );
            if let Err(e) = comm.mark_read(role_string.clone(), msg_id).await {
                tracing::debug!(role = %role, error = %e, "mark_read after circuit-break failed");
            }
            replied.lock().await.clear_failures(role, msg_id);
            continue;
        }

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

        // ADR-2026-05-08-2400 atomic-claim: only the first persona to call
        // claim_persona_turn(thread_id) gets to emit a Confirm. Others
        // mark the message read and stay silent — no inference burned.
        // Bilateral DMs (no thread_id) skip the claim and proceed.
        if let Some(ref tid) = msg.thread_id {
            if !tid.is_empty() {
                let claimed = try_claim_thread(tid, role, msg_id).await;
                if !claimed {
                    tracing::info!(
                        role = %role, msg_id, thread = %tid,
                        "org_responder: thread already claimed by peer; staying silent"
                    );
                    if let Err(e) = comm.mark_read(role_string.clone(), msg_id).await {
                        tracing::debug!(role = %role, error = %e, "mark_read after silent claim failed");
                    }
                    continue;
                }
            }
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

        tracing::info!(
            role = %role, msg_id, from = %from, thread = ?thread_id,
            "org_responder: replying to unanswered DM"
        );

        // ADR-2026-05-08-2500: SOP-enabled personas route to the new typed
        // tool path instead of the free-prose Confirm/Silent contract.
        if crate::orchestration::sop_executor::is_sop_persona(role) {
            let repo_root = std::env::var("HEX_REPO_ROOT")
                .unwrap_or_else(|_| "/home/gary/hex-intf".to_string());
            let sop_result = crate::orchestration::sop_executor::run(role, &content, &repo_root).await;
            tracing::info!(
                role = %role, msg_id,
                intent = %sop_result.intent,
                emitted = ?sop_result.emitted_action_kind,
                trace = ?sop_result.phase_trace,
                "org_responder: SOP run complete"
            );
            // Send the structured chat card back to the operator.
            if let Err(e) = comm
                .send_dm(
                    role_string.clone(),
                    from.clone(),
                    sop_result.chat_card.clone(),
                    thread_id.clone(),
                )
                .await
            {
                tracing::warn!(role = %role, msg_id, error = %e, "org_responder: SOP send_dm failed");
            }
            if let Err(e) = comm.mark_read(role_string.clone(), msg_id).await {
                tracing::debug!(role = %role, msg_id, error = %e, "mark_read after SOP failed");
            }
            continue;
        }

        // History assembly:
        //  - Threaded DM (board meeting / multi-mention group): pull EVERY
        //    message in the thread (CEO + each peer's reply) so the CTO
        //    can see what the CPO said and respond to peers, not just to
        //    the CEO. Other-role replies are tagged 'user' with a "from:"
        //    prefix so the LLM can tell who said what.
        //  - Bilateral DM: just the prior turns between this role and the
        //    counterparty.
        let history = if let Some(ref tid) = thread_id {
            match build_thread_history(comm, tid, role, msg_id).await {
                Ok(h) => h,
                Err(e) => {
                    tracing::debug!(role = %role, error = %e, "org_responder: thread history fetch failed; falling back to bilateral");
                    build_conversation_history(&messages, role, &from, msg_id)
                }
            }
        } else {
            build_conversation_history(&messages, role, &from, msg_id)
        };

        // Persona's recent thoughts — the audit trail of WHY it said what
        // it said in past replies. Injecting these as a system-prompt
        // appendix lets the persona reference its own prior reasoning.
        let thoughts = persona.recent_thoughts(role, 5).await;

        let is_board = thread_id.as_deref().map(|t| t.starts_with("board-")).unwrap_or(false);
        // Operator-style asks (questions, "what/how/status/explain/…") use a
        // free-form conversational prompt and skip the strict Confirm/Silent
        // parser. Applies to board broadcasts too — a "status check" sent to
        // all execs is still informational, not a directive. Without this,
        // exec replies to status asks were dropped as off-contract.
        let chat_mode = is_conversational(&content);
        // Pick the right model for this mode. chat = rambling-tolerant; commit
        // = strict-format model so the Confirm:/Silent contract holds.
        let model = if chat_mode { model_chat } else { model_commit };
        // Acquire a concurrency slot before firing inference so we don't
        // queue 9 simultaneous calls at a small local model.
        let _permit = match sem.clone().acquire_owned().await {
            Ok(p) => p,
            Err(_) => continue, // semaphore closed — bail
        };
        let reply = match generate_reply(http, inference_url, role, &content, &history, &thoughts, is_board, chat_mode, model).await {
            Ok(r) => r,
            Err(e) => {
                let dropped = replied.lock().await.record_failure(role, msg_id);
                tracing::warn!(role = %role, msg_id, error = %e, dropped, "org_responder: inference failed; will retry");
                let (status_code, model_id) = parse_inference_error(&e);
                persona.record_failure(role, &model_id, status_code).await;
                continue;
            }
        };

        if reply.trim().is_empty() {
            tracing::debug!(role = %role, msg_id, "org_responder: empty inference reply; will retry");
            continue;
        }

        // ADR-2026-05-08-2400 strict output filter applies ONLY to commitment-
        // mode replies. Conversational replies (chat_mode=true) bypass the
        // parser and ship verbatim.
        let trimmed_reply = reply.trim();
        let first = trimmed_reply
            .lines()
            .next()
            .map(|l| l.trim_start())
            .unwrap_or("");
        let is_confirm = first.to_ascii_lowercase().starts_with("confirm:");
        let is_silent_token = trimmed_reply.eq_ignore_ascii_case("silent")
            || trimmed_reply.eq_ignore_ascii_case("silent.")
            || first.to_ascii_lowercase().starts_with("silent");
        if !chat_mode && !is_confirm && !is_silent_token {
            tracing::warn!(
                role = %role,
                msg_id,
                preview = %trimmed_reply.chars().take(80).collect::<String>(),
                "org_responder: off-contract reply dropped (not Confirm: or Silent)"
            );
            persona.record_success(role).await;
            if let Err(e) = comm.mark_read(role_string.clone(), msg_id).await {
                tracing::debug!(role = %role, error = %e, "mark_read after off-contract failed");
            }
            continue;
        }

        // Inference succeeded — clear any pending ban + failure counter
        // (both the persona-level supervisor counter and our per-message one).
        persona.record_success(role).await;
        replied.lock().await.clear_failures(role, msg_id);

        // ADR-2026-05-08-2400 Silent sentinel — applies to commitment-mode
        // DMs only. Conversational replies ship verbatim even if they happen
        // to start with the word "silent".
        if is_silent_token && !chat_mode {
            tracing::info!(role = %role, msg_id, "org_responder: persona chose Silent");
            if let Err(e) = comm.mark_read(role_string.clone(), msg_id).await {
                tracing::warn!(role = %role, msg_id, error = %e, "org_responder: mark_read after silent failed");
            }
            continue;
        }

        let reply_for_send = reply.clone();
        let reply_for_journal = reply.clone();
        let reply_for_commitment = reply.clone();
        let thread_id_for_commitment = thread_id.clone().unwrap_or_default();

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

        // Parse Confirm/PLAN lines and write commitments to STDB. Detached
        // — failures here MUST NOT block the responder loop.
        let role_for_commitment = role.to_string();
        tokio::spawn(async move {
            crate::orchestration::commitment_parser::extract_and_record(
                &role_for_commitment,
                &reply_for_commitment,
                &thread_id_for_commitment,
                msg_id,
            )
            .await;
        });

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
    is_board: bool,
    chat_mode: bool,
    model: &str,
) -> Result<String, String> {
    // Commit-mode keeps the message list TINY (just the current ask) so the
    // small format-following model isn't drowned in prior turns. Chat-mode
    // includes the recent history so the persona can answer in context.
    let history_slice: &[ChatTurn] = if chat_mode {
        history
    } else {
        let cap = history.len().min(2); // last 2 turns max in commit-mode
        &history[history.len() - cap..]
    };
    let truncate_chars = if chat_mode { 400 } else { 160 };
    let mut chat_messages: Vec<serde_json::Value> = history_slice
        .iter()
        .map(|t| {
            serde_json::json!({
                "role": t.role,
                "content": truncate(&t.content, truncate_chars),
            })
        })
        .collect();
    chat_messages.push(serde_json::json!({
        "role": "user",
        "content": content,
    }));

    // Augment system prompt with the persona's recent thoughts so it
    // can reference its own prior reasoning ("why did I say X earlier").
    let mut system = if chat_mode { conversational_prompt(role) } else { persona_prompt(role) };
    // ADR-2026-05-08-2400 retired the board-meeting PLAN/Amend/Silent protocol.
    // Atomic claim_persona_turn now ensures only one persona inferences per
    // thread; that single persona uses the strict Confirm/Silent prompt
    // from persona_prompt(). is_board is preserved for one minor hint only.
    if is_board {
        system.push_str(
            "\n\nNOTE: this thread is a board broadcast (CEO addressed all execs). \
             You won the atomic-claim race for this turn — your peers will stay \
             silent. Pick the action best suited to YOUR role's domain. If the \
             ask isn't really yours (better fit for another exec), reply Silent \
             and the operator will re-address it.",
        );
    }
    // For commit-mode (strict Confirm:/Silent) we DROP thoughts + grounding.
    // Small format-following models (nemotron-mini, etc.) drift when the
    // system prompt balloons past ~500 tokens. Standalone test with the same
    // ask but no extra context: 33 tokens, perfect Confirm. Full prompt:
    // 3631 input tokens → rambling "Here's the response required by…".
    // Chat-mode keeps the rich context so personas can reference history.
    if chat_mode {
        if !thoughts.is_empty() {
            system.push_str("\n\nYour recent reasoning (newest first):\n");
            for (kind, body) in thoughts.iter().take(5) {
                let line = truncate(body, 240);
                system.push_str(&format!("- [{}] {}\n", kind, line));
            }
        }
        let grounding = crate::orchestration::repo_grounding::grounding_block(20);
        if !grounding.is_empty() {
            system.push_str(&grounding);
        }
    }

    let body = serde_json::json!({
        "model": model,
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

/// Build conversation history from EVERY message in `thread_id`. Messages
/// authored by `me` are tagged "assistant"; everyone else is "user", and
/// for non-`from`-counterparty turns the speaker name is prefixed inline
/// ("[cpo→ceo]: ...") so the LLM can tell who said what when more than
/// two parties share the thread.
async fn build_thread_history(
    comm: &Arc<SpacetimeAgentCommAdapter>,
    thread_id: &str,
    me: &str,
    current_msg_id: u64,
) -> Result<Vec<ChatTurn>, String> {
    let mut msgs = comm
        .query_thread_messages(thread_id.to_string(), Some(80))
        .await
        .map_err(|e| format!("query_thread_messages: {}", e))?;
    msgs.retain(|m| m.id.is_some() && m.id != Some(current_msg_id));
    msgs.sort_by_key(|m| m.id.unwrap_or(0));

    let n = msgs.len();
    let start = n.saturating_sub(HISTORY_TURNS * 2); // multi-party threads need wider window

    Ok(msgs[start..]
        .iter()
        .map(|m| {
            if m.from_agent == me {
                ChatTurn {
                    role: "assistant".to_string(),
                    content: m.message.clone(),
                }
            } else {
                let speaker = &m.from_agent;
                let to_label = m
                    .to_agent
                    .as_deref()
                    .filter(|t| !t.is_empty())
                    .unwrap_or("all");
                ChatTurn {
                    role: "user".to_string(),
                    content: format!("[{}→{}]: {}", speaker, to_label, m.message),
                }
            }
        })
        .collect())
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
    let model = std::env::var("HEX_THOUGHT_SUMMARIZER_MODEL")
        .unwrap_or_else(|_| THOUGHT_SUMMARIZER_MODEL_DEFAULT.to_string());
    let body = serde_json::json!({
        "model": model,
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
    let raw = json
        .get("content")
        .and_then(|c| c.as_str())
        .ok_or_else(|| format!("missing content in response: {}", json))?;
    // Thinking models (qwen3, deepseek-r1, etc.) wrap reasoning in
    // <think>…</think>. The main reply path's inference gateway strips
    // them, but the thought summarizer was journaling raw reasoning —
    // and at THOUGHT_MAX_TOKENS=96 the whole budget is usually consumed
    // by <think>, leaving truncated mid-reasoning in the journal.
    let stripped = strip_think_block(raw);
    let cleaned = stripped.trim();
    // Contract says "ONE sentence (max 30 words)". Take the first
    // sentence and cap at 240 chars as belt-and-braces.
    let one_line = cleaned
        .split_terminator(['.', '\n'])
        .next()
        .unwrap_or(cleaned)
        .trim();
    Ok(one_line.chars().take(240).collect())
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

/// Atomic claim of a thread for a single persona (ADR-2026-05-08-2400).
/// Returns true if THIS role won the claim; false if a peer already
/// holds it (caller should stay silent + mark_read).
async fn try_claim_thread(thread_id: &str, role: &str, originating_msg_id: u64) -> bool {
    let host = std::env::var("HEX_SPACETIMEDB_HOST")
        .unwrap_or_else(|_| "http://127.0.0.1:3033".to_string());
    let db = std::env::var("HEX_STDB_DATABASE")
        .unwrap_or_else(|_| hex_core::stdb_database_for_module("hexflo-coordination").to_string());
    let url = format!("{}/v1/database/{}/call/claim_persona_turn", host, db);
    let body = serde_json::json!([thread_id, role, originating_msg_id]);
    let http = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
    {
        Ok(c) => c,
        Err(_) => return true, // fail-open: if STDB is down, let the persona reply
    };
    match http.post(&url).json(&body).send().await {
        Ok(resp) => resp.status().is_success(),
        Err(_) => true, // fail-open on transport
    }
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
