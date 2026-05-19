//! SpacetimeDB dead-letter adapter — IDeadLetterPort impl (ADR-2605190900 P2.2).
//!
//! Wraps the `dead_letter_record` / `dead_letter_replay` reducers in
//! `hexflo-coordination` + a SQL read path for the dashboard `/api/dead-letter`
//! list. Same transport pattern as `spacetime_heartbeat.rs`.

use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use hex_core::domain::dead_letter::DeadLetterRecord;
use hex_core::ports::dead_letter::{DeadLetterError, IDeadLetterPort};
use serde_json::Value;

pub struct SpacetimeDeadLetterAdapter {
    http: reqwest::Client,
    host: String,
    database: String,
}

impl SpacetimeDeadLetterAdapter {
    pub fn new(host: String, database: String) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .pool_max_idle_per_host(4)
            .build()
            .expect("failed to build HTTP client");
        Self { http, database, host }
    }

    async fn call_reducer(&self, reducer: &str, args: Value) -> Result<(), DeadLetterError> {
        let url = format!("{}/v1/database/{}/call/{}", self.host, self.database, reducer);
        let res = self
            .http
            .post(&url)
            .json(&args)
            .send()
            .await
            .map_err(|e| DeadLetterError::BackendUnreachable(e.to_string()))?;
        if !res.status().is_success() {
            let status = res.status();
            let body = res.text().await.unwrap_or_else(|_| "<no body>".to_string());
            return Err(DeadLetterError::Other(format!(
                "reducer {} failed ({}): {}",
                reducer, status, body
            )));
        }
        Ok(())
    }

    async fn sql_query(&self, query: &str) -> Result<Vec<Value>, DeadLetterError> {
        let url = format!("{}/v1/database/{}/sql", self.host, self.database);
        let res = self
            .http
            .post(&url)
            .header("Content-Type", "text/plain")
            .body(query.to_string())
            .send()
            .await
            .map_err(|e| DeadLetterError::BackendUnreachable(e.to_string()))?;
        if !res.status().is_success() {
            let status = res.status();
            let body = res.text().await.unwrap_or_else(|_| "<no body>".to_string());
            return Err(DeadLetterError::Other(format!(
                "SQL query failed ({}): {}",
                status, body
            )));
        }
        let body: Value = res
            .json()
            .await
            .map_err(|e| DeadLetterError::Other(format!("SQL parse: {}", e)))?;
        let rows = body
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|first| first.get("rows"))
            .and_then(|r| r.as_array())
            .cloned()
            .unwrap_or_default();
        Ok(rows)
    }

    fn parse_row(row: &Value) -> Option<DeadLetterRecord> {
        let arr = row.as_array()?;
        let get_str = |i: usize| arr.get(i).and_then(|v| v.as_str()).unwrap_or("").to_string();
        let get_u32 = |i: usize| arr.get(i).and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        let get_i32 = |i: usize| arr.get(i).and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        Some(DeadLetterRecord {
            task_id: get_str(0),
            kind: get_str(1),
            payload: get_str(2),
            last_error: get_str(3),
            attempt_count: get_u32(4),
            first_failed_at: get_str(5),
            last_failed_at: get_str(6),
            original_priority: get_i32(7),
        })
    }
}

#[async_trait]
impl IDeadLetterPort for SpacetimeDeadLetterAdapter {
    async fn record(
        &self,
        task_id: &str,
        kind: &str,
        payload: &str,
        last_error: &str,
        attempt_count: u32,
        original_priority: i32,
    ) -> Result<(), DeadLetterError> {
        let timestamp = Utc::now().to_rfc3339();
        self.call_reducer(
            "dead_letter_record",
            serde_json::json!([
                task_id,
                kind,
                payload,
                last_error,
                attempt_count,
                original_priority,
                timestamp,
            ]),
        )
        .await
    }

    async fn list(&self) -> Result<Vec<DeadLetterRecord>, DeadLetterError> {
        let rows = self
            .sql_query(
                "SELECT task_id, kind, payload, last_error, attempt_count, \
                 first_failed_at, last_failed_at, original_priority \
                 FROM dead_letter",
            )
            .await?;
        let mut out = Vec::with_capacity(rows.len());
        for r in rows.iter() {
            if let Some(rec) = Self::parse_row(r) {
                out.push(rec);
            }
        }
        // Sort newest-failure-first — the dashboard shows the most
        // recent breakage at the top.
        out.sort_by(|a, b| b.last_failed_at.cmp(&a.last_failed_at));
        Ok(out)
    }

    async fn replay(&self, task_id: &str) -> Result<Option<DeadLetterRecord>, DeadLetterError> {
        // Read first so we can return the row shape to the caller (it
        // needs payload + priority to re-enqueue at the dispatcher API).
        // The replay reducer's delete-by-pk is idempotent; if the row
        // is gone by the time we delete, that's fine.
        let safe = task_id.replace('\'', "''");
        let q = format!(
            "SELECT task_id, kind, payload, last_error, attempt_count, \
             first_failed_at, last_failed_at, original_priority \
             FROM dead_letter WHERE task_id = '{}'",
            safe
        );
        let rows = self.sql_query(&q).await?;
        let rec = rows.first().and_then(Self::parse_row);
        if rec.is_some() {
            self.call_reducer("dead_letter_replay", serde_json::json!([task_id]))
                .await?;
        }
        Ok(rec)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn adapter_is_object_safe() {
        let adapter = SpacetimeDeadLetterAdapter::new(
            "http://127.0.0.1:3033".to_string(),
            "hex".to_string(),
        );
        let _boxed: Arc<dyn IDeadLetterPort> = Arc::new(adapter);
    }
}
