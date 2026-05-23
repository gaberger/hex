//! Auto-rollback observer for persona_prompt (ADR-2026-05-23-0900 Path B
//! item 7).
//!
//! Tokio task that polls persona_health + persona_prompt every minute. For
//! any persona whose currently-active prompt was operator-applied (or a
//! prior auto-rollback) AND has accumulated failure events since apply,
//! fires `persona_prompt_rollback` to revert to the prior history version.
//!
//! Design constraints:
//!
//! - **Only operator-driven state gets rolled back.** A persona running on
//!   the cold-start seed body never gets auto-rolled-back — there's no
//!   prior version to revert to that wouldn't just re-seed the same body.
//!   Detected by the `applied:` / `rollback:` prefix on `seeded_by`.
//!
//! - **Grace period.** New applies need ~5 min to warm caches + clear the
//!   prior persona's failure counter. Rollback during the grace period
//!   would false-positive on stale failures attributed to the new body.
//!
//! - **Per-role throttle.** Once a rollback fires for role X, no further
//!   rollback for X for 30 min. Prevents thrash if the body the rollback
//!   reverts to ALSO regresses (operator should investigate manually).
//!
//! - **Failure threshold below ban threshold.** persona_health bans at 3
//!   failures within 60s; we rollback at 2 so the autonomous fix lands
//!   before the persona gets benched.
//!
//! - **Fail-open.** Any transport/parse error in the observer is logged
//!   at debug and skipped. The system never STOPS rolling forward because
//!   the auto-rollback observer is unreachable.
//!
//! - **Disabled by default in dev** when `HEX_AUTO_ROLLBACK=0`. Enabled
//!   in production where the operator wants the loop to self-heal.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Default poll cadence. Coarse enough that a 1-min STDB query is cheap;
/// fine enough that the typical 60s persona_health window closes before
/// the next tick.
const DEFAULT_TICK_SECS: u64 = 60;

/// Minimum age of an applied prompt before it's eligible for auto-rollback.
/// Stale failures from the prior persona body can take ~minutes to clear
/// once the new body is active; 300s is conservative.
const DEFAULT_GRACE_SECS: i64 = 300;

/// Per-role cooldown after a rollback fires. Prevents thrash if the
/// reverted-to version is also bad (operator should investigate manually).
const DEFAULT_COOLDOWN_SECS: i64 = 30 * 60;

/// Number of failures within the persona_health 60s rolling window that
/// triggers an auto-rollback. Set to 2 — one below the ban threshold of
/// 3 — so the autonomous fix lands BEFORE the persona gets benched.
const DEFAULT_FAILURE_THRESHOLD: u32 = 2;

/// Spawn the observer task. Returns immediately. The task runs until
/// nexus shuts down. Pass `enabled=false` (or set HEX_AUTO_ROLLBACK=0)
/// to skip spawning entirely.
pub fn spawn(stdb_host: String, hex_db: String) {
    let enabled = match std::env::var("HEX_AUTO_ROLLBACK") {
        Ok(s) if s == "0" || s.eq_ignore_ascii_case("false") => false,
        _ => true,
    };
    if !enabled {
        info!("persona_prompt_observer: disabled via HEX_AUTO_ROLLBACK=0");
        return;
    }

    let tick = std::env::var("HEX_AUTO_ROLLBACK_TICK_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(DEFAULT_TICK_SECS);
    let grace = std::env::var("HEX_AUTO_ROLLBACK_GRACE_SECS")
        .ok()
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(DEFAULT_GRACE_SECS);
    let cooldown = std::env::var("HEX_AUTO_ROLLBACK_COOLDOWN_SECS")
        .ok()
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(DEFAULT_COOLDOWN_SECS);
    let threshold = std::env::var("HEX_AUTO_ROLLBACK_FAILURE_THRESHOLD")
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(DEFAULT_FAILURE_THRESHOLD);

    info!(
        tick_secs = tick,
        grace_secs = grace,
        cooldown_secs = cooldown,
        failure_threshold = threshold,
        "persona_prompt_observer: spawning (HEX_AUTO_ROLLBACK env vars override)"
    );

    let cooldowns: Arc<RwLock<HashMap<String, i64>>> = Arc::new(RwLock::new(HashMap::new()));

    tokio::spawn(async move {
        // Wait a beat so STDB hydration completes before the first tick.
        tokio::time::sleep(Duration::from_secs(20)).await;
        let http = match reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                warn!(error = %e, "persona_prompt_observer: failed to build http client; disabled");
                return;
            }
        };
        let mut ticker = tokio::time::interval(Duration::from_secs(tick));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            ticker.tick().await;
            if let Err(e) = run_tick(
                &http,
                &stdb_host,
                &hex_db,
                grace,
                cooldown,
                threshold,
                &cooldowns,
            )
            .await
            {
                debug!(error = %e, "persona_prompt_observer: tick error (continuing)");
            }
        }
    });
}

/// One observer tick. Pulls applied/rollback persona_prompt rows, joins
/// with persona_health, and fires rollback for any role over the failure
/// threshold past the grace period and outside the cooldown.
async fn run_tick(
    http: &reqwest::Client,
    stdb_host: &str,
    hex_db: &str,
    grace_secs: i64,
    cooldown_secs: i64,
    failure_threshold: u32,
    cooldowns: &Arc<RwLock<HashMap<String, i64>>>,
) -> Result<(), String> {
    // 1. Pull operator-driven persona_prompt rows (applied: / rollback:).
    //    We filter in-process because STDB SQL doesn't support prefix
    //    matching on string columns.
    let prompt_rows = sql(
        http,
        stdb_host,
        hex_db,
        "SELECT role, seeded_at, seeded_by FROM persona_prompt",
    )
    .await?;

    let now_micros = chrono::Utc::now().timestamp_micros();
    let now_secs = now_micros / 1_000_000;

    let mut operator_driven: Vec<(String, i64)> = Vec::new(); // (role, seeded_at_micros)
    for row in &prompt_rows {
        let role = match row.first().and_then(|v| v.as_str()) {
            Some(r) => r.to_string(),
            None => continue,
        };
        let seeded_by = row.get(2).and_then(|v| v.as_str()).unwrap_or("");
        if !seeded_by.starts_with("applied:") && !seeded_by.starts_with("rollback:") {
            continue;
        }
        let seeded_at_micros = match row.get(1) {
            Some(v) => match parse_timestamp_cell(v) {
                Some(m) => m,
                None => continue,
            },
            None => continue,
        };
        // Skip if still inside grace period.
        if (now_micros - seeded_at_micros) < grace_secs * 1_000_000 {
            continue;
        }
        operator_driven.push((role, seeded_at_micros));
    }

    if operator_driven.is_empty() {
        return Ok(());
    }

    // 2. Pull persona_health for the operator-driven roles. Sparse table
    //    in practice — usually 0–3 rows. Walking the whole table is fine.
    let health_rows = sql(
        http,
        stdb_host,
        hex_db,
        "SELECT role, recent_failures FROM persona_health",
    )
    .await?;
    let health_by_role: HashMap<String, u32> = health_rows
        .iter()
        .filter_map(|r| {
            let role = r.first().and_then(|v| v.as_str())?.to_string();
            let n = r.get(1).and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            Some((role, n))
        })
        .collect();

    // 3. For each candidate, check threshold + cooldown.
    for (role, _seeded_at) in operator_driven {
        let recent = health_by_role.get(&role).copied().unwrap_or(0);
        if recent < failure_threshold {
            continue;
        }
        // Cooldown check.
        {
            let map = cooldowns.read().await;
            if let Some(&last_fired_secs) = map.get(&role) {
                if (now_secs - last_fired_secs) < cooldown_secs {
                    continue;
                }
            }
        }

        // Fire rollback.
        info!(
            role = %role,
            recent_failures = recent,
            "persona_prompt_observer: regression detected — firing rollback"
        );
        match call_rollback(http, stdb_host, hex_db, &role).await {
            Ok(_) => {
                cooldowns.write().await.insert(role.clone(), now_secs);
                info!(role = %role, "persona_prompt_observer: rollback applied");
            }
            Err(e) => {
                warn!(role = %role, error = %e, "persona_prompt_observer: rollback failed");
            }
        }
    }

    Ok(())
}

/// Issue `persona_prompt_rollback(role, 0)`. `0` means "most recent
/// superseded version" — the cleanest semantics for one-shot regression
/// recovery.
async fn call_rollback(
    http: &reqwest::Client,
    stdb_host: &str,
    hex_db: &str,
    role: &str,
) -> Result<(), String> {
    let url = format!("{}/v1/database/{}/call/persona_prompt_rollback", stdb_host, hex_db);
    let body = serde_json::json!([role, 0u64]);
    let res = http
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("transport: {}", e))?;
    if !res.status().is_success() {
        return Err(format!(
            "HTTP {}: {}",
            res.status(),
            res.text().await.unwrap_or_default()
        ));
    }
    Ok(())
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
    let body: serde_json::Value = res
        .json()
        .await
        .map_err(|e| format!("sql json parse: {}", e))?;
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

/// Pull microseconds-since-epoch from an STDB Timestamp cell. STDB serializes
/// `Timestamp` as `[micros_i64]` (single-element array for the Product type).
fn parse_timestamp_cell(v: &serde_json::Value) -> Option<i64> {
    if let Some(arr) = v.as_array() {
        if let Some(n) = arr.first().and_then(|x| x.as_i64()) {
            return Some(n);
        }
    }
    // Fallback: if STDB serialized as a debug string with the micros key
    // (older clients).
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_timestamp_from_array_cell() {
        let v = serde_json::json!([1779546367014507_i64]);
        assert_eq!(parse_timestamp_cell(&v), Some(1779546367014507));
    }

    #[test]
    fn parses_timestamp_from_debug_string() {
        let s = "Timestamp { __timestamp_micros_since_unix_epoch__: 1779546367014507 }";
        let v = serde_json::json!(s);
        assert_eq!(parse_timestamp_cell(&v), Some(1779546367014507));
    }

    #[test]
    fn rejects_unparseable_timestamp() {
        assert_eq!(parse_timestamp_cell(&serde_json::json!("hello")), None);
        assert_eq!(parse_timestamp_cell(&serde_json::json!(null)), None);
        assert_eq!(parse_timestamp_cell(&serde_json::json!({})), None);
    }
}
