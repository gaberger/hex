//! Parse the structured-output classifier response emitted by the persona
//! inference call (ADR-2026-05-17-2030 Phase 1, workplan
//! wp-sop-pipeline-redesign-phase-1 P1.2).
//!
//! Replaces the free-form `Confirm:`/`Silent` prose contract with a typed
//! JSON-schema classifier. The parser is the single chokepoint that
//! enforces:
//!
//!   * JSON shape — malformed input becomes `InvariantError::MalformedJson`
//!     so the caller can run a bounded reparse-budget loop (P3) instead of
//!     silently dropping.
//!   * The `from=operator` invariant — operator-direct asks are forbidden
//!     from receiving a `defer` or `reject` decision (rule from ADR
//!     §Schema Changes #3, partially shipped in d2b3f06e).
//!   * Per-decision required fields — accept needs a `tool_plan`, route
//!     needs a `target_persona`, clarify needs a `question`, reject/defer
//!     need a `reason`, request_tool needs a `tool_spec`. A decision that
//!     names itself but omits its evidence is treated as a contract
//!     violation, not a silent drop.
//!
//! Zero I/O — pure parse + validate. The async strict-JSON adapter that
//! wraps inference + reparse budget lives in `classifier_adapter.rs` (P3).

use super::classifier_types::{ClassifierDecision, ClassifierResponse};

use thiserror::Error;

/// Reasons a classifier response is rejected before dispatch.
///
/// The variants partition off-contract output into three operational
/// classes the caller treats differently:
///
///   * `MalformedJson` is retryable — the inference adapter re-prompts
///     the model with a stricter hint up to the reparse budget (P3).
///   * `DecisionNotAllowedForOperator` is a schema violation that bypasses
///     the retry loop — re-prompting cannot change the user identity, so
///     the response escalates straight to the operator inbox (P5).
///   * `MissingRequiredField` is also non-retryable in the same sense:
///     the decision names itself but the supporting evidence is absent;
///     escalate rather than spin.
#[derive(Debug, Error)]
pub enum InvariantError {
    #[error("classifier output is not valid JSON: {0}")]
    MalformedJson(String),

    #[error(
        "decision {0:?} is not allowed on from=operator traffic \
         (operator-direct asks may only be accept / route / clarify / request_tool)"
    )]
    DecisionNotAllowedForOperator(ClassifierDecision),

    #[error("decision {decision:?} is missing required field `{field}`")]
    MissingRequiredField {
        decision: ClassifierDecision,
        field: &'static str,
    },
}

/// Convert raw LLM output into a validated `ClassifierResponse`.
///
/// `from_operator` carries the upstream identity check — the parser uses
/// it to enforce the schema invariant that operator-direct traffic never
/// receives a defer/reject verdict.
pub trait ClassifierParser: Send + Sync {
    fn parse(
        &self,
        raw: &str,
        from_operator: bool,
    ) -> Result<ClassifierResponse, InvariantError>;
}

/// Default parser: strip optional markdown fences, then `serde_json::from_str`.
///
/// LLMs frequently wrap a JSON object in ```` ```json … ``` ```` fences even
/// when instructed not to (especially the smaller tier-1 models). Stripping
/// fences in the parser keeps the strict re-prompt path reserved for genuine
/// shape problems rather than burning a retry on a cosmetic wrapper.
#[derive(Debug, Default, Clone, Copy)]
pub struct SerdeJsonClassifierParser;

impl SerdeJsonClassifierParser {
    pub fn new() -> Self {
        Self
    }

    /// Strip a leading ```` ```json ```` / ```` ``` ```` fence and the matching
    /// trailing ```` ``` ```` if present. Pass-through for unwrapped JSON.
    fn strip_fences(raw: &str) -> &str {
        let s = raw.trim();
        let inner = if let Some(rest) = s.strip_prefix("```json") {
            rest
        } else if let Some(rest) = s.strip_prefix("```JSON") {
            rest
        } else if let Some(rest) = s.strip_prefix("```") {
            rest
        } else {
            return s;
        };
        let inner = inner.trim_start_matches(|c: char| c == '\n' || c == '\r');
        let inner = inner.strip_suffix("```").unwrap_or(inner);
        inner.trim()
    }
}

impl ClassifierParser for SerdeJsonClassifierParser {
    fn parse(
        &self,
        raw: &str,
        from_operator: bool,
    ) -> Result<ClassifierResponse, InvariantError> {
        let json = Self::strip_fences(raw);
        let resp: ClassifierResponse = serde_json::from_str(json)
            .map_err(|e| InvariantError::MalformedJson(e.to_string()))?;

        if from_operator
            && matches!(
                resp.decision,
                ClassifierDecision::Defer | ClassifierDecision::Reject
            )
        {
            return Err(InvariantError::DecisionNotAllowedForOperator(resp.decision));
        }

        match &resp.decision {
            ClassifierDecision::Accept => {
                if resp.tool_plan.is_none() {
                    return Err(InvariantError::MissingRequiredField {
                        decision: ClassifierDecision::Accept,
                        field: "tool_plan",
                    });
                }
            }
            ClassifierDecision::Route => {
                if resp
                    .target_persona
                    .as_deref()
                    .map(str::trim)
                    .map(str::is_empty)
                    .unwrap_or(true)
                {
                    return Err(InvariantError::MissingRequiredField {
                        decision: ClassifierDecision::Route,
                        field: "target_persona",
                    });
                }
            }
            ClassifierDecision::Clarify => {
                if resp
                    .question
                    .as_deref()
                    .map(str::trim)
                    .map(str::is_empty)
                    .unwrap_or(true)
                {
                    return Err(InvariantError::MissingRequiredField {
                        decision: ClassifierDecision::Clarify,
                        field: "question",
                    });
                }
            }
            ClassifierDecision::Reject => {
                if resp
                    .reason
                    .as_deref()
                    .map(str::trim)
                    .map(str::is_empty)
                    .unwrap_or(true)
                {
                    return Err(InvariantError::MissingRequiredField {
                        decision: ClassifierDecision::Reject,
                        field: "reason",
                    });
                }
            }
            ClassifierDecision::Defer => {
                if resp
                    .reason
                    .as_deref()
                    .map(str::trim)
                    .map(str::is_empty)
                    .unwrap_or(true)
                {
                    return Err(InvariantError::MissingRequiredField {
                        decision: ClassifierDecision::Defer,
                        field: "reason",
                    });
                }
            }
            ClassifierDecision::RequestTool => {
                if resp.tool_spec.is_none() {
                    return Err(InvariantError::MissingRequiredField {
                        decision: ClassifierDecision::RequestTool,
                        field: "tool_spec",
                    });
                }
            }
        }

        Ok(resp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parser() -> SerdeJsonClassifierParser {
        SerdeJsonClassifierParser::new()
    }

    // ---------------------------------------------------------------------
    // Happy-path fixtures — one per ClassifierDecision variant.
    //
    // Decision strings use snake_case per the wire contract documented in
    // wp-sop-pipeline-redesign-phase-1 (decision ∈ accept/defer/route/
    // clarify/reject/request_tool). P1.1 derives serde with the matching
    // `#[serde(rename_all = "snake_case")]` so these fixtures round-trip.
    // ---------------------------------------------------------------------

    #[test]
    fn happy_path_accept_with_tool_plan() {
        let raw = r#"{
            "decision": "accept",
            "tool_plan": [
                {"tool": "code_patch", "intent": "edit org_responder dispatch"}
            ],
            "cost_usd": 0.0012
        }"#;
        let out = parser().parse(raw, false).expect("accept should parse");
        assert!(matches!(out.decision, ClassifierDecision::Accept));
        assert!(out.tool_plan.is_some());
        assert_eq!(out.tool_plan.as_ref().unwrap().len(), 1);
    }

    #[test]
    fn happy_path_defer_from_peer() {
        let raw = r#"{
            "decision": "defer",
            "reason": "blocked on P2.1 STDB table",
            "cost_usd": 0.0009
        }"#;
        let out = parser().parse(raw, false).expect("defer from peer parses");
        assert!(matches!(out.decision, ClassifierDecision::Defer));
        assert_eq!(out.reason.as_deref(), Some("blocked on P2.1 STDB table"));
    }

    #[test]
    fn happy_path_route_to_persona() {
        let raw = r#"{
            "decision": "route",
            "target_persona": "cto",
            "cost_usd": 0.0010
        }"#;
        let out = parser().parse(raw, true).expect("route from operator parses");
        assert!(matches!(out.decision, ClassifierDecision::Route));
        assert_eq!(out.target_persona.as_deref(), Some("cto"));
    }

    #[test]
    fn happy_path_clarify_with_question() {
        let raw = r#"{
            "decision": "clarify",
            "question": "Which subsystem are you asking about — nexus or hex-cli?",
            "cost_usd": 0.0008
        }"#;
        let out = parser().parse(raw, true).expect("clarify parses");
        assert!(matches!(out.decision, ClassifierDecision::Clarify));
        assert!(out.question.as_deref().unwrap().contains("nexus"));
    }

    #[test]
    fn happy_path_reject_from_peer() {
        let raw = r#"{
            "decision": "reject",
            "reason": "request is out of scope for ports layer",
            "cost_usd": 0.0007
        }"#;
        let out = parser().parse(raw, false).expect("reject from peer parses");
        assert!(matches!(out.decision, ClassifierDecision::Reject));
    }

    #[test]
    fn happy_path_request_tool_with_spec() {
        let raw = r#"{
            "decision": "request_tool",
            "tool_spec": {"name": "grep_workplan", "rationale": "need wp dep lookups"},
            "cost_usd": 0.0011
        }"#;
        let out = parser().parse(raw, true).expect("request_tool parses");
        assert!(matches!(out.decision, ClassifierDecision::RequestTool));
        assert!(out.tool_spec.is_some());
    }

    // ---------------------------------------------------------------------
    // Invariant-violation fixtures — 4 cases covering both classes.
    // ---------------------------------------------------------------------

    #[test]
    fn operator_invariant_rejects_defer() {
        let raw = r#"{
            "decision": "defer",
            "reason": "I'll get to it later",
            "cost_usd": 0.0009
        }"#;
        let err = parser()
            .parse(raw, true)
            .expect_err("operator+defer must violate invariant");
        match err {
            InvariantError::DecisionNotAllowedForOperator(ClassifierDecision::Defer) => {}
            other => panic!("expected DecisionNotAllowedForOperator(Defer), got {other:?}"),
        }
    }

    #[test]
    fn operator_invariant_rejects_reject() {
        let raw = r#"{
            "decision": "reject",
            "reason": "not doing it",
            "cost_usd": 0.0009
        }"#;
        let err = parser()
            .parse(raw, true)
            .expect_err("operator+reject must violate invariant");
        match err {
            InvariantError::DecisionNotAllowedForOperator(ClassifierDecision::Reject) => {}
            other => panic!("expected DecisionNotAllowedForOperator(Reject), got {other:?}"),
        }
    }

    #[test]
    fn missing_field_accept_without_tool_plan() {
        let raw = r#"{
            "decision": "accept",
            "cost_usd": 0.0009
        }"#;
        let err = parser()
            .parse(raw, false)
            .expect_err("accept without tool_plan must fail");
        match err {
            InvariantError::MissingRequiredField {
                decision: ClassifierDecision::Accept,
                field: "tool_plan",
            } => {}
            other => panic!("expected MissingRequiredField(Accept, tool_plan), got {other:?}"),
        }
    }

    #[test]
    fn missing_field_route_without_target_persona() {
        // Present-but-empty string counts as missing — operators cannot
        // route to "".
        let raw = r#"{
            "decision": "route",
            "target_persona": "",
            "cost_usd": 0.0009
        }"#;
        let err = parser()
            .parse(raw, false)
            .expect_err("route without target_persona must fail");
        match err {
            InvariantError::MissingRequiredField {
                decision: ClassifierDecision::Route,
                field: "target_persona",
            } => {}
            other => panic!("expected MissingRequiredField(Route, target_persona), got {other:?}"),
        }
    }

    // ---------------------------------------------------------------------
    // Malformed-JSON fixtures — 3 cases.
    // ---------------------------------------------------------------------

    #[test]
    fn malformed_plain_prose() {
        let err = parser()
            .parse("I think we should accept this", false)
            .expect_err("plain prose is not JSON");
        assert!(matches!(err, InvariantError::MalformedJson(_)));
    }

    #[test]
    fn malformed_truncated_object() {
        let err = parser()
            .parse(r#"{"decision": "accept", "tool_plan": ["#, false)
            .expect_err("truncated JSON is malformed");
        assert!(matches!(err, InvariantError::MalformedJson(_)));
    }

    #[test]
    fn malformed_unquoted_keys() {
        let err = parser()
            .parse(r#"{decision: "accept", cost_usd: 0.0}"#, false)
            .expect_err("unquoted keys are not valid JSON");
        assert!(matches!(err, InvariantError::MalformedJson(_)));
    }

    // ---------------------------------------------------------------------
    // Strip-fences regression — LLMs wrap JSON in ```json ``` even after
    // being told not to; we strip rather than burn a reparse retry.
    // ---------------------------------------------------------------------

    #[test]
    fn strips_markdown_fences_before_parse() {
        let raw = "```json\n{\"decision\": \"clarify\", \"question\": \"what file?\", \"cost_usd\": 0.0}\n```";
        let out = parser().parse(raw, true).expect("fenced JSON should parse");
        assert!(matches!(out.decision, ClassifierDecision::Clarify));
    }
}
