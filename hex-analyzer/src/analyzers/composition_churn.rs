//! Composition-drift detector.
//!
//! "How often is wiring being rewritten relative to how often we're
//! writing decisions about it?" Cheap operational signal: if commits
//! that touch composition-root / `lib.rs` / `*compose*.rs` outpace
//! accepted ADRs over the same window, the architecture is drifting
//! ahead of the recorded reasoning. We flag when
//! `commits_touching_wiring / accepted_adrs_in_window > 1.5`.
//!
//! Inputs:
//! 1. `git log --since=<window> -- '*composition-root*' '*compose*.rs'
//!    '*lib.rs'` — distinct commit hashes plus the union of touched
//!    file paths.
//! 2. `docs/adrs/*.md` — ADRs whose markdown declares both
//!    `Status: Accepted` and a `Date: YYYY-MM-DD` falling inside the
//!    window (compared against `SystemTime::now() - window`).
//!
//! ## Edge cases
//!
//! * **Not a git repo.** `git log` exits non-zero → empty report; the
//!   detector is silent rather than fatal so it composes with the
//!   "run everything when no flag set" CLI default on non-git paths.
//! * **Zero accepted ADRs.** Division by zero would flag noisily on
//!   any project that doesn't keep ADRs, which is most of them. We
//!   only emit when there's *also* wiring churn — `ratio = +inf` and
//!   the threshold check fires; no churn → no finding.
//! * **No `docs/adrs/`.** Treated as zero accepted ADRs.
//!
//! ## Output schema
//!
//! ```json
//! {"findings":[{
//!   "kind": "composition_churn",
//!   "window": "30d",
//!   "commits_touching_wiring": 8,
//!   "accepted_adrs_in_window": 2,
//!   "ratio": 4.0,
//!   "files_touched": ["hex-nexus/src/composition_root.rs", "src/lib.rs"]
//! }]}
//! ```

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Default analysis window if the caller doesn't pass one.
pub const DEFAULT_WINDOW: &str = "30d";

/// Threshold above which churn-vs-decisions is considered drift.
/// commits / accepted-adrs > 1.5 → finding.
pub const RATIO_THRESHOLD: f64 = 1.5;

/// Pathspecs handed verbatim to `git log`. Cover both naming
/// conventions:
///
///   * `*composition-root*` — TS / docs style (`composition-root.ts`).
///   * `*composition*.rs` — Rust style (`composition.rs`,
///     `composition_root.rs`). Note `*compose*.rs` from the original
///     spec only matches the literal substring "compose" so it skips
///     `composition.rs` (no `e` after `compos`); we keep both, the
///     `compose` form catches Docker-style / function-composition
///     wiring while `composition` catches Rust idiomatic naming.
///   * `*compose*.rs` — workplan-spec literal.
///   * `*lib.rs` — top-level crate root files.
pub const WIRING_PATHSPECS: &[&str] = &[
    "*composition-root*",
    "*composition*.rs",
    "*compose*.rs",
    "*lib.rs",
];

/// One finding row. Field order mirrors the schema in the workplan;
/// `kind` and `window` are added so the improver can route by detector
/// kind and so a JSON dump is self-describing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CompositionChurnFinding {
    pub kind: String,
    pub window: String,
    pub commits_touching_wiring: usize,
    pub accepted_adrs_in_window: usize,
    /// Rounded to 4 decimals so JSON output is bit-stable across runs.
    /// `f64::INFINITY` (when accepted_adrs_in_window == 0 but commits >0)
    /// serializes as JSON `null` — readers should treat that as
    /// "undefined ratio".
    pub ratio: f64,
    pub files_touched: Vec<String>,
}

/// Top-level envelope emitted by `--composition-churn`.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct CompositionChurnReport {
    pub findings: Vec<CompositionChurnFinding>,
}

/// Run the composition-churn detector over `root` for the given window.
///
/// `window` accepts `Nh` / `Nd` / `Nw` (e.g. `24h`, `30d`, `2w`). The
/// numeric portion is forwarded to `git log --since="N <unit> ago"` and
/// also used to compute the ADR-date cutoff.
pub fn analyze(root: &Path, window: &str) -> anyhow::Result<CompositionChurnReport> {
    let parsed = parse_window(window)?;
    let (commits, files_touched) = git_log_wiring(root, &parsed.git_since)?;

    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let cutoff = now_secs.saturating_sub(parsed.secs);
    let accepted_adrs = count_accepted_adrs_since(root, cutoff)?;

    let ratio = compute_ratio(commits.len(), accepted_adrs);

    let mut findings = Vec::new();
    if ratio > RATIO_THRESHOLD {
        let files_sorted: Vec<String> = files_touched.into_iter().collect();
        findings.push(CompositionChurnFinding {
            kind: "composition_churn".to_string(),
            window: window.to_string(),
            commits_touching_wiring: commits.len(),
            accepted_adrs_in_window: accepted_adrs,
            ratio: round4(ratio),
            files_touched: files_sorted,
        });
    }

    Ok(CompositionChurnReport { findings })
}

// ── Internals ────────────────────────────────────────────────────────

/// Parsed window: kept side-by-side so the seconds value (used for ADR
/// cutoff) and the git-friendly string (`"30 days ago"`) come from the
/// same source of truth — no chance of one drifting from the other.
struct ParsedWindow {
    secs: u64,
    git_since: String,
}

fn parse_window(window: &str) -> anyhow::Result<ParsedWindow> {
    let s = window.trim();
    if s.is_empty() {
        anyhow::bail!("window cannot be empty");
    }
    let last = s.chars().last().unwrap();
    let n_str = &s[..s.len() - last.len_utf8()];
    let n: u64 = n_str
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid window {window:?}: expected `<N><unit>`"))?;
    let (secs_per, unit_word) = match last {
        'h' | 'H' => (3600u64, "hours"),
        'd' | 'D' => (86_400, "days"),
        'w' | 'W' => (86_400 * 7, "weeks"),
        other => anyhow::bail!(
            "unknown window unit {other:?} in {window:?}; expected h, d, or w"
        ),
    };
    Ok(ParsedWindow {
        secs: n.saturating_mul(secs_per),
        git_since: format!("{n} {unit_word} ago"),
    })
}

/// Returns (distinct commit hashes, union of touched file paths).
/// On any git failure (not a repo, git not on PATH) returns empty —
/// the detector is best-effort, not a hard requirement.
fn git_log_wiring(
    root: &Path,
    since: &str,
) -> anyhow::Result<(Vec<String>, BTreeSet<String>)> {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(root)
        .arg("log")
        .arg(format!("--since={since}"))
        .arg("--pretty=format:COMMIT %H")
        .arg("--name-only")
        .arg("--");
    for pathspec in WIRING_PATHSPECS {
        cmd.arg(pathspec);
    }

    let output = match cmd.output() {
        Ok(o) => o,
        Err(_) => return Ok((Vec::new(), BTreeSet::new())),
    };
    if !output.status.success() {
        return Ok((Vec::new(), BTreeSet::new()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut commits: Vec<String> = Vec::new();
    let mut files: BTreeSet<String> = BTreeSet::new();
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(hash) = line.strip_prefix("COMMIT ") {
            commits.push(hash.to_string());
        } else {
            files.insert(line.to_string());
        }
    }
    // De-dupe in case the same hash shows up across path-spec hits.
    commits.sort();
    commits.dedup();
    Ok((commits, files))
}

fn count_accepted_adrs_since(root: &Path, cutoff_secs: u64) -> anyhow::Result<usize> {
    let adr_dir = root.join("docs").join("adrs");
    if !adr_dir.is_dir() {
        return Ok(0);
    }
    let mut count = 0;
    for entry in std::fs::read_dir(&adr_dir)? {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Some((accepted, date_str)) = parse_adr_status_and_date(&content) else {
            continue;
        };
        if !accepted {
            continue;
        }
        let Some(adr_secs) = iso_date_to_secs(&date_str) else {
            continue;
        };
        if adr_secs >= cutoff_secs {
            count += 1;
        }
    }
    Ok(count)
}

/// Returns `(is_accepted, date_string)` from an ADR's frontmatter.
/// Both fields must be present; missing date or non-Accepted status
/// returns `None`. Recognizes both `**Status:** Accepted` (modern
/// timestamp ADRs) and `Status: Accepted` (legacy sequential ADRs).
fn parse_adr_status_and_date(content: &str) -> Option<(bool, String)> {
    let mut accepted = false;
    let mut date: Option<String> = None;
    // Frontmatter is always near the top — bound the scan so an ADR
    // body that quotes "Status: Rejected" in prose can't poison a
    // legitimately accepted file.
    for line in content.lines().take(40) {
        if let Some(rest) = field_value(line, "status") {
            if rest.trim().eq_ignore_ascii_case("accepted") {
                accepted = true;
            }
        }
        if let Some(rest) = field_value(line, "date") {
            if let Some(d) = first_iso_date(rest.trim()) {
                date = Some(d);
            }
        }
    }
    Some((accepted, date?))
}

/// Strip leading markdown markers + the field name and return the
/// remainder. Recognizes:
///   * `Status: ...`           (plain)
///   * `**Status:** ...`       (bold)
///   * `## Status: ...`        (heading)
///   * `- **Status**: ...`     (list item)
fn field_value<'a>(line: &'a str, field: &str) -> Option<&'a str> {
    let stripped =
        line.trim_start_matches(|c: char| matches!(c, '#' | '*' | '-' | ' ' | '\t'));
    let lower = stripped.to_ascii_lowercase();
    if !lower.starts_with(&field.to_ascii_lowercase()) {
        return None;
    }
    let after_field = &stripped[field.len()..];
    // Tolerate `**field**:` shape — strip any combination of `*`, `:`,
    // and whitespace before the value.
    let trimmed = after_field
        .trim_start_matches(|c: char| matches!(c, '*' | ':' | ' ' | '\t'));
    // Sanity: there must have been at least one ':' separating field
    // from value, otherwise we'd match a sentence starting with the
    // field name in prose.
    if !after_field.contains(':') {
        return None;
    }
    Some(trimmed)
}

fn first_iso_date(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    if bytes.len() < 10 {
        return None;
    }
    let prefix = &s[..10];
    let mut chars = prefix.chars();
    let valid = (0..10).all(|i| {
        let c = chars.next().unwrap_or(' ');
        match i {
            0..=3 | 5..=6 | 8..=9 => c.is_ascii_digit(),
            4 | 7 => c == '-',
            _ => false,
        }
    });
    if valid {
        Some(prefix.to_string())
    } else {
        None
    }
}

/// `YYYY-MM-DD` → seconds since UNIX epoch (midnight UTC of that date).
/// Uses the standard Julian-day formula, which is exact for any date
/// past 1970-01-01.
fn iso_date_to_secs(date: &str) -> Option<u64> {
    let parts: Vec<&str> = date.split('-').collect();
    if parts.len() != 3 {
        return None;
    }
    let y: i64 = parts[0].parse().ok()?;
    let m: i64 = parts[1].parse().ok()?;
    let d: i64 = parts[2].parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let jdn = julian_day_number(y, m, d);
    let epoch_jdn = julian_day_number(1970, 1, 1);
    let days = jdn.checked_sub(epoch_jdn)?;
    if days < 0 {
        return None;
    }
    Some((days as u64) * 86_400)
}

fn julian_day_number(y: i64, m: i64, d: i64) -> i64 {
    let a = (14 - m) / 12;
    let yy = y + 4800 - a;
    let mm = m + 12 * a - 3;
    d + (153 * mm + 2) / 5 + 365 * yy + yy / 4 - yy / 100 + yy / 400 - 32_045
}

fn compute_ratio(commits: usize, adrs: usize) -> f64 {
    if commits == 0 {
        return 0.0;
    }
    if adrs == 0 {
        return f64::INFINITY;
    }
    commits as f64 / adrs as f64
}

fn round4(x: f64) -> f64 {
    if !x.is_finite() {
        return x;
    }
    (x * 10_000.0).round() / 10_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_window_accepts_days_hours_weeks() {
        assert_eq!(parse_window("30d").unwrap().secs, 30 * 86_400);
        assert_eq!(parse_window("24h").unwrap().secs, 24 * 3_600);
        assert_eq!(parse_window("2w").unwrap().secs, 14 * 86_400);
        assert_eq!(parse_window("30d").unwrap().git_since, "30 days ago");
        assert_eq!(parse_window("24h").unwrap().git_since, "24 hours ago");
        assert_eq!(parse_window("2w").unwrap().git_since, "2 weeks ago");
    }

    #[test]
    fn parse_window_rejects_garbage() {
        assert!(parse_window("").is_err());
        assert!(parse_window("d").is_err()); // no number
        assert!(parse_window("7x").is_err()); // unknown unit
        assert!(parse_window("abc").is_err());
    }

    #[test]
    fn compute_ratio_handles_zero_adrs() {
        assert_eq!(compute_ratio(0, 0), 0.0);
        assert!(compute_ratio(5, 0).is_infinite());
        assert_eq!(compute_ratio(4, 2), 2.0);
        assert_eq!(compute_ratio(3, 6), 0.5);
    }

    #[test]
    fn parse_adr_recognizes_bold_and_plain_status() {
        let bold = "# ADR-001\n\n**Status:** Accepted\n**Date:** 2026-04-01\n\nbody\n";
        let plain = "# ADR-002\n\nStatus: Accepted\nDate: 2026-04-01\n";
        let heading = "# ADR-003\n## Status: Accepted\n## Date: 2026-04-01\n";
        for src in [bold, plain, heading] {
            let (acc, date) = parse_adr_status_and_date(src).expect("parsed");
            assert!(acc, "{src}");
            assert_eq!(date, "2026-04-01", "{src}");
        }
    }

    #[test]
    fn parse_adr_skips_non_accepted() {
        let src = "# ADR-X\n\n**Status:** Proposed\n**Date:** 2026-04-01\n";
        let (acc, _) = parse_adr_status_and_date(src).unwrap();
        assert!(!acc);
    }

    #[test]
    fn parse_adr_returns_none_when_date_missing() {
        let src = "# ADR-X\n\n**Status:** Accepted\n\nbody only\n";
        assert!(parse_adr_status_and_date(src).is_none());
    }

    #[test]
    fn iso_date_to_secs_matches_julian_arithmetic() {
        // 1970-01-01 → 0
        assert_eq!(iso_date_to_secs("1970-01-01"), Some(0));
        // 1970-01-02 → 86400
        assert_eq!(iso_date_to_secs("1970-01-02"), Some(86_400));
        // 2026-04-27 should be a stable, large positive value.
        let v = iso_date_to_secs("2026-04-27").unwrap();
        // sanity: more than 56 years past epoch
        assert!(v > 56 * 365 * 86_400);
    }

    #[test]
    fn first_iso_date_extracts_leading_date() {
        assert_eq!(
            first_iso_date("2026-04-27 (revised)"),
            Some("2026-04-27".to_string())
        );
        assert_eq!(first_iso_date("garbage"), None);
        assert_eq!(first_iso_date("2026/04/27"), None);
    }
}
