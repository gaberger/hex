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
//! P1.2 fills in the seven detectors (`detect_*`) backed by fixture tests.
//!
//! Exit-code contract (used by [`exit_code`]):
//!   - `0` clean
//!   - `1` warnings only
//!   - `2` any error (or any finding when `--strict`)

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use chrono::NaiveDate;
use regex::Regex;
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

// ── Run scaffold ──────────────────────────────────────────────────────────

/// Scan `docs/adrs/`, parse each file, and return all findings.
pub async fn run() -> anyhow::Result<Vec<Finding>> {
    let adr_dir = find_adr_dir()
        .ok_or_else(|| anyhow::anyhow!("No docs/adrs/ directory found"))?;
    let adrs = collect_adrs(&adr_dir).await?;
    let now = chrono::Local::now().date_naive();
    Ok(scan(&adrs, now))
}

/// Pure detection over a pre-loaded ADR corpus. Split out from [`run`] so tests
/// (and the sched tick handler in P3) can drive it without touching the FS or
/// the system clock.
pub fn scan(adrs: &[(PathBuf, String)], now: NaiveDate) -> Vec<Finding> {
    let mut findings = Vec::new();

    // Per-file detectors.
    for (path, content) in adrs {
        findings.extend(detect_unparseable_status(path, content));
        findings.extend(detect_id_format_mismatch(path, content));
        findings.extend(detect_missing_required_field(path, content));
        findings.extend(detect_stale_proposed(path, content, now));
        findings.extend(detect_superseded_unlinked(path, content));
    }

    // Cross-file detectors.
    findings.extend(detect_duplicate_ids(adrs));
    findings.extend(detect_dangling_dependencies(adrs));

    findings
}

// ── Field extraction helpers ─────────────────────────────────────────────

fn adr_id_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"ADR-\d+").unwrap())
}

/// Strict canonical-format extraction. Mirrors the parent module's
/// `parse_adr_status`: accepts `**Status:** value`, `status: value`, or
/// `## Status\nvalue`. Buggy variants (`**Status**:`, `- **Status**:`) are
/// rejected so the caller can flag them as `UnparseableStatus`.
fn strict_extract_status_value(content: &str) -> Option<String> {
    let lines: Vec<&str> = content.lines().collect();
    for (i, raw) in lines.iter().enumerate() {
        let trimmed = raw.trim();
        let lower = trimmed.to_lowercase();

        let val = if lower.starts_with("**status:**") {
            trimmed["**Status:**".len()..].trim().to_string()
        } else if lower.starts_with("status:") && !lower.starts_with("status_") {
            trimmed["status:".len()..].trim().to_string()
        } else if lower == "## status" || lower == "## status:" {
            let mut j = i + 1;
            while j < lines.len() && lines[j].trim().is_empty() {
                j += 1;
            }
            if j >= lines.len() {
                continue;
            }
            lines[j].trim().trim_matches('*').trim().to_string()
        } else {
            continue;
        };
        return Some(val);
    }
    None
}

/// Lenient: returns true if any line *looks like* a Status field, including
/// the buggy `**Status**:` and `- **Status**:` variants. Used to distinguish
/// `UnparseableStatus` (field present, value bad) from `MissingRequiredField`
/// (field absent entirely).
fn lenient_has_status_line(content: &str) -> bool {
    for line in content.lines() {
        let stripped = line.trim().trim_start_matches("- ").trim_start();
        let lower = stripped.to_lowercase();
        if lower.starts_with("**status:**")
            || lower.starts_with("**status**:")
            || (lower.starts_with("status:") && !lower.starts_with("status_"))
            || lower == "## status"
            || lower == "## status:"
        {
            return true;
        }
    }
    false
}

/// Classify a raw status value against the lifecycle enum. Returns `None` for
/// unrecognized values OR multi-status enum listings (e.g.
/// `**Status:** Proposed | Accepted`).
fn classify_status(value: &str) -> Option<&'static str> {
    let lower = value.to_lowercase();
    let cleaned: String = lower
        .chars()
        .map(|c| if c.is_alphabetic() || c == ' ' { c } else { ' ' })
        .collect();
    let words: Vec<&str> = cleaned.split_whitespace().collect();

    let known = ["proposed", "accepted", "deprecated", "superseded", "abandoned"];
    let matches: Vec<&'static str> = known
        .iter()
        .copied()
        .filter(|k| words.iter().any(|w| *w == *k))
        .collect();

    match matches.len() {
        1 => Some(matches[0]),
        _ => None, // 0 = invalid; 2+ = enum listing
    }
}

fn extract_date(content: &str) -> Option<NaiveDate> {
    for line in content.lines() {
        let stripped = line.trim().trim_start_matches("- ").trim_start();
        let lower = stripped.to_lowercase();
        let value: Option<&str> = if lower.starts_with("**date:**") {
            Some(stripped["**Date:**".len()..].trim())
        } else if lower.starts_with("**date**:") {
            Some(stripped["**Date**:".len()..].trim())
        } else if lower.starts_with("date:") {
            Some(stripped["date:".len()..].trim())
        } else {
            None
        };
        if let Some(v) = value {
            if let Some(token) = v.split_whitespace().next() {
                if let Ok(date) = NaiveDate::parse_from_str(token, "%Y-%m-%d") {
                    return Some(date);
                }
            }
        }
    }
    None
}

fn lenient_has_date_line(content: &str) -> bool {
    for line in content.lines() {
        let stripped = line.trim().trim_start_matches("- ").trim_start();
        let lower = stripped.to_lowercase();
        if lower.starts_with("**date:**")
            || lower.starts_with("**date**:")
            || lower.starts_with("date:")
        {
            return true;
        }
    }
    false
}

fn has_h1_title(content: &str) -> bool {
    content.lines().any(|l| l.trim_start().starts_with("# "))
}

fn extract_h1_adr_id(content: &str) -> Option<String> {
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(title) = trimmed.strip_prefix("# ") {
            return adr_id_re().find(title).map(|m| m.as_str().to_string());
        }
    }
    None
}

fn extract_filename_adr_id(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    adr_id_re().find(stem).map(|m| m.as_str().to_string())
}

fn has_superseded_by(content: &str) -> bool {
    for line in content.lines() {
        let stripped = line.trim().trim_start_matches("- ").trim_start();
        let lower = stripped.to_lowercase();
        // Accept canonical `**Superseded by:**`, the buggy `**Superseded by**:`,
        // and the YAML-style `superseded by:` / `superseded-by:`.
        if lower.starts_with("**superseded by:**")
            || lower.starts_with("**superseded by**:")
            || lower.starts_with("superseded by:")
            || lower.starts_with("superseded-by:")
        {
            return true;
        }
    }
    false
}

fn extract_dependencies(content: &str) -> Vec<String> {
    let mut deps = Vec::new();
    for line in content.lines() {
        let stripped = line.trim().trim_start_matches("- ").trim_start();
        let lower = stripped.to_lowercase();
        let value: Option<&str> = if lower.starts_with("**depends on:**") {
            Some(&stripped["**Depends on:**".len()..])
        } else if lower.starts_with("**depends on**:") {
            Some(&stripped["**Depends on**:".len()..])
        } else if lower.starts_with("depends on:") {
            Some(&stripped["depends on:".len()..])
        } else if lower.starts_with("depends-on:") {
            Some(&stripped["depends-on:".len()..])
        } else {
            None
        };
        if let Some(v) = value {
            for m in adr_id_re().find_iter(v) {
                deps.push(m.as_str().to_string());
            }
        }
    }
    deps
}

// ── Per-file detectors ───────────────────────────────────────────────────

fn detect_unparseable_status(path: &Path, content: &str) -> Vec<Finding> {
    if !lenient_has_status_line(content) {
        // No status line at all → MissingRequiredField, not UnparseableStatus.
        return Vec::new();
    }
    let parseable = strict_extract_status_value(content)
        .as_deref()
        .and_then(classify_status)
        .is_some();
    if parseable {
        return Vec::new();
    }
    let adr_id = extract_filename_adr_id(path).unwrap_or_default();
    let detail = match strict_extract_status_value(content) {
        // Strict parse succeeded but value didn't classify (e.g. "Banana" or
        // "Proposed | Accepted").
        Some(v) => format!("Status value `{}` is not a recognized lifecycle state", v.trim()),
        // Strict parse failed — buggy frontmatter form (`**Status**:`, `- **Status**:`).
        None => "Status field uses non-canonical format (expected `**Status:** <value>`)".to_string(),
    };
    vec![finding(adr_id, path.to_path_buf(), FindingKind::UnparseableStatus, detail)]
}

fn detect_id_format_mismatch(path: &Path, content: &str) -> Vec<Finding> {
    let filename_id = match extract_filename_adr_id(path) {
        Some(id) => id,
        None => return Vec::new(),
    };
    let h1_id = match extract_h1_adr_id(content) {
        Some(id) => id,
        // No ADR-ID in H1 → can't compare. (Missing H1 entirely is caught by
        // MissingRequiredField; an H1 without an ADR-ID is just a stylistic
        // choice we don't flag.)
        None => return Vec::new(),
    };
    if filename_id == h1_id {
        return Vec::new();
    }
    vec![finding(
        filename_id.clone(),
        path.to_path_buf(),
        FindingKind::IdFormatMismatch,
        format!("filename ID {} but H1 title says {}", filename_id, h1_id),
    )]
}

fn detect_missing_required_field(path: &Path, content: &str) -> Vec<Finding> {
    let adr_id = extract_filename_adr_id(path).unwrap_or_default();
    let mut findings = Vec::new();
    if !lenient_has_status_line(content) {
        findings.push(finding(
            adr_id.clone(),
            path.to_path_buf(),
            FindingKind::MissingRequiredField,
            "missing required field: Status",
        ));
    }
    if !lenient_has_date_line(content) {
        findings.push(finding(
            adr_id.clone(),
            path.to_path_buf(),
            FindingKind::MissingRequiredField,
            "missing required field: Date",
        ));
    }
    if !has_h1_title(content) {
        findings.push(finding(
            adr_id,
            path.to_path_buf(),
            FindingKind::MissingRequiredField,
            "missing required field: H1 title",
        ));
    }
    findings
}

fn detect_stale_proposed(path: &Path, content: &str, now: NaiveDate) -> Vec<Finding> {
    let status = strict_extract_status_value(content)
        .as_deref()
        .and_then(classify_status);
    if status != Some("proposed") {
        return Vec::new();
    }
    let date = match extract_date(content) {
        Some(d) => d,
        None => return Vec::new(),
    };
    let age = now.signed_duration_since(date).num_days();
    if age <= 30 {
        return Vec::new();
    }
    let adr_id = extract_filename_adr_id(path).unwrap_or_default();
    vec![finding(
        adr_id,
        path.to_path_buf(),
        FindingKind::StaleProposed,
        format!("Proposed since {} ({} days ago, threshold 30)", date, age),
    )]
}

fn detect_superseded_unlinked(path: &Path, content: &str) -> Vec<Finding> {
    let status = strict_extract_status_value(content)
        .as_deref()
        .and_then(classify_status);
    if status != Some("superseded") {
        return Vec::new();
    }
    if has_superseded_by(content) {
        return Vec::new();
    }
    let adr_id = extract_filename_adr_id(path).unwrap_or_default();
    vec![finding(
        adr_id,
        path.to_path_buf(),
        FindingKind::SupersededUnlinked,
        "Status is Superseded but no `Superseded by:` field found",
    )]
}

// ── Cross-file detectors ─────────────────────────────────────────────────

fn detect_duplicate_ids(adrs: &[(PathBuf, String)]) -> Vec<Finding> {
    let mut groups: HashMap<String, Vec<&PathBuf>> = HashMap::new();
    for (path, _) in adrs {
        if let Some(id) = extract_filename_adr_id(path) {
            groups.entry(id).or_default().push(path);
        }
    }
    let mut findings = Vec::new();
    for (id, paths) in &groups {
        if paths.len() > 1 {
            let other_names: Vec<String> = paths
                .iter()
                .filter_map(|p| p.file_name().and_then(|n| n.to_str()).map(String::from))
                .collect();
            for path in paths {
                findings.push(finding(
                    id.clone(),
                    path.to_path_buf(),
                    FindingKind::DuplicateId,
                    format!(
                        "ADR ID {} appears in {} files: {}",
                        id,
                        paths.len(),
                        other_names.join(", ")
                    ),
                ));
            }
        }
    }
    findings.sort_by(|a, b| a.file_path.cmp(&b.file_path));
    findings
}

fn detect_dangling_dependencies(adrs: &[(PathBuf, String)]) -> Vec<Finding> {
    let known: HashSet<String> = adrs
        .iter()
        .filter_map(|(p, _)| extract_filename_adr_id(p))
        .collect();
    let mut findings = Vec::new();
    for (path, content) in adrs {
        let mut seen_in_this_file: HashSet<String> = HashSet::new();
        for dep in extract_dependencies(content) {
            if known.contains(&dep) {
                continue;
            }
            if !seen_in_this_file.insert(dep.clone()) {
                continue;
            }
            let adr_id = extract_filename_adr_id(path).unwrap_or_default();
            findings.push(finding(
                adr_id,
                path.clone(),
                FindingKind::DanglingDependency,
                format!("Depends on {} which is not present in docs/adrs/", dep),
            ));
        }
    }
    findings
}

// ── Auto-fix patch generation (P2.1, ADR-2604270800 §1a) ─────────────────

/// A regex-based text transformation emitted by [`Finding::auto_fix_patch`]
/// for Tier-A findings. The shadow-promotion orchestrator in P2.2 calls
/// [`TextEdit::apply`] inside the auto-fix worktree, then re-runs `doctor
/// --strict` against the rewritten file as a self-check.
///
/// Modeled as an ordered list of regex `(pattern, replacement)` pairs rather
/// than absolute byte spans because a single Tier-A finding can target
/// multiple lines (e.g. `UnparseableStatus` normalizes Status + Date +
/// Depends on + Relates to in one shot — all four fields tend to be wrong
/// together when the bullet-prefixed bold-outside-colon form was used).
#[derive(Debug, Clone)]
pub struct TextEdit {
    pub replacements: Vec<Replacement>,
}

#[derive(Debug, Clone)]
pub struct Replacement {
    pub pattern: String,
    pub replacement: String,
}

impl TextEdit {
    /// Apply every replacement in order, returning the rewritten content.
    /// Idempotent: re-applying an already-canonical document is a no-op.
    pub fn apply(&self, content: &str) -> anyhow::Result<String> {
        let mut out = content.to_string();
        for r in &self.replacements {
            let re = Regex::new(&r.pattern)
                .map_err(|e| anyhow::anyhow!("invalid auto-fix regex `{}`: {}", r.pattern, e))?;
            out = re.replace_all(&out, r.replacement.as_str()).into_owned();
        }
        Ok(out)
    }
}

impl Finding {
    /// Return the auto-fix patch for this finding, or `None` if the kind is
    /// Tier B/C (drafted-only or notify-only per ADR-2604270800 §1a).
    ///
    /// Today, only [`FindingKind::UnparseableStatus`] yields a patch — it
    /// normalizes the bullet-prefixed bold-outside-colon frontmatter form
    /// (`- **Status**: …`) to the canonical `**Status:** …`. Date,
    /// Depends-on, and Relates-to lines are normalized in the same pass
    /// because they tend to drift together (verified against the three real
    /// ADRs hand-normalized in this session).
    pub fn auto_fix_patch(&self) -> Option<TextEdit> {
        if self.tier != AutoFixTier::A {
            return None;
        }
        match self.kind {
            FindingKind::UnparseableStatus => Some(unparseable_status_patch()),
            // No other Tier-A kinds in the rule table today. Adding a kind to
            // Tier A without giving it a patch here is a programmer error
            // caught by `auto_fix_patch_present_for_every_tier_a` below.
            _ => None,
        }
    }
}

/// The four-line frontmatter normalization shared by every Tier-A finding.
/// Each pattern uses `(?m)` so `^`/`$` match line boundaries.
fn unparseable_status_patch() -> TextEdit {
    TextEdit {
        replacements: vec![
            Replacement {
                pattern: r"(?m)^- \*\*Status\*\*: (.+)$".to_string(),
                replacement: "**Status:** $1".to_string(),
            },
            Replacement {
                pattern: r"(?m)^- \*\*Date\*\*: (.+)$".to_string(),
                replacement: "**Date:** $1".to_string(),
            },
            Replacement {
                pattern: r"(?m)^- \*\*Depends on\*\*: (.+)$".to_string(),
                replacement: "**Depends on:** $1".to_string(),
            },
            Replacement {
                pattern: r"(?m)^- \*\*Relates to\*\*: (.+)$".to_string(),
                replacement: "**Relates to:** $1".to_string(),
            },
        ],
    }
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

    // ── Rule-table tests (P1.1) ──────────────────────────────────────────

    #[test]
    fn rule_table_covers_every_kind() {
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
            let _ = rule_for(k);
        }
        assert_eq!(RULE_TABLE.len(), all.len(), "rule table cardinality drifted from FindingKind");
    }

    #[test]
    fn rule_table_matches_adr_severity_column() {
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

    // ── Helper-function tests (P1.2) ─────────────────────────────────────

    #[test]
    fn classify_status_single_keyword() {
        assert_eq!(classify_status("Proposed"), Some("proposed"));
        assert_eq!(classify_status("Accepted"), Some("accepted"));
        assert_eq!(classify_status("**Accepted** — 2026-04-10"), Some("accepted"));
    }

    #[test]
    fn classify_status_rejects_enum_listing() {
        assert_eq!(classify_status("Proposed | Accepted"), None);
        assert_eq!(classify_status("Proposed | Accepted | Deprecated"), None);
    }

    #[test]
    fn classify_status_rejects_unknown_value() {
        assert_eq!(classify_status("Banana"), None);
        assert_eq!(classify_status(""), None);
    }

    #[test]
    fn lenient_has_status_catches_buggy_forms() {
        assert!(lenient_has_status_line("- **Status**: Proposed"));
        assert!(lenient_has_status_line("**Status**: Accepted"));
        assert!(lenient_has_status_line("**Status:** Accepted"));
        assert!(lenient_has_status_line("status: Accepted"));
        assert!(lenient_has_status_line("## Status\nAccepted"));
        assert!(!lenient_has_status_line("# ADR-001\n\nNo status here"));
    }

    #[test]
    fn extract_date_handles_canonical_form() {
        let d = extract_date("**Date:** 2026-04-27\n").unwrap();
        assert_eq!(d, NaiveDate::from_ymd_opt(2026, 4, 27).unwrap());
    }

    #[test]
    fn extract_date_handles_buggy_bold_colon_form() {
        let d = extract_date("**Date**: 2026-04-27\n").unwrap();
        assert_eq!(d, NaiveDate::from_ymd_opt(2026, 4, 27).unwrap());
    }

    #[test]
    fn extract_dependencies_parses_canonical_line() {
        let deps = extract_dependencies("**Depends on:** ADR-2604101600 (one), ADR-027 (two)\n");
        assert_eq!(deps, vec!["ADR-2604101600".to_string(), "ADR-027".to_string()]);
    }

    #[test]
    fn extract_dependencies_ignores_other_adr_mentions() {
        let content = "## Context\n\nReferences ADR-001 in prose.\n\n**Depends on:** ADR-002\n";
        let deps = extract_dependencies(content);
        assert_eq!(deps, vec!["ADR-002".to_string()]);
    }

    #[test]
    fn extract_h1_adr_id_finds_canonical_title() {
        let content = "# ADR-2604270800: My Title\n";
        assert_eq!(extract_h1_adr_id(content), Some("ADR-2604270800".to_string()));
    }

    #[test]
    fn extract_h1_adr_id_returns_none_when_no_id() {
        let content = "# A plain title\n";
        assert_eq!(extract_h1_adr_id(content), None);
    }

    // ── Detector tests against fixtures (P1.2) ───────────────────────────

    fn fixture_dir() -> PathBuf {
        let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        p.push("tests/fixtures/adr-doctor");
        p
    }

    fn read_fixture(name: &str) -> (PathBuf, String) {
        let path = fixture_dir().join(name);
        let content = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("failed to read fixture {}: {}", path.display(), e));
        (path, content)
    }

    fn kinds(findings: &[Finding]) -> Vec<FindingKind> {
        findings.iter().map(|f| f.kind).collect()
    }

    // UnparseableStatus ─────────────────────────────────────────────────

    const FX_CLEAN: &str = "ADR-2604010001-clean.md";
    const FX_UNPARSE_BULLET: &str = "ADR-2604010002-unparseable-status-bullet-bold.md";
    const FX_UNPARSE_BOLD_OUT: &str = "ADR-2604010003-unparseable-status-bold-colon-outside.md";
    const FX_UNPARSE_ENUM: &str = "ADR-2604010004-unparseable-status-enum-listing.md";
    const FX_UNPARSE_INVALID: &str = "ADR-2604010005-unparseable-status-invalid-value.md";
    const FX_ID_MISMATCH: &str = "ADR-2604010006-id-mismatch.md";
    const FX_MISSING_STATUS: &str = "ADR-2604010007-missing-status.md";
    const FX_MISSING_DATE: &str = "ADR-2604010008-missing-date.md";
    const FX_MISSING_H1: &str = "ADR-2604010009-missing-h1.md";
    const FX_STALE: &str = "ADR-2604010010-stale-proposed.md";
    const FX_RECENT: &str = "ADR-2604010011-recent-proposed.md";
    const FX_SUPER_NO_LINK: &str = "ADR-2604010012-superseded-no-link.md";
    const FX_SUPER_LINKED: &str = "ADR-2604010013-superseded-with-link.md";
    const FX_DEP_TARGET: &str = "ADR-2604010014-dep-target.md";
    const FX_DEP_GOOD: &str = "ADR-2604010015-dep-good.md";
    const FX_DEP_DANGLING: &str = "ADR-2604010016-dep-dangling.md";
    const FX_DUP_A: &str = "ADR-2604010099-a.md";
    const FX_DUP_B: &str = "ADR-2604010099-b.md";

    #[test]
    fn unparseable_status_flags_bullet_bold_form() {
        // `- **Status**: Proposed` — colon outside bold, bullet-prefixed.
        let (path, content) = read_fixture(FX_UNPARSE_BULLET);
        let findings = detect_unparseable_status(&path, &content);
        assert_eq!(kinds(&findings), vec![FindingKind::UnparseableStatus]);
        assert!(findings[0].detail.contains("non-canonical"));
    }

    #[test]
    fn unparseable_status_flags_bold_colon_outside() {
        // `**Status**: Proposed` — colon outside bold.
        let (path, content) = read_fixture(FX_UNPARSE_BOLD_OUT);
        let findings = detect_unparseable_status(&path, &content);
        assert_eq!(kinds(&findings), vec![FindingKind::UnparseableStatus]);
    }

    #[test]
    fn unparseable_status_flags_enum_listing() {
        // `**Status:** Proposed | Accepted | Deprecated` — multi-value enum.
        let (path, content) = read_fixture(FX_UNPARSE_ENUM);
        let findings = detect_unparseable_status(&path, &content);
        assert_eq!(kinds(&findings), vec![FindingKind::UnparseableStatus]);
        assert!(findings[0].detail.contains("not a recognized lifecycle"));
    }

    #[test]
    fn unparseable_status_flags_invalid_value() {
        // `**Status:** Banana` — unknown value.
        let (path, content) = read_fixture(FX_UNPARSE_INVALID);
        let findings = detect_unparseable_status(&path, &content);
        assert_eq!(kinds(&findings), vec![FindingKind::UnparseableStatus]);
    }

    #[test]
    fn unparseable_status_clean_returns_nothing() {
        let (path, content) = read_fixture(FX_CLEAN);
        let findings = detect_unparseable_status(&path, &content);
        assert!(findings.is_empty(), "clean fixture must not flag UnparseableStatus");
    }

    #[test]
    fn unparseable_status_silent_when_no_status_line() {
        // No Status line → not UnparseableStatus (that's MissingRequiredField).
        let (path, content) = read_fixture(FX_MISSING_STATUS);
        let findings = detect_unparseable_status(&path, &content);
        assert!(findings.is_empty());
    }

    // IdFormatMismatch ──────────────────────────────────────────────────

    #[test]
    fn id_format_mismatch_flags_h1_filename_divergence() {
        let (path, content) = read_fixture(FX_ID_MISMATCH);
        let findings = detect_id_format_mismatch(&path, &content);
        assert_eq!(kinds(&findings), vec![FindingKind::IdFormatMismatch]);
        assert_eq!(findings[0].adr_id, "ADR-2604010006");
        assert!(findings[0].detail.contains("ADR-2604010006"));
        assert!(findings[0].detail.contains("ADR-9999999999"));
    }

    #[test]
    fn id_format_mismatch_clean_returns_nothing() {
        let (path, content) = read_fixture(FX_CLEAN);
        let findings = detect_id_format_mismatch(&path, &content);
        assert!(findings.is_empty());
    }

    // MissingRequiredField ──────────────────────────────────────────────

    #[test]
    fn missing_required_field_flags_absent_status() {
        let (path, content) = read_fixture(FX_MISSING_STATUS);
        let findings = detect_missing_required_field(&path, &content);
        assert_eq!(kinds(&findings), vec![FindingKind::MissingRequiredField]);
        assert!(findings[0].detail.contains("Status"));
    }

    #[test]
    fn missing_required_field_flags_absent_date() {
        let (path, content) = read_fixture(FX_MISSING_DATE);
        let findings = detect_missing_required_field(&path, &content);
        assert_eq!(kinds(&findings), vec![FindingKind::MissingRequiredField]);
        assert!(findings[0].detail.contains("Date"));
    }

    #[test]
    fn missing_required_field_flags_absent_h1_title() {
        let (path, content) = read_fixture(FX_MISSING_H1);
        let findings = detect_missing_required_field(&path, &content);
        assert_eq!(kinds(&findings), vec![FindingKind::MissingRequiredField]);
        assert!(findings[0].detail.contains("H1"));
    }

    #[test]
    fn missing_required_field_clean_returns_nothing() {
        let (path, content) = read_fixture(FX_CLEAN);
        let findings = detect_missing_required_field(&path, &content);
        assert!(findings.is_empty());
    }

    // StaleProposed ─────────────────────────────────────────────────────

    #[test]
    fn stale_proposed_flags_old_date() {
        let (path, content) = read_fixture(FX_STALE);
        // The fixture has Date: 2025-01-01; now = 2026-04-27 → 481 days old.
        let now = NaiveDate::from_ymd_opt(2026, 4, 27).unwrap();
        let findings = detect_stale_proposed(&path, &content, now);
        assert_eq!(kinds(&findings), vec![FindingKind::StaleProposed]);
        assert!(findings[0].detail.contains("days ago"));
    }

    #[test]
    fn stale_proposed_does_not_flag_recent() {
        let (path, content) = read_fixture(FX_RECENT);
        // The fixture has Date: 2026-04-25; now = 2026-04-27 → 2 days old.
        let now = NaiveDate::from_ymd_opt(2026, 4, 27).unwrap();
        let findings = detect_stale_proposed(&path, &content, now);
        assert!(findings.is_empty());
    }

    #[test]
    fn stale_proposed_does_not_flag_accepted_status() {
        let (path, content) = read_fixture(FX_CLEAN);
        let now = NaiveDate::from_ymd_opt(2030, 1, 1).unwrap();
        let findings = detect_stale_proposed(&path, &content, now);
        assert!(findings.is_empty(), "Accepted ADR must not trigger StaleProposed");
    }

    #[test]
    fn stale_proposed_threshold_is_30_days_inclusive() {
        let (path, content) = read_fixture(FX_STALE);
        // Exactly 30 days after Date: 2025-01-01 → 2025-01-31. Should NOT fire (≤ 30).
        let now = NaiveDate::from_ymd_opt(2025, 1, 31).unwrap();
        assert!(detect_stale_proposed(&path, &content, now).is_empty());
        // 31 days → fires.
        let now = NaiveDate::from_ymd_opt(2025, 2, 1).unwrap();
        assert_eq!(kinds(&detect_stale_proposed(&path, &content, now)),
                   vec![FindingKind::StaleProposed]);
    }

    // SupersededUnlinked ────────────────────────────────────────────────

    #[test]
    fn superseded_unlinked_flags_missing_link() {
        let (path, content) = read_fixture(FX_SUPER_NO_LINK);
        let findings = detect_superseded_unlinked(&path, &content);
        assert_eq!(kinds(&findings), vec![FindingKind::SupersededUnlinked]);
    }

    #[test]
    fn superseded_with_link_returns_nothing() {
        let (path, content) = read_fixture(FX_SUPER_LINKED);
        let findings = detect_superseded_unlinked(&path, &content);
        assert!(findings.is_empty(), "Superseded with link must not flag");
    }

    #[test]
    fn superseded_unlinked_does_not_flag_other_statuses() {
        let (path, content) = read_fixture(FX_CLEAN);
        let findings = detect_superseded_unlinked(&path, &content);
        assert!(findings.is_empty());
    }

    // DuplicateId (cross-file) ──────────────────────────────────────────

    #[test]
    fn duplicate_id_flags_collision() {
        let (path_a, content_a) = read_fixture(FX_DUP_A);
        let (path_b, content_b) = read_fixture(FX_DUP_B);
        let corpus = vec![
            (path_a.clone(), content_a),
            (path_b.clone(), content_b),
        ];
        let findings = detect_duplicate_ids(&corpus);
        assert_eq!(findings.len(), 2, "both colliding files should be flagged");
        for f in &findings {
            assert_eq!(f.kind, FindingKind::DuplicateId);
            assert_eq!(f.adr_id, "ADR-2604010099");
        }
        let paths: HashSet<&PathBuf> = findings.iter().map(|f| &f.file_path).collect();
        assert!(paths.contains(&path_a));
        assert!(paths.contains(&path_b));
    }

    #[test]
    fn duplicate_id_clean_corpus_returns_nothing() {
        let (path_a, content_a) = read_fixture(FX_CLEAN);
        let (path_b, content_b) = read_fixture(FX_RECENT);
        let corpus = vec![(path_a, content_a), (path_b, content_b)];
        let findings = detect_duplicate_ids(&corpus);
        assert!(findings.is_empty());
    }

    // DanglingDependency (cross-file) ───────────────────────────────────

    #[test]
    fn dangling_dependency_flags_missing_target() {
        let (path, content) = read_fixture(FX_DEP_DANGLING);
        // Corpus contains only this one file → its dep on ADR-9999999999 is dangling.
        let corpus = vec![(path.clone(), content)];
        let findings = detect_dangling_dependencies(&corpus);
        assert_eq!(kinds(&findings), vec![FindingKind::DanglingDependency]);
        assert!(findings[0].detail.contains("ADR-9999999999"));
    }

    #[test]
    fn dangling_dependency_satisfied_when_target_present() {
        let (path_a, content_a) = read_fixture(FX_DEP_TARGET);
        let (path_b, content_b) = read_fixture(FX_DEP_GOOD);
        let corpus = vec![(path_a, content_a), (path_b, content_b)];
        let findings = detect_dangling_dependencies(&corpus);
        assert!(findings.is_empty(), "dep target is in corpus, no flag");
    }

    #[test]
    fn dangling_dependency_dedupes_within_file() {
        // If a file lists the same missing ID twice on the Depends on line,
        // we emit one finding, not two.
        let path = PathBuf::from("ADR-1111111111-synthetic.md");
        let content = "**Depends on:** ADR-9999999999 (one), ADR-9999999999 (dup)\n";
        let corpus = vec![(path.clone(), content.to_string())];
        let findings = detect_dangling_dependencies(&corpus);
        assert_eq!(findings.len(), 1);
    }

    // ── Auto-fix patch tests (P2.1) ──────────────────────────────────────
    //
    // Three positive cases mirror real ADR frontmatter that was hand-normalized
    // earlier in this session (ADR-2604141400, ADR-2604150100, ADR-2604150130).
    // The "before" text in each case is the exact buggy form pulled from the
    // pre-normalization commit; "after" is the canonical form post-fix.

    fn unparseable_finding() -> Finding {
        finding(
            "ADR-2604010002",
            PathBuf::from("ADR-2604010002.md"),
            FindingKind::UnparseableStatus,
            "buggy frontmatter form",
        )
    }

    #[test]
    fn auto_fix_patch_normalizes_brain_queue_swarm_lease_frontmatter() {
        // Real pre-normalization frontmatter from
        // docs/adrs/ADR-2604141400-brain-queue-swarm-lease.md.
        let before = "\
- **Status**: §1 Accepted 2026-04-14 (P1 scope complete — git-evidence guard, inline fallback, queue history); §2 (swarm-lease + task states) Proposed
- **Date**: 2026-04-14
- **Depends on**: ADR-2604132330 (brain queue), ADR-2604150000 (brain→sched rename), ADR-027 (HexFlo)
- **Relates to**: feedback_verify_before_done, feedback_use_hexflo_hex_agent
";
        let expected = "\
**Status:** §1 Accepted 2026-04-14 (P1 scope complete — git-evidence guard, inline fallback, queue history); §2 (swarm-lease + task states) Proposed
**Date:** 2026-04-14
**Depends on:** ADR-2604132330 (brain queue), ADR-2604150000 (brain→sched rename), ADR-027 (HexFlo)
**Relates to:** feedback_verify_before_done, feedback_use_hexflo_hex_agent
";
        let patch = unparseable_finding().auto_fix_patch().expect("Tier-A finding must have a patch");
        assert_eq!(patch.apply(before).unwrap(), expected);
    }

    #[test]
    fn auto_fix_patch_normalizes_worktree_merge_drop_commits_frontmatter() {
        // Real pre-normalization frontmatter from
        // docs/adrs/ADR-2604150100-worktree-merge-fast-forward-drops-commits.md.
        let before = "\
- **Status**: Proposed
- **Date**: 2026-04-15
- **Depends on**: ADR-2604131930 (original worktree-merge integrity claim)
- **Relates to**: `feedback_use_hex_worktree_merge.md`
";
        let expected = "\
**Status:** Proposed
**Date:** 2026-04-15
**Depends on:** ADR-2604131930 (original worktree-merge integrity claim)
**Relates to:** `feedback_use_hex_worktree_merge.md`
";
        let patch = unparseable_finding().auto_fix_patch().unwrap();
        assert_eq!(patch.apply(before).unwrap(), expected);
    }

    #[test]
    fn auto_fix_patch_normalizes_worktree_cleanup_frontmatter() {
        // Real pre-normalization frontmatter from
        // docs/adrs/ADR-2604150130-worktree-cleanup-kills-active-worktree.md.
        let before = "\
- **Status**: Proposed
- **Date**: 2026-04-15
- **Depends on**: ADR-2604150100 (related worktree-merge drop-commits bug)
- **Relates to**: `feedback_enforce_worktrees.md`, `feedback_use_hex_worktree_merge.md`
";
        let expected = "\
**Status:** Proposed
**Date:** 2026-04-15
**Depends on:** ADR-2604150100 (related worktree-merge drop-commits bug)
**Relates to:** `feedback_enforce_worktrees.md`, `feedback_use_hex_worktree_merge.md`
";
        let patch = unparseable_finding().auto_fix_patch().unwrap();
        assert_eq!(patch.apply(before).unwrap(), expected);
    }

    // Negative case 1 — Tier B finding has no auto-fix patch (StaleProposed
    // requires drafting in a worktree, not a mechanical rewrite).
    #[test]
    fn auto_fix_patch_returns_none_for_tier_b_finding() {
        let f = finding(
            "ADR-001",
            PathBuf::from("x.md"),
            FindingKind::StaleProposed,
            "",
        );
        assert_eq!(f.tier, AutoFixTier::B, "preconditions: StaleProposed must be Tier B");
        assert!(f.auto_fix_patch().is_none());
    }

    // Negative case 2 — Tier C finding has no auto-fix patch (DuplicateId
    // requires human judgment about which ADR-ID to renumber).
    #[test]
    fn auto_fix_patch_returns_none_for_tier_c_finding() {
        let f = finding(
            "ADR-001",
            PathBuf::from("x.md"),
            FindingKind::DuplicateId,
            "",
        );
        assert_eq!(f.tier, AutoFixTier::C, "preconditions: DuplicateId must be Tier C");
        assert!(f.auto_fix_patch().is_none());
    }

    // Negative case 3 — applying the patch to already-canonical content is a
    // no-op. This is what makes the shadow-promotion safety check work: the
    // P2.2 orchestrator re-runs doctor in --strict mode after applying, and
    // would catch any spurious mutation that re-introduced a finding.
    #[test]
    fn auto_fix_patch_is_idempotent_on_canonical_content() {
        let canonical = "\
# ADR-2604010001: Clean

**Status:** Accepted
**Date:** 2026-04-15
**Depends on:** ADR-027
**Relates to:** feedback_x

## Context
Body that mentions `**Status**: foo` inside a code span — must NOT be
rewritten because it isn't at the start of a line as a bullet.
";
        let patch = unparseable_finding().auto_fix_patch().unwrap();
        let after = patch.apply(canonical).unwrap();
        assert_eq!(after, canonical, "canonical input must round-trip unchanged");
    }

    // Bonus: re-applying the patch to its own output is also a no-op (proves
    // the regex doesn't accidentally match the canonical form it produces).
    #[test]
    fn auto_fix_patch_double_apply_is_stable() {
        let buggy = "\
- **Status**: Proposed
- **Date**: 2026-04-15
";
        let patch = unparseable_finding().auto_fix_patch().unwrap();
        let once = patch.apply(buggy).unwrap();
        let twice = patch.apply(&once).unwrap();
        assert_eq!(once, twice);
    }

    // Defense-in-depth: every Tier-A kind in the rule table must produce a
    // patch (otherwise shadow-promotion would silently no-op on real
    // findings). Today there's exactly one Tier-A kind, but this test will
    // catch the mistake of adding a second kind to Tier A without wiring up
    // its patch.
    #[test]
    fn auto_fix_patch_present_for_every_tier_a() {
        for (kind, tier, _) in RULE_TABLE {
            if *tier != AutoFixTier::A {
                continue;
            }
            let f = finding("ADR-X", PathBuf::from("x.md"), *kind, "");
            assert!(
                f.auto_fix_patch().is_some(),
                "Tier-A kind {:?} has no auto_fix_patch — shadow-promotion would no-op",
                kind,
            );
        }
    }

    // Integration: scan() composes all detectors over a corpus ──────────

    #[test]
    fn scan_combines_per_file_and_cross_file_findings() {
        let (p_dup_a, c_dup_a) = read_fixture(FX_DUP_A);
        let (p_dup_b, c_dup_b) = read_fixture(FX_DUP_B);
        let (p_clean, c_clean) = read_fixture(FX_CLEAN);
        let corpus = vec![
            (p_dup_a, c_dup_a),
            (p_dup_b, c_dup_b),
            (p_clean, c_clean),
        ];
        let now = NaiveDate::from_ymd_opt(2026, 4, 27).unwrap();
        let findings = scan(&corpus, now);
        let dup = findings.iter().filter(|f| f.kind == FindingKind::DuplicateId).count();
        assert_eq!(dup, 2, "two duplicate-id findings expected, got {:?}", findings);
    }
}
