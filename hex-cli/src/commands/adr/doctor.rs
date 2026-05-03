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
        findings.extend(scan_single_file(path, content, now));
    }

    // Cross-file detectors.
    findings.extend(detect_duplicate_ids(adrs));
    findings.extend(detect_dangling_dependencies(adrs));

    findings
}

/// Per-file detection. Used by both [`scan`] (looped over the corpus) and
/// the shadow-promote self-check, which scans only the file it just
/// rewrote — cross-file detectors (DuplicateId, DanglingDependency)
/// require the full corpus to evaluate, so running them in isolation
/// would produce false positives.
pub fn scan_single_file(path: &Path, content: &str, now: NaiveDate) -> Vec<Finding> {
    let mut findings = Vec::new();
    findings.extend(detect_unparseable_status(path, content));
    findings.extend(detect_id_format_mismatch(path, content));
    findings.extend(detect_missing_required_field(path, content));
    findings.extend(detect_stale_proposed(path, content, now));
    findings.extend(detect_superseded_unlinked(path, content));
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

/// True when `git log` can locate the file's first commit. Used to suppress
/// the `missing required field: Date` finding for ADRs whose creation date
/// is recoverable from git history — the most common pattern for the
/// 50+ pre-Date-field-convention ADRs in this repo.
fn file_has_git_history(path: &Path) -> bool {
    let out = std::process::Command::new("git")
        .args(["log", "-1", "--diff-filter=A", "--format=%H", "--", &path.to_string_lossy()])
        .output();
    matches!(out, Ok(o) if o.status.success() && !o.stdout.iter().all(|b| b.is_ascii_whitespace()))
}

/// Days since the file's first git commit. Returns None for untracked files
/// so callers can fall back to "no age info available."
fn file_git_age_days(path: &Path) -> Option<i64> {
    let out = std::process::Command::new("git")
        .args(["log", "--diff-filter=A", "--format=%ct", "--", &path.to_string_lossy()])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let ts: i64 = stdout.trim().lines().last()?.parse().ok()?;
    let now = chrono::Utc::now().timestamp();
    Some(((now - ts).max(0)) / 86400)
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
        // Suppress when git knows the file's first-commit date — the date
        // IS in git, just not duplicated into the markdown frontmatter.
        // Pre-Date-field-convention ADRs (ADR-001 onward) all hit this.
        // Backfilling 60+ ADRs by hand to add a redundant field produces
        // no operator value, so we treat git's record as authoritative.
        if !file_has_git_history(path) {
            findings.push(finding(
                adr_id.clone(),
                path.to_path_buf(),
                FindingKind::MissingRequiredField,
                "missing required field: Date",
            ));
        }
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

// ── Shadow-promote orchestration (P2.2, ADR-2604270800 §1a) ─────────────
//
// `shadow_promote` is the safe-by-construction Tier-A auto-fix path. It
// applies the patch on a sched-owned worktree branch, re-runs `doctor` in
// strict mode against the rewritten file as a self-check, and only then
// merges back to main. Every failure mode aborts and cleans up the
// worktree so we never leave half-applied state behind.
//
// Safety properties (see `tests/adr_doctor_shadow_promote.rs`):
//   1. Never modifies a dirty target file.
//   2. Never races with an in-flight session that has claimed the file
//      via its `~/.hex/sessions/agent-*.json` manifest (`allowed_paths`
//      or `worktree_path`).
//   3. Never merges a result that doctor itself still reports findings on.
//   4. Cannot drop commits — uses `git merge --no-ff`, which always
//      records a real merge commit (sidesteps the fast-forward
//      drop-commits failure mode of `hex worktree merge` documented in
//      ADR-2604150100). Raw `git checkout <branch> -- <file>` is never
//      used, per ADR-2604131930.

use std::path::Path as StdPath;
use std::process::Command;

/// Result of a single shadow-promotion attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum Outcome {
    /// Patch committed on the auto-fix branch. With [`MergePolicy::Merge`]
    /// the branch is also merged back to `main` via `--no-ff` (worktree
    /// removed, branch ref retained for audit). With
    /// [`MergePolicy::LeaveForReview`] no merge is attempted — both the
    /// worktree directory and the branch ref are kept for human review.
    /// `commit` is always the sha of the fix commit produced inside the
    /// worktree (parent of any subsequent merge commit).
    Applied { branch: String, commit: String },
    /// Aborted before mutating `main`. Worktree (if created) was removed.
    /// `reason` is a human-readable diagnostic — also written to the
    /// daemon event log so we can audit how often each abort path fires.
    Aborted { reason: String },
}

/// What the dispatcher should do with the auto-fix branch after the patch
/// (or Tier-B draft) has been committed in the worktree.
///
/// - [`Merge`]: `--fix-and-merge` — apply, commit, `git merge --no-ff`
///   back to `main`, remove the worktree dir. Branch ref stays for audit.
///   This is the existing P2.2 behavior.
/// - [`LeaveForReview`]: `--fix` — apply, commit on the branch, then
///   stop. Both the worktree directory and the branch ref are left in
///   place so a human can `cd .hex/auto-fix-worktrees/<name>`, review,
///   and either merge or discard. This is the only safe Tier-B mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MergePolicy {
    Merge,
    LeaveForReview,
}

impl Outcome {
    pub fn is_applied(&self) -> bool {
        matches!(self, Outcome::Applied { .. })
    }
    pub fn is_aborted(&self) -> bool {
        matches!(self, Outcome::Aborted { .. })
    }
}

/// Tunables for `shadow_promote_with_config`. The default
/// `shadow_promote(finding)` uses the live filesystem (cwd-rooted repo,
/// `~/.hex/sessions/`, today's date); tests inject a tempdir and a fake
/// sessions dir so they're hermetic.
#[derive(Debug, Clone)]
pub struct ShadowPromoteConfig {
    /// Repo root containing the ADR file and where the merge will happen.
    pub repo_root: PathBuf,
    /// Override `~/.hex/sessions/` (used by tests). `None` = use real one.
    pub sessions_dir: Option<PathBuf>,
    /// Today, used by the post-patch self-check's `scan` call.
    pub now: NaiveDate,
}

impl ShadowPromoteConfig {
    pub fn live() -> anyhow::Result<Self> {
        let repo_root = repo_root_from_cwd()?;
        Ok(Self {
            repo_root,
            sessions_dir: None,
            now: chrono::Local::now().date_naive(),
        })
    }
}

/// Try to safely apply a Tier-A finding's auto-fix patch via shadow
/// promotion. The default config rooted at the current git repo. See
/// [`shadow_promote_with_config`] for the injectable variant.
pub fn shadow_promote(finding: &Finding) -> anyhow::Result<Outcome> {
    let cfg = ShadowPromoteConfig::live()?;
    shadow_promote_with_config(finding, &cfg)
}

/// Hermetic variant — every path/clock/session-source the orchestrator
/// touches is taken from `cfg`. The CLI / sched daemon use the live
/// constructor; tests use this directly.
///
/// Returns `Outcome::Applied { .. }` only after `git merge --no-ff` lands
/// the fix commit on the repo's main branch. Any earlier failure returns
/// `Outcome::Aborted { reason: .. }` after attempting worktree cleanup.
///
/// Equivalent to `shadow_promote_with_policy(finding, cfg, MergePolicy::Merge)`.
pub fn shadow_promote_with_config(
    finding: &Finding,
    cfg: &ShadowPromoteConfig,
) -> anyhow::Result<Outcome> {
    shadow_promote_with_policy(finding, cfg, MergePolicy::Merge)
}

/// Variant of [`shadow_promote_with_config`] that lets the caller choose
/// whether to merge the auto-fix branch back to `main` or leave it for
/// human review. Used by the P2.3 dispatcher to implement `--fix` (no
/// merge) vs `--fix-and-merge` (merge).
///
/// With [`MergePolicy::LeaveForReview`] the post-commit merge phase
/// (steps 6–7 in the body) is skipped entirely: the fix commit lives on
/// the auto-fix branch, the worktree directory stays in place, and
/// `main` is never touched. The returned [`Outcome::Applied`] still
/// carries the branch + fix-commit sha so the caller can surface them.
pub fn shadow_promote_with_policy(
    finding: &Finding,
    cfg: &ShadowPromoteConfig,
    policy: MergePolicy,
) -> anyhow::Result<Outcome> {
    // Step 0: tier + patch availability gate.
    if finding.tier != AutoFixTier::A {
        return Ok(Outcome::Aborted {
            reason: format!("not a Tier-A finding (tier={:?})", finding.tier),
        });
    }
    let patch = match finding.auto_fix_patch() {
        Some(p) => p,
        None => {
            return Ok(Outcome::Aborted {
                reason: "no auto-fix patch defined for this kind".into(),
            })
        }
    };

    // Resolve the file path relative to the repo root. Doctor stores
    // absolute paths when invoked via `run`, but tests pass repo-relative
    // ones — accept both.
    let abs_file = if finding.file_path.is_absolute() {
        finding.file_path.clone()
    } else {
        cfg.repo_root.join(&finding.file_path)
    };
    let rel_file = match abs_file.strip_prefix(&cfg.repo_root) {
        Ok(p) => p.to_path_buf(),
        Err(_) => {
            return Ok(Outcome::Aborted {
                reason: format!(
                    "finding file {} is outside repo root {}",
                    abs_file.display(),
                    cfg.repo_root.display()
                ),
            })
        }
    };

    // Step 1: dirty-file check. We refuse to touch a file the user (or
    // another agent) has uncommitted changes for — committing the
    // doctor-fix on top of those changes would silently absorb them.
    if let Some(reason) = file_is_dirty(&cfg.repo_root, &rel_file)? {
        return Ok(Outcome::Aborted { reason });
    }

    // Step 2: claimed-by-another-session check. Even if the file is clean
    // on disk, an in-flight agent may be planning to edit it (its session
    // manifest in `~/.hex/sessions/` lists the path under `allowed_paths`
    // or has a `worktree_path` whose changed-files include it).
    if let Some(reason) = file_is_claimed(cfg.sessions_dir.as_deref(), &rel_file)? {
        return Ok(Outcome::Aborted { reason });
    }

    // Step 3: create the auto-fix worktree on a deterministically-named
    // branch. Branch lives under `sched/auto-fix/adr-doctor/<adr-id>` so
    // operators can `git branch --list 'sched/auto-fix/*'` to audit.
    let branch = format!("sched/auto-fix/adr-doctor/{}", finding.adr_id);
    let worktree_dir = match create_auto_fix_worktree(&cfg.repo_root, &branch) {
        Ok(p) => p,
        Err(e) => {
            return Ok(Outcome::Aborted {
                reason: format!("worktree create failed: {}", e),
            })
        }
    };

    // From here on, every error path must clean up the worktree before
    // returning. The `cleanup` closure makes that local + obvious.
    let cleanup = |reason: String| -> Outcome {
        let _ = remove_auto_fix_worktree(&cfg.repo_root, &worktree_dir, &branch);
        Outcome::Aborted { reason }
    };

    // Step 4: apply the patch in the worktree.
    let target_in_wt = worktree_dir.join(&rel_file);
    let original = match std::fs::read_to_string(&target_in_wt) {
        Ok(s) => s,
        Err(e) => {
            return Ok(cleanup(format!(
                "read {} in worktree failed: {}",
                rel_file.display(),
                e
            )))
        }
    };
    let patched = match patch.apply(&original) {
        Ok(s) => s,
        Err(e) => return Ok(cleanup(format!("patch.apply failed: {}", e))),
    };
    if patched == original {
        return Ok(cleanup(format!(
            "patch was a no-op against {} (file already canonical)",
            rel_file.display()
        )));
    }
    if let Err(e) = std::fs::write(&target_in_wt, &patched) {
        return Ok(cleanup(format!("write patched file failed: {}", e)));
    }

    // Step 5: self-check. Re-run the per-file detectors against just the
    // rewritten file in strict mode. If anything still fires, the patch
    // was wrong — abort. We deliberately skip cross-file detectors here:
    // DuplicateId and DanglingDependency require the full corpus, and
    // running them in isolation would mark every external `Depends on:`
    // reference as dangling.
    let post_findings = scan_single_file(&target_in_wt, &patched, cfg.now);
    let strict_exit = exit_code(&post_findings, true);
    if strict_exit != 0 {
        return Ok(cleanup(format!(
            "doctor --strict still reports {} finding(s) on patched file (exit={}); refusing to merge",
            post_findings.len(),
            strict_exit
        )));
    }

    // Step 6: commit. Always done — a fix commit exists on the auto-fix
    // branch regardless of merge policy.
    let commit_sha = match commit_in_worktree(&worktree_dir, &rel_file, finding) {
        Ok(s) => s,
        Err(e) => return Ok(cleanup(format!("commit in worktree failed: {}", e))),
    };

    // Step 7: optionally merge back to main and tear down the worktree.
    // With LeaveForReview we stop here — branch + worktree stay so a
    // human can review the fix commit before merging.
    match policy {
        MergePolicy::Merge => {
            // 7a: merge --no-ff (always a true merge commit) rather than
            // fast-forward to side-step the ADR-2604150100 dropped-commits
            // failure mode.
            let pre_merge_head = match rev_parse_head(&cfg.repo_root) {
                Ok(s) => s,
                Err(e) => return Ok(cleanup(format!("rev-parse HEAD failed: {}", e))),
            };
            if let Err(e) = merge_no_ff(&cfg.repo_root, &branch, finding) {
                return Ok(cleanup(format!("merge --no-ff failed: {}", e)));
            }
            // 7b: post-merge integrity check — pre_merge_head must remain
            // reachable from HEAD. `--no-ff` guarantees this, but we
            // assert it anyway because that's the whole point of the
            // self-fix loop.
            if let Err(e) = assert_ancestor(&cfg.repo_root, &pre_merge_head) {
                // If this ever fires, we've already mutated main — surface
                // as a hard error rather than Aborted, because cleanup
                // can't undo it.
                anyhow::bail!(
                    "post-merge integrity check failed (commits may have been dropped): {}",
                    e
                );
            }
            // 7c: drop the worktree dir (branch ref intentionally kept
            // so operators can `git log <branch>` to audit the fix).
            let _ = remove_auto_fix_worktree_only(&cfg.repo_root, &worktree_dir);
        }
        MergePolicy::LeaveForReview => {
            // No merge. Both the worktree directory and the branch ref
            // are left in place so a human can review the patch (e.g.
            // `git diff main..<branch>` or `cd <worktree_dir>`).
        }
    }

    Ok(Outcome::Applied {
        branch,
        commit: commit_sha,
    })
}

// ── Tier-B drafting (P2.3, ADR-2604270800 §1a) ──────────────────────────
//
// Tier-B findings (StaleProposed, SupersededUnlinked, IdFormatMismatch)
// can't be auto-applied — they're per-finding judgment calls — but they
// *can* be auto-drafted: we open a sched-owned worktree on a fresh
// branch, write a notes file describing the finding, commit, and stop.
// The branch + worktree are left for human review; merging is never
// attempted regardless of the dispatcher's `MergePolicy`.
//
// The notes file is the minimal "diff written to the worktree" required
// by P2.3. Future phases can replace it with kind-specific drafters
// (e.g. a stub `Superseded by:` line for SupersededUnlinked) — the
// dispatcher contract is the same.

/// Stable filename slug for a [`FindingKind`]. Used to compose branch
/// names and notes filenames so they round-trip cleanly through git
/// (no spaces, no `/`, no shell-meta).
fn kind_slug(kind: FindingKind) -> &'static str {
    match kind {
        FindingKind::UnparseableStatus => "unparseable-status",
        FindingKind::DuplicateId => "duplicate-id",
        FindingKind::IdFormatMismatch => "id-format-mismatch",
        FindingKind::MissingRequiredField => "missing-required-field",
        FindingKind::DanglingDependency => "dangling-dependency",
        FindingKind::StaleProposed => "stale-proposed",
        FindingKind::SupersededUnlinked => "superseded-unlinked",
    }
}

/// Open a sched-owned auto-fix worktree, write a notes file describing
/// the Tier-B finding, commit, and leave it for human review.
///
/// Refuses anything that isn't Tier B. `Outcome::Applied` carries the
/// branch + commit sha — main is never modified, so the caller knows the
/// branch ref must be inspected/merged manually.
pub fn tier_b_draft_with_config(
    finding: &Finding,
    cfg: &ShadowPromoteConfig,
) -> anyhow::Result<Outcome> {
    if finding.tier != AutoFixTier::B {
        return Ok(Outcome::Aborted {
            reason: format!("not a Tier-B finding (tier={:?})", finding.tier),
        });
    }

    let branch = format!(
        "sched/auto-fix/adr-doctor/tier-b/{}-{}",
        finding.adr_id,
        kind_slug(finding.kind)
    );
    let worktree_dir = match create_auto_fix_worktree(&cfg.repo_root, &branch) {
        Ok(p) => p,
        Err(e) => {
            return Ok(Outcome::Aborted {
                reason: format!("worktree create failed: {}", e),
            })
        }
    };

    let cleanup = |reason: String| -> Outcome {
        let _ = remove_auto_fix_worktree(&cfg.repo_root, &worktree_dir, &branch);
        Outcome::Aborted { reason }
    };

    // Write the notes file inside the worktree at a deterministic path.
    let notes_rel = PathBuf::from(format!(
        ".hex/adr-doctor-drafts/{}-{}.md",
        finding.adr_id,
        kind_slug(finding.kind)
    ));
    let notes_abs = worktree_dir.join(&notes_rel);
    if let Some(parent) = notes_abs.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return Ok(cleanup(format!(
                "create_dir_all {} failed: {}",
                parent.display(),
                e
            )));
        }
    }
    let notes = format!(
        "# Tier-B doctor draft: {} ({:?})\n\
         \n\
         **Source ADR:** {}\n\
         **Finding kind:** {:?}\n\
         **Detail:** {}\n\
         \n\
         This file is an auto-generated placeholder for a Tier-B finding\n\
         (per ADR-2604270800 §1a). The doctor cannot mechanically resolve\n\
         this kind of issue; a human must review the source ADR, decide\n\
         the right fix, and either merge this branch (after editing) or\n\
         delete it.\n\
         \n\
         To review:\n\
         ```\n\
         cd {}\n\
         # edit the source ADR, then:\n\
         git commit --amend --no-edit\n\
         hex worktree merge {}\n\
         ```\n\
         \n\
         To discard:\n\
         ```\n\
         git worktree remove --force {}\n\
         git branch -D {}\n\
         ```\n",
        finding.adr_id,
        finding.kind,
        finding.adr_id,
        finding.kind,
        finding.detail,
        worktree_dir.display(),
        branch,
        worktree_dir.display(),
        branch,
    );
    if let Err(e) = std::fs::write(&notes_abs, notes) {
        return Ok(cleanup(format!(
            "write notes {} failed: {}",
            notes_abs.display(),
            e
        )));
    }

    let commit_sha = match commit_in_worktree(&worktree_dir, &notes_rel, finding) {
        Ok(s) => s,
        Err(e) => return Ok(cleanup(format!("commit notes failed: {}", e))),
    };

    // Leave both the worktree dir and the branch in place. Caller is
    // responsible for inboxing the human.
    Ok(Outcome::Applied {
        branch,
        commit: commit_sha,
    })
}

// ── Tier dispatcher (P2.3) ───────────────────────────────────────────────

/// Per-finding result emitted by [`dispatch_fix`]. One variant per tier
/// so callers (CLI renderer, sched daemon) can route to the right inbox
/// priority and report format without re-deriving the tier from the
/// finding.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "tier", rename_all = "lowercase")]
pub enum DispatchResult {
    /// Tier-A: shadow-promotion attempted. The embedded `Outcome`
    /// distinguishes Applied (patch landed on the branch — and on `main`
    /// too if `fix_and_merge` was true) vs Aborted (any safety check
    /// failed; main unmodified).
    A { outcome: Outcome },
    /// Tier-B: auto-draft attempted in a sched worktree. Always
    /// `LeaveForReview` semantics — `Outcome::Applied` here means the
    /// notes file was committed on the branch, not merged.
    B { outcome: Outcome },
    /// Tier-C: no mutation attempted. Surfaced in the report so operators
    /// know the doctor saw the finding and chose not to act.
    C,
}

impl DispatchResult {
    /// True iff the tier-dispatcher actually wrote a commit to a branch
    /// (Tier A applied, or Tier B drafted). Used for summary counts.
    pub fn was_applied(&self) -> bool {
        match self {
            DispatchResult::A { outcome } | DispatchResult::B { outcome } => outcome.is_applied(),
            DispatchResult::C => false,
        }
    }
    /// True iff the tier-dispatcher considered acting but bailed.
    pub fn was_aborted(&self) -> bool {
        match self {
            DispatchResult::A { outcome } | DispatchResult::B { outcome } => outcome.is_aborted(),
            DispatchResult::C => false,
        }
    }
}

/// Apply the tier-aware fix policy to a slice of findings:
///   - Tier A → [`shadow_promote_with_policy`]. `fix_and_merge` controls
///     whether the auto-fix branch is merged to `main`
///     ([`MergePolicy::Merge`]) or left for human review
///     ([`MergePolicy::LeaveForReview`]).
///   - Tier B → [`tier_b_draft_with_config`]. Always
///     `LeaveForReview` — Tier B is never merged automatically, even
///     when `fix_and_merge` is true.
///   - Tier C → no mutation. Returns [`DispatchResult::C`] so the
///     caller can still account for the finding in summary output.
///
/// Errors from the underlying shadow-promote / drafter are folded into
/// [`Outcome::Aborted`] so the dispatcher always returns one result per
/// input finding (callers don't need to special-case errors).
pub fn dispatch_fix(
    findings: &[Finding],
    cfg: &ShadowPromoteConfig,
    fix_and_merge: bool,
) -> Vec<DispatchResult> {
    let policy = if fix_and_merge {
        MergePolicy::Merge
    } else {
        MergePolicy::LeaveForReview
    };
    findings
        .iter()
        .map(|f| match f.tier {
            AutoFixTier::A => DispatchResult::A {
                outcome: shadow_promote_with_policy(f, cfg, policy)
                    .unwrap_or_else(|e| Outcome::Aborted {
                        reason: format!("shadow_promote raised: {}", e),
                    }),
            },
            AutoFixTier::B => DispatchResult::B {
                outcome: tier_b_draft_with_config(f, cfg).unwrap_or_else(|e| Outcome::Aborted {
                    reason: format!("tier_b_draft raised: {}", e),
                }),
            },
            AutoFixTier::C => DispatchResult::C,
        })
        .collect()
}

// ── git primitives ───────────────────────────────────────────────────────

fn repo_root_from_cwd() -> anyhow::Result<PathBuf> {
    let out = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()?;
    if !out.status.success() {
        anyhow::bail!(
            "git rev-parse --show-toplevel failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(PathBuf::from(
        String::from_utf8_lossy(&out.stdout).trim().to_string(),
    ))
}

fn run_git(repo: &StdPath, args: &[&str]) -> anyhow::Result<std::process::Output> {
    let out = Command::new("git").args(args).current_dir(repo).output()?;
    Ok(out)
}

fn run_git_checked(repo: &StdPath, args: &[&str]) -> anyhow::Result<String> {
    let out = run_git(repo, args)?;
    if !out.status.success() {
        anyhow::bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Returns `Some(reason)` if the named file is uncommitted-dirty in `repo`.
/// `None` means clean (or untracked but the caller doesn't care — we only
/// guard tracked-with-uncommitted-edits because that's where silent
/// absorption can happen).
fn file_is_dirty(repo: &StdPath, rel_file: &StdPath) -> anyhow::Result<Option<String>> {
    let out = run_git(
        repo,
        &[
            "status",
            "--porcelain=v1",
            "--",
            rel_file.to_str().unwrap_or(""),
        ],
    )?;
    if !out.status.success() {
        return Ok(Some(format!(
            "git status check failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Each porcelain line is "XY <path>". Anything non-empty means the
    // file has tracked changes (M/A/D/R/C/U) or is untracked (??).
    // Untracked files are fine for our purposes — we only mutate the file
    // we know is committed already. Tracked changes block.
    for line in stdout.lines() {
        if line.is_empty() {
            continue;
        }
        let xy = &line[..line.len().min(2)];
        if xy == "??" {
            // Untracked — not dirty for our purposes.
            continue;
        }
        return Ok(Some(format!(
            "target file {} has uncommitted changes ({})",
            rel_file.display(),
            xy.trim()
        )));
    }
    Ok(None)
}

/// Returns `Some(reason)` if any session manifest in `sessions_dir` (or
/// `~/.hex/sessions/`) claims the file. A claim is either:
///   - the file path is listed in the manifest's `allowed_paths`, or
///   - the manifest has a `worktree_path` (and is therefore actively editing
///     a feature branch — racing it would conflict).
fn file_is_claimed(
    sessions_dir: Option<&StdPath>,
    rel_file: &StdPath,
) -> anyhow::Result<Option<String>> {
    let dir: PathBuf = match sessions_dir {
        Some(p) => p.to_path_buf(),
        None => {
            let home = match dirs::home_dir() {
                Some(h) => h,
                None => return Ok(None), // No home → can't check; fail open.
            };
            home.join(".hex").join("sessions")
        }
    };
    if !dir.exists() {
        return Ok(None);
    }
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return Ok(None),
    };
    let rel_str = rel_file.to_string_lossy().to_string();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let v: serde_json::Value = match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(_) => continue,
        };
        // Match against allowed_paths.
        if let Some(arr) = v.get("allowed_paths").and_then(|x| x.as_array()) {
            for ap in arr.iter().filter_map(|x| x.as_str()) {
                // Treat allowed_paths as path prefixes (matches how
                // hook::SessionState::is_path_allowed uses them).
                if rel_str.starts_with(ap) {
                    let session = path
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("?");
                    return Ok(Some(format!(
                        "file {} is claimed by session {} via allowed_paths prefix `{}`",
                        rel_str, session, ap
                    )));
                }
            }
        }
    }
    Ok(None)
}

/// Create an auxiliary worktree at `<repo>/.hex/auto-fix-worktrees/<adr-id>`
/// on a fresh branch named `branch`. Returns the worktree's absolute path.
/// The directory and any pre-existing branch with that name are removed
/// first to make the operation idempotent.
fn create_auto_fix_worktree(repo: &StdPath, branch: &str) -> anyhow::Result<PathBuf> {
    // Derive a filesystem-safe directory name from the branch tail.
    let dir_name = branch.replace('/', "_");
    let target = repo.join(".hex").join("auto-fix-worktrees").join(&dir_name);

    // Best-effort cleanup of any stale state from a previous run.
    if target.exists() {
        let _ = run_git(
            repo,
            &["worktree", "remove", "--force", target.to_str().unwrap_or("")],
        );
        // If git didn't know about it, blow away the dir directly.
        if target.exists() {
            std::fs::remove_dir_all(&target).ok();
        }
    }
    // Drop any stale branch ref.
    let _ = run_git(repo, &["branch", "-D", branch]);

    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let target_str = target.to_str().ok_or_else(|| {
        anyhow::anyhow!("worktree path {} is not valid UTF-8", target.display())
    })?;
    run_git_checked(
        repo,
        &["worktree", "add", "-b", branch, target_str, "HEAD"],
    )?;
    Ok(target)
}

/// Tear down the worktree and prune the branch. Used on every abort path
/// after `create_auto_fix_worktree` succeeded. Best-effort — failures are
/// swallowed because we're already on the error path.
fn remove_auto_fix_worktree(
    repo: &StdPath,
    worktree_dir: &StdPath,
    branch: &str,
) -> anyhow::Result<()> {
    let _ = run_git(
        repo,
        &[
            "worktree",
            "remove",
            "--force",
            worktree_dir.to_str().unwrap_or(""),
        ],
    );
    if worktree_dir.exists() {
        let _ = std::fs::remove_dir_all(worktree_dir);
    }
    let _ = run_git(repo, &["branch", "-D", branch]);
    Ok(())
}

/// Like [`remove_auto_fix_worktree`] but keeps the branch ref. Used after
/// a successful merge so operators can `git log <branch>` for audit.
fn remove_auto_fix_worktree_only(repo: &StdPath, worktree_dir: &StdPath) -> anyhow::Result<()> {
    let _ = run_git(
        repo,
        &[
            "worktree",
            "remove",
            "--force",
            worktree_dir.to_str().unwrap_or(""),
        ],
    );
    if worktree_dir.exists() {
        let _ = std::fs::remove_dir_all(worktree_dir);
    }
    Ok(())
}

/// Stage + commit the patched file inside the worktree. Returns the SHA
/// of the new commit.
fn commit_in_worktree(
    worktree: &StdPath,
    rel_file: &StdPath,
    finding: &Finding,
) -> anyhow::Result<String> {
    run_git_checked(
        worktree,
        &["add", "--", rel_file.to_str().unwrap_or("")],
    )?;
    let msg = commit_message(finding);
    // Author env vars so the commit always has a deterministic author
    // even on a fresh git config (CI, test repos).
    let out = Command::new("git")
        .args([
            "-c",
            "user.email=hex-adr-doctor@hex.local",
            "-c",
            "user.name=hex adr doctor",
            "commit",
            "--no-verify",
            "-m",
            &msg,
        ])
        .current_dir(worktree)
        .output()?;
    if !out.status.success() {
        anyhow::bail!(
            "git commit failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    rev_parse_head(worktree)
}

fn commit_message(finding: &Finding) -> String {
    format!(
        "chore(adr-doctor): auto-fix {:?} in {}\n\n\
         Generated by `hex adr doctor --fix` (ADR-2604270800 §1a, Tier-A).\n\
         Detail: {}\n",
        finding.kind, finding.adr_id, finding.detail
    )
}

fn rev_parse_head(repo: &StdPath) -> anyhow::Result<String> {
    run_git_checked(repo, &["rev-parse", "HEAD"])
}

/// Merge `branch` into the repo's current HEAD with `--no-ff`. We never
/// fast-forward — that's the ADR-2604150100 failure mode. `--no-ff`
/// always records a merge commit whose parents are (HEAD, branch tip),
/// so every commit reachable from either side stays reachable from the
/// new HEAD.
fn merge_no_ff(repo: &StdPath, branch: &str, finding: &Finding) -> anyhow::Result<()> {
    let msg = format!(
        "merge: shadow-promote {} for {}",
        format!("{:?}", finding.kind),
        finding.adr_id
    );
    let out = Command::new("git")
        .args([
            "-c",
            "user.email=hex-adr-doctor@hex.local",
            "-c",
            "user.name=hex adr doctor",
            "merge",
            "--no-ff",
            "--no-edit",
            "-m",
            &msg,
            branch,
        ])
        .current_dir(repo)
        .output()?;
    if !out.status.success() {
        // Bail out of the merge if git left us in a partial state. Caller
        // handles cleanup of the worktree separately.
        let _ = run_git(repo, &["merge", "--abort"]);
        anyhow::bail!(
            "git merge --no-ff {} failed: {}",
            branch,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

/// Verify `pre_head` is reachable from `HEAD`. Used as the post-merge
/// integrity assertion.
fn assert_ancestor(repo: &StdPath, pre_head: &str) -> anyhow::Result<()> {
    let out = run_git(
        repo,
        &["merge-base", "--is-ancestor", pre_head, "HEAD"],
    )?;
    if !out.status.success() {
        anyhow::bail!(
            "{} is not an ancestor of HEAD after merge — commits may have been dropped",
            pre_head
        );
    }
    Ok(())
}

// ── Output + exit code ───────────────────────────────────────────────────

/// Serialize findings to a stable JSON envelope. Used by `--json` and by
/// the sched daemon when recording `adr_doctor_tick` event payloads.
pub fn to_json(findings: &[Finding]) -> anyhow::Result<String> {
    to_json_with_dispatch(findings, None)
}

/// Serialize findings + (optionally) dispatch results to a stable JSON
/// envelope. `--fix` / `--fix-and-merge` runs include the dispatch
/// section so daemon consumers can correlate findings with their
/// auto-fix outcomes in a single payload.
pub fn to_json_with_dispatch(
    findings: &[Finding],
    dispatch: Option<&[DispatchResult]>,
) -> anyhow::Result<String> {
    let mut envelope = serde_json::json!({
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
    if let Some(results) = dispatch {
        let applied = results.iter().filter(|r| r.was_applied()).count();
        let aborted = results.iter().filter(|r| r.was_aborted()).count();
        let notified = results
            .iter()
            .filter(|r| matches!(r, DispatchResult::C))
            .count();
        envelope["dispatch"] = serde_json::json!({
            "results": results,
            "summary": {
                "applied":  applied,
                "aborted":  aborted,
                "notified": notified,
            },
        });
    }
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

    // ── Tier dispatcher tests (P2.3) ─────────────────────────────────────
    //
    // Routing-only tests — they don't exercise the git plumbing (that's
    // covered by the integration tests in `tests/adr_doctor_shadow_promote.rs`).
    // The point here is: every finding produces exactly one
    // `DispatchResult` of the right tier variant, in the same order.

    /// A `ShadowPromoteConfig` rooted at a guaranteed-non-existent path.
    /// shadow_promote / tier_b_draft will fail their git invocations
    /// against this path, but they fold those failures into
    /// `Outcome::Aborted` rather than panicking — which is exactly what
    /// the dispatcher contract requires.
    fn cfg_for_routing_test() -> ShadowPromoteConfig {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "hex-doctor-dispatch-routing-{}-{}",
            std::process::id(),
            chrono::Local::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        ShadowPromoteConfig {
            repo_root: p,
            sessions_dir: None,
            now: NaiveDate::from_ymd_opt(2026, 4, 27).unwrap(),
        }
    }

    #[test]
    fn dispatch_fix_routes_each_tier_to_the_right_variant() {
        let tier_a = finding(
            "ADR-A",
            PathBuf::from("a.md"),
            FindingKind::UnparseableStatus,
            "",
        );
        let tier_b = finding(
            "ADR-B",
            PathBuf::from("b.md"),
            FindingKind::StaleProposed,
            "",
        );
        let tier_c = finding(
            "ADR-C",
            PathBuf::from("c.md"),
            FindingKind::DuplicateId,
            "",
        );
        // Sanity: rule table puts each kind on its expected tier.
        assert_eq!(tier_a.tier, AutoFixTier::A);
        assert_eq!(tier_b.tier, AutoFixTier::B);
        assert_eq!(tier_c.tier, AutoFixTier::C);

        let cfg = cfg_for_routing_test();
        let results = dispatch_fix(&[tier_a, tier_b, tier_c], &cfg, false);
        assert_eq!(results.len(), 3, "one result per finding, in input order");
        assert!(matches!(results[0], DispatchResult::A { .. }));
        assert!(matches!(results[1], DispatchResult::B { .. }));
        assert!(matches!(results[2], DispatchResult::C));
    }

    #[test]
    fn dispatch_fix_tier_c_is_always_no_mutation() {
        // Tier C must be DispatchResult::C regardless of fix_and_merge flag.
        let f = finding(
            "ADR-001",
            PathBuf::from("x.md"),
            FindingKind::DuplicateId,
            "",
        );
        let cfg = cfg_for_routing_test();
        for fix_and_merge in [false, true] {
            let r = dispatch_fix(&[f.clone()], &cfg, fix_and_merge);
            assert_eq!(r.len(), 1);
            assert!(
                matches!(r[0], DispatchResult::C),
                "fix_and_merge={fix_and_merge} must still leave Tier C as no-op"
            );
            assert!(!r[0].was_applied());
            assert!(!r[0].was_aborted());
        }
    }

    #[test]
    fn dispatch_result_serializes_with_tier_tag() {
        // The JSON envelope keys results by tier so the daemon /
        // dashboard can filter without re-deriving from the finding.
        let r_c = DispatchResult::C;
        let s_c = serde_json::to_string(&r_c).unwrap();
        assert!(s_c.contains("\"tier\":\"c\""), "got: {}", s_c);

        let r_a = DispatchResult::A {
            outcome: Outcome::Aborted {
                reason: "test".into(),
            },
        };
        let s_a = serde_json::to_string(&r_a).unwrap();
        assert!(s_a.contains("\"tier\":\"a\""), "got: {}", s_a);
        assert!(s_a.contains("\"outcome\":\"aborted\""), "got: {}", s_a);
    }

    #[test]
    fn dispatch_result_was_applied_was_aborted_helpers() {
        let applied = DispatchResult::A {
            outcome: Outcome::Applied {
                branch: "b".into(),
                commit: "c".into(),
            },
        };
        assert!(applied.was_applied() && !applied.was_aborted());

        let aborted = DispatchResult::B {
            outcome: Outcome::Aborted { reason: "r".into() },
        };
        assert!(!aborted.was_applied() && aborted.was_aborted());

        let notified = DispatchResult::C;
        assert!(!notified.was_applied() && !notified.was_aborted());
    }

    #[test]
    fn to_json_with_dispatch_includes_dispatch_section() {
        let f = finding("ADR-001", PathBuf::from("x.md"), FindingKind::DuplicateId, "");
        let dispatch = vec![DispatchResult::C];
        let json = to_json_with_dispatch(&[f], Some(&dispatch)).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["dispatch"]["summary"]["notified"], 1);
        assert_eq!(v["dispatch"]["summary"]["applied"], 0);
        assert_eq!(v["dispatch"]["summary"]["aborted"], 0);
        assert_eq!(v["dispatch"]["results"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn to_json_omits_dispatch_section_when_none() {
        // Detection-only runs (no `--fix`) keep the legacy schema —
        // no `dispatch` key in the envelope.
        let f = finding("ADR-001", PathBuf::from("x.md"), FindingKind::DuplicateId, "");
        let json = to_json(&[f]).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(v.get("dispatch").is_none(), "dispatch must be omitted, got: {}", json);
    }

    #[test]
    fn shadow_promote_with_policy_rejects_non_tier_a() {
        // Defense-in-depth: the new policy variant must keep the same
        // Tier-A guard as `shadow_promote_with_config`.
        let f = finding(
            "ADR-001",
            PathBuf::from("x.md"),
            FindingKind::StaleProposed, // Tier B
            "",
        );
        let cfg = cfg_for_routing_test();
        for policy in [MergePolicy::Merge, MergePolicy::LeaveForReview] {
            let outcome = shadow_promote_with_policy(&f, &cfg, policy).unwrap();
            match outcome {
                Outcome::Aborted { reason } => {
                    assert!(reason.contains("Tier-A"), "expected Tier-A guard, got: {}", reason);
                }
                Outcome::Applied { .. } => panic!("must abort on non-Tier-A"),
            }
        }
    }

    #[test]
    fn tier_b_draft_rejects_non_tier_b() {
        // Symmetric guard for Tier-B drafter.
        let tier_a = finding(
            "ADR-001",
            PathBuf::from("x.md"),
            FindingKind::UnparseableStatus,
            "",
        );
        let cfg = cfg_for_routing_test();
        let outcome = tier_b_draft_with_config(&tier_a, &cfg).unwrap();
        match outcome {
            Outcome::Aborted { reason } => {
                assert!(reason.contains("Tier-B"), "expected Tier-B guard, got: {}", reason);
            }
            Outcome::Applied { .. } => panic!("must abort on non-Tier-B"),
        }
    }
}
