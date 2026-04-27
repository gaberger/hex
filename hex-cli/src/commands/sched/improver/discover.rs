//! Discovery surface for the self-improvement loop (ADR-2604271100 P1).
//!
//! Detectors are configured as data in `hex-cli/assets/improver/detectors.toml`
//! (rules-as-data, ADR-2604142243). Each entry names a `Source`, a shell
//! command, and minor parsing knobs. [`discover`] runs every detector,
//! parses each one's stdout JSON, and emits a [`Hypothesis`] per finding.
//!
//! Hypotheses are deduped by `(source, scope)` so a detector that re-fires
//! on the same scope across ticks does not stack notifications. Re-firing
//! across distinct ticks is the dispatcher's problem (P5), not ours.

use std::collections::HashSet;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::assets::Assets;

// ── Types ────────────────────────────────────────────────────────────────

/// Where a hypothesis came from. One variant per `[[detector]]` entry in
/// `assets/improver/detectors.toml`. The variants are an enum (not free
/// strings) so the rule table is closed — adding a new signal requires a
/// matching code change, which is how we keep `act()` tier-routing honest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    AdrDoctor,
    ReconcileStrict,
    GitDrift,
    InboxStale,
    AdrLifecycle,
    EscalationReport,
    PunchList,
}

impl Source {
    /// Every variant — used to assert rule-table completeness in tests.
    pub fn all() -> &'static [Source] {
        use Source::*;
        &[
            AdrDoctor,
            ReconcileStrict,
            GitDrift,
            InboxStale,
            AdrLifecycle,
            EscalationReport,
            PunchList,
        ]
    }
}

/// Hypothesis severity. Maps onto the rubric in P3 (judge) and the inbox
/// priority in P4 (act). `Info` exists for low-stakes signals we want to
/// surface but never auto-apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

/// A single proposed improvement, derived from one detector finding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hypothesis {
    /// Stable id derived from `(source, scope)` — same scope re-firing
    /// produces the same id, which the dedup pass below uses.
    pub id: String,
    pub source: Source,
    /// What this hypothesis is about (ADR id, workplan id, branch name, …).
    /// Detectors that emit unscoped findings produce an empty string.
    pub scope: String,
    pub severity: Severity,
    /// Raw detector finding, preserved verbatim so P2 (variant generation)
    /// has full context without re-running the detector.
    pub evidence: Value,
    pub generated_at: DateTime<Utc>,
}

/// Detector rule-table entry. Loaded from `assets/improver/detectors.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct Detector {
    pub source: Source,
    /// Shell command run as `sh -c <cmd>` with cwd = repo root.
    pub cmd: String,
    /// RFC-6901 JSON pointer into stdout to locate the findings array.
    /// Default `/findings`; if absent, the whole stdout is treated as the
    /// findings array (or as a single finding if it's an object).
    #[serde(default)]
    pub findings_pointer: Option<String>,
    /// Most diagnostic CLIs signal "findings present" via non-zero exit;
    /// default true so the rule table stays terse.
    #[serde(default = "default_true")]
    pub allow_nonzero_exit: bool,
    /// Operator-facing description — surfaced by `hex sched improver discover`.
    #[serde(default)]
    pub description: String,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize)]
struct DetectorTable {
    #[serde(default)]
    detector: Vec<Detector>,
}

// ── Public API ───────────────────────────────────────────────────────────

/// Load the embedded detector rule table.
pub fn load_detectors() -> Result<Vec<Detector>> {
    let raw = Assets::get_str("improver/detectors.toml")
        .context("improver/detectors.toml not embedded in hex-cli assets")?;
    let table: DetectorTable =
        toml::from_str(&raw).context("parse improver/detectors.toml")?;
    Ok(table.detector)
}

/// Run every embedded detector against `repo` and emit hypotheses.
pub fn discover(repo: &Path) -> Result<Vec<Hypothesis>> {
    let detectors = load_detectors()?;
    Ok(discover_with(repo, &detectors))
}

/// Same as [`discover`] but with an explicit detector list. Used by tests
/// (which inject synthetic detectors) and by the CLI subcommand when an
/// operator wants to preview a subset.
///
/// Failure of an individual detector (spawn error, non-JSON stdout, parse
/// error) is logged and skipped — one broken detector must not abort the
/// whole sweep, otherwise a flaky CLI on a single tier could silently
/// stall the improvement loop.
pub fn discover_with(repo: &Path, detectors: &[Detector]) -> Vec<Hypothesis> {
    let mut hypotheses = Vec::new();
    let mut seen: HashSet<(Source, String)> = HashSet::new();
    let now = Utc::now();
    for det in detectors {
        let raw = match run_detector(repo, det) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(source = ?det.source, error = %e, "detector failed");
                continue;
            }
        };
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        let parsed: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(source = ?det.source, error = %e, "detector emitted non-JSON");
                continue;
            }
        };
        for finding in extract_findings(&parsed, det.findings_pointer.as_deref()) {
            let scope = extract_scope(&finding, det.source);
            let key = (det.source, scope.clone());
            if !seen.insert(key) {
                continue;
            }
            hypotheses.push(Hypothesis {
                id: hypothesis_id(det.source, &scope),
                source: det.source,
                scope,
                severity: extract_severity(&finding),
                evidence: finding,
                generated_at: now,
            });
        }
    }
    hypotheses
}

// ── Internals ────────────────────────────────────────────────────────────

fn run_detector(repo: &Path, det: &Detector) -> Result<String> {
    let output = Command::new("sh")
        .arg("-c")
        .arg(&det.cmd)
        .current_dir(repo)
        .output()
        .with_context(|| format!("spawn detector: {}", det.cmd))?;
    if !output.status.success() && !det.allow_nonzero_exit {
        anyhow::bail!(
            "detector exited {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn extract_findings(value: &Value, pointer: Option<&str>) -> Vec<Value> {
    let ptr = pointer.unwrap_or("/findings");
    if let Some(target) = value.pointer(ptr) {
        return match target {
            Value::Array(arr) => arr.clone(),
            other => vec![other.clone()],
        };
    }
    match value {
        Value::Array(arr) => arr.clone(),
        Value::Object(_) => vec![value.clone()],
        _ => Vec::new(),
    }
}

/// Per-source scope-field preference. Each detector emits its own
/// finding shape; we look for the most specific id available.
fn extract_scope(finding: &Value, source: Source) -> String {
    let candidates: &[&str] = match source {
        Source::AdrDoctor | Source::AdrLifecycle => &["adr_id", "scope", "id", "path"],
        Source::ReconcileStrict => &["workplan_id", "scope", "id", "path"],
        Source::InboxStale => &["notification_id", "id", "scope"],
        Source::EscalationReport => &["task_id", "agent_id", "scope", "id"],
        Source::PunchList => &["item_id", "scope", "id"],
        Source::GitDrift => &["path", "branch", "scope"],
    };
    for key in candidates {
        if let Some(s) = finding.get(*key).and_then(|v| v.as_str()) {
            if !s.is_empty() {
                return s.to_string();
            }
        }
    }
    String::new()
}

fn extract_severity(finding: &Value) -> Severity {
    match finding.get("severity").and_then(|v| v.as_str()) {
        Some("error") | Some("err") => Severity::Error,
        Some("warning") | Some("warn") => Severity::Warning,
        Some("info") => Severity::Info,
        _ => Severity::Warning,
    }
}

/// Stable id from `(source, scope)`. Re-firing on the same scope yields
/// the same id; downstream stages can use it as a primary key.
fn hypothesis_id(source: Source, scope: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    source.hash(&mut h);
    scope.hash(&mut h);
    format!("hyp-{:016x}", h.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn rule_table_has_one_entry_per_source_variant() {
        let detectors = load_detectors().expect("load embedded detectors.toml");
        assert_eq!(
            detectors.len(),
            Source::all().len(),
            "detectors.toml must have exactly one entry per Source variant",
        );
        let observed: HashSet<Source> = detectors.iter().map(|d| d.source).collect();
        for src in Source::all() {
            assert!(observed.contains(src), "missing detector for {:?}", src);
        }
        for d in &detectors {
            assert!(!d.cmd.is_empty(), "detector {:?} has empty cmd", d.source);
        }
    }

    #[test]
    fn discover_emits_adr_doctor_hypothesis_for_unparseable_adr() {
        // Fixture repo with a known unparseable ADR (Status field outside
        // the lifecycle enum). The synthetic detector below is conditional
        // on the fixture file existing in cwd, so the cwd plumbing is part
        // of what's exercised — not just the JSON pipeline.
        let dir = tempdir().expect("tempdir");
        let adrs = dir.path().join("docs/adrs");
        fs::create_dir_all(&adrs).expect("mkdir docs/adrs");
        fs::write(
            adrs.join("ADR-2699999999-broken.md"),
            "# ADR-2699999999 — broken fixture\n\nStatus: Spaghetti\nDate: 2026-04-27\n",
        )
        .expect("write adr fixture");

        let cmd = "[ -f docs/adrs/ADR-2699999999-broken.md ] \
            && echo '{\"findings\":[{\"adr_id\":\"ADR-2699999999\",\"severity\":\"error\",\"kind\":\"unparseable_status\"}]}' \
            || echo '{\"findings\":[]}'";
        let detector = Detector {
            source: Source::AdrDoctor,
            cmd: cmd.to_string(),
            findings_pointer: None,
            allow_nonzero_exit: true,
            description: String::new(),
        };

        let hypotheses = discover_with(dir.path(), &[detector]);
        assert_eq!(hypotheses.len(), 1, "expected exactly one hypothesis");
        let h = &hypotheses[0];
        assert_eq!(h.source, Source::AdrDoctor);
        assert_eq!(h.scope, "ADR-2699999999");
        assert_eq!(h.severity, Severity::Error);
        assert_eq!(
            h.evidence.get("kind").and_then(|v| v.as_str()),
            Some("unparseable_status"),
        );
        assert!(h.id.starts_with("hyp-"));
    }

    #[test]
    fn dedup_collapses_repeats_on_same_source_scope() {
        let dir = tempdir().expect("tempdir");
        let make = |severity: &str| Detector {
            source: Source::AdrDoctor,
            cmd: format!(
                "echo '{{\"findings\":[{{\"adr_id\":\"ADR-X\",\"severity\":\"{}\"}}]}}'",
                severity,
            ),
            findings_pointer: None,
            allow_nonzero_exit: true,
            description: String::new(),
        };
        let hypotheses = discover_with(dir.path(), &[make("error"), make("warning")]);
        assert_eq!(hypotheses.len(), 1, "(source, scope) duplicates must collapse");
        // Whichever fired first wins — we assert the surviving severity is one of the two.
        assert!(matches!(hypotheses[0].severity, Severity::Error | Severity::Warning));
    }

    #[test]
    fn detector_failures_are_skipped_not_fatal() {
        let dir = tempdir().expect("tempdir");
        let bad = Detector {
            source: Source::GitDrift,
            cmd: "this-command-does-not-exist-aaa".to_string(),
            findings_pointer: None,
            allow_nonzero_exit: false,
            description: String::new(),
        };
        let good = Detector {
            source: Source::AdrDoctor,
            cmd: "echo '{\"findings\":[{\"adr_id\":\"ADR-Y\",\"severity\":\"warning\"}]}'"
                .to_string(),
            findings_pointer: None,
            allow_nonzero_exit: true,
            description: String::new(),
        };
        let hypotheses = discover_with(dir.path(), &[bad, good]);
        assert_eq!(hypotheses.len(), 1, "good detector must still produce output");
        assert_eq!(hypotheses[0].source, Source::AdrDoctor);
    }

    #[test]
    fn extract_findings_falls_back_to_root_array() {
        let v: Value =
            serde_json::from_str(r#"[{"id":"A","severity":"info"}]"#).unwrap();
        let out = extract_findings(&v, None);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].get("id").and_then(|x| x.as_str()), Some("A"));
    }

    #[test]
    fn hypothesis_id_is_stable() {
        let a = hypothesis_id(Source::AdrDoctor, "ADR-X");
        let b = hypothesis_id(Source::AdrDoctor, "ADR-X");
        let c = hypothesis_id(Source::AdrDoctor, "ADR-Y");
        assert_eq!(a, b, "same (source, scope) must produce same id");
        assert_ne!(a, c, "different scope must produce different id");
    }
}
