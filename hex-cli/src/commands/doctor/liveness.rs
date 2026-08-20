//! `hex doctor liveness` — end-to-end self-test of the autonomous loop
//! (ADR-2026-05-19-0900 P5).
//!
//! Walks the synthetic ping/pong shape:
//!   1. POST a sched-task `kind=ping payload=<uuid>` to `/api/sched/queue`.
//!   2. Wait for `improver_event { kind=pong, related=<uuid> }` to land
//!      in STDB (with a 60s deadline).
//!   3. Report per-stage timing so the operator sees which link broke
//!      when a stage misses.
//!
//! The receiving side (`pong` emission) is wired by the dispatcher
//! when it encounters `kind=ping` — that wire-up lives in sched.rs,
//! which is on CRITICAL_FILES. Documented at the bottom of this module.
//!
//! Until the operator lands the dispatcher side, `hex doctor liveness`
//! reports `STDB:ok`, `sched-enqueue:ok`, then the wait stage times out.
//! That is the diagnostic the ADR's postmortem section captured: "the
//! system was supposed to surface its own brokenness and didn't." Now
//! it does — visibly, in <60s, with a named broken stage.

use std::time::Duration;

use chrono::Utc;
use colored::Colorize;
use uuid::Uuid;

/// How long to wait for the pong before giving up.
const DEADLINE_SECS: u64 = 60;

/// How often to poll for the pong row in STDB.
const POLL_INTERVAL_MS: u64 = 500;

#[derive(Debug, Clone)]
struct StageReport {
    name: &'static str,
    duration: Duration,
    outcome: &'static str,
    detail: Option<String>,
}

pub async fn run() -> anyhow::Result<()> {
    let ping_id = Uuid::new_v4().to_string();
    let start = std::time::Instant::now();
    let mut stages = Vec::with_capacity(4);

    println!("{} hex doctor liveness", "\u{2b21}".cyan());
    println!("  ping_id: {}", ping_id.dimmed());
    println!();

    // Stage 1: STDB reachable.
    let s = std::time::Instant::now();
    let stdb_ok = probe_stdb().await;
    stages.push(StageReport {
        name: "stdb",
        duration: s.elapsed(),
        outcome: if stdb_ok { "ok" } else { "FAIL" },
        detail: if stdb_ok { None } else { Some("STDB ping failed — endpoint discovery hierarchy exhausted".to_string()) },
    });
    if !stdb_ok {
        report(&stages, start.elapsed(), false);
        return Err(anyhow::anyhow!("liveness: STDB unreachable"));
    }

    // Stage 2: nexus reachable.
    let s = std::time::Instant::now();
    let nexus_ok = probe_nexus().await;
    stages.push(StageReport {
        name: "nexus",
        duration: s.elapsed(),
        outcome: if nexus_ok { "ok" } else { "FAIL" },
        detail: if nexus_ok { None } else { Some("nexus /api/version unreachable".to_string()) },
    });
    if !nexus_ok {
        report(&stages, start.elapsed(), false);
        return Err(anyhow::anyhow!("liveness: nexus unreachable"));
    }

    // Stage 3: enqueue ping task.
    let s = std::time::Instant::now();
    let enqueue_result = enqueue_ping(&ping_id).await;
    let enqueue_ok = enqueue_result.is_ok();
    stages.push(StageReport {
        name: "sched-enqueue",
        duration: s.elapsed(),
        outcome: if enqueue_ok { "ok" } else { "FAIL" },
        detail: enqueue_result.err(),
    });
    if !enqueue_ok {
        report(&stages, start.elapsed(), false);
        return Err(anyhow::anyhow!("liveness: failed to enqueue ping"));
    }

    // Stage 4: wait for pong.
    let s = std::time::Instant::now();
    let pong_ok = wait_for_pong(&ping_id, Duration::from_secs(DEADLINE_SECS)).await;
    stages.push(StageReport {
        name: "loop-pong",
        duration: s.elapsed(),
        outcome: if pong_ok { "ok" } else { "FAIL" },
        detail: if pong_ok {
            None
        } else {
            Some(format!(
                "no improver_event {{kind=pong, related={}}} within {}s — dispatcher never claimed the ping task; check /api/worker-pool/check?role=ping",
                ping_id, DEADLINE_SECS
            ))
        },
    });

    report(&stages, start.elapsed(), pong_ok);
    if pong_ok {
        Ok(())
    } else {
        Err(anyhow::anyhow!("liveness: loop did not close within deadline"))
    }
}

fn report(stages: &[StageReport], total: Duration, overall_ok: bool) {
    println!();
    for s in stages {
        let badge = if s.outcome == "ok" { "✓".green() } else { "✗".red() };
        println!(
            "  {} {:<18} {:>7.0}ms  {}",
            badge,
            s.name,
            s.duration.as_millis() as f64,
            s.outcome
        );
        if let Some(detail) = &s.detail {
            println!("    {}", detail.dimmed());
        }
    }
    println!();
    let verdict = if overall_ok {
        "PASS — loop closed end-to-end".green().bold()
    } else {
        "FAIL — first broken stage named above".red().bold()
    };
    println!("  {verdict}  (total {:.1}s)", total.as_secs_f64());
}

// ── stage helpers ──────────────────────────────────────────────────

async fn probe_stdb() -> bool {
    let host = std::env::var("HEX_SPACETIMEDB_HOST")
        .unwrap_or_else(|_| "http://127.0.0.1:3033".to_string());
    let http = match reqwest::Client::builder().timeout(Duration::from_secs(2)).build() {
        Ok(c) => c,
        Err(_) => return false,
    };
    http.get(format!("{host}/v1/ping"))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

async fn probe_nexus() -> bool {
    let host = nexus_base_url();
    let http = match reqwest::Client::builder().timeout(Duration::from_secs(2)).build() {
        Ok(c) => c,
        Err(_) => return false,
    };
    http.get(format!("{host}/api/version"))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

async fn enqueue_ping(ping_id: &str) -> Result<(), String> {
    // /api/hexflo/memory requires X-Hex-Agent-Id — easiest to delegate to
    // the existing `hex brain enqueue` path which already plumbs the
    // registered-session auth. This also matches the operator's actual
    // queue surface: the liveness probe rides the same path real work
    // takes, so a successful ping really did exercise the dispatcher.
    let output = tokio::process::Command::new("hex")
        .args(["brain", "enqueue", "hex-command", "--priority", "9", "--"])
        .arg(format!("ping {}", ping_id))
        .output()
        .await
        .map_err(|e| format!("spawn `hex brain enqueue`: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(format!(
            "hex brain enqueue failed (exit {:?}): {} {}",
            output.status.code(),
            stderr.trim(),
            stdout.trim()
        ));
    }
    Ok(())
}

async fn wait_for_pong(ping_id: &str, deadline: Duration) -> bool {
    let stdb_host = std::env::var("HEX_SPACETIMEDB_HOST")
        .unwrap_or_else(|_| "http://127.0.0.1:3033".to_string());
    let database = std::env::var("HEX_STDB_DATABASE").unwrap_or_else(|_| "hex".to_string());
    let http = match reqwest::Client::builder().timeout(Duration::from_secs(2)).build() {
        Ok(c) => c,
        Err(_) => return false,
    };
    let url = format!("{stdb_host}/v1/database/{database}/sql");
    let safe = ping_id.replace('\'', "''");
    let query = format!(
        "SELECT id FROM improver_event WHERE kind = 'pong' AND scope = '{}' LIMIT 1",
        safe
    );

    let start = std::time::Instant::now();
    while start.elapsed() < deadline {
        if let Ok(res) = http
            .post(&url)
            .header("Content-Type", "text/plain")
            .body(query.clone())
            .send()
            .await
        {
            if res.status().is_success() {
                if let Ok(body) = res.json::<serde_json::Value>().await {
                    let has_row = body
                        .as_array()
                        .and_then(|a| a.first())
                        .and_then(|f| f.get("rows"))
                        .and_then(|r| r.as_array())
                        .map(|rows| !rows.is_empty())
                        .unwrap_or(false);
                    if has_row {
                        return true;
                    }
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
    }
    false
}

fn nexus_base_url() -> String {
    std::env::var("HEX_NEXUS_URL").unwrap_or_else(|_| "http://127.0.0.1:5555".to_string())
}

// Suppress the unused import warning during the period the binding
// isn't called by main.rs (operator wire-up TBD).
#[allow(dead_code)]
fn _ts_placeholder() -> String {
    Utc::now().to_rfc3339()
}

// ============================================================
// Operator wire-up (one line in main.rs — protected file)
// ============================================================
//
// In hex-cli/src/main.rs, inside the Commands::Doctor match arm, add
// the `liveness` branch alongside `composition`:
//
//     Commands::Doctor { verbose, fix, check } => {
//         if check.as_deref() == Some("composition") {
//             doctor::composition::run_composition_check().await;
//             Ok(())
//         } else if check.as_deref() == Some("liveness") {
//             doctor::liveness::run().await
//         } else {
//             doctor::run_doctor(verbose, fix).await
//         }
//     }
//
// Tier-C operator-only edit because main.rs is on CRITICAL_FILES.
//
// Dispatcher side (also Tier-C — sched.rs is protected):
// When the dispatcher pops a sched-task with kind="ping", it should
// call improver_event_record with kind="pong", source="Liveness",
// scope=<payload uuid>, related=0 inside the same dispatch tick. That
// is the entire "pong handler" — it's a no-op in execution terms; the
// loop-closure signal IS the improver_event row landing in STDB.
// ============================================================
