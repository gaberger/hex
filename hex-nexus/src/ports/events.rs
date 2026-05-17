//! Shared data types for tool-call event observability (ADR-2604012137).
//!
//! These types are used by the in-memory event adapter (adapters/events.rs)
//! and the REST route handlers (routes/events.rs).

use serde::{Deserialize, Serialize};

/// A single tool-call event row from the `tool_events` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolEvent {
    pub id: i64,
    pub session_id: String,
    pub agent_id: Option<String>,
    pub event_type: String,
    pub tool_name: Option<String>,
    /// Tool input JSON, truncated to 4 KB.
    pub input_json: Option<String>,
    /// Tool result JSON, truncated to 4 KB.
    pub result_json: Option<String>,
    pub exit_code: Option<i32>,
    pub duration_ms: Option<i64>,
    // Full audit fields
    pub model_used: Option<String>,
    pub context_strategy: Option<String>,
    pub rl_action: Option<String>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cost_usd: Option<f64>,
    pub hex_layer: Option<String>,
    pub created_at: String,
}

/// Request body for `POST /api/events`.
///
/// Sent by `hex hook observe pre/post` after reading the Claude Code
/// PreToolUse / PostToolUse hook JSON from stdin.
#[derive(Debug, Deserialize)]
pub struct InsertEventRequest {
    pub session_id: String,
    pub agent_id: Option<String>,
    pub event_type: String,
    pub tool_name: Option<String>,
    pub input_json: Option<String>,
    pub result_json: Option<String>,
    pub exit_code: Option<i32>,
    pub duration_ms: Option<i64>,
    pub model_used: Option<String>,
    pub context_strategy: Option<String>,
    pub rl_action: Option<String>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cost_usd: Option<f64>,
    pub hex_layer: Option<String>,
}
