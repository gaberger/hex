//! SpacetimeDB heartbeat adapter — IHeartbeatPort impl (ADR-2605190900 P1.3).
//!
//! Wraps the `worker_process_register` / `worker_process_status` /
//! `worker_process_deregister` reducers in `hexflo-coordination` so any
//! long-running nexus-side component can publish liveness rows without
//! knowing STDB's HTTP transport.
//!
//! Pattern mirrors `spacetime_agent_comm.rs`:
//!   - reqwest HTTP client with 5s timeout, pool_max_idle = 4
//!   - POST /v1/database/<db>/call/<reducer> with JSON args
//!   - SQL queries via POST /v1/database/<db>/sql (text/plain)
//!
//! No Drop impl auto-deregisters: STDB calls are async + fallible and
//! Drop is sync. Callers issue `deregister()` explicitly on graceful
//! shutdown; missed deregisters get cleaned up by `supervisor_tick`
//! reaping stale heartbeats.

use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use hex_core::domain::heartbeat::{HeartbeatStatus, WorkerHeartbeat};
use hex_core::ports::heartbeat::{HeartbeatError, IHeartbeatPort};
use serde_json::Value;

/// HTTP client for the `hex` SpacetimeDB module (which hosts
/// hexflo-coordination — see ADR-2026-04-05-0900 multi-database model).
pub struct SpacetimeHeartbeatAdapter {
    http: reqwest::Client,
    host: String,
    database: String,
}

impl SpacetimeHeartbeatAdapter {
    /// `host` is the base STDB URL (e.g. `http://127.0.0.1:3033`).
    /// `database` is typically `hex` — the module that publishes the
    /// hexflo-coordination tables in production today.
    pub fn new(host: String, database: String) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .pool_max_idle_per_host(4)
            .build()
            .expect("failed to build HTTP client");

        Self { http, database, host }
    }

    async fn call_reducer(&self, reducer: &str, args: Value) -> Result<(), HeartbeatError> {
        let url = format!("{}/v1/database/{}/call/{}", self.host, self.database, reducer);

        let res = self
            .http
            .post(&url)
            .json(&args)
            .send()
            .await
            .map_err(|e| HeartbeatError::BackendUnreachable(e.to_string()))?;

        if !res.status().is_success() {
            let status = res.status();
            let body = res.text().await.unwrap_or_else(|_| "<no body>".to_string());
            return Err(HeartbeatError::Other(format!(
                "reducer {} failed ({}): {}",
                reducer, status, body
            )));
        }
        Ok(())
    }

    async fn sql_query(&self, query: &str) -> Result<Vec<Value>, HeartbeatError> {
        let url = format!("{}/v1/database/{}/sql", self.host, self.database);
        let res = self
            .http
            .post(&url)
            .header("Content-Type", "text/plain")
            .body(query.to_string())
            .send()
            .await
            .map_err(|e| HeartbeatError::BackendUnreachable(e.to_string()))?;

        if !res.status().is_success() {
            let status = res.status();
            let body = res.text().await.unwrap_or_else(|_| "<no body>".to_string());
            return Err(HeartbeatError::Other(format!(
                "SQL query failed ({}): {}",
                status, body
            )));
        }

        let body: Value = res
            .json()
            .await
            .map_err(|e| HeartbeatError::Other(format!("SQL parse: {}", e)))?;

        // STDB SQL response shape: [{ "rows": [...] }, ...]
        let rows = body
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|first| first.get("rows"))
            .and_then(|r| r.as_array())
            .cloned()
            .unwrap_or_default();

        Ok(rows)
    }
}

#[async_trait]
impl IHeartbeatPort for SpacetimeHeartbeatAdapter {
    async fn register(
        &self,
        worker_id: &str,
        pool_id: &str,
        role: &str,
        pid: u32,
        host: &str,
    ) -> Result<String, HeartbeatError> {
        // worker_process_register(id, pool_id, role, host, pid: i64)
        // — see spacetime-modules/hexflo-coordination/src/lib.rs:4399.
        // The STDB reducer is upsert by id, so a re-register with the
        // same worker_id refreshes the row without losing started_at.
        self.call_reducer(
            "worker_process_register",
            serde_json::json!([
                worker_id,
                pool_id,
                role,
                host,
                pid as i64,
            ]),
        )
        .await?;
        Ok(worker_id.to_string())
    }

    async fn beat(
        &self,
        worker_id: &str,
        status: HeartbeatStatus,
        evidence: Option<&str>,
    ) -> Result<(), HeartbeatError> {
        // worker_process_status updates last_heartbeat + status + evidence
        // in one call. The older worker_process_heartbeat reducer (only
        // updates last_heartbeat) is left in place for backward compat
        // with components that don't yet self-report status.
        self.call_reducer(
            "worker_process_status",
            serde_json::json!([
                worker_id,
                status.as_str(),
                evidence.unwrap_or(""),
            ]),
        )
        .await
    }

    async fn deregister(&self, worker_id: &str) -> Result<(), HeartbeatError> {
        // worker_process_deregister is idempotent on the STDB side.
        self.call_reducer(
            "worker_process_deregister",
            serde_json::json!([worker_id]),
        )
        .await
    }

    async fn list_alive(
        &self,
        role: &str,
        ttl: Duration,
    ) -> Result<Vec<WorkerHeartbeat>, HeartbeatError> {
        // Pull every row for the role, then filter client-side by TTL.
        // STDB's SQL surface doesn't reliably evaluate timestamp math
        // on string columns (last_heartbeat is stored as RFC 3339 text
        // — see worker_process schema), so we age-out in Rust.
        let safe_role = role.replace('\'', "''");
        let q = format!(
            "SELECT id, pool_id, role, pid, host, started_at, last_heartbeat, status, evidence \
             FROM worker_process WHERE role = '{}'",
            safe_role
        );
        let rows = self.sql_query(&q).await?;
        let now = Utc::now();
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            // STDB SQL returns row as an array of column values in the
            // SELECT order. The `id` column is the worker handle.
            let Some(arr) = row.as_array() else { continue };
            let get_str = |i: usize| arr.get(i).and_then(|v| v.as_str()).unwrap_or("").to_string();
            let get_i64 = |i: usize| arr.get(i).and_then(|v| v.as_i64()).unwrap_or(0);

            let last_hb = get_str(6);
            // Client-side TTL filter — parse the RFC3339 timestamp, skip
            // if older than `now - ttl`. Malformed timestamps are treated
            // as expired so a broken row doesn't masquerade as alive.
            let alive = match chrono::DateTime::parse_from_rfc3339(&last_hb) {
                Ok(dt) => (now - dt.with_timezone(&Utc)).to_std().map(|d| d <= ttl).unwrap_or(false),
                Err(_) => false,
            };
            if !alive {
                continue;
            }
            let status_str = get_str(7);
            let status = HeartbeatStatus::parse(&status_str).unwrap_or(HeartbeatStatus::Healthy);
            out.push(WorkerHeartbeat {
                worker_id: get_str(0),
                pool_id: get_str(1),
                role: get_str(2),
                pid: get_i64(3) as u32,
                host: get_str(4),
                registered_at: get_str(5),
                last_heartbeat_at: last_hb,
                status,
                evidence: {
                    let e = get_str(8);
                    if e.is_empty() { None } else { Some(e) }
                },
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn adapter_is_object_safe() {
        // Compile-time: the adapter satisfies dyn IHeartbeatPort.
        let adapter = SpacetimeHeartbeatAdapter::new(
            "http://127.0.0.1:3033".to_string(),
            "hex".to_string(),
        );
        let _boxed: Arc<dyn IHeartbeatPort> = Arc::new(adapter);
    }
}
