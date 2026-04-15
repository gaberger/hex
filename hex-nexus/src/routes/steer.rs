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

/// One rule in the classifier table. `match_fn` returns true when the
/// (already-lowercased) directive matches. The table is iterated in
/// declaration order, so position == precedence.
struct Rule {
    label: &'static str,
    /// Human-readable description of the signals this rule keys on.
    /// Surfaces in the rules listing for debuggability.
    signals: &'static str,
    match_fn: fn(&str) -> bool,
}

/// Classifier rules in precedence order. Ordering is part of the spec —
/// constraint imperatives ("must"/"never"/"always") MUST precede weak
/// priority hints ("before"/"first") because a phrase like "tests must
/// pass before merge" is semantically a constraint, not a priority. A
/// pre-existing bug had priority before constraint and misfired on that
/// exact phrase.
const RULES: &[Rule] = &[
    Rule {
        label: "constraint_add",
        signals: "must / never / always / all+should",
        match_fn: |s| {
            s.contains("must")
                || s.contains("never")
                || s.contains("always")
                || (s.contains("all ") && s.contains(" should"))
        },
    },
    Rule {
        label: "approach_change",
        signals: "switch to / replace / use…instead",
        match_fn: |s| {
            s.contains("switch to")
                || s.contains("replace")
                || (s.contains("use ") && s.contains("instead"))
        },
    },
    Rule {
        label: "quality_preference",
        signals: "optimize for / prefer / focus on",
        match_fn: |s| {
            s.contains("optimize for") || s.contains("prefer") || s.contains("focus on")
        },
    },
    Rule {
        label: "priority_change",
        signals: "first / prioritize / before / demo / urgent",
        match_fn: |s| {
            s.contains("first")
                || s.contains("prioritize")
                || s.contains("before")
                || s.contains("demo")
                || s.contains("urgent")
        },
    },
];

/// Classify a directive into one of: constraint_add, approach_change,
/// quality_preference, priority_change, general.
///
/// Precedence is encoded in `RULES` order — first match wins.
fn classify_directive(directive: &str) -> &'static str {
    let lower = directive.to_lowercase();
    RULES
        .iter()
        .find(|r| (r.match_fn)(&lower))
        .map(|r| r.label)
        .unwrap_or("general")
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

    /// Regression: phrases combining a constraint imperative with a
    /// priority hint must classify as constraint, not priority.
    /// Documents the precedence rule that the table-ordering encodes.
    #[test]
    fn test_constraint_beats_priority_when_both_signals_present() {
        assert_eq!(
            classify_directive("Tests must pass before merge"),
            "constraint_add"
        );
        assert_eq!(
            classify_directive("Always run linter first"),
            "constraint_add"
        );
        assert_eq!(
            classify_directive("Never demo without auth"),
            "constraint_add"
        );
    }

    /// The rule table is the spec — assert it stays well-formed so a
    /// future contributor can't add a rule with an empty label or move
    /// a rule above constraint without noticing.
    #[test]
    fn test_rule_table_invariants() {
        assert!(!RULES.is_empty());
        assert_eq!(
            RULES[0].label,
            "constraint_add",
            "constraint MUST be first to win over weak priority hints"
        );
        for r in RULES {
            assert!(!r.label.is_empty());
            assert!(!r.signals.is_empty());
        }
    }
}
