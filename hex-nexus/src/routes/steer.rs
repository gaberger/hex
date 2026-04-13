//! REST endpoint for developer steering directives (ADR-2604131500 P1.4).
//!
//! POST /api/steer — classify and store a natural-language directive.

use axum::{
    extract::State,
    Json,
};
use http::StatusCode;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::orchestration::directive::execute_directive;
use crate::state::SharedState;

fn no_state_port() -> (StatusCode, Json<Value>) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({ "error": "IStatePort not initialized (no SpacetimeDB backend)" })),
    )
}

#[derive(Debug, Deserialize)]
pub struct SteerRequest {
    pub project_id: String,
    pub directive: String,
}

/// Classify a directive into one of: priority_change, approach_change,
/// constraint_add, quality_preference, general.
fn classify_directive(directive: &str) -> &'static str {
    let lower = directive.to_lowercase();

    // Priority signals
    if lower.contains("first")
        || lower.contains("prioritize")
        || lower.contains("before")
        || lower.contains("demo")
        || lower.contains("urgent")
    {
        return "priority_change";
    }

    // Approach change signals
    if lower.contains("switch to")
        || lower.contains("replace")
        || (lower.contains("use ") && lower.contains("instead"))
    {
        return "approach_change";
    }

    // Constraint signals
    if lower.contains("must")
        || lower.contains("never")
        || lower.contains("always")
        || (lower.contains("all ") && lower.contains(" should"))
    {
        return "constraint_add";
    }

    // Quality preference signals
    if lower.contains("optimize for")
        || lower.contains("prefer")
        || lower.contains("focus on")
    {
        return "quality_preference";
    }

    "general"
}

/// POST /api/steer — receive and classify a developer directive.
///
/// Stores the directive in HexFlo memory keyed by `steer:{project_id}:{timestamp}`.
pub async fn handle_steer(
    State(state): State<SharedState>,
    Json(body): Json<SteerRequest>,
) -> (StatusCode, Json<Value>) {
    let port = match &state.state_port {
        Some(p) => p,
        None => return no_state_port(),
    };

    if body.directive.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "directive must not be empty" })),
        );
    }

    let classification = classify_directive(&body.directive);
    let timestamp = chrono::Utc::now().to_rfc3339();

    // Store in HexFlo memory
    let key = format!("steer:{}:{}", body.project_id, timestamp);
    let value = serde_json::to_string(&json!({
        "project_id": body.project_id,
        "directive": body.directive,
        "classification": classification,
        "created_at": timestamp,
    }))
    .unwrap_or_default();

    if let Err(e) = port.hexflo_memory_store(&key, &value, "global").await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        );
    }

    // Execute the directive (P5.3) — priority reordering, constraint storage, etc.
    let execution = execute_directive(&state, &body.project_id, &body.directive, classification).await;

    match execution {
        Ok(result) => (
            StatusCode::OK,
            Json(json!({
                "ok": true,
                "classification": classification,
                "message": "Directive received and classified.",
                "execution": {
                    "applied": result.applied,
                    "summary": result.summary,
                    "tasks_reordered": result.tasks_reordered,
                    "agents_reassigned": result.agents_reassigned,
                },
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("Directive stored but execution failed: {}", e) })),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_priority() {
        assert_eq!(classify_directive("Do the login page first"), "priority_change");
        assert_eq!(classify_directive("Prioritize the API"), "priority_change");
        assert_eq!(classify_directive("This is urgent"), "priority_change");
    }

    #[test]
    fn test_classify_approach() {
        assert_eq!(classify_directive("Switch to PostgreSQL"), "approach_change");
        assert_eq!(classify_directive("Use Redis instead"), "approach_change");
        assert_eq!(classify_directive("Replace the HTTP client"), "approach_change");
    }

    #[test]
    fn test_classify_constraint() {
        assert_eq!(classify_directive("Tests must pass before merge"), "constraint_add");
        assert_eq!(classify_directive("Never use unsafe code"), "constraint_add");
        assert_eq!(classify_directive("Always log errors"), "constraint_add");
    }

    #[test]
    fn test_classify_quality() {
        assert_eq!(classify_directive("Optimize for latency"), "quality_preference");
        assert_eq!(classify_directive("Prefer simplicity"), "quality_preference");
        assert_eq!(classify_directive("Focus on test coverage"), "quality_preference");
    }

    #[test]
    fn test_classify_general() {
        assert_eq!(classify_directive("Update the README"), "general");
        assert_eq!(classify_directive("Add a changelog entry"), "general");
    }
}
