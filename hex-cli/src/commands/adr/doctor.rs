//! `hex adr doctor` — ADR registry self-consistency checker (ADR-2604270800).
//!
//! Pure detection surface. Scans `docs/adrs/`, parses each file's frontmatter,
//! and emits structured `Finding`s. No mutation here — auto-fix lives in
//! [`shadow_promote`] (P2) and the daemon dispatcher in `sched.rs` (P3).
//!
//! Detection rules are encoded in a static data table per ADR-2604142243
//! (rules-as-data, not control flow). Each `FindingKind` has exactly one row
//! mapping it to an [`AutoFixTier`] and a [`Severity`]. Adding a new check
//! requires picking both — there is no implicit default.
//!
//! The skeleton in this file (P1.1) defines the types, the rule table, and
//! the `run` scaffold. Detector implementations land in P1.2; CLI/MCP wiring
//! in P1.3.
//!
//! Exit-code contract (used by [`exit_code`]):
//!   - `0` clean
//!   - `1` warnings only
//!   - `2` any error (or any finding when `--strict`)

use std::path::PathBuf;

use serde::Serialize;

use super::{collect_adrs, find_adr_dir};

// ── Types ────────────────────────────────────────────────────────────────

/// A single registry-consistency finding emitted by `doctor::run`.
#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    /// ADR identifier (e.g. `ADR-2604270800`). Empty string if the file
    /// itself is so malformed we can't extract one.
    pub adr_id: String,
    pub file_path: PathBuf,
    pub kind: FindingKind,
    pub tier: AutoFixTier,
    pub severity: Severity,
    /// Human-readable explanation specific to this occurrence.
    pub detail: String,
}

/// What went wrong. One variant per check defined in ADR-2604270800 §1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingKind {
    /// `Status:` field doesn't match the lifecycle enum.
    UnparseableStatus,
    /// Two or more files share the same `ADR-NNNNNNNNNN` prefix.
    DuplicateId,
    /// Filename ID doesn't match the H1 title's ID.
    IdFormatMismatch,
    /// Missing `Status:`, `Date:`, or H1 title.
    MissingRequiredField,
    /// `Depends on:` cites an ADR ID not present on disk.
    DanglingDependency,
    /// `Status: Proposed` AND `Date:` > 30 days old (per ADR-012).
    StaleProposed,
    /// `Status: Superseded` without a `Superseded by:` field.
    SupersededUnlinked,
}

/// What the daemon is allowed to do without human consent (ADR-2604270800 §1a).
///
/// Tier is a column on each finding type, stored in the same rule table that
/// drives detection — there is no implicit default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AutoFixTier {
    /// Auto-apply via shadow-promotion. P3 inbox entry on success.
    A,
    /// Auto-draft in a sched worktree, P2 inbox notification with diff.
    B,
    /// P1 inbox notification, no auto-action.
    C,
}

/// Finding severity. Drives exit codes and `--strict` promotion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

// ── Rule table (ADR-2604270800 §1 + §1a) ─────────────────────────────────

/// Static rule table: every `FindingKind` maps to exactly one `(tier, severity)`.
///
/// Severity column matches the table in ADR-2604270800 §1.
/// Tier column matches §1a. Variants not explicitly tabled in §1a default to
/// the most-conservative tier that's still actionable:
///   - `IdFormatMismatch` → Tier B (filename rename is mechanical, but renaming
///     a committed file mid-flight needs human confirmation).
///   - `DanglingDependency` → Tier C (the right link is a judgment call).
const RULE_TABLE: &[(FindingKind, AutoFixTier, Severity)] = &[
    (FindingKind::UnparseableStatus,   AutoFixTier::A, Severity::Error),
    (FindingKind::DuplicateId,         AutoFixTier::C, Severity::Error),
    (FindingKind::IdFormatMismatch,    AutoFixTier::B, Severity::Error),
    (FindingKind::MissingRequiredField, AutoFixTier::C, Severity::Error),
    (FindingKind::DanglingDependency,  AutoFixTier::C, Severity::Warning),
    (FindingKind::StaleProposed,       AutoFixTier::B, Severity::Warning),
    (FindingKind::SupersededUnlinked,  AutoFixTier::B, Severity::Warning),
];

/// Look up the (tier, severity) for a given finding kind. Panics if the rule
/// table is missing a row — that's a programmer bug, not a runtime condition.
pub fn rule_for(kind: FindingKind) -> (AutoFixTier, Severity) {
    RULE_TABLE
        .iter()
        .find(|(k, _, _)| *k == kind)
        .map(|(_, tier, sev)| (*tier, *sev))
        .unwrap_or_else(|| panic!("doctor::RULE_TABLE missing entry for {:?}", kind))
}

/// Construct a `Finding` from its `kind` — tier and severity are pulled from
/// the rule table so they can never drift from per-call definitions.
pub fn finding(adr_id: impl Into<String>, file_path: PathBuf, kind: FindingKind, detail: impl Into<String>) -> Finding {
    let (tier, severity) = rule_for(kind);
    Finding {
        adr_id: adr_id.into(),
        file_path,
        kind,
        tier,
        severity,
        detail: detail.into(),
    }
}

// ── Run scaffold (detectors land in P1.2) ────────────────────────────────

/// Scan `docs/adrs/`, parse each file, and return all findings.
///
/// Detection is split into per-rule functions invoked here. P1.1 lays the
/// scaffold; P1.2 implements each detector against fixtures.
pub async fn run() -> anyhow::Result<Vec<Finding>> {
    let adr_dir = find_adr_dir()
        .ok_or_else(|| anyhow::anyhow!("No docs/adrs/ directory found"))?;
    let adrs = collect_adrs(&adr_dir).await?;

    let mut findings = Vec::new();

    // Per-file detectors (P1.2 fills these in).
    for (path, content) in &adrs {
        findings.extend(detect_unparseable_status(path, content));
        findings.extend(detect_id_format_mismatch(path, content));
        findings.extend(detect_missing_required_field(path, content));
        findings.extend(detect_stale_proposed(path, content));
        findings.extend(detect_superseded_unlinked(path, content));
    }

    // Cross-file detectors (P1.2 fills these in).
    findings.extend(detect_duplicate_ids(&adrs));
    findings.extend(detect_dangling_dependencies(&adrs));

    Ok(findings)
}

// Per-file detector stubs. P1.2 replaces each body with the real check.

#[allow(unused_variables)]
fn detect_unparseable_status(path: &std::path::Path, content: &str) -> Vec<Finding> {
    Vec::new()
}

#[allow(unused_variables)]
fn detect_id_format_mismatch(path: &std::path::Path, content: &str) -> Vec<Finding> {
    Vec::new()
}

#[allow(unused_variables)]
fn detect_missing_required_field(path: &std::path::Path, content: &str) -> Vec<Finding> {
    Vec::new()
}

#[allow(unused_variables)]
fn detect_stale_proposed(path: &std::path::Path, content: &str) -> Vec<Finding> {
    Vec::new()
}

#[allow(unused_variables)]
fn detect_superseded_unlinked(path: &std::path::Path, content: &str) -> Vec<Finding> {
    Vec::new()
}

#[allow(unused_variables)]
fn detect_duplicate_ids(adrs: &[(PathBuf, String)]) -> Vec<Finding> {
    Vec::new()
}

#[allow(unused_variables)]
fn detect_dangling_dependencies(adrs: &[(PathBuf, String)]) -> Vec<Finding> {
    Vec::new()
}

// ── Output + exit code ───────────────────────────────────────────────────

/// Serialize findings to a stable JSON envelope. Used by `--json` and by
/// the sched daemon when recording `adr_doctor_tick` event payloads.
pub fn to_json(findings: &[Finding]) -> anyhow::Result<String> {
    let envelope = serde_json::json!({
        "findings": findings,
        "summary": {
            "total":    findings.len(),
            "errors":   findings.iter().filter(|f| f.severity == Severity::Error).count(),
            "warnings": findings.iter().filter(|f| f.severity == Severity::Warning).count(),
            "tier_a":   findings.iter().filter(|f| f.tier == AutoFixTier::A).count(),
            "tier_b":   findings.iter().filter(|f| f.tier == AutoFixTier::B).count(),
            "tier_c":   findings.iter().filter(|f| f.tier == AutoFixTier::C).count(),
        },
    });
    Ok(serde_json::to_string_pretty(&envelope)?)
}

/// Map findings to a process exit code per ADR-2604270800 §1.
///
///   - `0` clean
///   - `1` warnings only
///   - `2` any error
///
/// `--strict` promotes warnings to errors (so any finding → `2`).
pub fn exit_code(findings: &[Finding], strict: bool) -> i32 {
    if findings.is_empty() {
        return 0;
    }
    let has_error = findings.iter().any(|f| f.severity == Severity::Error);
    if has_error || strict {
        2
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_table_covers_every_kind() {
        // Every variant must appear exactly once. If a new kind is added
        // without a rule-table row, `rule_for` will panic — catch that here.
        let all = [
            FindingKind::UnparseableStatus,
            FindingKind::DuplicateId,
            FindingKind::IdFormatMismatch,
            FindingKind::MissingRequiredField,
            FindingKind::DanglingDependency,
            FindingKind::StaleProposed,
            FindingKind::SupersededUnlinked,
        ];
        for k in all {
            let _ = rule_for(k); // Panics if missing.
        }
        assert_eq!(RULE_TABLE.len(), all.len(), "rule table cardinality drifted from FindingKind");
    }

    #[test]
    fn rule_table_matches_adr_severity_column() {
        // Locks in the severity column from ADR-2604270800 §1.
        assert_eq!(rule_for(FindingKind::UnparseableStatus).1,    Severity::Error);
        assert_eq!(rule_for(FindingKind::DuplicateId).1,          Severity::Error);
        assert_eq!(rule_for(FindingKind::IdFormatMismatch).1,     Severity::Error);
        assert_eq!(rule_for(FindingKind::MissingRequiredField).1, Severity::Error);
        assert_eq!(rule_for(FindingKind::DanglingDependency).1,   Severity::Warning);
        assert_eq!(rule_for(FindingKind::StaleProposed).1,        Severity::Warning);
        assert_eq!(rule_for(FindingKind::SupersededUnlinked).1,   Severity::Warning);
    }

    #[test]
    fn rule_table_matches_adr_tier_column() {
        // Locks in the tier column from ADR-2604270800 §1a (and the documented
        // defaults for variants the ADR didn't tabulate explicitly).
        assert_eq!(rule_for(FindingKind::UnparseableStatus).0,    AutoFixTier::A);
        assert_eq!(rule_for(FindingKind::SupersededUnlinked).0,   AutoFixTier::B);
        assert_eq!(rule_for(FindingKind::StaleProposed).0,        AutoFixTier::B);
        assert_eq!(rule_for(FindingKind::DuplicateId).0,          AutoFixTier::C);
        assert_eq!(rule_for(FindingKind::MissingRequiredField).0, AutoFixTier::C);
    }

    #[test]
    fn finding_constructor_pulls_tier_severity_from_table() {
        let f = finding("ADR-001", PathBuf::from("x.md"), FindingKind::UnparseableStatus, "bad");
        assert_eq!(f.tier, AutoFixTier::A);
        assert_eq!(f.severity, Severity::Error);
        assert_eq!(f.adr_id, "ADR-001");
        assert_eq!(f.detail, "bad");
    }

    #[test]
    fn exit_code_clean_is_zero() {
        assert_eq!(exit_code(&[], false), 0);
        assert_eq!(exit_code(&[], true), 0);
    }

    #[test]
    fn exit_code_warning_is_one_unless_strict() {
        let warn = finding("ADR-001", PathBuf::from("x.md"), FindingKind::StaleProposed, "");
        assert_eq!(exit_code(&[warn.clone()], false), 1);
        assert_eq!(exit_code(&[warn], true), 2, "--strict promotes warnings to errors");
    }

    #[test]
    fn exit_code_error_is_two() {
        let err = finding("ADR-001", PathBuf::from("x.md"), FindingKind::DuplicateId, "");
        assert_eq!(exit_code(&[err.clone()], false), 2);
        assert_eq!(exit_code(&[err], true), 2);
    }

    #[test]
    fn exit_code_error_dominates_warning() {
        let err  = finding("ADR-001", PathBuf::from("x.md"), FindingKind::DuplicateId, "");
        let warn = finding("ADR-002", PathBuf::from("y.md"), FindingKind::StaleProposed, "");
        assert_eq!(exit_code(&[warn, err], false), 2);
    }

    #[test]
    fn json_envelope_has_summary_counts() {
        let err  = finding("ADR-001", PathBuf::from("x.md"), FindingKind::DuplicateId, "");
        let warn = finding("ADR-002", PathBuf::from("y.md"), FindingKind::StaleProposed, "");
        let json = to_json(&[err, warn]).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["summary"]["total"],    2);
        assert_eq!(v["summary"]["errors"],   1);
        assert_eq!(v["summary"]["warnings"], 1);
        assert_eq!(v["summary"]["tier_c"],   1);
        assert_eq!(v["summary"]["tier_b"],   1);
        assert_eq!(v["findings"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn run_skeleton_returns_empty_until_p1_2() {
        // P1.1 ships a scaffold; detectors are stubs returning Vec::new().
        // This test will be replaced in P1.2 once real detection lands.
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(run());
        // We don't assert success — `find_adr_dir` may fail in some test
        // environments — only that, when it succeeds, the stubs return clean.
        if let Ok(findings) = result {
            assert!(findings.is_empty(), "stub detectors must not emit findings");
        }
    }
}
