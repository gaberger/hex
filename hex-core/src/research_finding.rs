//! Structured research finding schema (ADR / workplan `wp-idle-research-swarm`, P1.3).
//!
//! `Finding` is the canonical record emitted by research analysts (architecture,
//! code-quality, drift, performance, etc.) when the idle-research swarm sweeps
//! the project. Findings are persisted as YAML so they stay diff-friendly in
//! version control and trivially editable by humans reviewing the swarm output.

use serde::{Deserialize, Serialize};

/// Severity classification for a research finding.
///
/// Ordered from least to most urgent. Deserialized from lowercase strings
/// (`"info"`, `"low"`, `"medium"`, `"high"`, `"critical"`) so YAML authored by
/// hand reads naturally.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Low,
    Medium,
    High,
    Critical,
}

/// Research domain this finding belongs to.
///
/// Kept open-ended via the `Other` variant so new analyst types can be added
/// without a breaking change to the schema. Known domains get first-class
/// variants for static checking and better YAML ergonomics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Domain {
    Architecture,
    CodeQuality,
    Drift,
    Performance,
    Security,
    Documentation,
    #[serde(untagged)]
    Other(String),
}

/// What kind of follow-up action the swarm suggests for a finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    /// Open a new draft workplan to address the finding.
    DraftWorkplan,
    /// Attach the finding to an existing workplan as a new task.
    AmendWorkplan,
    /// File an ADR proposing a decision.
    DraftAdr,
    /// Persist the finding into the cross-session memory store under the
    /// `idle-sweep` namespace. Used for low-urgency observations that should
    /// stay searchable but don't justify a workplan or ADR.
    Memory,
    /// No action required beyond surfacing the finding to a human.
    Informational,
}

/// The action suggested by the analyst that produced this finding.
///
/// `draft_ref` is an opaque pointer — typically a draft workplan filename
/// (e.g. `drafts/draft-2604241147-fix-llama-cpp-inference-provider.json`) or an
/// ADR id. When `kind` is `Informational`, `draft_ref` is `None`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuggestedAction {
    pub kind: ActionKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft_ref: Option<String>,
}

/// A structured finding produced by a research analyst.
///
/// Serialized as YAML for on-disk storage; also serializable to JSON for
/// transport over the nexus bus. `evidence` is a freeform list of short
/// strings — typically file:line references or commands whose output
/// motivated the finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub id: String,
    pub domain: Domain,
    pub severity: Severity,
    pub title: String,
    #[serde(default)]
    pub evidence: Vec<String>,
    pub suggested_action: SuggestedAction,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Finding {
        Finding {
            id: "f-2604241200-arch-cross-adapter".into(),
            domain: Domain::Architecture,
            severity: Severity::High,
            title: "adapter→adapter import violates hex rules".into(),
            evidence: vec![
                "adapters/primary/cli.rs:42 imports adapters/secondary/db.rs".into(),
                "hex analyze . --json: violations[0].rule = ADR-hex-adapter-isolation".into(),
            ],
            suggested_action: SuggestedAction {
                kind: ActionKind::DraftWorkplan,
                draft_ref: Some("drafts/draft-2604241200-fix-cross-adapter-import.json".into()),
            },
        }
    }

    #[test]
    fn yaml_round_trip() {
        let finding = sample();
        let yaml = serde_yaml::to_string(&finding).expect("serialize yaml");
        let back: Finding = serde_yaml::from_str(&yaml).expect("deserialize yaml");
        assert_eq!(finding, back);
    }

    #[test]
    fn yaml_shape_is_human_readable() {
        // Guards the on-disk spelling that humans will hand-edit. If any of
        // these strings change we want the test to force a conscious update.
        let finding = sample();
        let yaml = serde_yaml::to_string(&finding).expect("serialize yaml");
        assert!(yaml.contains("domain: architecture"), "yaml = {yaml}");
        assert!(yaml.contains("severity: high"), "yaml = {yaml}");
        assert!(yaml.contains("kind: draft_workplan"), "yaml = {yaml}");
    }

    #[test]
    fn informational_omits_draft_ref() {
        let finding = Finding {
            id: "f-info-1".into(),
            domain: Domain::Documentation,
            severity: Severity::Info,
            title: "README mentions removed flag --foo".into(),
            evidence: vec!["README.md:120".into()],
            suggested_action: SuggestedAction {
                kind: ActionKind::Informational,
                draft_ref: None,
            },
        };
        let yaml = serde_yaml::to_string(&finding).expect("serialize yaml");
        assert!(
            !yaml.contains("draft_ref"),
            "draft_ref should be skipped when None; yaml = {yaml}"
        );
        let back: Finding = serde_yaml::from_str(&yaml).expect("deserialize yaml");
        assert_eq!(finding, back);
    }

    #[test]
    fn unknown_domain_falls_through_to_other() {
        let yaml = r#"
id: f-custom-1
domain: supply_chain
severity: medium
title: dependency with known CVE
evidence:
  - "Cargo.lock: foo 1.2.3"
suggested_action:
  kind: draft_adr
  draft_ref: ADR-2026-04-24-1200-supply-chain
"#;
        let finding: Finding = serde_yaml::from_str(yaml).expect("deserialize yaml");
        assert_eq!(finding.domain, Domain::Other("supply_chain".into()));
        assert_eq!(finding.severity, Severity::Medium);
        assert_eq!(finding.suggested_action.kind, ActionKind::DraftAdr);
    }

    #[test]
    fn json_round_trip_still_works() {
        let finding = sample();
        let json = serde_json::to_string(&finding).expect("serialize json");
        let back: Finding = serde_json::from_str(&json).expect("deserialize json");
        assert_eq!(finding, back);
    }
}
