//! `memory_search` — surfaces hex memory entries during SOP GROUND.
//!
//! Closes ADR-2026-05-10-2200 (Proposed) / wp-memory-search-tool.
//!
//! Without this tool, the SOP GROUND phase has no read access to the
//! `hexflo_memory` table — personas never see lessons, gaps, project
//! decisions, or any prior-session context that the operator or other
//! agents have written via `hex memory store`. Memory becomes
//! decorative: operators query it from the CLI but personas can't.
//!
//! With this tool wired into `ground_for_intent`, each SOP run pulls
//! the lessons + gaps + project entries that match the intent keywords
//! before the LLM enters REASON, so the model can ground against
//! "what we already know" instead of relearning every session.
//!
//! Backing store: `~/.hex/memory.jsonl` through `hex_exec::local_store`.
//! It was the `hexflo_memory` SpacetimeDB table reached over HTTP, until
//! ADR-2608241500 — which made reading a lesson depend on a database.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::Instant;

use super::{Tool, ToolResult};

/// Default cap on returned memory entries. Memory rows are small
/// (~500 bytes each per the seed conventions); 12 keeps the prompt
/// budget modest while covering the typical SOP context window.
const MAX_RESULTS_DEFAULT: usize = 12;
const MAX_RESULTS_HARD_CAP: usize = 50;

pub struct MemorySearch;

#[async_trait]
impl Tool for MemorySearch {
    fn name(&self) -> &'static str {
        "memory_search"
    }
    fn description(&self) -> &'static str {
        "Search hex persistent memory (the local memory feed) by substring \
         match on key OR value. Use in the GROUND phase to surface prior \
         lessons, known gaps, and project context the operator or other \
         agents have stored. Key prefix conventions: lesson:* (don't-repeat-this), \
         gap:* (known issues), project:* (in-flight context), decision:* (recorded \
         choices). Returns at most 12 entries by default."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Substring to match against both key and value. \
                                    Use a prefix like 'lesson:' to filter to one category, \
                                    or a keyword like 'workplan' to find anything related. \
                                    Empty string returns recent entries (capped).",
                },
                "max_results": {
                    "type": "integer",
                    "description": "Max number of entries to return. Default 12, cap 50.",
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, input: Value) -> ToolResult {
        let start = Instant::now();
        let query = match input.get("query").and_then(|v| v.as_str()) {
            Some(q) => q,
            None => {
                return ToolResult::err(
                    "missing required field 'query'",
                    start.elapsed().as_millis() as u64,
                )
            }
        };
        let max_results = input
            .get("max_results")
            .and_then(|v| v.as_u64())
            .map(|n| (n as usize).min(MAX_RESULTS_HARD_CAP))
            .unwrap_or(MAX_RESULTS_DEFAULT);

        // Local memory (ADR-2608241500). This was a GET to the daemon's
        // /api/hexflo/memory/search, which read a SpacetimeDB table — so
        // grounding the loop in past lessons needed a database to be up.
        let all_results: Vec<Value> = crate::local_store::memory_search(query)
            .into_iter()
            .map(|(key, value)| json!({ "key": key, "value": value }))
            .collect();

        let total = all_results.len();
        let truncated = total > max_results;
        let results: Vec<Value> = all_results.into_iter().take(max_results).collect();

        let output = json!({
            "query": query,
            "total_matches": total,
            "returned": results.len(),
            "results": results,
        });
        if truncated {
            ToolResult::ok_truncated(output, start.elapsed().as_millis() as u64)
        } else {
            ToolResult::ok(output, start.elapsed().as_millis() as u64)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_metadata_correct() {
        let t = MemorySearch;
        assert_eq!(t.name(), "memory_search");
        assert!(t.description().contains("hex persistent memory"));
        let schema = t.input_schema();
        assert_eq!(schema.get("type").and_then(|v| v.as_str()), Some("object"));
        let required = schema.get("required").and_then(|v| v.as_array()).unwrap();
        assert!(required.iter().any(|v| v.as_str() == Some("query")));
    }

    #[test]
    fn caps_max_results_at_hard_cap() {
        // Verify the cap math, not the HTTP — actual execute() requires
        // a running nexus and is exercised by the integration probe.
        let input = json!({"query": "x", "max_results": 1000});
        let n = input
            .get("max_results")
            .and_then(|v| v.as_u64())
            .map(|n| (n as usize).min(MAX_RESULTS_HARD_CAP))
            .unwrap_or(MAX_RESULTS_DEFAULT);
        assert_eq!(n, MAX_RESULTS_HARD_CAP);
    }
}
