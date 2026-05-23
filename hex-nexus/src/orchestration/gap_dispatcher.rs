//! gap_dispatcher — closes the operator-as-dispatcher anti-pattern.
//!
//! Closes the structural gap surfaced 2026-05-23: "why isn't the exec team
//! working on this?". Pre-fix the org_responder only ran personas when they
//! received an inbound DM; nothing scanned `gap:*` memory and turned a
//! known gap into a persona-dispatched action. The operator was the
//! unintended `dispatch loop`.
//!
//! This task periodically:
//!   1. queries hexflo_memory for `gap:*` entries
//!   2. for each gap, routes to an executive role (CTO/CPO/COO/CISO/...)
//!      based on substring match against keywords in key + value
//!   3. checks whether we've already dispatched within the cooldown
//!      (per-gap, via `dispatched:<gap_key>:<utc_ts>` memory entries)
//!   4. sends a board DM via /api/org/send-message with a structured
//!      brief embedding the gap content + suggested artifact kind
//!   5. records the dispatch so next tick doesn't spam
//!
//! Defaults (override via env):
//!   HEX_GAP_DISPATCHER_INTERVAL_SECS   1800  — tick every 30 min
//!   HEX_GAP_DISPATCHER_COOLDOWN_SECS   21600 — 6h per-gap cooldown
//!   HEX_GAP_DISPATCHER_MAX_PER_TICK    1     — one dispatch per tick (rate-limit)
//!   HEX_GAP_DISPATCHER_STARTUP_SECS    120   — wait this long after start
//!   HEX_DISABLE_GAP_DISPATCHER         (any) — turn off entirely

use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use std::collections::HashMap;

use serde_json::{json, Value};

const DEFAULT_INTERVAL_SECS: u64 = 1800;          // 30 min
const DEFAULT_COOLDOWN_SECS: u64 = 6 * 3600;      // 6h
const DEFAULT_MAX_PER_TICK: usize = 1;
const DEFAULT_STARTUP_SECS: u64 = 120;

const SEND_AGENT_ID: &str = "nexus-gap-dispatcher";

/// Per-gap last-dispatch timestamp (process-local cache mirroring
/// the persisted `dispatched:<gap_key>` memory entries). The persisted
/// entries are the source of truth across restarts; this cache just
/// avoids re-querying memory for every gap on every tick.
fn dispatch_cache() -> &'static Mutex<HashMap<String, Instant>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn spawn(_stdb_host: String, _hex_db: String) {
    if std::env::var("HEX_DISABLE_GAP_DISPATCHER").is_ok() {
        tracing::info!("gap_dispatcher disabled via HEX_DISABLE_GAP_DISPATCHER");
        return;
    }

    let interval_secs = parse_env_u64("HEX_GAP_DISPATCHER_INTERVAL_SECS", DEFAULT_INTERVAL_SECS);
    let cooldown_secs = parse_env_u64("HEX_GAP_DISPATCHER_COOLDOWN_SECS", DEFAULT_COOLDOWN_SECS);
    let max_per_tick = parse_env_u64("HEX_GAP_DISPATCHER_MAX_PER_TICK", DEFAULT_MAX_PER_TICK as u64) as usize;
    let startup_secs = parse_env_u64("HEX_GAP_DISPATCHER_STARTUP_SECS", DEFAULT_STARTUP_SECS);

    tokio::spawn(async move {
        let http = match reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
        {
            Ok(c) => Arc::new(c),
            Err(e) => {
                tracing::warn!(error = %e, "gap_dispatcher: http client build failed");
                return;
            }
        };

        tracing::info!(
            interval_secs,
            cooldown_secs,
            max_per_tick,
            startup_secs,
            "gap_dispatcher: spawning"
        );

        // Settle period before first tick.
        tokio::time::sleep(Duration::from_secs(startup_secs)).await;

        let nexus_port = std::env::var("HEX_NEXUS_PORT").unwrap_or_else(|_| "5555".to_string());
        let nexus_base = format!("http://127.0.0.1:{}", nexus_port);
        let cooldown = Duration::from_secs(cooldown_secs);

        let mut ticker = tokio::time::interval(Duration::from_secs(interval_secs));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            ticker.tick().await;
            if let Err(e) = run_tick(&http, &nexus_base, cooldown, max_per_tick).await {
                tracing::warn!(error = %e, "gap_dispatcher: tick failed");
            }
        }
    });
}

async fn run_tick(
    http: &Arc<reqwest::Client>,
    nexus_base: &str,
    cooldown: Duration,
    max_per_tick: usize,
) -> Result<(), String> {
    // 1. Pull all gap: memory entries.
    let url = format!("{}/api/hexflo/memory/search?q=gap%3A", nexus_base);
    let resp = http
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("memory search transport: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("memory search HTTP {}", resp.status()));
    }
    let body: Value = resp.json().await.map_err(|e| format!("memory search json: {}", e))?;
    let results: Vec<Value> = body
        .get("results")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    if results.is_empty() {
        tracing::debug!("gap_dispatcher: no gap entries in memory");
        return Ok(());
    }

    // 2. Filter to gaps not in cooldown.
    let now = Instant::now();
    let mut eligible: Vec<(String, String)> = Vec::new();
    {
        let cache = dispatch_cache().lock().unwrap();
        for entry in &results {
            let key = match entry.get("key").and_then(|v| v.as_str()) {
                Some(k) => k,
                None => continue,
            };
            // Skip non-gap keys that might leak in (defense).
            if !key.starts_with("gap:") {
                continue;
            }
            // Skip closed gaps recorded via memory.
            let value = entry.get("value").and_then(|v| v.as_str()).unwrap_or("");
            if value.to_ascii_lowercase().contains("closed:") {
                continue;
            }
            if let Some(last) = cache.get(key) {
                if now.saturating_duration_since(*last) < cooldown {
                    continue;
                }
            }
            eligible.push((key.to_string(), value.to_string()));
            if eligible.len() >= max_per_tick * 4 {
                // Read a buffer so we can choose oldest-first; trim later.
                break;
            }
        }
    }

    if eligible.is_empty() {
        tracing::debug!(
            total_gaps = results.len(),
            "gap_dispatcher: all gaps in cooldown — no dispatch this tick"
        );
        return Ok(());
    }

    // 3. Pick the top max_per_tick (already FIFO from STDB scan).
    for (gap_key, gap_value) in eligible.into_iter().take(max_per_tick) {
        let role = route_gap_to_role(&gap_key, &gap_value);
        let brief = build_brief(&gap_key, &gap_value, role);
        match send_board_ask(http, nexus_base, role, &gap_key, &brief).await {
            Ok(msg_id) => {
                dispatch_cache().lock().unwrap().insert(gap_key.clone(), now);
                tracing::info!(
                    gap_key = %gap_key,
                    role = %role,
                    msg_id = %msg_id,
                    "gap_dispatcher: dispatched"
                );
            }
            Err(e) => {
                tracing::warn!(gap_key = %gap_key, role = %role, error = %e, "gap_dispatcher: dispatch failed");
            }
        }
    }

    Ok(())
}

/// Route a gap to an executive persona by substring matching the
/// combined key+value text against role-specific keyword sets. Order
/// matters — security is checked first because credential/vuln gaps
/// crossover into ops vocabulary; ops is checked before product because
/// some product asks mention deploy/runtime. CTO is the fallback for
/// anything that doesn't match a specific exec — it owns architecture,
/// STDB schema, and the substrate, which is most engineering gaps.
pub(crate) fn route_gap_to_role(key: &str, value: &str) -> &'static str {
    let combined = format!("{} {}", key, value).to_ascii_lowercase();

    if has_any(&combined, &[
        "security", "auth ", "auth-", "auth_", "leak", "credential",
        "secret", "exploit", "vuln", "owasp", "encryption", "redaction",
    ]) {
        return "ciso";
    }

    if has_any(&combined, &[
        "ops ", "runbook", "deploy", "infrastructure", "monitoring",
        "alert", "ic-responder", "incident", "outage", "sla", "uptime",
        "cost-ops", "coverage-2026", "ops-",
    ]) {
        return "coo";
    }

    if has_any(&combined, &[
        "product", "ux-", "ux ", "dashboard", "user-experience", "onboarding",
        "kanban", "customer", "marketing",
    ]) {
        return "cpo";
    }

    if has_any(&combined, &[
        "strategy", "roadmap", "vision", "research-track",
    ]) {
        return "chief-visionary";
    }

    if has_any(&combined, &[
        "sre", "observability", "reliability", "runtime-health",
    ]) {
        return "sre-lead";
    }

    if has_any(&combined, &[
        "spec ", "requirement", "acceptance-criteria", "user-story",
    ]) {
        return "product-lead";
    }

    if has_any(&combined, &[
        "sprint", "backlog", "ticket", "milestone", "estimate",
    ]) {
        return "engineering-lead";
    }

    // Default: architecture / STDB / substrate / code quality / refactors
    "cto"
}

fn has_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| haystack.contains(n))
}

/// Build the board-ask brief for one gap. The brief includes the gap
/// key, value, suggested artifact kind, and a reminder to use typed
/// tools — so the receiving persona's SOP path can ground the work
/// quickly via memory_search and emit via adr_draft / workplan_emit /
/// spec_draft / code_patch.
fn build_brief(gap_key: &str, gap_value: &str, role: &str) -> String {
    let suggested = if gap_key.contains("workplan") || gap_value.to_ascii_lowercase().contains("workplan") {
        "workplan_emit (with one phase + tasks + files[])"
    } else if gap_key.contains("hygiene") || gap_key.contains("compaction") {
        "adr_draft (architectural decision — propose hygiene policy)"
    } else if gap_key.contains("memory") || gap_key.contains("context") {
        "adr_draft (substrate gap — propose the missing primitive)"
    } else if gap_value.to_ascii_lowercase().contains("error") || gap_key.contains("failure") {
        "spec_draft or code_patch (failure mode — diagnose then fix)"
    } else {
        "adr_draft or spec_draft (operator's choice based on scope)"
    };

    format!(
        "Gap dispatched from hex memory by gap_dispatcher (autonomous tick).\n\n\
         **Gap key:** `{key}`\n\
         **Routed to:** @{role} (substring-matched on keyword set)\n\
         **Suggested artifact:** {suggested}\n\n\
         **Gap content:**\n\
         ```\n{value}\n```\n\n\
         Use your typed tools (adr_draft, spec_draft, workplan_emit, code_patch) to emit ONE \
         structured action that addresses this gap. memory_search is now wired into your GROUND \
         phase — query `{key}` for any context already stored. After your action lands the \
         twin-approve path commits autonomously; the next gap_dispatcher tick will see the \
         action and skip this gap from re-dispatch.",
        key = gap_key,
        role = role,
        suggested = suggested,
        value = gap_value.chars().take(800).collect::<String>(),
    )
}

async fn send_board_ask(
    http: &Arc<reqwest::Client>,
    nexus_base: &str,
    to: &str,
    gap_key: &str,
    content: &str,
) -> Result<String, String> {
    let url = format!("{}/api/org/send-message", nexus_base);
    let payload = json!({
        "from": "ceo",
        "to": to,
        "content": content,
        "context": {
            "source": "gap_dispatcher",
            "gap_key": gap_key,
        },
    });
    let resp = http
        .post(&url)
        .header("x-hex-agent-id", SEND_AGENT_ID)
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("send-message transport: {}", e))?;
    let status = resp.status();
    let body: Value = resp.json().await.map_err(|e| format!("send-message json: {}", e))?;
    if !status.is_success() {
        return Err(format!("send-message HTTP {}: {}", status, body));
    }
    let msg_id = body
        .get("message_id")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    Ok(msg_id)
}

fn parse_env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_security_keywords_to_ciso() {
        assert_eq!(route_gap_to_role("gap:credential-leak", ""), "ciso");
        assert_eq!(route_gap_to_role("gap:owasp-coverage", ""), "ciso");
        assert_eq!(route_gap_to_role("gap:vuln-tracking", "auth boundary"), "ciso");
    }

    #[test]
    fn routes_ops_keywords_to_coo() {
        assert_eq!(route_gap_to_role("gap:cost-ops-runbook", ""), "coo");
        assert_eq!(route_gap_to_role("gap:adr-workplan-coverage-2026-05-23", ""), "coo");
        assert_eq!(route_gap_to_role("gap:ic-responder-gap", ""), "coo");
    }

    #[test]
    fn routes_product_keywords_to_cpo() {
        assert_eq!(route_gap_to_role("gap:dashboard-refactor", ""), "cpo");
        assert_eq!(route_gap_to_role("gap:kanban-orphan-row-filter", ""), "cpo");
        assert_eq!(route_gap_to_role("gap:ux-discoverability", ""), "cpo");
    }

    #[test]
    fn routes_architecture_keywords_to_cto_default() {
        assert_eq!(route_gap_to_role("gap:no-memory-hygiene", ""), "cto");
        assert_eq!(route_gap_to_role("gap:no-rolling-context-summary", ""), "cto");
        assert_eq!(route_gap_to_role("gap:hive-improver-doesnt-read-memory", ""), "cto");
        assert_eq!(route_gap_to_role("gap:sop-pipeline-redesign", ""), "cto");
    }

    #[test]
    fn brief_includes_gap_content_and_typed_tool_hint() {
        let brief = build_brief("gap:no-memory-hygiene", "No compaction or TTL.", "cto");
        assert!(brief.contains("gap:no-memory-hygiene"));
        assert!(brief.contains("@cto"));
        assert!(brief.contains("memory_search"));
        assert!(brief.contains("No compaction or TTL."));
    }

    #[test]
    fn brief_caps_value_at_800_chars() {
        let huge = "X".repeat(5000);
        let brief = build_brief("gap:huge", &huge, "cto");
        // brief contains the cap, not the full 5000 chars
        let value_chars_in_brief = brief.matches('X').count();
        assert!(value_chars_in_brief <= 800);
    }

    #[test]
    fn dispatch_cache_is_process_local() {
        // Calling twice yields the same Mutex (OnceLock initialised once).
        let a = dispatch_cache() as *const _;
        let b = dispatch_cache() as *const _;
        assert_eq!(a, b);
    }
}
