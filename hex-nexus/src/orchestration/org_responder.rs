//! Org responder background task.
//!
//! Polls `agent_messages` for unanswered DMs addressed to ANY persona in the
//! org chart (execs, leads, and ICs — see `Roster` / `roster()`). For each
//! unanswered DM, generates a reply via the local `/api/inference/complete`
//! endpoint using a persona-flavoured system prompt and writes the reply
//! back as a DM to the original sender. Marks the source DM as read so it
//! isn't re-processed.
//!
//! IC-responder widening (2026-05-18, ADR-2026-05-20-ic-responder-gap): the
//! original responder only polled 5 execs. Asks routed to ICs (hex-coder,
//! dashboard-ux-architect, validation-judge, etc.) registered as `online`
//! in persona_pool but nothing read their inbox — silent black hole. The
//! widened allowlist below covers every persona under
//! hex-cli/assets/agents/hex/hex/. ICs use the same strict Confirm/Silent
//! contract as execs; the factory pipeline (drafter→twin→executor) consumes
//! their Confirm rows and produces the actual artifacts.
//!
//! "Unanswered" is determined by `read_by NOT contains role` — the responder
//! always calls `mark_read(role, msg_id)` after replying, which doubles as
//! the processed-marker.
//!
//! Phase 2 (ADR follow-on): after every successful reply, fires a tokio task
//! that prompts the persona for a one-line reasoning summary and writes a
//! `kind=decision` row to chat-relay.agent_thought via SpacetimePersonaSupervisor.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::{Mutex, Semaphore};

use crate::adapters::spacetime_agent_comm::SpacetimeAgentCommAdapter;
use crate::adapters::spacetime_persona::SpacetimePersonaSupervisor;
use crate::orchestration::classifier_adapter::StrictJsonClassifierAdapter;
use crate::orchestration::classifier_parser::InvariantError;
use crate::orchestration::classifier_types::{ClassifierDecision, ClassifierResponse};
use crate::routes::chat::strip_think_block;
use hex_core::domain::messages::{ContentBlock, StopReason};
use hex_core::ports::agent_comm::IAgentCommPort;
use hex_core::ports::inference::{
    futures_stream, HealthStatus, IInferencePort, InferenceCapabilities, InferenceError,
    InferenceRequest, InferenceResponse, ModelInfo, ModelTier, StreamChunk,
};

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
// Benchmark-driven Ollama defaults (scripts/bench-persona-prompts.py, v2 prompts):
//   chat-mode   → qwen3.5:9b      avg 0.94, best at grounded status answers
//   commit-mode → nemotron-mini   avg 1.00 on Confirm: format with few-shot
//
// BUT — when the inference pipeline routes through OpenRouter (no local
// Ollama registered, or only ANTHROPIC_API_KEY/OPENROUTER_API_KEY set),
// those Ollama names fail with HTTP 400 "not a valid model ID". Defaults
// are now OpenRouter slugs that work in either path.
//
// gpt-4o-mini over claude-haiku-4.5 because Anthropic safety-filters
// persona prompts that mention "security audit", "OWASP", "exploit",
// etc. — common terms for the CTO/CISO roles. OpenAI's filter is much
// more permissive for legit ops/dev content. Switch back to anthropic
// via HEX_RESPONDER_CHAT_MODEL when filtering relaxes.
//
// Set HEX_RESPONDER_CHAT_MODEL=qwen3.5:9b when local Ollama is
// registered and the bench-pinned models are preferred.
const REPLY_MODEL_CHAT_DEFAULT: &str = "openai/gpt-4o-mini";
const REPLY_MODEL_COMMIT_DEFAULT: &str = "openai/gpt-4o-mini";
/// Model for the secondary "why this reply" prompt. Pinned to a small
/// non-thinking format-follower so summaries don't burn THOUGHT_MAX_TOKENS=96
/// on `<think>` reasoning. Override: HEX_THOUGHT_SUMMARIZER_MODEL.
const THOUGHT_SUMMARIZER_MODEL_DEFAULT: &str = "openai/gpt-4o-mini";
/// Cap concurrent inference calls so we don't queue 9 simultaneous
/// requests at Ollama for a 4B model. Override with HEX_RESPONDER_CONCURRENCY.
const REPLY_CONCURRENCY_DEFAULT: usize = 3;

/// The persona roster the responder will reply on behalf of. Sourced from
/// `hex-cli/assets/agents/hex/hex/*.yml` at startup — every YAML with a
/// `name:` field becomes a poll target, and its `role:` field becomes the
/// title used in system prompts. No hardcoded allowlist to drift.
///
/// History:
/// - 2026-05-11: lead-tier roles widened so dashboard's `@engineering-lead`
///   default got replies.
/// - 2026-05-18: 20 IC personas added (IC-responder-gap ADR).
/// - 2026-05-18 (this refactor): replaced the hand-maintained array with
///   `parse_agent_yamls()` — adding a new YAML now automatically enables
///   responder coverage.
#[derive(Debug, Default)]
struct Roster {
    /// Persona names, e.g. `["ceo", "cto", "hex-coder", ...]`.
    names: Vec<String>,
    /// Persona names as a HashSet for cheap @mention validation.
    name_set: HashSet<String>,
    /// `name -> role` map (role = the human-readable title from YAML).
    titles: HashMap<String, String>,
}

/// Tiny fallback for the case where YAMLs aren't on disk (e.g. running
/// hex-nexus from an unrelated cwd during dev). Just enough to keep the
/// exec/leads loop alive — IC asks won't be answered, but the operator
/// still has the c-suite. In production the YAML scan succeeds.
const FALLBACK_ROLES: &[&str] = &[
    "ceo", "cto", "cpo", "coo", "ciso", "chief-visionary", "chief-architect",
    "engineering-lead", "product-lead", "sre-lead", "validation-judge",
];

fn build_roster_from_yamls() -> Option<Roster> {
    let nodes = match crate::routes::org_chart::parse_agent_yamls() {
        Ok(n) if !n.is_empty() => n,
        Ok(_) => {
            tracing::warn!("org_responder: parse_agent_yamls returned empty set; using fallback");
            return None;
        }
        Err(e) => {
            tracing::warn!(error = %e, "org_responder: parse_agent_yamls failed; using fallback");
            return None;
        }
    };
    let mut names = Vec::with_capacity(nodes.len());
    let mut name_set = HashSet::with_capacity(nodes.len());
    let mut titles = HashMap::with_capacity(nodes.len());
    for n in &nodes {
        if n.name.is_empty() { continue; }
        let title = if n.role.is_empty() || n.role == "Unknown" {
            "Specialist".to_string()
        } else {
            n.role.clone()
        };
        names.push(n.name.clone());
        name_set.insert(n.name.clone());
        titles.insert(n.name.clone(), title);
    }
    names.sort();
    Some(Roster { names, name_set, titles })
}

fn build_fallback_roster() -> Roster {
    let mut names: Vec<String> = FALLBACK_ROLES.iter().map(|s| s.to_string()).collect();
    let name_set: HashSet<String> = FALLBACK_ROLES.iter().map(|s| s.to_string()).collect();
    let titles: HashMap<String, String> = FALLBACK_ROLES
        .iter()
        .map(|r| ((*r).to_string(), "Executive".to_string()))
        .collect();
    names.sort();
    Roster { names, name_set, titles }
}

fn roster() -> &'static Roster {
    static ROSTER: OnceLock<Roster> = OnceLock::new();
    ROSTER.get_or_init(|| {
        let r = build_roster_from_yamls().unwrap_or_else(build_fallback_roster);
        tracing::info!(
            count = r.names.len(),
            sample = ?r.names.iter().take(6).collect::<Vec<_>>(),
            "org_responder: roster loaded"
        );
        r
    })
}

fn role_title(role: &str) -> &'static str {
    // The Roster owns its titles; we hand back &'static str by leaking once
    // per unique role. The roster is bounded (≤ persona-YAML count) and the
    // OnceLock makes initialization happen exactly once, so this leaks at
    // most one short string per persona for the lifetime of the process.
    static TITLE_CACHE: OnceLock<HashMap<String, &'static str>> = OnceLock::new();
    let cache = TITLE_CACHE.get_or_init(|| {
        roster()
            .titles
            .iter()
            .map(|(k, v)| (k.clone(), &*Box::leak(v.clone().into_boxed_str())))
            .collect()
    });
    cache.get(role).copied().unwrap_or("Specialist")
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
/// Extract the first peer mention in a reply that isn't the speaker.
/// Two passes:
///   1. Explicit @<role> mentions (high precision, what the prompt asks for)
///   2. Plain role names preceded by coordination verbs ("ask the X",
///      "consult X", "contact the X", etc.) — catches the common case
///      where the model says "Ask the UX-designer" without the @ prefix.
/// Used by the inter-persona auto-CC.
fn first_peer_mention(reply: &str, speaker: &str) -> Option<String> {
    // Valid peers are exactly the persona roster (any persona with a YAML
    // is a valid CC target). Same source as the responder roster — no separate
    // allowlist to drift.
    let valid_peers: &HashSet<String> = &roster().name_set;
    // Pass 1: explicit @ mentions
    for cap in reply
        .split(|c: char| !c.is_alphanumeric() && c != '-' && c != '_' && c != '@')
        .filter(|s| s.starts_with('@'))
    {
        let name = cap.trim_start_matches('@').to_ascii_lowercase();
        if name == speaker { continue; }
        if valid_peers.contains(&name) {
            return Some(name);
        }
    }
    // Pass 2: ANY mention of a peer role name in the body. Lower
    // precision (a passing reference will CC too) but matches the
    // model's natural style — most replies say "the X handles Y"
    // rather than "ask the X". Operator gets a bit of cross-talk in
    // exchange for actual agent-to-agent collaboration.
    let lower = reply.to_ascii_lowercase();
    for peer in valid_peers.iter() {
        if peer == speaker { continue; }
        // Require word-boundary match so "cto" doesn't fire on "actor"
        let needles = [
            format!(" {} ", peer),
            format!(" {}.", peer),
            format!(" {},", peer),
            format!(" {}?", peer),
            format!(" {}\n", peer),
            format!("the {}", peer),
        ];
        for n in needles {
            if lower.contains(&n) {
                return Some(peer.to_string());
            }
        }
    }
    None
}

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
           by including @<role> in your reply — the runtime will CC them, so \
           they receive your message and weigh in. ONE peer per reply, max.\n\
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

/// Persona-flavoured system prompt for the structured-decision classifier
/// (ADR-2026-05-17-2030 Phase 1, workplan wp-sop-pipeline-redesign-phase-1 P4.1).
///
/// Replaces the prior Confirm:/Silent prose contract. The persona now emits
/// a single JSON object with a `decision` enum + per-decision required
/// fields; the StrictJsonClassifierAdapter (P3.1) parses, validates, and
/// reparses on malformed output. Off-contract responses no longer drop
/// silently — they escalate to the operator inbox via P5.
///
/// The from=operator invariant is documented here for the model but
/// authoritatively enforced by the parser (P1.2): operator-direct asks
/// may only resolve to `accept`, `route`, `clarify`, or `request_tool`.
fn persona_prompt(role: &str) -> String {
    // Single source of truth for role titles — keeps IC titles in sync with
    // the conversational path without duplicating the match arm.
    let role_title = role_title(role);
    format!(
        "You are the {role_title} ({role}) in a hexagonal AIOS organization. \
         You are acting as an inbox classifier for ONE inbound message.\n\n\
         === STRICT OUTPUT CONTRACT (HARD — malformed output is escalated, not dropped) ===\n\n\
         Respond with EXACTLY ONE JSON object and nothing else. No prose, no markdown, no code fences.\n\n\
         Top-level keys:\n\
           - `decision` (required): one of the snake_case strings below.\n\
           - `cost_usd` (required, number — you may use 0).\n\n\
         Per-decision required field (omit unused optional keys; do not include nulls):\n\
           - `accept`        — this persona will act NOW. Requires `tool_plan`: \
                              array of `{{ \"tool\": string, \"intent\": string }}`.\n\
           - `defer`         — busy/blocked. Requires `reason`. Forbidden on from=operator traffic.\n\
           - `route`         — forward to a peer. Requires `target_persona`: peer role name.\n\
           - `clarify`       — need more information. Requires `question`.\n\
           - `reject`        — refuse the ask. Requires `reason`. Forbidden on from=operator traffic.\n\
           - `request_tool`  — need a new tool. Requires `tool_spec`: JSON object \
                              with at minimum `name` + `rationale`.\n\n\
         === FROM=OPERATOR INVARIANT ===\n\
         When the user turn begins with `from=operator`, you MUST NOT pick `defer` or `reject`. \
         Operator-direct asks resolve to `accept`, `route`, `clarify`, or `request_tool` only. \
         If the ask is genuinely outside your domain, prefer `route` with a `target_persona`.\n\n\
         === FORBIDDEN ===\n\
         - Free prose, acknowledgments, status updates outside the JSON object\n\
         - Multiple JSON objects — pick ONE decision\n\
         - Confirm: / Silent prefixes (legacy contract — retired)\n\
         - Markdown fences, leading whitespace, trailing commentary\n\
         - `null` values — omit the key instead\n\n\
         You have NO tools beyond emitting this classifier object. The factory pipeline \
         (drafter→twin→executor) will consume the parsed `tool_plan` from an `accept`, the \
         `target_persona` from a `route`, the `question` from a `clarify`, etc., and produce \
         the actual artifact.\n\n\
         === EXAMPLES (these are the only valid output shapes) ===\n\
         {{\"decision\":\"accept\",\"tool_plan\":[{{\"tool\":\"code_patch\",\"intent\":\"patch hex-cli/src/commands/plan.rs\"}}],\"cost_usd\":0}}\n\
         {{\"decision\":\"route\",\"target_persona\":\"ciso\",\"cost_usd\":0}}\n\
         {{\"decision\":\"clarify\",\"question\":\"Which workplan should I target — wp-sop-phase-1 or wp-sop-phase-2?\",\"cost_usd\":0}}\n\
         {{\"decision\":\"request_tool\",\"tool_spec\":{{\"name\":\"grep_workplan\",\"rationale\":\"need wp dep lookups\"}},\"cost_usd\":0}}\n\n\
         Begin your reply with `{{` now."
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
    // Warm the YAML-driven roster eagerly so the count + sample names show
    // up in the log before the first tick rather than on lazy init.
    let _ = roster();
    tokio::spawn(async move {
        tracing::info!(
            concurrency = concurrency,
            chat_model = %model_chat,
            commit_model = %model_commit,
            roster_size = roster().names.len(),
            "org_responder: parallelism + per-mode pinned models + roster"
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
            let roles = &roster().names;
            let mut handles = Vec::with_capacity(roles.len());
            for role in roles {
                let role = role.clone();
                let comm = comm.clone();
                let persona = persona.clone();
                let http = http.clone();
                let inference_url = inference_url.clone();
                let replied = replied.clone();
                let sem = sem.clone();
                let model_chat = model_chat.clone();
                let model_commit = model_commit.clone();
                handles.push(tokio::spawn(async move {
                    if let Err(e) = process_role(&role, &comm, &persona, &http, &inference_url, &replied, &sem, &model_chat, &model_commit).await {
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
        // free-form conversational prompt and skip the structured classifier.
        // Applies to board broadcasts too — a "status check" sent to all execs
        // is still informational, not a directive.
        let chat_mode = is_conversational(&content);
        let from_operator = from == "operator";
        // Pick the right model per mode. chat = rambling-tolerant; commit =
        // strict JSON-output classifier.
        let model_for_mode = if chat_mode { model_chat } else { model_commit };
        // Acquire a concurrency slot before firing inference so we don't
        // queue 9 simultaneous calls at a small local model.
        let _permit = match sem.clone().acquire_owned().await {
            Ok(p) => p,
            Err(_) => continue, // semaphore closed — bail
        };

        if chat_mode {
            // -- Conversational path (chat_mode bypass). Unchanged contract:
            // free-form prose reply, send verbatim, CC any @peer mention. --
            let reply = match generate_reply(
                http,
                inference_url,
                role,
                &content,
                &history,
                &thoughts,
                is_board,
                true,
                model_for_mode,
            )
            .await
            {
                Ok(r) => r,
                Err(e) => {
                    let dropped = replied.lock().await.record_failure(role, msg_id);
                    tracing::warn!(role = %role, msg_id, error = %e, dropped, "org_responder: chat inference failed; will retry");
                    let (status_code, model_id) = parse_inference_error(&e);
                    persona.record_failure(role, &model_id, status_code).await;
                    continue;
                }
            };

            if reply.trim().is_empty() {
                tracing::debug!(role = %role, msg_id, "org_responder: empty chat reply; will retry");
                continue;
            }

            persona.record_success(role).await;
            replied.lock().await.clear_failures(role, msg_id);

            let reply_for_send = reply.clone();
            let reply_for_journal = reply.clone();
            let reply_for_commitment = reply.clone();
            let thread_id_for_commitment = thread_id.clone().unwrap_or_default();

            if let Err(e) = comm
                .send_dm(
                    role_string.clone(),
                    from.clone(),
                    reply_for_send.clone(),
                    thread_id.clone(),
                )
                .await
            {
                tracing::warn!(role = %role, msg_id, to = %from, error = %e, "org_responder: send_dm failed; will retry");
                continue;
            }
            if let Err(e) = comm.mark_read(role_string.clone(), msg_id).await {
                tracing::warn!(role = %role, msg_id, error = %e, "org_responder: mark_read after reply failed");
            }

            // Inter-persona @mention CC — kept for chat-mode only. The new
            // classifier path uses an explicit `route` decision instead.
            if from_operator {
                if let Some(peer) = first_peer_mention(&reply_for_send, role) {
                    let coord = format!(
                        "[CC from @{} — operator asked: \"{}\"]\n\n@{} you said:\n{}\n\nYour input is requested.",
                        role,
                        msg.message.chars().take(200).collect::<String>(),
                        role,
                        reply_for_send.chars().take(400).collect::<String>(),
                    );
                    let role_for_cc = role_string.clone();
                    let peer_clone = peer.clone();
                    if let Err(e) = comm
                        .send_dm(role_for_cc, peer.clone(), coord, thread_id.clone())
                        .await
                    {
                        tracing::warn!(role = %role, peer = %peer_clone, error = %e, "org_responder: inter-persona CC failed");
                    } else {
                        tracing::info!(role = %role, peer = %peer, "org_responder: CC'd peer based on @mention");
                    }
                }
            }

            spawn_commitment_and_journal(
                role,
                msg_id,
                content.clone(),
                reply_for_commitment,
                reply_for_journal,
                thread_id_for_commitment,
                persona.clone(),
                http.clone(),
                inference_url.to_string(),
            );
            continue;
        }

        // -- Structured-decision classifier path (ADR-2026-05-17-2030 P4.1).
        // Replaces the d2b3f06e Confirm/Silent prose parser. Off-contract
        // output no longer drops silently — exhausted reparse budget +
        // invariant violations escalate to the operator inbox via P5. --
        let system_prompt = persona_prompt(role);
        let shim: Arc<dyn IInferencePort> = Arc::new(HttpInferenceShim::new(
            http.clone(),
            inference_url.to_string(),
        ));
        let classifier = StrictJsonClassifierAdapter::new(shim, model_for_mode.to_string());
        let (resp, attempts) = match classifier
            .classify_with_attempts(&system_prompt, &content, from_operator)
            .await
        {
            Ok(t) => t,
            Err(invariant) => {
                let dropped = replied.lock().await.record_failure(role, msg_id);
                tracing::warn!(
                    role = %role,
                    msg_id,
                    error = %invariant,
                    dropped,
                    "org_responder: classifier failed; escalating to operator inbox"
                );
                escalate_classifier_failure(
                    &comm,
                    &role_string,
                    msg_id,
                    &from,
                    role,
                    &content,
                    thread_id.as_deref(),
                    &invariant,
                )
                .await;
                continue;
            }
        };

        persona.record_success(role).await;
        replied.lock().await.clear_failures(role, msg_id);

        // Persist the parsed classifier row (best-effort — STDB outage MUST
        // NOT block dispatch).
        let tool_plan_json = serde_json::to_string(&resp.tool_plan)
            .unwrap_or_else(|_| "null".to_string());
        let tool_spec_json = serde_json::to_string(&resp.tool_spec)
            .unwrap_or_else(|_| "null".to_string());
        post_classifier_response_open(
            msg_id,
            &from,
            role,
            decision_str(&resp.decision),
            &tool_plan_json,
            resp.reason.as_deref().unwrap_or(""),
            resp.target_persona.as_deref().unwrap_or(""),
            resp.question.as_deref().unwrap_or(""),
            &tool_spec_json,
            attempts,
            "parsed",
            resp.cost_usd,
        )
        .await;

        // Route per decision. `route` triggers a peer forward; `request_tool`
        // raises an operator inbox notification. Every decision also produces
        // a structured reply to the source so the ask is never silently
        // dropped.
        let routed_reply = route_decision(
            &comm,
            &role_string,
            role,
            &from,
            &content,
            thread_id.as_deref(),
            &resp,
            msg_id,
        )
        .await;

        let reply_for_send = routed_reply.clone();
        let reply_for_journal = routed_reply.clone();
        let reply_for_commitment = routed_reply.clone();
        let thread_id_for_commitment = thread_id.clone().unwrap_or_default();

        if let Err(e) = comm
            .send_dm(
                role_string.clone(),
                from.clone(),
                reply_for_send,
                thread_id.clone(),
            )
            .await
        {
            tracing::warn!(role = %role, msg_id, to = %from, error = %e, "org_responder: send_dm (classifier reply) failed; will retry");
            continue;
        }
        if let Err(e) = comm.mark_read(role_string.clone(), msg_id).await {
            tracing::warn!(role = %role, msg_id, error = %e, "org_responder: mark_read after classifier reply failed");
        }

        spawn_commitment_and_journal(
            role,
            msg_id,
            content.clone(),
            reply_for_commitment,
            reply_for_journal,
            thread_id_for_commitment,
            persona.clone(),
            http.clone(),
            inference_url.to_string(),
        );
    }

    Ok(())
}

/// Map a [`ClassifierDecision`] to its wire-format snake_case string. Used
/// when persisting the `classifier_response` STDB row — the column expects
/// the same vocabulary the LLM emits, kept here as a `&'static str` so the
/// hot path doesn't go through serde for a six-arm match.
fn decision_str(d: &ClassifierDecision) -> &'static str {
    match d {
        ClassifierDecision::Accept => "accept",
        ClassifierDecision::Defer => "defer",
        ClassifierDecision::Route => "route",
        ClassifierDecision::Clarify => "clarify",
        ClassifierDecision::Reject => "reject",
        ClassifierDecision::RequestTool => "request_tool",
    }
}

/// Synthesize a structured reply for the source persona based on the
/// classifier's decision. Side-effects:
/// - `Route` → forwards the original ask to `target_persona`.
/// - `RequestTool` → raises a priority-2 inbox notification for the operator.
///
/// Returns the textual reply that will be sent back to `from` (operator or
/// peer). Every decision produces a reply — silent drops are eliminated by
/// the ADR-2026-05-17-2030 contract.
#[allow(clippy::too_many_arguments)]
async fn route_decision(
    comm: &Arc<SpacetimeAgentCommAdapter>,
    role_string: &str,
    role: &str,
    from: &str,
    original_content: &str,
    thread_id: Option<&str>,
    resp: &ClassifierResponse,
    msg_id: u64,
) -> String {
    let thread_id_owned = thread_id.map(|s| s.to_string());
    match &resp.decision {
        ClassifierDecision::Accept => {
            let plan_str = resp
                .tool_plan
                .as_ref()
                .map(|steps| {
                    steps
                        .iter()
                        .map(|s| format!("- {}: {}", s.tool, s.intent))
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default();
            format!(
                "Confirm: I ({}) will execute the tool plan (msg_id={}).\n{}",
                role, msg_id, plan_str
            )
        }
        ClassifierDecision::Defer => format!(
            "Defer: {}",
            resp.reason.as_deref().unwrap_or("(no reason given)")
        ),
        ClassifierDecision::Route => {
            let tp = resp.target_persona.as_deref().unwrap_or("");
            if !tp.is_empty() {
                let forward = format!(
                    "[Routed from @{} on behalf of @{}]: \"{}\"",
                    role,
                    from,
                    original_content.chars().take(400).collect::<String>(),
                );
                if let Err(e) = comm
                    .send_dm(
                        role_string.to_string(),
                        tp.to_string(),
                        forward,
                        thread_id_owned,
                    )
                    .await
                {
                    tracing::warn!(role = %role, peer = %tp, error = %e, "org_responder: route forward failed");
                } else {
                    tracing::info!(role = %role, peer = %tp, "org_responder: routed ask to peer");
                }
                format!("Routed to @{}.", tp)
            } else {
                "Route: (target_persona empty — dispatch suppressed)".to_string()
            }
        }
        ClassifierDecision::Clarify => format!(
            "Clarify: {}",
            resp.question.as_deref().unwrap_or("(no question given)")
        ),
        ClassifierDecision::Reject => format!(
            "Reject: {}",
            resp.reason.as_deref().unwrap_or("(no reason given)")
        ),
        ClassifierDecision::RequestTool => {
            let spec_compact = resp
                .tool_spec
                .as_ref()
                .map(|v| serde_json::to_string(v).unwrap_or_default())
                .unwrap_or_default();
            let payload = serde_json::json!({
                "role": role,
                "msg_id": msg_id,
                "thread_id": thread_id.unwrap_or_default(),
                "from": from,
                "tool_spec": resp.tool_spec,
                "summary": "persona requests a new tool",
            })
            .to_string();
            post_inbox_notify("operator", 2, "request_tool", &payload).await;
            format!(
                "Requested new tool — escalated to operator inbox. Spec: {}",
                spec_compact.chars().take(160).collect::<String>()
            )
        }
    }
}

/// Spawn the detached commitment-parser + journal-thought tasks the
/// responder fires after every successful DM. Same shape for both
/// `chat_mode` and classifier paths so the audit trail stays uniform.
#[allow(clippy::too_many_arguments)]
fn spawn_commitment_and_journal(
    role: &str,
    msg_id: u64,
    inbound_content: String,
    reply_for_commitment: String,
    reply_for_journal: String,
    thread_id_for_commitment: String,
    persona: Arc<SpacetimePersonaSupervisor>,
    http: reqwest::Client,
    inference_url: String,
) {
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

    let role_for_journal = role.to_string();
    tokio::spawn(async move {
        match generate_thought_summary(
            &http,
            &inference_url,
            &role_for_journal,
            &inbound_content,
            &reply_for_journal,
        )
        .await
        {
            Ok(summary) if !summary.trim().is_empty() => {
                persona
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

// ============================================================================
// Classifier wiring (ADR-2026-05-17-2030 P4.1)
// ============================================================================

/// Thin [`IInferencePort`] shim that forwards `complete()` to the existing
/// nexus-local `/api/inference/complete` HTTP endpoint. Lets the new
/// `StrictJsonClassifierAdapter` reuse the same routing + provider plumbing
/// the responder already used via `generate_reply`, without threading a
/// full inference port through `spawn()`.
///
/// The shim only implements `complete()`; `stream()` panics because the
/// classifier adapter calls `complete()` exclusively (P3.1 contract).
struct HttpInferenceShim {
    http: reqwest::Client,
    url: String,
}

impl HttpInferenceShim {
    fn new(http: reqwest::Client, url: String) -> Self {
        Self { http, url }
    }
}

#[async_trait]
impl IInferencePort for HttpInferenceShim {
    async fn complete(
        &self,
        request: InferenceRequest,
    ) -> Result<InferenceResponse, InferenceError> {
        // Flatten our single-Text-block user turn into an OpenAI-style
        // `{role, content}` message. The classifier adapter always sends one
        // user message with one Text block, so the join is effectively
        // identity for our case.
        let messages_json: Vec<serde_json::Value> = request
            .messages
            .iter()
            .map(|m| {
                let role = match m.role {
                    hex_core::domain::messages::Role::User => "user",
                    hex_core::domain::messages::Role::Assistant => "assistant",
                };
                let text = m
                    .content
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("");
                serde_json::json!({ "role": role, "content": text })
            })
            .collect();

        let body = serde_json::json!({
            "model": request.model,
            "messages": messages_json,
            "system": request.system_prompt,
            "max_tokens": request.max_tokens,
        });

        let resp = self
            .http
            .post(&self.url)
            .json(&body)
            .send()
            .await
            .map_err(|e| InferenceError::Network(e.to_string()))?;
        let status = resp.status();
        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| InferenceError::Network(e.to_string()))?;
        if !status.is_success() {
            return Err(InferenceError::ApiError {
                status: status.as_u16(),
                body: json.to_string(),
            });
        }
        let content_str = json
            .get("content")
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .to_string();
        let model_used = json
            .get("model")
            .and_then(|m| m.as_str())
            .unwrap_or(&request.model)
            .to_string();
        let input_tokens = json
            .get("input_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let output_tokens = json
            .get("output_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        Ok(InferenceResponse {
            content: vec![ContentBlock::Text { text: content_str }],
            model_used,
            stop_reason: StopReason::EndTurn,
            input_tokens,
            output_tokens,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            latency_ms: 0,
        })
    }

    async fn stream(
        &self,
        _request: InferenceRequest,
    ) -> Result<
        Box<dyn futures_stream::Stream<Item = StreamChunk> + Send + Unpin>,
        InferenceError,
    > {
        Err(InferenceError::ProviderUnavailable(
            "HttpInferenceShim does not implement stream()".to_string(),
        ))
    }

    async fn health(&self) -> Result<HealthStatus, InferenceError> {
        Ok(HealthStatus::Ok { models: vec![] })
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities {
            models: vec![ModelInfo {
                id: "shim".to_string(),
                provider: "http-shim".to_string(),
                tier: ModelTier::Local,
                context_window: 8_192,
            }],
            supports_tool_use: false,
            supports_thinking: false,
            supports_caching: false,
            supports_streaming: false,
            max_context_tokens: 8_192,
            cost_per_mtok_input: 0.0,
            cost_per_mtok_output: 0.0,
        }
    }
}

/// STDB host + database lookup used by the reducer-call helpers.
/// Same env vars as [`try_claim_thread`] / `escalate_to_operator` so the
/// classifier writers can't drift onto a different database.
fn stdb_endpoint(reducer: &str) -> (reqwest::Client, String) {
    let host = std::env::var("HEX_SPACETIMEDB_HOST")
        .unwrap_or_else(|_| "http://127.0.0.1:3033".to_string());
    let db = std::env::var("HEX_STDB_DATABASE")
        .unwrap_or_else(|_| hex_core::stdb_database_for_module("hexflo-coordination").to_string());
    let url = format!("{}/v1/database/{}/call/{}", host, db, reducer);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());
    (client, url)
}

/// Persist a single `classifier_response` row via the STDB reducer
/// (ADR-2026-05-17-2030 P2.1). Best-effort — STDB outage or a missing
/// reducer (P2.1 still in flight) MUST NOT block dispatch.
#[allow(clippy::too_many_arguments)]
async fn post_classifier_response_open(
    msg_id: u64,
    from_role: &str,
    to_role: &str,
    decision: &str,
    tool_plan_json: &str,
    reason: &str,
    target_persona: &str,
    question: &str,
    tool_spec_json: &str,
    reparse_attempts: u32,
    final_outcome: &str,
    cost_usd: f32,
) {
    let (client, url) = stdb_endpoint("classifier_response_open");
    let body = serde_json::json!([
        msg_id,
        from_role,
        to_role,
        decision,
        tool_plan_json,
        reason,
        target_persona,
        question,
        tool_spec_json,
        reparse_attempts,
        final_outcome,
        cost_usd,
    ]);
    match client.post(&url).json(&body).send().await {
        Ok(resp) if resp.status().is_success() => {
            tracing::debug!(
                msg_id,
                decision = %decision,
                final_outcome = %final_outcome,
                attempts = reparse_attempts,
                "classifier_response_open: persisted"
            );
        }
        Ok(resp) => {
            tracing::warn!(
                msg_id,
                decision = %decision,
                status = %resp.status(),
                "classifier_response_open: STDB reducer rejected call"
            );
        }
        Err(e) => {
            tracing::warn!(
                msg_id,
                decision = %decision,
                error = %e,
                "classifier_response_open: STDB transport failed"
            );
        }
    }
}

/// Fire-and-forget `notify_agent` reducer call. Used by `request_tool`
/// decisions and by `escalate_classifier_failure` to surface
/// classifier-loop blowups to the operator inbox.
async fn post_inbox_notify(agent_id: &str, priority: u8, kind: &str, payload: &str) {
    let (client, url) = stdb_endpoint("notify_agent");
    let now = chrono::Utc::now().to_rfc3339();
    let body = serde_json::json!([agent_id, priority, kind, payload, now]);
    match client.post(&url).json(&body).send().await {
        Ok(resp) if resp.status().is_success() => {
            tracing::info!(
                agent_id = %agent_id,
                kind = %kind,
                priority,
                "notify_agent: operator-inbox escalation queued"
            );
        }
        Ok(resp) => {
            tracing::warn!(
                agent_id = %agent_id,
                kind = %kind,
                status = %resp.status(),
                "notify_agent: STDB reducer rejected call (operator may not be registered as agent)"
            );
        }
        Err(e) => {
            tracing::warn!(
                agent_id = %agent_id,
                kind = %kind,
                error = %e,
                "notify_agent: STDB transport failed"
            );
        }
    }
}

/// Escalate a classifier failure to the operator inbox + persist a
/// `classifier_response` row tagged with the appropriate `final_outcome`.
///
/// This is the operator-inbox escalation primitive referenced by the P5.1
/// task — placed here in P4.1 so the new dispatch path has somewhere to
/// branch on its `Err(InvariantError)` arm. P5.1 may elaborate the helper
/// (e.g. structured payload schema) but the call sites belong to P4.1.
#[allow(clippy::too_many_arguments)]
async fn escalate_classifier_failure(
    comm: &Arc<SpacetimeAgentCommAdapter>,
    role_string: &str,
    msg_id: u64,
    from: &str,
    role: &str,
    original_content: &str,
    thread_id: Option<&str>,
    invariant: &InvariantError,
) {
    let (final_outcome, attempts_used) = match invariant {
        // MalformedJson reached us only after the budget exhausted.
        InvariantError::MalformedJson(_) => ("escalated", 3),
        // Schema-violations cost exactly one inference call.
        InvariantError::DecisionNotAllowedForOperator(_)
        | InvariantError::MissingRequiredField { .. } => ("invariant_violation", 1),
    };
    let decision_label = match invariant {
        InvariantError::DecisionNotAllowedForOperator(d) => decision_str(d),
        _ => "invariant_error",
    };
    post_classifier_response_open(
        msg_id,
        from,
        role,
        decision_label,
        "null",
        &invariant.to_string(),
        "",
        "",
        "null",
        attempts_used,
        final_outcome,
        0.0,
    )
    .await;

    let payload = serde_json::json!({
        "role": role,
        "msg_id": msg_id,
        "thread_id": thread_id.unwrap_or_default(),
        "from": from,
        "original_message": original_content.chars().take(400).collect::<String>(),
        "error": invariant.to_string(),
        "final_outcome": final_outcome,
        "attempts": attempts_used,
    })
    .to_string();
    post_inbox_notify("operator", 2, "classifier_escalation", &payload).await;

    // Mark the source DM read so the responder loop doesn't re-fire on the
    // same message every 4s tick. The escalation row + inbox notification
    // are now the durable record of the ask — the inbound DM has done its job.
    if let Err(e) = comm
        .mark_read(role_string.to_string(), msg_id)
        .await
    {
        tracing::warn!(
            role = %role,
            msg_id,
            error = %e,
            "org_responder: mark_read after classifier escalation failed"
        );
    }
}
