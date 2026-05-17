//! L5 ADR conformance check (ADR-2604261311 L5 / ADR-2604261500 C6).
//!
//! Pre-promotion gate. Before the promote orchestrator flips a live
//! binding, this checker scans the project's `docs/adrs/` directory and
//! decides whether the swap would violate an Accepted ADR. If it would,
//! the orchestrator skips the ticket (it stays in `shadow_green` until
//! the operator either retracts it or extends the ADR).
//!
//! Day-one conformance rules (intentionally narrow — extension point
//! documented; future rules join the same `Vec<Rule>`):
//!
//! - **R1 — `manifest.version != "deprecated"`.** Canary version string
//!   that explicitly marks an adapter the substrate is not allowed to
//!   re-promote (e.g. one whose ADR was Superseded; the operator wrote
//!   `version: "deprecated"` in the manifest at swap-propose time to
//!   signal this).
//! - **R2 — adapter_id pattern `adr-NNNN-*` references a non-Accepted
//!   ADR.** If the candidate adapter id encodes an ADR id, that ADR's
//!   Status must be Accepted. Catches the case where an operator tries
//!   to promote an adapter whose authorizing ADR was Superseded.
//!
//! The ADR registry is loaded lazily on first `check_promotion` call
//! and refreshed on every call (cheap; the docs/adrs/ directory rarely
//! has more than a few hundred files; refresh-per-tick is acceptable
//! for an L5 layer that runs every 30s alongside the promote orchestrator).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::ports::state::SwapTicketRecord;

#[derive(Debug, Clone)]
pub struct AdrRecord {
    pub id: String, // e.g. "ADR-2604261500"
    pub status: AdrStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdrStatus {
    Proposed,
    Accepted,
    Deprecated,
    Superseded,
    Unknown(String),
}

impl AdrStatus {
    fn parse(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "proposed" => AdrStatus::Proposed,
            "accepted" => AdrStatus::Accepted,
            "deprecated" => AdrStatus::Deprecated,
            "superseded" => AdrStatus::Superseded,
            other => AdrStatus::Unknown(other.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConformanceViolation {
    DeprecatedVersion {
        ticket_id: String,
        candidate_adapter_id: String,
    },
    AdapterReferencesNonAcceptedAdr {
        ticket_id: String,
        candidate_adapter_id: String,
        adr_id: String,
        adr_status: String,
    },
}

impl std::fmt::Display for ConformanceViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConformanceViolation::DeprecatedVersion {
                ticket_id,
                candidate_adapter_id,
            } => write!(
                f,
                "ticket {} candidate {} has manifest.version=\"deprecated\" — substrate refuses to re-promote",
                ticket_id, candidate_adapter_id
            ),
            ConformanceViolation::AdapterReferencesNonAcceptedAdr {
                ticket_id,
                candidate_adapter_id,
                adr_id,
                adr_status,
            } => write!(
                f,
                "ticket {} candidate {} references {} which is {} (must be Accepted)",
                ticket_id, candidate_adapter_id, adr_id, adr_status
            ),
        }
    }
}

pub struct AdrConformanceChecker {
    adrs_dir: PathBuf,
}

impl AdrConformanceChecker {
    pub fn new(adrs_dir: impl Into<PathBuf>) -> Self {
        Self {
            adrs_dir: adrs_dir.into(),
        }
    }

    /// Re-read the ADR directory and return the parsed registry.
    /// Fresh-per-call by design — the directory is small and operator
    /// edits should take effect immediately.
    pub fn load_registry(&self) -> BTreeMap<String, AdrRecord> {
        let mut map = BTreeMap::new();
        let entries = match std::fs::read_dir(&self.adrs_dir) {
            Ok(e) => e,
            Err(_) => return map,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("md") {
                continue;
            }
            if let Some(rec) = parse_adr(&path) {
                map.insert(rec.id.to_lowercase(), rec);
            }
        }
        map
    }

    pub fn check_promotion(&self, ticket: &SwapTicketRecord) -> Vec<ConformanceViolation> {
        let registry = self.load_registry();
        check_against(ticket, &registry)
    }
}

fn parse_adr(path: &Path) -> Option<AdrRecord> {
    let stem = path.file_stem()?.to_str()?;
    // Match the ADR-NNNNNNNNNN prefix (case-insensitive). Filenames
    // observed in the project: "ADR-2604261500-...", "ADR-2604170001-...".
    let id = stem
        .split_once('-')
        .filter(|(prefix, _)| prefix.eq_ignore_ascii_case("adr"))
        .and_then(|(_, rest)| {
            let id_part = rest.split('-').next()?;
            // The id portion should be all digits.
            if !id_part.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            Some(format!("ADR-{}", id_part))
        })?;
    let body = std::fs::read_to_string(path).ok()?;
    let status = body
        .lines()
        .find_map(|l| {
            l.strip_prefix("**Status:**")
                .map(|rest| AdrStatus::parse(rest))
        })
        .unwrap_or(AdrStatus::Unknown("(missing)".into()));
    Some(AdrRecord { id, status })
}

fn check_against(
    ticket: &SwapTicketRecord,
    registry: &BTreeMap<String, AdrRecord>,
) -> Vec<ConformanceViolation> {
    let mut violations = vec![];

    // R1 — deprecated canary. We only see manifest.version through the
    // serialized candidate_manifest_json field; parse defensively.
    if let Ok(manifest) = serde_json::from_str::<serde_json::Value>(&ticket.candidate_manifest_json) {
        if manifest.get("version").and_then(|v| v.as_str()) == Some("deprecated") {
            violations.push(ConformanceViolation::DeprecatedVersion {
                ticket_id: ticket.id.clone(),
                candidate_adapter_id: ticket.candidate_adapter_id.clone(),
            });
        }
    }

    // R2 — adapter_id pattern adr-NNNN must reference an Accepted ADR.
    if let Some(adr_id) = extract_adr_id(&ticket.candidate_adapter_id) {
        match registry.get(&adr_id.to_lowercase()) {
            Some(rec) if rec.status != AdrStatus::Accepted => {
                violations.push(ConformanceViolation::AdapterReferencesNonAcceptedAdr {
                    ticket_id: ticket.id.clone(),
                    candidate_adapter_id: ticket.candidate_adapter_id.clone(),
                    adr_id: rec.id.clone(),
                    adr_status: format!("{:?}", rec.status),
                });
            }
            None => {
                // ADR id referenced but not present in the registry —
                // the operator is naming an ADR that doesn't exist.
                violations.push(ConformanceViolation::AdapterReferencesNonAcceptedAdr {
                    ticket_id: ticket.id.clone(),
                    candidate_adapter_id: ticket.candidate_adapter_id.clone(),
                    adr_id,
                    adr_status: "Missing from docs/adrs/".into(),
                });
            }
            _ => {}
        }
    }

    violations
}

/// Extract an ADR-id from an adapter_id of the form `adr-NNNNNNNNNN-...`
/// or `ADR-NNNNNNNNNN-...`. Returns canonical `ADR-NNNNNNNNNN`.
fn extract_adr_id(adapter_id: &str) -> Option<String> {
    let lower = adapter_id.to_lowercase();
    let rest = lower.strip_prefix("adr-")?;
    let id_part: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if id_part.is_empty() {
        return None;
    }
    Some(format!("ADR-{}", id_part))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn ticket(id: &str, candidate: &str, manifest_json: &str) -> SwapTicketRecord {
        SwapTicketRecord {
            id: id.into(),
            project_id: "test".into(),
            port_id: "inference".into(),
            incumbent_adapter_id: "mock-a".into(),
            candidate_adapter_id: candidate.into(),
            candidate_manifest_json: manifest_json.into(),
            state: "shadow_green".into(),
            shadow_traffic_fraction: 1.0,
            shadow_window_seconds: 300,
            shadow_started_at: "2026-04-26T18:00:00Z".into(),
            success_criteria_json: "[]".into(),
            created_at: "2026-04-26T18:00:00Z".into(),
            updated_at: "2026-04-26T18:00:00Z".into(),
        }
    }

    fn registry_with(records: &[(&str, AdrStatus)]) -> BTreeMap<String, AdrRecord> {
        records
            .iter()
            .map(|(id, status)| {
                (
                    id.to_lowercase(),
                    AdrRecord {
                        id: id.to_string(),
                        status: status.clone(),
                    },
                )
            })
            .collect()
    }

    #[test]
    fn r1_blocks_deprecated_version_canary() {
        let t = ticket("t1", "any-adapter", r#"{"version":"deprecated"}"#);
        let v = check_against(&t, &BTreeMap::new());
        assert_eq!(v.len(), 1);
        assert!(matches!(v[0], ConformanceViolation::DeprecatedVersion { .. }));
    }

    #[test]
    fn r1_passes_non_deprecated_versions() {
        let t = ticket("t1", "any-adapter", r#"{"version":"0.1.0"}"#);
        let v = check_against(&t, &BTreeMap::new());
        assert!(v.is_empty());
    }

    #[test]
    fn r2_blocks_adapter_referencing_superseded_adr() {
        let t = ticket("t1", "ADR-2604120202-tier-routing", r#"{"version":"0.1.0"}"#);
        let reg = registry_with(&[("ADR-2604120202", AdrStatus::Superseded)]);
        let v = check_against(&t, &reg);
        assert_eq!(v.len(), 1);
        match &v[0] {
            ConformanceViolation::AdapterReferencesNonAcceptedAdr {
                adr_id, adr_status, ..
            } => {
                assert_eq!(adr_id, "ADR-2604120202");
                assert!(adr_status.contains("Superseded"));
            }
            other => panic!("wrong violation: {:?}", other),
        }
    }

    #[test]
    fn r2_passes_adapter_referencing_accepted_adr() {
        let t = ticket("t1", "ADR-2604261500-substrate", r#"{"version":"0.1.0"}"#);
        let reg = registry_with(&[("ADR-2604261500", AdrStatus::Accepted)]);
        let v = check_against(&t, &reg);
        assert!(v.is_empty());
    }

    #[test]
    fn r2_blocks_adapter_referencing_missing_adr() {
        let t = ticket("t1", "ADR-2099-99-99-9999-imaginary", r#"{"version":"0.1.0"}"#);
        let v = check_against(&t, &BTreeMap::new());
        assert_eq!(v.len(), 1);
        match &v[0] {
            ConformanceViolation::AdapterReferencesNonAcceptedAdr { adr_status, .. } => {
                assert!(adr_status.contains("Missing"));
            }
            other => panic!("wrong violation: {:?}", other),
        }
    }

    #[test]
    fn r2_passes_adapter_with_no_adr_pattern() {
        let t = ticket("t1", "mock-fallback", r#"{"version":"0.1.0"}"#);
        let v = check_against(&t, &BTreeMap::new());
        assert!(v.is_empty());
    }

    #[test]
    fn parse_adr_extracts_id_and_status_from_real_file_shape() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("ADR-2604261500-substrate.md");
        std::fs::write(
            &path,
            "# ADR-2604261500: ...\n\n**Status:** Accepted\n**Date:** 2026-04-26\n",
        )
        .expect("write");
        let rec = parse_adr(&path).expect("parsed");
        assert_eq!(rec.id, "ADR-2604261500");
        assert_eq!(rec.status, AdrStatus::Accepted);
    }

    #[test]
    fn parse_adr_handles_lowercase_filename_prefix() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("ADR-2604170001-bootstrap.md");
        std::fs::write(&path, "**Status:** Accepted\n").expect("write");
        let rec = parse_adr(&path).expect("parsed");
        assert_eq!(rec.id, "ADR-2604170001");
    }

    #[test]
    fn extract_adr_id_handles_both_prefixes() {
        assert_eq!(
            extract_adr_id("ADR-2604120202-something"),
            Some("ADR-2604120202".into())
        );
        assert_eq!(
            extract_adr_id("ADR-2604120202-something"),
            Some("ADR-2604120202".into())
        );
        assert_eq!(extract_adr_id("not-an-ADR-id"), None);
        assert_eq!(extract_adr_id("ADR-not-numeric"), None);
    }

    #[test]
    fn checker_loads_registry_from_real_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("ADR-2604261500-foo.md"),
            "**Status:** Accepted\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("ADR-2604120202-bar.md"),
            "**Status:** Superseded\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("README.md"), "not an ADR").unwrap();
        let checker = AdrConformanceChecker::new(dir.path());
        let reg = checker.load_registry();
        assert_eq!(reg.len(), 2);
        assert_eq!(reg["ADR-2604261500"].status, AdrStatus::Accepted);
        assert_eq!(reg["ADR-2604120202"].status, AdrStatus::Superseded);
    }
}
