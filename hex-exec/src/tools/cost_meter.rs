//! `cost_meter` — token spend totals from the local usage log.
//!
//! Shows where inference budget goes, grouped by model, by the loop that spent
//! it, or by what it was spent on. Reads `.hex/local-store/usage.jsonl`, which
//! every completion appends to (ADR-2608241500 P3.2); before that it queried a
//! SpacetimeDB table the daemon maintained.

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
        "Read token-spend totals from the local usage log. Returns grouped \
         token counts and cost summaries over a time window. Use this to \
         understand cost distribution across models, roles, or intents."
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

        // Read the local usage log (ADR-2608241500 P3.2). This used to be an
        // HTTP SQL query against SpacetimeDB's `inference_log`, a table the
        // daemon wrote on the agent's behalf. In-process, the caller that
        // placed each request records its own accounting, so the meter reads
        // what the loop actually spent rather than what a daemon observed.
        use hex_core::ports::local_store::ILocalStore;
        use std::collections::HashMap;

        let cutoff = chrono::Utc::now() - chrono::Duration::seconds(window_secs as i64);
        let rows = match crate::store::FileStore::current().usage_since(&cutoff.to_rfc3339()) {
            Ok(r) => r,
            Err(e) => {
                return ToolResult::err(
                    format!("usage read failed: {}", e),
                    start.elapsed().as_millis() as u64,
                );
            }
        };

        let mut agg: HashMap<String, (u64, u64, f64)> = HashMap::new();
        let mut total_input = 0u64;
        let mut total_output = 0u64;
        let mut total_cost = 0.0f64;

        for row in &rows {
            let key = match group_by {
                "role" => row.role.clone(),
                "intent" => row.intent.clone(),
                _ => row.model.clone(),
            };
            let key = if key.is_empty() { "(unknown)".to_string() } else { key };
            let cost = row.cost_usd.unwrap_or(0.0);

            total_input += row.input_tokens;
            total_output += row.output_tokens;
            total_cost += cost;

            let entry = agg.entry(key).or_insert((0, 0, 0.0));
            entry.0 += row.input_tokens;
            entry.1 += row.output_tokens;
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
            "groups_truncated": rows.len() > MAX_GROUPS,
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
