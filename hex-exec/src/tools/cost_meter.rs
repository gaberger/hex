//! `cost_meter` — queries SpacetimeDB inference_log table for token spend totals.
//!
//! Used by CPO to understand cost distribution across models, roles, or intents.
//! Queries STDB hex database via HTTP POST /v1/database/hex/sql with a SQL
//! body, returns grouped spend summaries.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::Instant;

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
        "Read token-spend totals from the local inference log. Returns \
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

        // A local read. Was a SQL query against SpacetimeDB's `inference_log` over HTTP, which
        // meant asking "what have I spent" required a database that the daemon owned — and after
        // Phase 1 there is no daemon logging inference in the first place. Rows come back in the
        // same [key, input, output, cost, created_at] shape, so the aggregation below is unchanged.
        let rows_owned: Vec<Value> = crate::local_store::spend_rows(&group_by, 5000);
        let rows = &rows_owned;

        // Aggregate in Rust: STDB SQL doesn't have SUM/GROUP BY. Each row is
        // [group_key, input_tokens, output_tokens, cost_usd, created_at].
        // cost_usd is stored as String (parse to f64). Time-window filter via
        // string-prefix compare on ISO-8601 created_at.
        use std::collections::HashMap;
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let cutoff_secs = now_secs.saturating_sub(window_secs as i64);

        let mut agg: HashMap<String, (u64, u64, f64)> = HashMap::new();
        let mut total_input = 0u64;
        let mut total_output = 0u64;
        let mut total_cost = 0.0f64;

        for row in rows.iter() {
            let arr = match row.as_array() {
                Some(a) if a.len() >= 5 => a,
                _ => continue,
            };
            // Best-effort window filter: created_at is ISO-8601 string;
            // parse to unix secs if possible, otherwise include in window.
            let created_at = arr[4].as_str().unwrap_or("");
            let row_secs = chrono::DateTime::parse_from_rfc3339(created_at)
                .map(|dt| dt.timestamp())
                .unwrap_or(now_secs);
            if row_secs < cutoff_secs {
                continue;
            }
            let key = arr[0].as_str().unwrap_or("(unknown)").to_string();
            let inp = arr[1].as_u64().unwrap_or(0);
            let out = arr[2].as_u64().unwrap_or(0);
            // cost_usd stored as String per inference_log schema
            let cost: f64 = arr[3].as_str().and_then(|s| s.parse().ok()).unwrap_or(0.0);

            total_input += inp;
            total_output += out;
            total_cost += cost;

            let entry = agg.entry(key).or_insert((0, 0, 0.0));
            entry.0 += inp;
            entry.1 += out;
            entry.2 += cost;
        }

        let mut groups: Vec<Value> = agg
            .into_iter()
            .map(|(key, (inp, out, cost))| {
                json!({
                    "key": key,
                    "input_tokens": inp,
                    "output_tokens": out,
                    "cost_usd": cost,
                })
            })
            .collect();
        // Sort descending by cost
        groups.sort_by(|a, b| {
            let ac = a.get("cost_usd").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let bc = b.get("cost_usd").and_then(|v| v.as_f64()).unwrap_or(0.0);
            bc.partial_cmp(&ac).unwrap_or(std::cmp::Ordering::Equal)
        });
        groups.truncate(MAX_GROUPS);

        let elapsed = start.elapsed().as_millis() as u64;
        let result = json!({
            "groups": groups,
            "totals": {
                "input_tokens": total_input,
                "output_tokens": total_output,
                "cost_usd": total_cost,
            },
            // Back-compat aliases for older callers
            "total_input_tokens": total_input,
            "total_output_tokens": total_output,
            "total_cost_usd": total_cost,
            "window_secs": window_secs,
            "group_by": group_by,
            "rows_scanned": rows.len(),
            "rows_truncated": rows.len() >= 5000,
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
