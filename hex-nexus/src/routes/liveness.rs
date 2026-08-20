//! `/api/liveness` — dashboard badge endpoint (ADR-2026-05-19-0900 P5.2).
//!
//! Returns a snapshot of the chain's end-to-end health: nexus + STDB
//! reachable, supervisor_tick observed recently, at least one healthy
//! worker_process row. NOT a deep ping/pong — that's `hex doctor
//! liveness` (CLI-only, runs on demand).
//!
//! Dashboard top-bar reads this on a poll; the badge goes:
//!   green  — all checks pass
//!   yellow — STDB or nexus ok but no recent supervisor_tick / workers
//!   red    — STDB or nexus failed
//!
//! Cheap (one SQL query + one HTTP probe) so it can poll every 5s.

use std::sync::Arc;
use std::time::Duration;

use axum::{extract::State, http::StatusCode, response::Json};
use chrono::Utc;
use serde::Serialize;

use crate::state::AppState;

#[derive(Serialize)]
pub struct LivenessResponse {
    /// Single-word verdict consumed by the dashboard color logic.
    pub status: &'static str,
    pub probed_at: String,
    pub stdb: ProbeStage,
    pub nexus: ProbeStage,
    pub supervisor: ProbeStage,
    pub workers: ProbeStage,
}

#[derive(Serialize)]
pub struct ProbeStage {
    pub ok: bool,
    pub detail: String,
}

pub async fn get_liveness(State(_state): State<Arc<AppState>>) -> Result<Json<LivenessResponse>, StatusCode> {
    let stdb_host = std::env::var("HEX_SPACETIMEDB_HOST")
        .unwrap_or_else(|_| "http://127.0.0.1:3033".to_string());
    let database = std::env::var("HEX_STDB_DATABASE").unwrap_or_else(|_| "hex".to_string());
    let nexus_self = std::env::var("HEX_NEXUS_URL").unwrap_or_else(|_| "http://127.0.0.1:5555".to_string());

    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Stage 1: STDB /v1/ping
    let stdb = match http.get(format!("{stdb_host}/v1/ping")).send().await {
        Ok(r) if r.status().is_success() => ProbeStage {
            ok: true,
            detail: format!("{} reachable", stdb_host),
        },
        Ok(r) => ProbeStage {
            ok: false,
            detail: format!("STDB returned {}", r.status()),
        },
        Err(e) => ProbeStage {
            ok: false,
            detail: format!("STDB transport: {}", e),
        },
    };

    // Stage 2: nexus self-probe (we're inside nexus, so this is a sanity
    // check on the binding rather than a true external reach)
    let nexus = match http.get(format!("{nexus_self}/api/version")).send().await {
        Ok(r) if r.status().is_success() => ProbeStage {
            ok: true,
            detail: format!("{} responding", nexus_self),
        },
        Ok(r) => ProbeStage {
            ok: false,
            detail: format!("nexus self-probe {}", r.status()),
        },
        Err(e) => ProbeStage {
            ok: false,
            detail: format!("nexus self-probe transport: {}", e),
        },
    };

    // Stage 3: supervisor_tick recency — at least one tick within 60s.
    // Reads worker_pool_intent.updated_at as a proxy because the
    // supervisor_event log doesn't carry tick heartbeats today (a tick
    // that touches no pools writes nothing). Future: dedicated tick row.
    let supervisor = check_supervisor_recent(&http, &stdb_host, &database).await;

    // Stage 4: at least one healthy worker_process row.
    let workers = check_workers_healthy(&http, &stdb_host, &database).await;

    // Verdict:
    //   red    if stdb or nexus fail
    //   yellow if stdb + nexus ok but supervisor or workers fail
    //   green  otherwise
    let status: &'static str = if !stdb.ok || !nexus.ok {
        "red"
    } else if !supervisor.ok || !workers.ok {
        "yellow"
    } else {
        "green"
    };

    Ok(Json(LivenessResponse {
        status,
        probed_at: Utc::now().to_rfc3339(),
        stdb,
        nexus,
        supervisor,
        workers,
    }))
}

async fn check_supervisor_recent(http: &reqwest::Client, host: &str, db: &str) -> ProbeStage {
    // Pull the newest supervisor_event row and compare its ts to now.
    // Fresh-enough = within 60s (supervisor_tick fires every 10s, so 60s
    // gives 6× margin).
    let url = format!("{host}/v1/database/{db}/sql");
    let res = http
        .post(&url)
        .header("Content-Type", "text/plain")
        .body("SELECT ts FROM supervisor_event")
        .send()
        .await;
    let body: serde_json::Value = match res {
        Ok(r) if r.status().is_success() => match r.json().await {
            Ok(b) => b,
            Err(_) => {
                return ProbeStage {
                    ok: false,
                    detail: "supervisor_event response not JSON".to_string(),
                };
            }
        },
        Ok(r) => {
            return ProbeStage {
                ok: false,
                detail: format!("supervisor_event query HTTP {}", r.status()),
            };
        }
        Err(e) => {
            return ProbeStage {
                ok: false,
                detail: format!("supervisor_event query transport: {}", e),
            };
        }
    };
    let rows = body
        .as_array()
        .and_then(|a| a.first())
        .and_then(|f| f.get("rows"))
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();
    if rows.is_empty() {
        return ProbeStage {
            ok: false,
            detail: "no supervisor_event rows — supervisor_init not run? (`spacetime call supervisor_init`)".to_string(),
        };
    }
    // Find the newest ts. STDB stores Timestamp in the Debug format
    // "Timestamp { __timestamp_micros_since_unix_epoch__: NNNN }".
    // Extract integer micros for comparison.
    let now_micros = Utc::now().timestamp_micros();
    let mut newest_micros: i64 = 0;
    for row in &rows {
        let Some(arr) = row.as_array() else { continue };
        let ts = arr.first().and_then(|v| v.as_str()).unwrap_or("");
        let key = "__timestamp_micros_since_unix_epoch__:";
        if let Some(pos) = ts.find(key) {
            // The STDB Debug format renders as `: 1234567 }` — note the
            // leading space and the trailing ` }`. Trim leading whitespace
            // BEFORE the digit scan; the prior version found `.is_ascii_digit()`
            // false on the first char (space) and aborted with end=0.
            let tail = ts[pos + key.len()..].trim_start();
            let end = tail.find(|c: char| !c.is_ascii_digit()).unwrap_or(tail.len());
            if let Ok(n) = tail[..end].parse::<i64>() {
                if n > newest_micros {
                    newest_micros = n;
                }
            }
        }
    }
    // Guard against the unparseable-timestamp case — newest_micros == 0
    // means we have rows but couldn't parse any ts (e.g. STDB schema
    // change). Reporting "age = unix epoch from now" produces an
    // absurd 1_779_000_000s — useless. Be explicit instead.
    if newest_micros == 0 {
        return ProbeStage {
            ok: false,
            detail: format!(
                "{} supervisor_event row(s) present but timestamps unparseable — schema drift?",
                rows.len()
            ),
        };
    }
    let age_micros = now_micros - newest_micros;
    let age_secs = age_micros / 1_000_000;
    if age_secs <= 60 {
        ProbeStage {
            ok: true,
            detail: format!("last event {}s ago", age_secs),
        }
    } else {
        ProbeStage {
            ok: false,
            detail: format!("last event {}s ago (>60s — supervisor stalled?)", age_secs),
        }
    }
}

async fn check_workers_healthy(http: &reqwest::Client, host: &str, db: &str) -> ProbeStage {
    let url = format!("{host}/v1/database/{db}/sql");
    let res = http
        .post(&url)
        .header("Content-Type", "text/plain")
        .body("SELECT id FROM worker_process WHERE exited_at = ''")
        .send()
        .await;
    let body: serde_json::Value = match res {
        Ok(r) if r.status().is_success() => match r.json().await {
            Ok(b) => b,
            Err(_) => {
                return ProbeStage {
                    ok: false,
                    detail: "worker_process response not JSON".to_string(),
                };
            }
        },
        Ok(r) => {
            return ProbeStage {
                ok: false,
                detail: format!("worker_process query HTTP {}", r.status()),
            };
        }
        Err(e) => {
            return ProbeStage {
                ok: false,
                detail: format!("worker_process query transport: {}", e),
            };
        }
    };
    let count = body
        .as_array()
        .and_then(|a| a.first())
        .and_then(|f| f.get("rows"))
        .and_then(|r| r.as_array())
        .map(|rows| rows.len())
        .unwrap_or(0);
    if count > 0 {
        ProbeStage {
            ok: true,
            detail: format!("{} non-exited worker(s)", count),
        }
    } else {
        ProbeStage {
            ok: false,
            detail: "no live worker_process rows — components not registered with IHeartbeatPort yet".to_string(),
        }
    }
}
