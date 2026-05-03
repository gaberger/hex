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
    /// Meta-detector: templates in the improver Q-table that ran enough
    /// times to be meaningful but didn't resolve their targets. Surfaces
    /// broken (source, action_kind) mappings in act::derive.
    QStarvation,
    /// Action-quality detector: workplan files with missing required
    /// fields (title, phase title, task id/title). Catches the failure
    /// mode where reconcile actions clear their target hypothesis but
    /// also corrupt the file (the existing ReconcileStrict reward
    /// attribution can't tell positive resolution from destructive
    /// resolution; this detector closes that gap).
    WorkplanIntegrity,
    /// Project-structure detector: canonical hexagonal-architecture
    /// layers missing in the current project (use cases, primary
    /// adapters, composition root). Surfaces structural gaps so the
    /// improver can propose drafts that close them.
    LayerCoverage,
    /// Build-readiness detector: typecheck or test-suite failures in
    /// the current project. Closes the gap between "structural layers
    /// exist" (LayerCoverage) and "the code in those layers actually
    /// compiles + tests pass." Without this, LayerCoverage:DraftWorkplan
    /// can credit +1.0 for adding broken code (the layer dir exists, so
    /// the missing_layer hypothesis clears, even though typecheck fails).
    BuildReadiness,
    /// Test-coverage detector: source files lacking sibling test files
    /// (or test files with zero test cases). Surfaces uncovered code as
    /// hypotheses so a tester swarm can generate test suites. Distinct
    /// from BuildReadiness which checks "do existing tests pass" — this
    /// one checks "do tests EXIST."
    TestCoverage,
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
            QStarvation,
            WorkplanIntegrity,
            LayerCoverage,
            BuildReadiness,
            TestCoverage,
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
                // Homeostasis: a detector that can't be invoked is itself a
                // finding. Emit a synthetic hypothesis tagged to this detector's
                // own source so the broken-detector signal shows up in the same
                // pane as real findings — invisible-failure mode is the bug
                // class we're guarding against (TOML/CLI drift).
                hypotheses.push(detector_health_hypothesis(
                    det,
                    "spawn_or_exit_error",
                    &e.to_string(),
                    now,
                ));
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
                // Same rationale as above: a detector that runs but produces
                // unparseable stdout (e.g. a CLI emitting a coloured table when
                // the TOML expected `--json` output) is reported as a finding.
                hypotheses.push(detector_health_hypothesis(
                    det,
                    "non_json_stdout",
                    &e.to_string(),
                    now,
                ));
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

/// Build a synthetic hypothesis for a detector that failed to produce parseable
/// findings. Tags it to the detector's own source with severity=Error and a
/// `detector_health` evidence object so a downstream act() can route it
/// distinctly from real findings if needed. Scope is the detector source name
/// so multiple failure-modes for the same detector dedup naturally via the
/// (source, scope) seen-set.
fn detector_health_hypothesis(
    det: &Detector,
    failure_kind: &str,
    detail: &str,
    now: DateTime<Utc>,
) -> Hypothesis {
    let scope = format!("detector:{:?}", det.source);
    Hypothesis {
        id: hypothesis_id(det.source, &scope),
        source: det.source,
        scope,
        severity: Severity::Error,
        evidence: serde_json::json!({
            "detector_health": failure_kind,
            "cmd": det.cmd,
            "detail": detail,
            "remediation": "align CLI surface with detectors.toml — usually a missing --json flag",
        }),
        generated_at: now,
    }
}

fn run_detector(repo: &Path, det: &Detector) -> Result<String> {
    let output = Command::new("sh")
        .arg("-c")
        .arg(&det.cmd)
        .current_dir(repo)
        .output()
        .with_context(|| format!("spawn detector: {}", det.cmd))?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        // Hard failure when allow_nonzero_exit=false (existing contract).
        if !det.allow_nonzero_exit {
            anyhow::bail!("detector exited {}: {}", output.status, stderr.trim());
        }
        // Soft failure case: allow_nonzero_exit=true is meant for diagnostic
        // CLIs that signal "findings present" via exit code while still
        // producing JSON on stdout. If stdout is empty, the CLI didn't run
        // the way the TOML expected — almost always TOML/CLI flag drift.
        // Surface this as a detector_health finding rather than swallowing
        // it (homeostasis: the improver reports its own broken detectors).
        if stdout.trim().is_empty() {
            anyhow::bail!(
                "detector exited {} with empty stdout (likely TOML/CLI drift): {}",
                output.status,
                stderr.trim()
            );
        }
    }
    Ok(stdout)
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
        Source::QStarvation => &["template", "scope"],
        Source::WorkplanIntegrity => &["workplan_id", "scope"],
        Source::LayerCoverage => &["layer", "scope"],
        Source::BuildReadiness => &["gate", "scope"],
        Source::TestCoverage => &["source", "scope"],
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
    fn detector_failures_surface_as_hypotheses_alongside_real_findings() {
        // Homeostatic contract: a detector that fails to produce parseable
        // findings doesn't get silently skipped — it surfaces as its own
        // synthetic hypothesis so the broken-detector signal is visible
        // in the hypothesis stream rather than buried in tracing logs.
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
        assert_eq!(
            hypotheses.len(),
            2,
            "broken detector emits a detector_health hypothesis, good detector emits its real finding"
        );
        // Good detector's finding still lands.
        let good_h = hypotheses.iter().find(|h| h.source == Source::AdrDoctor).expect("AdrDoctor hypothesis");
        assert_eq!(good_h.scope, "ADR-Y");
        // Broken detector's signal lands tagged to its own source with
        // scope=detector:<Source> so it dedups by detector identity.
        let bad_h = hypotheses.iter().find(|h| h.source == Source::GitDrift).expect("GitDrift health hypothesis");
        assert_eq!(bad_h.severity, Severity::Error);
        assert_eq!(bad_h.scope, "detector:GitDrift");
        assert_eq!(
            bad_h.evidence.get("detector_health").and_then(|v| v.as_str()),
            Some("spawn_or_exit_error"),
        );
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
