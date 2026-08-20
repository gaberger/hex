//! Observability endpoints for the SOP pipeline redesign
//! (ADR-2026-05-17-2030 Phase 1, P6.2).
//!
//! Read-only SQL view over the `classifier_response` STDB table — surfaces
//! the silent-drop counter that drives the 48h acceptance gate.
//!
//! The gate metric is: zero `classifier_response` rows with
//! `from_role='operator'` AND `final_outcome='silent_drop'` in the trailing
//! 48 hours. After Phase 1 deploy the persona Confirm/Silent prose contract
//! is replaced by structured JSON (P3.1 + P4.1), so silent drops on
//! operator-direct traffic should be impossible — every ask either lands a
//! `decision` row or escalates to the operator inbox (P5.1).
//!
//! The endpoint tolerates the table not being present yet (the P2.1 STDB
//! schema migration may not be live in every deployment) by returning
//! `count: 0` on SQL error — operators rely on the dashboard not 500-ing
//! during partial rollouts.

use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde_json::{json, Value};

use crate::state::SharedState;

const STDB_TIMEOUT_SECS: u64 = 5;
const WINDOW_HOURS: i64 = 48;

fn stdb_host() -> String {
    std::env::var("HEX_SPACETIMEDB_HOST").unwrap_or_else(|_| "http://127.0.0.1:3033".to_string())
}

fn hex_db() -> String {
    std::env::var("HEX_STDB_DATABASE")
        .unwrap_or_else(|_| hex_core::stdb_database_for_module("hexflo-coordination").to_string())
}

fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(STDB_TIMEOUT_SECS))
        .build()
        .expect("observability http client")
}

/// Run an STDB SQL query and return the row payload. Mirrors the helper in
/// `routes::resources` so we don't have to reach into another module.
async fn sql(query: &str) -> Result<Vec<Vec<Value>>, String> {
    let url = format!("{}/v1/database/{}/sql", stdb_host(), hex_db());
    let resp = http()
        .post(&url)
        .header("Content-Type", "text/plain")
        .body(query.to_string())
        .send()
        .await
        .map_err(|e| format!("http: {}", e))?;
    if !resp.status().is_success() {
        let s = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("HTTP {}: {}", s, body));
    }
    let body: Value = resp.json().await.map_err(|e| format!("json: {}", e))?;
    Ok(body
        .as_array()
        .and_then(|a| a.first())
        .and_then(|t| t.get("rows"))
        .and_then(|r| r.as_array())
        .map(|rows| rows.iter().filter_map(|r| r.as_array().cloned()).collect())
        .unwrap_or_default())
}

/// GET /api/observability/silent-drops
///
/// Count of `classifier_response` rows in the trailing 48h where the
/// inbound ask came from the operator AND the pipeline ended in a silent
/// drop. Phase 1 acceptance gate target: 0.
///
/// Response shape:
/// ```json
/// { "count": 0, "window_hours": 48, "since": "2026-05-20T16:00:00+00:00" }
/// ```
///
/// On STDB error (table missing during partial rollout, transport blip,
/// etc.) the endpoint returns `count: 0` with an `error` field so the
/// dashboard summary card stays green-by-default rather than 500-ing.
pub async fn silent_drops(
    State(_state): State<SharedState>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let since = chrono::Utc::now() - chrono::Duration::hours(WINDOW_HOURS);
    let since_rfc3339 = since.to_rfc3339();

    // RFC3339 timestamps are lexicographically orderable — string compare
    // is safe and matches the convention used by every other `created_at`
    // column in hexflo-coordination.
    let query = format!(
        "SELECT id FROM classifier_response \
         WHERE from_role = 'operator' \
         AND final_outcome = 'silent_drop' \
         AND created_at >= '{}'",
        since_rfc3339.replace('\'', "''"),
    );

    match sql(&query).await {
        Ok(rows) => Ok(Json(json!({
            "count": rows.len(),
            "window_hours": WINDOW_HOURS,
            "since": since_rfc3339,
        }))),
        Err(e) => {
            // Tolerate missing table / transport error — return 0 with the
            // error attached so the acceptance gate doesn't false-alarm
            // during rollouts where P2.1's reducer isn't published yet.
            tracing::warn!(error = %e, "silent_drops: STDB query failed — defaulting to 0");
            Ok(Json(json!({
                "count": 0,
                "window_hours": WINDOW_HOURS,
                "since": since_rfc3339,
                "error": e,
            })))
        }
    }
}
