//! Autonomous master-supervisor for persona-prompt improvement
//! (ADR-2026-05-23-0900 Path B item 5 — final form).
//!
//! Tokio task that ticks every HEX_HIVE_IMPROVE_INTERVAL_SECS (default 1h).
//! Each tick:
//!   1. Queries persona_prompt + persona_health for every operator-applied
//!      role.
//!   2. Picks the role with the highest recent_failures (tie-break by
//!      most-recently-failed). If no role has failures, also picks the
//!      role whose body is oldest (>24h since last apply) — speculative
//!      improvement on staleness.
//!   3. Runs the GROUND → DISPATCH → DEBATE → JUDGE → APPLY chain over
//!      the local /api/inference/complete endpoint, exactly mirroring the
//!      CLI `hex persona-prompt improve` command. Same models, same
//!      provider divergence, same fail-closed verdict gate.
//!   4. Auto-rollback observer (item 7) catches regressions if the apply
//!      goes through but behavior regresses — composed safety.
//!
//! Disabled with HEX_DISABLE_HIVE_IMPROVE=1 in dev / when the operator
//! wants exclusive write authority over persona_prompt.
//!
//! Note on cooperative-hive (Path B item 6): this trigger is INDIVIDUAL
//! by default. After applying to role X, the next tick surfaces X's
//! adjacent personas (handoff peers via delegation.can_spawn +
//! communication.peers in the YAMLs) as elevated-priority candidates
//! so the neighborhood gets visited. Implemented as a one-tick boost
//! map rather than a synchronous multi-persona apply — keeps each
//! tick's blast radius bounded to one role.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;
use tracing::{debug, info, warn};

const DEFAULT_INTERVAL_SECS: u64 = 3600; // 1 hour
const DEFAULT_STARTUP_GRACE_SECS: u64 = 300; // 5 minutes after boot before first tick
const DEFAULT_FAILURE_THRESHOLD: u32 = 1; // Any failure is improvement-worthy
const DEFAULT_STALE_AFTER_HOURS: i64 = 24;

/// One-tick "boost" — roles that get prioritized on the next tick
/// because a related role was just improved. Cooperative-hive substrate
/// per Path B item 6.
type BoostMap = Arc<RwLock<HashSet<String>>>;

pub fn spawn(stdb_host: String, hex_db: String) {
    if std::env::var("HEX_DISABLE_HIVE_IMPROVE").is_ok() {
        info!("hive_improver: disabled via HEX_DISABLE_HIVE_IMPROVE");
        return;
    }

    let interval_secs = std::env::var("HEX_HIVE_IMPROVE_INTERVAL_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(DEFAULT_INTERVAL_SECS);
    let startup_grace_secs = std::env::var("HEX_HIVE_IMPROVE_STARTUP_GRACE_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(DEFAULT_STARTUP_GRACE_SECS);
    let failure_threshold = std::env::var("HEX_HIVE_IMPROVE_FAILURE_THRESHOLD")
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(DEFAULT_FAILURE_THRESHOLD);
    let stale_hours = std::env::var("HEX_HIVE_IMPROVE_STALE_HOURS")
        .ok()
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(DEFAULT_STALE_AFTER_HOURS);

    info!(
        interval_secs,
        startup_grace_secs,
        failure_threshold,
        stale_hours,
        "hive_improver: spawning (HEX_HIVE_IMPROVE_* env vars override)"
    );

    let boost: BoostMap = Arc::new(RwLock::new(HashSet::new()));

    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(startup_grace_secs)).await;
        let http = match reqwest::Client::builder()
            .timeout(Duration::from_secs(600))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                warn!(error = %e, "hive_improver: http client init failed; disabled");
                return;
            }
        };
        let mut ticker = tokio::time::interval(Duration::from_secs(interval_secs));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        // First tick fires immediately after grace.
        loop {
            ticker.tick().await;
            if let Err(e) = run_tick(
                &http,
                &stdb_host,
                &hex_db,
                failure_threshold,
                stale_hours,
                &boost,
            )
            .await
            {
                warn!(error = %e, "hive_improver: tick error (continuing)");
            }
        }
    });
}

async fn run_tick(
    http: &reqwest::Client,
    stdb_host: &str,
    hex_db: &str,
    failure_threshold: u32,
    stale_hours: i64,
    boost: &BoostMap,
) -> Result<(), String> {
    // 1. Pull candidate set: operator-applied roles only.
    let prompt_rows = sql(
        http,
        stdb_host,
        hex_db,
        "SELECT role, seeded_at, seeded_by, model_preferred, model_upgrade_to FROM persona_prompt",
    )
    .await?;
    let now_secs = chrono::Utc::now().timestamp();
    let stale_micros = stale_hours * 3600 * 1_000_000;
    let now_micros = chrono::Utc::now().timestamp_micros();

    let mut candidates: Vec<RoleSnapshot> = Vec::new();
    for row in &prompt_rows {
        let role = row.first().and_then(|v| v.as_str()).unwrap_or("").to_string();
        if role.is_empty() {
            continue;
        }
        let seeded_by = row.get(2).and_then(|v| v.as_str()).unwrap_or("").to_string();
        let operator_applied =
            seeded_by.starts_with("applied:") || seeded_by.starts_with("rollback:");
        let seeded_at_micros = row
            .get(1)
            .and_then(parse_timestamp_cell)
            .unwrap_or(0);
        let age_micros = now_micros.saturating_sub(seeded_at_micros);
        let stale = seeded_at_micros > 0 && age_micros > stale_micros;
        candidates.push(RoleSnapshot {
            role,
            seeded_by,
            seeded_at_micros,
            operator_applied,
            stale,
            model_preferred: row.get(3).and_then(|v| v.as_str()).unwrap_or("qwen2.5-coder:14b").to_string(),
            model_upgrade_to: row.get(4).and_then(|v| v.as_str()).unwrap_or("claude-sonnet-4-6").to_string(),
            recent_failures: 0,
            last_failure_micros: 0,
        });
    }

    // 2. Pull persona_health and merge.
    let health_rows = sql(
        http,
        stdb_host,
        hex_db,
        "SELECT role, recent_failures, last_failure_at FROM persona_health",
    )
    .await
    .unwrap_or_default();
    let health: HashMap<String, (u32, i64)> = health_rows
        .iter()
        .filter_map(|r| {
            let role = r.first().and_then(|v| v.as_str())?.to_string();
            let n = r.get(1).and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            let last_at = r
                .get(2)
                .and_then(|v| parse_string_timestamp_cell(v))
                .unwrap_or(0);
            Some((role, (n, last_at)))
        })
        .collect();
    for c in candidates.iter_mut() {
        if let Some((n, last_at)) = health.get(&c.role) {
            c.recent_failures = *n;
            c.last_failure_micros = *last_at;
        }
    }

    // 3. Pick the winner.
    //    Priority: (a) cooperative-hive boost set, (b) high recent_failures,
    //    (c) staleness. If nothing qualifies, skip this tick.
    let boost_set = boost.read().await.clone();
    let pick = pick_target(&candidates, failure_threshold, &boost_set);
    let pick = match pick {
        Some(p) => p,
        None => {
            debug!("hive_improver: no improvement candidates this tick (no failures, no stale, no boost)");
            return Ok(());
        }
    };

    // The chosen role's boost (if it was boosted) is consumed.
    if boost_set.contains(&pick.role) {
        boost.write().await.remove(&pick.role);
    }

    info!(
        role = %pick.role,
        recent_failures = pick.recent_failures,
        stale = pick.stale,
        from_boost = boost_set.contains(&pick.role),
        operator_applied = pick.operator_applied,
        "hive_improver: TICK — selected role for improvement"
    );

    // 4. Run the chain (GROUND → DISPATCH → DEBATE → JUDGE → APPLY).
    //    Output goes to the nexus log; no file artifact (CLI is for that).
    match run_improve(http, stdb_host, hex_db, &pick).await {
        Ok(outcome) => {
            info!(
                role = %pick.role,
                applied = outcome.applied,
                red = %outcome.red_verdict,
                blue = %outcome.blue_verdict,
                judge = %outcome.judge_verdict,
                rewriter_ms = outcome.rewriter_ms,
                debate_ms = outcome.debate_ms,
                judge_ms = outcome.judge_ms,
                "hive_improver: TICK COMPLETE"
            );
            // Cooperative-hive substrate: if applied, boost adjacent roles
            // for the next tick.
            if outcome.applied {
                let adj = cooperative_neighbors(&pick.role);
                if !adj.is_empty() {
                    let mut b = boost.write().await;
                    for r in &adj {
                        b.insert(r.clone());
                    }
                    info!(
                        role = %pick.role,
                        boosted = ?adj,
                        "hive_improver: cooperative-hive boost — peer roles prioritized next tick"
                    );
                }
            }
        }
        Err(e) => {
            warn!(role = %pick.role, error = %e, "hive_improver: chain failed");
        }
    }

    // Suppress unused-now-secs warning (kept for future audit-row writes).
    let _ = now_secs;
    Ok(())
}

#[derive(Clone, Debug)]
struct RoleSnapshot {
    role: String,
    seeded_by: String,
    seeded_at_micros: i64,
    operator_applied: bool,
    stale: bool,
    model_preferred: String,
    model_upgrade_to: String,
    recent_failures: u32,
    last_failure_micros: i64,
}

/// Pick the next role to improve. Pure function for testability.
fn pick_target(
    candidates: &[RoleSnapshot],
    failure_threshold: u32,
    boost_set: &HashSet<String>,
) -> Option<RoleSnapshot> {
    // 1. Boosted role with failures wins outright.
    if let Some(c) = candidates
        .iter()
        .filter(|c| boost_set.contains(&c.role) && c.recent_failures >= failure_threshold)
        .max_by_key(|c| c.recent_failures)
    {
        return Some(c.clone());
    }
    // 2. Any role over failure threshold.
    if let Some(c) = candidates
        .iter()
        .filter(|c| c.recent_failures >= failure_threshold)
        .max_by_key(|c| (c.recent_failures, c.last_failure_micros))
    {
        return Some(c.clone());
    }
    // 3. Boosted role even without failures (cooperative neighborhood sweep).
    if let Some(c) = candidates
        .iter()
        .filter(|c| boost_set.contains(&c.role))
        .next()
    {
        return Some(c.clone());
    }
    // 4. Stale operator-applied role — speculative improvement after 24h.
    if let Some(c) = candidates
        .iter()
        .filter(|c| c.stale && c.operator_applied)
        .max_by_key(|c| c.seeded_at_micros)
    {
        return Some(c.clone());
    }
    None
}

/// Static adjacency map for cooperative-hive item 6. After improving
/// role X, these peers get prioritized on the next tick so the
/// neighborhood gets a sweep instead of just the loudest persona.
///
/// Derived from the YAMLs' `delegation.can_spawn` + `communication.peers`
/// fields, hardcoded here to avoid a runtime YAML parse on every tick.
/// Update when the org chart changes.
fn cooperative_neighbors(role: &str) -> Vec<String> {
    match role {
        "cto" => vec![
            "engineering-lead".to_string(),
            "ciso".to_string(),
            "chief-architect".to_string(),
        ],
        "cpo" => vec!["product-lead".to_string(), "cto".to_string()],
        "coo" => vec!["sre-lead".to_string()],
        "ciso" => vec!["cto".to_string()],
        "chief-architect" => vec!["cto".to_string(), "engineering-lead".to_string()],
        "chief-visionary" => vec!["cto".to_string(), "cpo".to_string()],
        "engineering-lead" => vec!["cto".to_string()],
        "product-lead" => vec!["cpo".to_string()],
        "sre-lead" => vec!["coo".to_string()],
        _ => Vec::new(),
    }
}

#[derive(Debug)]
struct ImproveOutcome {
    applied: bool,
    red_verdict: String,
    blue_verdict: String,
    judge_verdict: String,
    rewriter_ms: u128,
    debate_ms: u128,
    judge_ms: u128,
}

async fn run_improve(
    http: &reqwest::Client,
    stdb_host: &str,
    hex_db: &str,
    pick: &RoleSnapshot,
) -> Result<ImproveOutcome, String> {
    // GROUND: pull current body and assemble evidence string.
    let safe = pick.role.replace('\'', "''");
    let rows = sql(
        http,
        stdb_host,
        hex_db,
        &format!(
            "SELECT classify_body FROM persona_prompt WHERE role = '{}'",
            safe
        ),
    )
    .await?;
    let current_body = rows
        .first()
        .and_then(|r| r.first().and_then(|v| v.as_str()))
        .unwrap_or("")
        .to_string();
    if current_body.is_empty() {
        return Err(format!("no persona_prompt body for role {}", pick.role));
    }
    let evidence = format!(
        "recent_failures (60s window): {}\nlast_failure_at: {}\nstale: {}\nseeded_by: {}\n",
        pick.recent_failures,
        if pick.last_failure_micros > 0 {
            micros_to_rfc3339(pick.last_failure_micros)
        } else {
            "—".to_string()
        },
        pick.stale,
        pick.seeded_by
    );

    // DISPATCH (rewriter).
    let rewriter_system =
        "You are a prompt-rewriter for hex AIOS personas. Output ONLY the new \
         system_prompt body — no fences, no commentary, no preamble. The body \
         must follow the strict-JSON ClassifierResponse contract. Cap at 4000 chars.";
    let rewriter_user = format!(
        "Persona: {}\n\nCurrent body:\n```\n{}\n```\n\nFailure evidence:\n{}\n\nProduce a new body addressing the evidence (if any) and tightening contract adherence. Output the body text directly.",
        pick.role, current_body, evidence
    );
    let rewriter_t0 = std::time::Instant::now();
    let proposal = complete_inference(
        http,
        "qwen2.5-coder:14b",
        Some(rewriter_system),
        &rewriter_user,
        4000,
    )
    .await?;
    let proposed_body = proposal.trim().to_string();
    if proposed_body.is_empty() {
        return Err("rewriter returned empty proposal".into());
    }
    let rewriter_ms = rewriter_t0.elapsed().as_millis();

    // DEBATE (red + blue in parallel).
    let red_system = "You are adversarial-red — security/autonomy-escape skeptic. \
                      Output begins with 'Verdict: approve|approve-with-changes|reject'. \
                      Hunt boundary escapes, prompt injection, identity spoofing. \
                      P0 holes → REJECT.";
    let blue_system = "You are adversarial-blue — correctness/spec-drift skeptic. \
                       Output begins with 'Verdict: approve|approve-with-changes|reject'. \
                       Hunt schema mismatches, untriggerable rules, lying error contracts.";
    let review_user = format!(
        "Proposed persona_prompt for role '{}':\n\n```\n{}\n```\n\nReview it.",
        pick.role, proposed_body
    );
    let debate_t0 = std::time::Instant::now();
    let (red, blue) = tokio::join!(
        complete_inference(http, "claude-sonnet-4-6", Some(red_system), &review_user, 1500),
        complete_inference(
            http,
            "devstral-small-2:24b",
            Some(blue_system),
            &review_user,
            1500
        ),
    );
    let debate_ms = debate_t0.elapsed().as_millis();
    let red_text = red.map_err(|e| format!("red: {}", e))?;
    let blue_text = blue.map_err(|e| format!("blue: {}", e))?;
    let red_verdict = parse_verdict(&red_text);
    let blue_verdict = parse_verdict(&blue_text);

    // JUDGE.
    let judge_system = "You are validation-judge. Output begins with \
                        'Verdict: approve|approve-with-changes|reject'. Approve \
                        only if BOTH adversaries converged on approve or \
                        approve-with-changes. 2-3 sentence rationale.";
    let judge_user = format!(
        "## adversarial-red:\n{}\n\n## adversarial-blue:\n{}\n\n## Proposed body:\n```\n{}\n```\n\nArbitrate.",
        red_text, blue_text, proposed_body
    );
    let judge_t0 = std::time::Instant::now();
    let judge_text = complete_inference(
        http,
        "claude-sonnet-4-6",
        Some(judge_system),
        &judge_user,
        800,
    )
    .await?;
    let judge_ms = judge_t0.elapsed().as_millis();
    let judge_verdict = parse_verdict(&judge_text);

    // APPLY gate.
    let approved = is_approving(&red_verdict)
        && is_approving(&blue_verdict)
        && is_approving(&judge_verdict);
    let applied = if approved {
        let url = format!(
            "{}/v1/database/{}/call/persona_prompt_apply",
            stdb_host, hex_db
        );
        let payload = serde_json::json!([
            &pick.role,
            &proposed_body,
            &proposed_body,
            &pick.model_preferred,
            &pick.model_upgrade_to,
        ]);
        match http.post(&url).json(&payload).send().await {
            Ok(r) if r.status().is_success() => true,
            Ok(r) => {
                warn!(
                    role = %pick.role,
                    status = %r.status(),
                    "hive_improver: apply rejected by reducer"
                );
                false
            }
            Err(e) => {
                warn!(role = %pick.role, error = %e, "hive_improver: apply transport error");
                false
            }
        }
    } else {
        false
    };

    Ok(ImproveOutcome {
        applied,
        red_verdict,
        blue_verdict,
        judge_verdict,
        rewriter_ms,
        debate_ms,
        judge_ms,
    })
}

// ── Helpers ─────────────────────────────────────────────────────────

async fn complete_inference(
    http: &reqwest::Client,
    model: &str,
    system: Option<&str>,
    user: &str,
    max_tokens: u32,
) -> Result<String, String> {
    let url = "http://127.0.0.1:5555/api/inference/complete";
    let mut messages = Vec::new();
    if let Some(sys) = system {
        messages.push(serde_json::json!({"role":"system","content":sys}));
    }
    messages.push(serde_json::json!({"role":"user","content":user}));
    let body = serde_json::json!({
        "model": model,
        "messages": messages,
        "max_tokens": max_tokens,
    });
    let res = http
        .post(url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("{} transport: {}", model, e))?;
    if !res.status().is_success() {
        return Err(format!("{} HTTP {}", model, res.status()));
    }
    let resp: serde_json::Value = res.json().await.map_err(|e| format!("json: {}", e))?;
    Ok(resp
        .get("content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string())
}

async fn sql(
    http: &reqwest::Client,
    stdb_host: &str,
    hex_db: &str,
    query: &str,
) -> Result<Vec<Vec<serde_json::Value>>, String> {
    let url = format!("{}/v1/database/{}/sql", stdb_host, hex_db);
    let res = http
        .post(&url)
        .header("Content-Type", "text/plain")
        .body(query.to_string())
        .send()
        .await
        .map_err(|e| format!("sql transport: {}", e))?;
    if !res.status().is_success() {
        return Err(format!("sql HTTP {}", res.status()));
    }
    let body: serde_json::Value = res.json().await.map_err(|e| format!("json: {}", e))?;
    let mut out: Vec<Vec<serde_json::Value>> = Vec::new();
    let collect = |rows: &serde_json::Value, out: &mut Vec<Vec<serde_json::Value>>| {
        if let Some(arr) = rows.as_array() {
            for r in arr {
                if let Some(cols) = r.as_array() {
                    out.push(cols.clone());
                }
            }
        }
    };
    if let Some(arr) = body.as_array() {
        for rs in arr {
            if let Some(rows) = rs.get("rows") {
                collect(rows, &mut out);
            }
        }
    } else if let Some(rows) = body.get("rows") {
        collect(rows, &mut out);
    }
    Ok(out)
}

fn parse_verdict(text: &str) -> String {
    let lower = text.to_lowercase();
    let needle = lower.find("verdict:").map(|i| i + "verdict:".len());
    let tail = needle.map(|i| &lower[i..]).unwrap_or(&lower);
    let head: String = tail.chars().take(80).collect();
    if head.contains("approve-with-changes") || head.contains("approve with changes") {
        "approve-with-changes".to_string()
    } else if head.contains("reject") {
        "reject".to_string()
    } else if head.contains("approve") {
        "approve".to_string()
    } else {
        "reject".to_string()
    }
}

fn is_approving(v: &str) -> bool {
    v == "approve" || v == "approve-with-changes"
}

/// STDB serializes `Timestamp` as `[micros_i64]` (single-element array
/// for the Product type containing one i64 field). Older clients may
/// serialize as a string with the Debug-format `Timestamp { ...: N }`.
/// Handle both.
fn parse_timestamp_cell(v: &serde_json::Value) -> Option<i64> {
    if let Some(arr) = v.as_array() {
        if let Some(n) = arr.first().and_then(|x| x.as_i64()) {
            return Some(n);
        }
    }
    if let Some(s) = v.as_str() {
        let key = "__timestamp_micros_since_unix_epoch__";
        if let Some(i) = s.find(key) {
            let digits: String = s[i + key.len()..]
                .chars()
                .skip_while(|c| !c.is_ascii_digit() && *c != '-')
                .take_while(|c| c.is_ascii_digit() || *c == '-')
                .collect();
            return digits.parse::<i64>().ok();
        }
    }
    None
}

/// `persona_health.last_failure_at` is `String` (legacy schema), not
/// `Timestamp`. The format is the Rust Debug rendering of a Timestamp.
fn parse_string_timestamp_cell(v: &serde_json::Value) -> Option<i64> {
    let s = v.as_str()?;
    let key = "__timestamp_micros_since_unix_epoch__";
    let i = s.find(key)? + key.len();
    let digits: String = s[i..]
        .chars()
        .skip_while(|c| !c.is_ascii_digit() && *c != '-')
        .take_while(|c| c.is_ascii_digit() || *c == '-')
        .collect();
    digits.parse::<i64>().ok()
}

fn micros_to_rfc3339(micros: i64) -> String {
    let secs = micros / 1_000_000;
    let nsec = ((micros % 1_000_000) * 1_000) as u32;
    chrono::DateTime::<chrono::Utc>::from_timestamp(secs, nsec)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_else(|| "?".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(role: &str, recent: u32, last_at: i64, stale: bool) -> RoleSnapshot {
        RoleSnapshot {
            role: role.to_string(),
            seeded_by: "applied:test".to_string(),
            seeded_at_micros: 1,
            operator_applied: true,
            stale,
            model_preferred: "qwen2.5-coder:14b".to_string(),
            model_upgrade_to: "claude-sonnet-4-6".to_string(),
            recent_failures: recent,
            last_failure_micros: last_at,
        }
    }

    #[test]
    fn picks_highest_failures_wins() {
        let candidates = vec![
            snap("cto", 1, 100, false),
            snap("cpo", 5, 200, false),
            snap("coo", 2, 300, false),
        ];
        let pick = pick_target(&candidates, 1, &HashSet::new()).unwrap();
        assert_eq!(pick.role, "cpo");
    }

    #[test]
    fn boost_overrides_pure_failure_ordering_when_boosted_has_failures() {
        let candidates = vec![
            snap("cto", 1, 100, false),
            snap("cpo", 5, 200, false),
            snap("engineering-lead", 2, 300, false),
        ];
        let mut boost = HashSet::new();
        boost.insert("engineering-lead".to_string());
        let pick = pick_target(&candidates, 1, &boost).unwrap();
        assert_eq!(pick.role, "engineering-lead");
    }

    #[test]
    fn boost_without_failures_still_fires_if_no_failures_anywhere() {
        let candidates = vec![
            snap("cto", 0, 0, false),
            snap("engineering-lead", 0, 0, false),
        ];
        let mut boost = HashSet::new();
        boost.insert("engineering-lead".to_string());
        let pick = pick_target(&candidates, 1, &boost).unwrap();
        assert_eq!(pick.role, "engineering-lead");
    }

    #[test]
    fn stale_role_picked_when_no_failures_no_boost() {
        let candidates = vec![
            snap("cto", 0, 0, false),
            snap("cpo", 0, 0, true),
        ];
        let pick = pick_target(&candidates, 1, &HashSet::new()).unwrap();
        assert_eq!(pick.role, "cpo");
    }

    #[test]
    fn no_pick_when_nothing_qualifies() {
        let candidates = vec![snap("cto", 0, 0, false), snap("cpo", 0, 0, false)];
        assert!(pick_target(&candidates, 1, &HashSet::new()).is_none());
    }

    #[test]
    fn cooperative_neighbors_for_known_roles() {
        let cto_peers = cooperative_neighbors("cto");
        assert!(cto_peers.contains(&"engineering-lead".to_string()));
        assert!(cto_peers.contains(&"ciso".to_string()));
        // Unknown role yields empty set (no boost applied).
        assert!(cooperative_neighbors("not-a-real-role").is_empty());
    }

    #[test]
    fn parse_verdict_handles_explicit() {
        assert_eq!(parse_verdict("Verdict: approve\nbody"), "approve");
        assert_eq!(parse_verdict("Verdict: reject\n# findings"), "reject");
        assert_eq!(
            parse_verdict("VERDICT: approve-with-changes"),
            "approve-with-changes"
        );
    }

    #[test]
    fn parse_verdict_fails_closed_on_no_verdict_keyword() {
        assert_eq!(parse_verdict("looks fine to me"), "reject");
        assert_eq!(parse_verdict(""), "reject");
    }
}
