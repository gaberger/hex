//! Structured classifier output types for the SOP pipeline redesign
//! (ADR-2026-05-17-2030 Phase 1).
//!
//! Replaces the prior `org_responder` Confirm/Silent prose contract with a
//! JSON-schema classifier response. Wire-format decisions are snake_case
//! strings (`accept`, `defer`, `route`, `clarify`, `reject`, `request_tool`)
//! to match the STDB `classifier_response.decision` column and the LLM
//! JSON schema documented in the ADR.
//!
//! These are pure types — no I/O, no STDB, no inference calls. The
//! `ClassifierParser` trait that turns raw LLM text into a
//! `ClassifierResponse` lives in `classifier_parser.rs` (P1.2).

use serde::{Deserialize, Serialize};

/// Verdict produced by the operator-direct classifier for an inbound ask.
///
/// Serialized as a snake_case string so the on-wire JSON and the STDB
/// `classifier_response.decision` column share one vocabulary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClassifierDecision {
    /// Persona will act on the ask now. `tool_plan` is required.
    Accept,
    /// Persona is busy / blocked; ask is deferred. `reason` is required.
    Defer,
    /// Forward to a different persona. `target_persona` is required.
    Route,
    /// Persona needs more information. `question` is required.
    Clarify,
    /// Persona refuses the ask outright. `reason` is required.
    Reject,
    /// Persona needs a new tool to proceed. `tool_spec` is required.
    RequestTool,
}

/// A single step in the persona's planned tool invocation chain.
///
/// `tool` is the tool name (e.g. `repo_grep`, `cargo_check`); `intent` is
/// the natural-language reason the persona chose it. Both are required —
/// empty strings are valid but discouraged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolPlanStep {
    pub tool: String,
    pub intent: String,
}

/// Structured response emitted by an operator-direct classifier call.
///
/// Per-decision required fields (enforced by `ClassifierParser` in P1.2):
/// - `Accept` → `tool_plan`
/// - `Defer` / `Reject` → `reason`
/// - `Route` → `target_persona`
/// - `Clarify` → `question`
/// - `RequestTool` → `tool_spec`
///
/// `cost_usd` accounts the inference spend for this classifier call and
/// is always populated (zero when offline / mocked).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClassifierResponse {
    pub decision: ClassifierDecision,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_plan: Option<Vec<ToolPlanStep>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_persona: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub question: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_spec: Option<serde_json::Value>,
    #[serde(default)]
    pub cost_usd: f32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn roundtrip(resp: &ClassifierResponse) -> ClassifierResponse {
        let s = serde_json::to_string(resp).expect("serialize");
        serde_json::from_str(&s).expect("deserialize")
    }

    #[test]
    fn decision_serializes_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&ClassifierDecision::RequestTool).unwrap(),
            "\"request_tool\""
        );
        assert_eq!(
            serde_json::to_string(&ClassifierDecision::Accept).unwrap(),
            "\"accept\""
        );
    }

    #[test]
    fn decision_deserializes_from_snake_case() {
        let d: ClassifierDecision = serde_json::from_str("\"defer\"").unwrap();
        assert_eq!(d, ClassifierDecision::Defer);
        let d: ClassifierDecision = serde_json::from_str("\"request_tool\"").unwrap();
        assert_eq!(d, ClassifierDecision::RequestTool);
    }

    #[test]
    fn unknown_decision_string_fails_to_deserialize() {
        let r: Result<ClassifierDecision, _> = serde_json::from_str("\"silent\"");
        assert!(r.is_err(), "Silent must not be a valid classifier decision");
    }

    #[test]
    fn accept_roundtrips() {
        let resp = ClassifierResponse {
            decision: ClassifierDecision::Accept,
            tool_plan: Some(vec![ToolPlanStep {
                tool: "repo_grep".into(),
                intent: "find call sites".into(),
            }]),
            reason: None,
            target_persona: None,
            question: None,
            tool_spec: None,
            cost_usd: 0.0021,
        };
        assert_eq!(roundtrip(&resp), resp);
    }

    #[test]
    fn defer_roundtrips() {
        let resp = ClassifierResponse {
            decision: ClassifierDecision::Defer,
            tool_plan: None,
            reason: Some("blocked on STDB outage".into()),
            target_persona: None,
            question: None,
            tool_spec: None,
            cost_usd: 0.0005,
        };
        assert_eq!(roundtrip(&resp), resp);
    }

    #[test]
    fn route_roundtrips() {
        let resp = ClassifierResponse {
            decision: ClassifierDecision::Route,
            tool_plan: None,
            reason: None,
            target_persona: Some("ciso".into()),
            question: None,
            tool_spec: None,
            cost_usd: 0.0007,
        };
        assert_eq!(roundtrip(&resp), resp);
    }

    #[test]
    fn clarify_roundtrips() {
        let resp = ClassifierResponse {
            decision: ClassifierDecision::Clarify,
            tool_plan: None,
            reason: None,
            target_persona: None,
            question: Some("Which workplan should I target?".into()),
            tool_spec: None,
            cost_usd: 0.0009,
        };
        assert_eq!(roundtrip(&resp), resp);
    }

    #[test]
    fn reject_roundtrips() {
        let resp = ClassifierResponse {
            decision: ClassifierDecision::Reject,
            tool_plan: None,
            reason: Some("out of persona scope".into()),
            target_persona: None,
            question: None,
            tool_spec: None,
            cost_usd: 0.0004,
        };
        assert_eq!(roundtrip(&resp), resp);
    }

    #[test]
    fn request_tool_roundtrips() {
        let resp = ClassifierResponse {
            decision: ClassifierDecision::RequestTool,
            tool_plan: None,
            reason: None,
            target_persona: None,
            question: None,
            tool_spec: Some(json!({
                "name": "stdb_query",
                "args_schema": { "sql": "string" }
            })),
            cost_usd: 0.0011,
        };
        assert_eq!(roundtrip(&resp), resp);
    }

    #[test]
    fn missing_optional_fields_deserialize_as_none() {
        let raw = r#"{"decision":"accept","cost_usd":0.0}"#;
        let resp: ClassifierResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(resp.decision, ClassifierDecision::Accept);
        assert!(resp.tool_plan.is_none());
        assert!(resp.reason.is_none());
        assert!(resp.target_persona.is_none());
        assert!(resp.question.is_none());
        assert!(resp.tool_spec.is_none());
    }

    #[test]
    fn none_fields_are_skipped_in_serialization() {
        let resp = ClassifierResponse {
            decision: ClassifierDecision::Accept,
            tool_plan: None,
            reason: None,
            target_persona: None,
            question: None,
            tool_spec: None,
            cost_usd: 0.0,
        };
        let s = serde_json::to_string(&resp).unwrap();
        assert!(!s.contains("tool_plan"));
        assert!(!s.contains("reason"));
        assert!(!s.contains("target_persona"));
        assert!(!s.contains("question"));
        assert!(!s.contains("tool_spec"));
        assert!(s.contains("\"decision\":\"accept\""));
        assert!(s.contains("\"cost_usd\""));
    }

    #[test]
    fn tool_plan_step_roundtrips() {
        let step = ToolPlanStep {
            tool: "cargo_check".into(),
            intent: "verify the patch compiles".into(),
        };
        let s = serde_json::to_string(&step).unwrap();
        let back: ToolPlanStep = serde_json::from_str(&s).unwrap();
        assert_eq!(back, step);
    }
}
