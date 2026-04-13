//! GET /api/briefing — paginated project briefing endpoint (ADR-2604131500 P1.1).
//!
//! Returns a compact summary of recent events grouped by session, with
//! configurable pagination to keep response size under 2 KB by default.

use axum::{
    extract::{Query, State},
    Json,
};
use http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

use crate::state::SharedState;

// ── Query Parameters ────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct BriefingParams {
    /// Max events per session/project group. Default 5. 0 = unlimited.
    pub limit: Option<u32>,
    /// ISO-8601 timestamp — only include events created after this time.
    pub since: Option<String>,
    /// When true (default), truncate event bodies to 200 chars.
    pub summary: Option<bool>,
}

// ── Response Types ──────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct SessionBriefing {
    session_id: String,
    events: Vec<Value>,
    total_events: usize,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    truncated: bool,
}

#[derive(Debug, Serialize)]
struct BriefingResponse {
    sessions: Vec<SessionBriefing>,
    total_sessions: usize,
    params: BriefingParamsEcho,
}

#[derive(Debug, Serialize)]
struct BriefingParamsEcho {
    limit: u32,
    since: Option<String>,
    summary: bool,
}

// ── Constants ───────────────────────────────────────────────────────────

const DEFAULT_LIMIT: u32 = 5;
const SUMMARY_TRUNCATE_LEN: usize = 200;

// ── Handler ─────────────────────────────────────────────────────────────

/// GET /api/briefing — paginated event briefing grouped by session.
pub async fn get_briefing(
    State(state): State<SharedState>,
    Query(params): Query<BriefingParams>,
) -> (StatusCode, Json<Value>) {
    let limit = params.limit.unwrap_or(DEFAULT_LIMIT);
    let summary = params.summary.unwrap_or(true);
    let since = params.since.clone();

    // Fetch all events from the ring buffer (up to 1000).
    // We use limit=0 internally to get everything, then paginate per-session.
    let all_events = state.event_adapter.list_events(None, 500).await;

    // Filter by `since` if provided.
    let filtered: Vec<_> = if let Some(ref since_ts) = since {
        all_events
            .into_iter()
            .filter(|e| e.created_at.as_str() > since_ts.as_str())
            .collect()
    } else {
        all_events
    };

    // Group by session_id.
    let mut groups: HashMap<String, Vec<&crate::ports::events::ToolEvent>> = HashMap::new();
    for event in &filtered {
        groups
            .entry(event.session_id.clone())
            .or_default()
            .push(event);
    }

    // Build per-session briefings.
    let mut sessions: Vec<SessionBriefing> = groups
        .into_iter()
        .map(|(session_id, events)| {
            let total_events = events.len();
            let is_unlimited = limit == 0;
            let take_count = if is_unlimited { total_events } else { limit as usize };
            let truncated = !is_unlimited && total_events > take_count;

            let briefing_events: Vec<Value> = events
                .into_iter()
                .take(take_count)
                .map(|e| {
                    let mut obj = json!({
                        "id": e.id,
                        "event_type": e.event_type,
                        "created_at": e.created_at,
                    });
                    let map = obj.as_object_mut().unwrap();

                    if let Some(ref tool) = e.tool_name {
                        map.insert("tool_name".into(), json!(tool));
                    }
                    if let Some(ref agent) = e.agent_id {
                        map.insert("agent_id".into(), json!(agent));
                    }
                    if let Some(exit) = e.exit_code {
                        map.insert("exit_code".into(), json!(exit));
                    }
                    if let Some(dur) = e.duration_ms {
                        map.insert("duration_ms".into(), json!(dur));
                    }
                    if let Some(ref model) = e.model_used {
                        map.insert("model_used".into(), json!(model));
                    }
                    if let Some(ref layer) = e.hex_layer {
                        map.insert("hex_layer".into(), json!(layer));
                    }
                    if let Some(inp) = e.input_tokens {
                        map.insert("input_tokens".into(), json!(inp));
                    }
                    if let Some(out) = e.output_tokens {
                        map.insert("output_tokens".into(), json!(out));
                    }
                    if let Some(cost) = e.cost_usd {
                        map.insert("cost_usd".into(), json!(cost));
                    }

                    // Include bodies only when summary=false; otherwise truncate.
                    if summary {
                        if let Some(ref input) = e.input_json {
                            map.insert(
                                "input_json".into(),
                                json!(truncate_str(input, SUMMARY_TRUNCATE_LEN)),
                            );
                        }
                        if let Some(ref result) = e.result_json {
                            map.insert(
                                "result_json".into(),
                                json!(truncate_str(result, SUMMARY_TRUNCATE_LEN)),
                            );
                        }
                    } else {
                        if let Some(ref input) = e.input_json {
                            map.insert("input_json".into(), json!(input));
                        }
                        if let Some(ref result) = e.result_json {
                            map.insert("result_json".into(), json!(result));
                        }
                    }

                    obj
                })
                .collect();

            SessionBriefing {
                session_id,
                events: briefing_events,
                total_events,
                truncated,
            }
        })
        .collect();

    // Sort sessions by most-recent event first.
    sessions.sort_by(|a, b| {
        let a_latest = a.events.first().and_then(|e| e.get("created_at")).and_then(|v| v.as_str());
        let b_latest = b.events.first().and_then(|e| e.get("created_at")).and_then(|v| v.as_str());
        b_latest.cmp(&a_latest)
    });

    let total_sessions = sessions.len();

    let response = BriefingResponse {
        sessions,
        total_sessions,
        params: BriefingParamsEcho {
            limit,
            since,
            summary,
        },
    };

    (StatusCode::OK, Json(json!(response)))
}

/// Truncate a string to `max_len` chars, appending "…" if truncated.
fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        return s.to_string();
    }
    let mut end = max_len;
    while !s.is_char_boundary(end) && end > 0 {
        end -= 1;
    }
    format!("{}…", &s[..end])
}
