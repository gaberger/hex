//! `cost_meter` — queries SpacetimeDB inference_log table for token spend totals.
//!
//! Used by CPO to understand cost distribution across models, roles, or intents.
//! Queries STDB hex database via HTTP POST /v1/database/hex/sql with a SQL
//! body, returns grouped spend summaries.

use async_trait::async_trait;
use reqwest::Client;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

use super::{Tool, ToolResult};

const DEFAULT_WINDOW_SECS: u64 = 3600;
const MAX_GROUPS: usize = 16;

pub struct CostMeter;

#[async_trait]
impl Tool for CostMeter {
    fn name(&self) -> &'static str {
        "cost_meter"
    }
    fn description(&self) -> &'static str {
        "Read token-spend totals from STDB inference_log table. Returns \
         grouped token counts and cost summaries over a time window. \
         Use this to understand cost distribution across models, roles, or intents."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "window_secs": {
                    "type": "integer",
                    "description": "Time window in seconds to query. Default 3600 (1 hour).",
                },
                "group_by": {
                    "type": "string",
                    "description": "Group dimension: 'model', 'role', or 'intent'. Default 'model'.",
                    "enum": ["model", "role", "intent"]
                }
            }
        })
    }
    async fn execute(&self, input: Value) -> ToolResult {
        let start = Instant::now();
        let window_secs = input
            .get("window_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(DEFAULT_WINDOW_SECS);
        let group_by = input
            .get("group_by")
            .and_then(|v| v.as_str())
            .unwrap_or("model");

        // Validate group_by
        if !["model", "role", "intent"].contains(&group_by) {
            return ToolResult::err(
                format!("invalid group_by: '{}'; must be model, role, or intent", group_by),
                start.elapsed().as_millis() as u64,
            );
        }

        // Build SQL query: SELECT group_by_col, sum(input_tokens), sum(output_tokens), sum(cost_usd)
        // FROM inference_log WHERE timestamp >= now() - window_secs GROUP BY group_by_col
        let sql = format!(
            "SELECT {}, SUM(input_tokens), SUM(output_tokens), SUM(cost_usd) \
             FROM inference_log \
             WHERE timestamp >= NOW() - INTERVAL '{} seconds' \
             GROUP BY {} \
             ORDER BY SUM(cost_usd) DESC",
            group_by, window_secs, group_by
        );

        // Query STDB via HTTP POST /v1/database/hex/sql with text/plain body
        let stdb_url = std::env::var("SPACETIME_URL")
            .unwrap_or_else(|_| "http://localhost:3000".to_string());
        let url = format!("{}/v1/database/hex/sql", stdb_url);

        let client = match Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                return ToolResult::err(
                    format!("failed to build HTTP client: {}", e),
                    start.elapsed().as_millis() as u64,
                );
            }
        };

        let resp = match client
            .post(&url)
            .header("Content-Type", "text/plain")
            .body(sql.clone())
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                return ToolResult::err(
                    format!("STDB query failed: {}", e),
                    start.elapsed().as_millis() as u64,
                );
            }
        };

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return ToolResult::err(
                format!("STDB returned {}: {}", status, body.chars().take(200).collect::<String>()),
                start.elapsed().as_millis() as u64,
            );
        }

        let json_resp: Value = match resp.json().await {
            Ok(j) => j,
            Err(e) => {
                return ToolResult::err(
                    format!("failed to parse STDB response: {}", e),
                    start.elapsed().as_millis() as u64,
                );
            }
        };

        // Parse STDB response. Expected shape: { "rows": [[key, inp, out, cost], ...] }
        let rows = match json_resp.get("rows").and_then(|v| v.as_array()) {
            Some(r) => r,
            None => {
                return ToolResult::err(
                    "STDB response missing 'rows' array",
                    start.elapsed().as_millis() as u64,
                );
            }
        };

        let mut groups: Vec<Value> = Vec::new();
        let mut total_input = 0u64;
        let mut total_output = 0u64;
        let mut total_cost = 0.0f64;

        for row in rows.iter().take(MAX_GROUPS) {
            let arr = match row.as_array() {
                Some(a) if a.len() >= 4 => a,
                _ => continue,
            };
            let key = arr[0].as_str().unwrap_or("(unknown)");
            let inp = arr[1].as_u64().unwrap_or(0);
            let out = arr[2].as_u64().unwrap_or(0);
            let cost = arr[3].as_f64().unwrap_or(0.0);

            total_input += inp;
            total_output += out;
            total_cost += cost;

            groups.push(json!({
                "key": key,
                "input_tokens": inp,
                "output_tokens": out,
                "cost_usd": cost,
            }));
        }

        let elapsed = start.elapsed().as_millis() as u64;
        let result = json!({
            "groups": groups,
            "total_input_tokens": total_input,
            "total_output_tokens": total_output,
            "total_cost_usd": total_cost,
            "window_secs": window_secs,
            "group_by": group_by,
            "truncated": rows.len() > MAX_GROUPS,
        });

        if rows.len() > MAX_GROUPS {
            ToolResult::ok_truncated(result, elapsed)
        } else {
            ToolResult::ok(result, elapsed)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn schema_has_group_by_enum() {
        let s = CostMeter.input_schema();
        let group_by = s.get("properties").and_then(|p| p.get("group_by")).unwrap();
        let enm = group_by.get("enum").and_then(|v| v.as_array()).unwrap();
        assert_eq!(enm.len(), 3);
        assert!(enm.iter().any(|v| v.as_str() == Some("model")));
    }
}
