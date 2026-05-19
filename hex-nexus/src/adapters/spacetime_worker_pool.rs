//! SpacetimeWorkerPoolAdapter — IWorkerPoolPort impl (ADR-2605190900 §1 + P3.4).
//!
//! Wraps the existing `worker_process` STDB table. The supervisor_tick
//! reducer (P3.2) keeps that table honest by reaping stale rows; this
//! adapter is the read-only consumer that the dispatcher gates on.
//!
//! Same transport pattern as spacetime_heartbeat.rs / spacetime_dead_letter.rs.

use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use hex_core::domain::worker_pool::ConsumerStatus;
use hex_core::ports::worker_pool::{IWorkerPoolPort, WorkerPoolError};
use serde_json::Value;

pub struct SpacetimeWorkerPoolAdapter {
    http: reqwest::Client,
    host: String,
    database: String,
}

impl SpacetimeWorkerPoolAdapter {
    pub fn new(host: String, database: String) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .pool_max_idle_per_host(4)
            .build()
            .expect("failed to build HTTP client");
        Self { http, database, host }
    }

    async fn sql_query(&self, query: &str) -> Result<Vec<Value>, WorkerPoolError> {
        let url = format!("{}/v1/database/{}/sql", self.host, self.database);
        let res = self
            .http
            .post(&url)
            .header("Content-Type", "text/plain")
            .body(query.to_string())
            .send()
            .await
            .map_err(|e| WorkerPoolError::BackendUnreachable(e.to_string()))?;
        if !res.status().is_success() {
            let status = res.status();
            let body = res.text().await.unwrap_or_else(|_| "<no body>".to_string());
            return Err(WorkerPoolError::Other(format!(
                "SQL query failed ({}): {}",
                status, body
            )));
        }
        let body: Value = res
            .json()
            .await
            .map_err(|e| WorkerPoolError::Other(format!("SQL parse: {}", e)))?;
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
impl IWorkerPoolPort for SpacetimeWorkerPoolAdapter {
    async fn ensure_consumer(
        &self,
        role: &str,
        ttl: Duration,
    ) -> Result<ConsumerStatus, WorkerPoolError> {
        // Pull every row for the role and bucket client-side. STDB SQL
        // can't do timestamp arithmetic on RFC 3339 strings reliably
        // (same constraint spacetime_heartbeat.rs::list_alive works
        // around). Volumes are small — even 50 workers per role is a
        // negligible scan.
        let safe = role.replace('\'', "''");
        let q = format!(
            "SELECT id, last_heartbeat, exited_at, status FROM worker_process \
             WHERE role = '{}' AND exited_at = ''",
            safe
        );
        let rows = self.sql_query(&q).await?;
        let now = Utc::now();
        let mut alive_count: u32 = 0;
        let mut youngest_stale_age_secs: Option<u64> = None;
        let mut stale_count: u32 = 0;

        for row in rows {
            let Some(arr) = row.as_array() else { continue };
            let get_str =
                |i: usize| arr.get(i).and_then(|v| v.as_str()).unwrap_or("").to_string();
            let last_heartbeat = get_str(1);
            let status = get_str(3);

            // Workers in "stopping" don't count toward alive — they're
            // mid-shutdown and the dispatcher shouldn't trust them.
            if status == "stopping" {
                continue;
            }

            let age_secs = match chrono::DateTime::parse_from_rfc3339(&last_heartbeat) {
                Ok(dt) => (now - dt.with_timezone(&Utc))
                    .to_std()
                    .map(|d| d.as_secs())
                    .unwrap_or(u64::MAX),
                Err(_) => u64::MAX,
            };

            if age_secs <= ttl.as_secs() {
                alive_count += 1;
            } else {
                stale_count += 1;
                youngest_stale_age_secs = Some(match youngest_stale_age_secs {
                    Some(prev) if prev <= age_secs => prev,
                    _ => age_secs,
                });
            }
        }

        if alive_count > 0 {
            Ok(ConsumerStatus::Alive {
                worker_count: alive_count,
            })
        } else if stale_count > 0 {
            Ok(ConsumerStatus::Degraded {
                worker_count: stale_count,
                oldest_heartbeat_age_secs: youngest_stale_age_secs.unwrap_or(0),
            })
        } else {
            Ok(ConsumerStatus::None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn adapter_is_object_safe() {
        let adapter = SpacetimeWorkerPoolAdapter::new(
            "http://127.0.0.1:3033".to_string(),
            "hex".to_string(),
        );
        let _boxed: Arc<dyn IWorkerPoolPort> = Arc::new(adapter);
    }
}
