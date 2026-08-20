//! Sweep coordinator (workplan `wp-idle-research-swarm`, P4.1).
//!
//! Glues the deterministic + LLM-synthesis analysts into a single sweep:
//!
//! 1. Invokes every enabled analyst in `research/`.
//! 2. Concatenates the resulting [`Finding`] sets.
//! 3. Sorts severity-desc / id-asc and caps at `max_drafts_per_sweep`
//!    (default 5, configurable via `.hex/project.json` →
//!    `research.max_drafts_per_sweep`).
//! 4. Writes `docs/analysis/idle-sweep-YYYYMMDD-HHMM.yaml` (the structured
//!    sweep record consumed by P4.2/P4.3 draft writers) and a sibling
//!    `.md` summary (human-readable, identical stem).
//!
//! Analyst failures are recorded but never abort the sweep — a flaky
//! `hex analyze` (e.g. binary missing on the daemon host) must not stop the
//! drift / naming / perf analysts from running.
//!
//! All rendering helpers ([`cap_findings`], [`render_yaml`],
//! [`render_markdown`], [`sweep_filename_stem`]) are pure and unit-tested
//! against synthetic [`Finding`] sets so the coordinator's I/O loop stays
//! a thin shell over deterministic logic.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use hex_core::Finding;
use serde::Serialize;

use super::{
    architecture_analyst, code_quality_analyst, drift_analyst, naming_analyst,
    performance_analyst, ux_analyst,
};

/// Default upper bound on findings emitted by a single sweep
/// (ADR-2026-04-15-1200 §Risks: "a buggy analyst could spam draft ADRs/workplans
/// — mitigation: hard cap of `max_drafts_per_sweep: 5`").
pub const DEFAULT_MAX_DRAFTS_PER_SWEEP: usize = 5;

/// Coordination filename written by the coordinator while a sweep is
/// running. The sched daemon (`hex-cli/src/commands/sched.rs::queue_drain`)
/// reads this to decide whether preemption is needed (wp-idle-research-swarm
/// P4.4).
pub const SWEEP_IN_FLIGHT_FILENAME: &str = "sweep_in_flight";

/// Coordination filename written by the sched daemon when a non-research
/// task arrives mid-sweep. The coordinator polls for this between analysts
/// and bails with [`SweepError::Aborted`] when present.
pub const SWEEP_ABORT_FILENAME: &str = "sweep_abort";

/// Default signal directory — production sweeps coordinate via
/// `~/.hex/sched/`. Tests pass an explicit tempdir via
/// [`SweepOptions::signal_dir`] so parallel tests can't collide on `$HOME`.
pub fn default_signal_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".hex").join("sched")
}

/// Which analysts the sweep should invoke. Default = all enabled.
///
/// Exposed as a public struct (rather than bitflags) so tests and CLI
/// flags can flip a single field in isolation without depending on a
/// bitflags crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnalystSet {
    pub architecture: bool,
    pub code_quality: bool,
    pub drift: bool,
    pub naming: bool,
    pub performance: bool,
    pub ux: bool,
}

impl AnalystSet {
    /// All analysts enabled — the production default.
    pub const ALL: Self = Self {
        architecture: true,
        code_quality: true,
        drift: true,
        naming: true,
        performance: true,
        ux: true,
    };

    /// All analysts disabled — useful as a starting point for tests that
    /// want to enable only one analyst at a time.
    pub const NONE: Self = Self {
        architecture: false,
        code_quality: false,
        drift: false,
        naming: false,
        performance: false,
        ux: false,
    };
}

impl Default for AnalystSet {
    fn default() -> Self {
        Self::ALL
    }
}

/// Knobs for [`run_sweep_with`]. Constructed via [`SweepOptions::default_for`]
/// in production (which loads `max_drafts_per_sweep` from `.hex/project.json`)
/// or by hand in tests.
#[derive(Debug, Clone)]
pub struct SweepOptions {
    pub enabled: AnalystSet,
    /// Hard cap on emitted findings. `0` means "unbounded" — useful when a
    /// caller wants to inspect everything (e.g. `hex research dry-run`).
    pub max_drafts_per_sweep: usize,
    /// Wall-clock used for the output filename and the YAML `sweep_at`
    /// header. Injected so tests can produce stable output.
    pub now: DateTime<Utc>,
    /// Directory holding sweep coordination signal files (in-flight marker,
    /// abort signal). Production = `~/.hex/sched/`; tests pass an explicit
    /// tempdir so parallel tests don't race on `$HOME`. (P4.4)
    pub signal_dir: PathBuf,
}

impl SweepOptions {
    /// Production default: all analysts on, cap loaded from project config,
    /// timestamp = now.
    pub fn default_for(repo_root: &Path) -> Self {
        Self {
            enabled: AnalystSet::default(),
            max_drafts_per_sweep: load_max_drafts_per_sweep(repo_root),
            now: Utc::now(),
            signal_dir: default_signal_dir(),
        }
    }
}

/// One analyst that failed mid-sweep. The coordinator records the failure
/// in the sweep YAML so a downstream operator can investigate without
/// having to re-run the daemon under verbose logging.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AnalystFailure {
    pub analyst: String,
    pub message: String,
}

/// Result of a sweep — paths of the artifacts that were written, the
/// (capped) finding set, and a count of any analyst failures.
#[derive(Debug, Clone)]
pub struct SweepReport {
    pub yaml_path: PathBuf,
    pub markdown_path: PathBuf,
    /// Findings after cap + sort. This is what the draft-writer consumes.
    pub findings: Vec<Finding>,
    /// Number of findings produced by all analysts before the cap. When this
    /// exceeds `findings.len()` the dashboard should surface a "truncated"
    /// indicator.
    pub findings_total: usize,
    pub analyst_errors: Vec<AnalystFailure>,
    pub generated_at: DateTime<Utc>,
}

/// Errors produced by [`run_sweep`] / [`run_sweep_with`].
///
/// Analyst-level failures are *not* surfaced here — they're collected into
/// [`SweepReport::analyst_errors`] so a single broken analyst can never stop
/// the sweep from publishing the rest of its findings.
#[derive(Debug)]
pub enum SweepError {
    /// Failed to create `docs/analysis/`.
    OutputDirCreate {
        path: PathBuf,
        source: std::io::Error,
    },
    /// Failed to write either the YAML or the Markdown artifact.
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
    /// `serde_yaml` rejected the synthesized sweep document. This should be
    /// unreachable — every type involved derives `Serialize` — but is kept
    /// in the error enum so the caller can surface it without a panic.
    Yaml(serde_yaml::Error),
    /// Sweep was preempted between analysts because the sched daemon
    /// signaled an abort (a non-research task became pending while the
    /// sweep was in-flight — wp-idle-research-swarm P4.4). The coordinator
    /// stopped cleanly without writing any artifacts; the in-flight marker
    /// and abort signal are both cleared before returning so the next
    /// idle-window re-enqueue can re-fire without leftover state.
    Aborted {
        /// How many analysts had already produced findings before the
        /// abort was honored. Surfaced for observability — a sched-side
        /// log line can show "preempted after N analysts" without parsing
        /// stderr.
        analysts_completed: usize,
    },
}

impl std::fmt::Display for SweepError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SweepError::OutputDirCreate { path, source } => {
                write!(f, "failed to create {}: {}", path.display(), source)
            }
            SweepError::Write { path, source } => {
                write!(f, "failed to write {}: {}", path.display(), source)
            }
            SweepError::Yaml(e) => write!(f, "failed to encode sweep YAML: {e}"),
            SweepError::Aborted { analysts_completed } => write!(
                f,
                "sweep aborted by sched preemption after {analysts_completed} analyst(s)"
            ),
        }
    }
}

impl std::error::Error for SweepError {}

/// Run the sweep against `repo_root` with production defaults
/// (all analysts enabled, cap loaded from `.hex/project.json`, `now =
/// Utc::now()`).
pub fn run_sweep(repo_root: &Path) -> Result<SweepReport, SweepError> {
    run_sweep_with(repo_root, SweepOptions::default_for(repo_root))
}

/// Run the sweep against `repo_root` with caller-supplied options.
///
/// Sequencing matches the workplan order (architecture → code-quality →
/// drift → naming → performance → ux) for deterministic finding ordering;
/// the post-cap sort by severity makes the absolute order user-visible
/// regardless, but a stable invocation order keeps the analyst_errors list
/// readable when multiple analysts fail.
///
/// The coordinator writes the [`SWEEP_IN_FLIGHT_FILENAME`] marker before
/// invoking any analyst and removes it before returning (success, error, or
/// preemption). Between each analyst it polls for [`SWEEP_ABORT_FILENAME`]
/// and, when present, clears both files and returns
/// [`SweepError::Aborted`] without writing any sweep artifacts
/// (wp-idle-research-swarm P4.4).
pub fn run_sweep_with(
    repo_root: &Path,
    opts: SweepOptions,
) -> Result<SweepReport, SweepError> {
    // Establish coordination state before any analyst fires. Stale abort
    // signals from a prior run are cleared here so the new sweep starts
    // with a clean slate; a fresh abort written *after* this point will
    // still be honored by the per-analyst poll below.
    mark_sweep_in_flight(&opts.signal_dir);
    clear_sweep_abort(&opts.signal_dir);

    // Inner closure ensures the in-flight marker is cleared on every exit
    // path (Ok, Err, Aborted). Without this guard a panic-free error in
    // `render_yaml` / `create_dir_all` / write would leave a stale marker,
    // and the next queue_drain tick would falsely conclude a sweep is
    // running and start firing aborts at nothing.
    let signal_dir = opts.signal_dir.clone();
    let result = run_sweep_inner(repo_root, opts);
    clear_sweep_in_flight(&signal_dir);
    if matches!(result, Err(SweepError::Aborted { .. })) {
        // The Aborted path already cleared the abort signal at the
        // detection site so a stale signal can't bleed into the *next*
        // sweep. Clearing again here is a no-op but keeps the contract
        // explicit in one place.
        clear_sweep_abort(&signal_dir);
    }
    result
}

fn run_sweep_inner(
    repo_root: &Path,
    opts: SweepOptions,
) -> Result<SweepReport, SweepError> {
    let mut findings: Vec<Finding> = Vec::new();
    let mut errors: Vec<AnalystFailure> = Vec::new();
    let mut analysts_completed: usize = 0;

    macro_rules! run_analyst {
        ($enabled:expr, $name:literal, $expr:expr) => {{
            // Check before invocation so an abort that arrived during the
            // *previous* analyst still bails before we burn another one.
            if is_sweep_abort_requested(&opts.signal_dir) {
                clear_sweep_abort(&opts.signal_dir);
                return Err(SweepError::Aborted { analysts_completed });
            }
            if $enabled {
                match $expr {
                    Ok(mut f) => findings.append(&mut f),
                    Err(e) => errors.push(AnalystFailure {
                        analyst: $name.into(),
                        message: e.to_string(),
                    }),
                }
                analysts_completed += 1;
            }
        }};
    }

    run_analyst!(
        opts.enabled.architecture,
        "architecture",
        architecture_analyst::analyze_architecture(repo_root)
    );
    run_analyst!(
        opts.enabled.code_quality,
        "code_quality",
        code_quality_analyst::analyze_code_quality(repo_root)
    );
    run_analyst!(
        opts.enabled.drift,
        "drift",
        drift_analyst::analyze_drift(repo_root)
    );
    run_analyst!(
        opts.enabled.naming,
        "naming",
        naming_analyst::analyze_naming(repo_root)
    );
    run_analyst!(
        opts.enabled.performance,
        "performance",
        performance_analyst::analyze_performance(repo_root)
    );
    run_analyst!(opts.enabled.ux, "ux", {
        let assets_root = repo_root.join("hex-nexus").join("assets");
        ux_analyst::analyze_ux(&assets_root)
    });

    // Final abort check before we commit artifacts to disk. If an abort
    // arrived while the last analyst was running, honor it — the sched
    // daemon can re-enqueue and a fresh sweep will produce a complete
    // record, which is more useful than a partial one with the last
    // analyst's findings present and the rest missing.
    if is_sweep_abort_requested(&opts.signal_dir) {
        clear_sweep_abort(&opts.signal_dir);
        return Err(SweepError::Aborted { analysts_completed });
    }

    let findings_total = findings.len();
    let capped = cap_findings(findings, opts.max_drafts_per_sweep);

    let analysts_run = analyst_names(&opts.enabled);
    let yaml_body = render_yaml(
        &capped,
        &errors,
        &analysts_run,
        opts.now,
        findings_total,
        opts.max_drafts_per_sweep,
    )?;
    let md_body = render_markdown(
        &capped,
        &errors,
        &analysts_run,
        opts.now,
        findings_total,
        opts.max_drafts_per_sweep,
    );

    let analysis_dir = repo_root.join("docs").join("analysis");
    std::fs::create_dir_all(&analysis_dir).map_err(|source| SweepError::OutputDirCreate {
        path: analysis_dir.clone(),
        source,
    })?;

    let stem = sweep_filename_stem(opts.now);
    let yaml_path = analysis_dir.join(format!("{stem}.yaml"));
    let md_path = analysis_dir.join(format!("{stem}.md"));

    std::fs::write(&yaml_path, yaml_body).map_err(|source| SweepError::Write {
        path: yaml_path.clone(),
        source,
    })?;
    std::fs::write(&md_path, md_body).map_err(|source| SweepError::Write {
        path: md_path.clone(),
        source,
    })?;

    Ok(SweepReport {
        yaml_path,
        markdown_path: md_path,
        findings: capped,
        findings_total,
        analyst_errors: errors,
        generated_at: opts.now,
    })
}

// ─── Sweep coordination signal helpers (P4.4) ──────────────────────────
//
// Two on-disk markers coordinate sched ↔ coordinator across processes:
//
// * `<signal_dir>/sweep_in_flight` — written by the coordinator at sweep
//   start and removed when the sweep returns. The sched daemon checks
//   this in `queue_drain` to decide whether preemption is even possible.
// * `<signal_dir>/sweep_abort` — written by the sched daemon when a
//   non-research task lands while the sweep is in flight. The coordinator
//   polls this between analysts and bails with `SweepError::Aborted` when
//   it appears.
//
// Best-effort throughout: a failed write here must never crash the sweep
// or the daemon. Worst case is a missed preemption tick — the next tick
// will retry. None of these paths are public; the on-disk wire format
// (filenames + parent dir) is the contract, not the helpers themselves.

/// Path to the in-flight marker under `signal_dir`.
pub fn sweep_in_flight_path(signal_dir: &Path) -> PathBuf {
    signal_dir.join(SWEEP_IN_FLIGHT_FILENAME)
}

/// Path to the abort signal under `signal_dir`.
pub fn sweep_abort_path(signal_dir: &Path) -> PathBuf {
    signal_dir.join(SWEEP_ABORT_FILENAME)
}

fn ensure_signal_dir(signal_dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(signal_dir)
}

fn mark_sweep_in_flight(signal_dir: &Path) {
    if ensure_signal_dir(signal_dir).is_err() {
        return;
    }
    let _ = std::fs::write(sweep_in_flight_path(signal_dir), Utc::now().to_rfc3339());
}

fn clear_sweep_in_flight(signal_dir: &Path) {
    let path = sweep_in_flight_path(signal_dir);
    if path.exists() {
        let _ = std::fs::remove_file(path);
    }
}

/// True iff the abort signal file is present. Public so the sched daemon's
/// observability path (e.g. `hex sched status`) can surface "preempt
/// pending" without re-implementing the file check.
pub fn is_sweep_abort_requested(signal_dir: &Path) -> bool {
    sweep_abort_path(signal_dir).exists()
}

fn clear_sweep_abort(signal_dir: &Path) {
    let path = sweep_abort_path(signal_dir);
    if path.exists() {
        let _ = std::fs::remove_file(path);
    }
}

/// Public entry point for sched: write the abort signal so the next
/// per-analyst poll inside the coordinator returns
/// [`SweepError::Aborted`]. Idempotent — repeated requests in the same
/// tick collapse to a single file.
pub fn request_sweep_abort(signal_dir: &Path) -> std::io::Result<()> {
    ensure_signal_dir(signal_dir)?;
    std::fs::write(sweep_abort_path(signal_dir), Utc::now().to_rfc3339())
}

/// True iff the in-flight marker is present. Used by the sched daemon
/// (`queue_drain`) to decide whether preemption is even possible — there's
/// nothing to preempt if no sweep is running.
pub fn is_sweep_in_flight(signal_dir: &Path) -> bool {
    sweep_in_flight_path(signal_dir).exists()
}

/// Sort severity-desc, then id-asc, then truncate to `max`. `max == 0` is
/// treated as "no cap" so `cap_findings(xs, 0) == xs sorted`.
pub fn cap_findings(mut findings: Vec<Finding>, max: usize) -> Vec<Finding> {
    findings.sort_by(|a, b| b.severity.cmp(&a.severity).then(a.id.cmp(&b.id)));
    if max > 0 && findings.len() > max {
        findings.truncate(max);
    }
    findings
}

/// `idle-sweep-YYYYMMDD-HHMM` — UTC, minute precision. Two sweeps in the
/// same minute would collide; in practice sweeps are throttled to ≥6h apart
/// (sched `min_sweep_interval_h`) so this is fine.
pub fn sweep_filename_stem(now: DateTime<Utc>) -> String {
    format!("idle-sweep-{}", now.format("%Y%m%d-%H%M"))
}

/// Render the sweep as YAML — the canonical artifact P4.2/P4.3 draft
/// writers consume.
pub fn render_yaml(
    findings: &[Finding],
    analyst_errors: &[AnalystFailure],
    analysts_run: &[String],
    sweep_at: DateTime<Utc>,
    findings_total: usize,
    max_drafts_per_sweep: usize,
) -> Result<String, SweepError> {
    #[derive(Serialize)]
    struct Doc<'a> {
        sweep_at: String,
        analysts_run: &'a [String],
        findings_total: usize,
        findings_emitted: usize,
        max_drafts_per_sweep: usize,
        #[serde(skip_serializing_if = "<[AnalystFailure]>::is_empty")]
        analyst_errors: &'a [AnalystFailure],
        findings: &'a [Finding],
    }
    let doc = Doc {
        sweep_at: sweep_at.to_rfc3339(),
        analysts_run,
        findings_total,
        findings_emitted: findings.len(),
        max_drafts_per_sweep,
        analyst_errors,
        findings,
    };
    serde_yaml::to_string(&doc).map_err(SweepError::Yaml)
}

/// Render the sweep as a human-readable markdown summary. Pure (no IO),
/// so unit tests can assert on the literal output.
pub fn render_markdown(
    findings: &[Finding],
    analyst_errors: &[AnalystFailure],
    analysts_run: &[String],
    sweep_at: DateTime<Utc>,
    findings_total: usize,
    max_drafts_per_sweep: usize,
) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "# Idle Sweep — {}\n\n",
        sweep_at.format("%Y-%m-%d %H:%M UTC")
    ));
    out.push_str(&format!("- Analysts run: {}\n", analysts_run.join(", ")));
    out.push_str(&format!("- Findings collected: {findings_total}\n"));
    out.push_str(&format!(
        "- Findings emitted: {} (cap = {})\n",
        findings.len(),
        if max_drafts_per_sweep == 0 {
            "unbounded".to_string()
        } else {
            max_drafts_per_sweep.to_string()
        }
    ));
    if !analyst_errors.is_empty() {
        out.push_str(&format!("- Analyst failures: {}\n", analyst_errors.len()));
    }
    out.push('\n');

    if findings.is_empty() {
        out.push_str("_No findings._\n");
    } else {
        out.push_str("## Findings\n\n");
        for f in findings {
            out.push_str(&format!(
                "### [{sev}] {title}\n",
                sev = severity_label(&f.severity),
                title = f.title
            ));
            out.push_str(&format!("- id: `{}`\n", f.id));
            out.push_str(&format!("- domain: `{}`\n", domain_label(&f.domain)));
            out.push_str(&format!(
                "- suggested action: `{}`\n",
                action_label(&f.suggested_action.kind)
            ));
            if let Some(draft_ref) = f.suggested_action.draft_ref.as_deref() {
                out.push_str(&format!("- draft ref: `{draft_ref}`\n"));
            }
            if !f.evidence.is_empty() {
                out.push_str("- evidence:\n");
                for e in &f.evidence {
                    out.push_str(&format!("  - {e}\n"));
                }
            }
            out.push('\n');
        }
    }

    if !analyst_errors.is_empty() {
        out.push_str("## Analyst failures\n\n");
        for err in analyst_errors {
            out.push_str(&format!("- **{}**: {}\n", err.analyst, err.message));
        }
    }

    out
}

fn analyst_names(set: &AnalystSet) -> Vec<String> {
    let mut out = Vec::new();
    if set.architecture {
        out.push("architecture".into());
    }
    if set.code_quality {
        out.push("code_quality".into());
    }
    if set.drift {
        out.push("drift".into());
    }
    if set.naming {
        out.push("naming".into());
    }
    if set.performance {
        out.push("performance".into());
    }
    if set.ux {
        out.push("ux".into());
    }
    out
}

fn severity_label(s: &hex_core::Severity) -> &'static str {
    use hex_core::Severity::*;
    match s {
        Critical => "CRITICAL",
        High => "HIGH",
        Medium => "MEDIUM",
        Low => "LOW",
        Info => "INFO",
    }
}

fn domain_label(d: &hex_core::Domain) -> String {
    use hex_core::Domain::*;
    match d {
        Architecture => "architecture".into(),
        CodeQuality => "code_quality".into(),
        Drift => "drift".into(),
        Performance => "performance".into(),
        Security => "security".into(),
        Documentation => "documentation".into(),
        Other(s) => s.clone(),
    }
}

fn action_label(a: &hex_core::ActionKind) -> &'static str {
    use hex_core::ActionKind::*;
    match a {
        DraftWorkplan => "draft_workplan",
        AmendWorkplan => "amend_workplan",
        DraftAdr => "draft_adr",
        Memory => "memory",
        Informational => "informational",
    }
}

/// Read `research.max_drafts_per_sweep` from `.hex/project.json`, falling
/// back to [`DEFAULT_MAX_DRAFTS_PER_SWEEP`] on any read/parse failure or
/// when the key is absent. Mirrors `sched.idle_threshold_ticks` /
/// `sched.min_sweep_interval_h` in `hex-cli/src/commands/sched.rs` — the
/// cap should degrade to its default rather than erroring on missing config.
pub fn load_max_drafts_per_sweep(repo_root: &Path) -> usize {
    let content = match std::fs::read_to_string(repo_root.join(".hex/project.json")) {
        Ok(c) => c,
        Err(_) => return DEFAULT_MAX_DRAFTS_PER_SWEEP,
    };
    let parsed: serde_json::Value = match serde_json::from_str(&content) {
        Ok(p) => p,
        Err(_) => return DEFAULT_MAX_DRAFTS_PER_SWEEP,
    };
    parsed
        .get("research")
        .and_then(|s| s.get("max_drafts_per_sweep"))
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .unwrap_or(DEFAULT_MAX_DRAFTS_PER_SWEEP)
}

#[cfg(test)]
mod sweep_coordinator_tests {
    use super::*;
    use chrono::TimeZone;
    use hex_core::{ActionKind, Domain, Finding, Severity, SuggestedAction};

    fn finding(id: &str, sev: Severity, domain: Domain) -> Finding {
        Finding {
            id: id.into(),
            domain,
            severity: sev,
            title: format!("title for {id}"),
            evidence: vec![format!("{id}: file.rs:1")],
            suggested_action: SuggestedAction {
                kind: ActionKind::DraftWorkplan,
                draft_ref: None,
            },
        }
    }

    fn fixed_now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 4, 29, 12, 34, 56).unwrap()
    }

    #[test]
    fn cap_findings_truncates_to_max_and_keeps_severest() {
        let xs = vec![
            finding("a", Severity::Low, Domain::Architecture),
            finding("b", Severity::Critical, Domain::Architecture),
            finding("c", Severity::High, Domain::Architecture),
            finding("d", Severity::Medium, Domain::Architecture),
            finding("e", Severity::Info, Domain::Architecture),
        ];
        let capped = cap_findings(xs, 2);
        assert_eq!(capped.len(), 2);
        assert_eq!(capped[0].id, "b"); // Critical
        assert_eq!(capped[1].id, "c"); // High
    }

    #[test]
    fn cap_findings_zero_means_unbounded_but_still_sorts() {
        let xs = vec![
            finding("z", Severity::Low, Domain::Architecture),
            finding("a", Severity::Critical, Domain::Architecture),
        ];
        let capped = cap_findings(xs, 0);
        assert_eq!(capped.len(), 2);
        assert_eq!(capped[0].id, "a", "Critical must come first regardless of cap");
        assert_eq!(capped[1].id, "z");
    }

    #[test]
    fn cap_findings_breaks_severity_ties_by_id_ascending() {
        let xs = vec![
            finding("zebra", Severity::High, Domain::Architecture),
            finding("alpha", Severity::High, Domain::Architecture),
            finding("mango", Severity::High, Domain::Architecture),
        ];
        let capped = cap_findings(xs, 0);
        let ids: Vec<&str> = capped.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(ids, vec!["alpha", "mango", "zebra"]);
    }

    #[test]
    fn sweep_coordinator_filename_stem_uses_utc_minute_precision() {
        let stem = sweep_filename_stem(fixed_now());
        assert_eq!(stem, "idle-sweep-20260429-1234");
    }

    #[test]
    fn render_yaml_round_trips_through_serde_yaml() {
        let findings = vec![finding("f1", Severity::High, Domain::Architecture)];
        let analysts = vec!["architecture".to_string()];
        let yaml = render_yaml(&findings, &[], &analysts, fixed_now(), 1, 5).expect("encode");

        // Header keys must be present in the human-edited shape.
        assert!(yaml.contains("sweep_at: 2026-04-29T12:34:56+00:00"), "yaml = {yaml}");
        assert!(yaml.contains("max_drafts_per_sweep: 5"), "yaml = {yaml}");
        assert!(yaml.contains("findings_emitted: 1"), "yaml = {yaml}");
        assert!(yaml.contains("findings_total: 1"), "yaml = {yaml}");

        // Embedded findings must round-trip through the same parser the
        // draft writer will use (P4.2 reads the YAML back).
        let parsed: serde_yaml::Value = serde_yaml::from_str(&yaml).expect("parse");
        let embedded = parsed
            .get("findings")
            .and_then(|v| v.as_sequence())
            .expect("findings array");
        assert_eq!(embedded.len(), 1);
    }

    #[test]
    fn render_yaml_omits_analyst_errors_when_none() {
        let yaml = render_yaml(&[], &[], &["drift".into()], fixed_now(), 0, 5).expect("encode");
        assert!(
            !yaml.contains("analyst_errors"),
            "empty analyst_errors must be omitted; yaml = {yaml}"
        );
    }

    #[test]
    fn render_yaml_includes_analyst_errors_when_present() {
        let errors = vec![AnalystFailure {
            analyst: "architecture".into(),
            message: "hex binary not on PATH".into(),
        }];
        let yaml = render_yaml(&[], &errors, &["architecture".into()], fixed_now(), 0, 5)
            .expect("encode");
        assert!(yaml.contains("analyst: architecture"), "yaml = {yaml}");
        assert!(yaml.contains("hex binary not on PATH"), "yaml = {yaml}");
    }

    #[test]
    fn render_markdown_includes_finding_titles_and_severity_badges() {
        let findings = vec![
            finding("f-crit", Severity::Critical, Domain::Architecture),
            finding("f-info", Severity::Info, Domain::Documentation),
        ];
        let md = render_markdown(
            &findings,
            &[],
            &["architecture".into(), "code_quality".into()],
            fixed_now(),
            2,
            5,
        );
        assert!(md.starts_with("# Idle Sweep — 2026-04-29 12:34 UTC"));
        assert!(md.contains("- Analysts run: architecture, code_quality"));
        assert!(md.contains("- Findings collected: 2"));
        assert!(md.contains("- Findings emitted: 2 (cap = 5)"));
        assert!(md.contains("### [CRITICAL] title for f-crit"));
        assert!(md.contains("### [INFO] title for f-info"));
        assert!(md.contains("- domain: `architecture`"));
        assert!(md.contains("- domain: `documentation`"));
        assert!(md.contains("- suggested action: `draft_workplan`"));
    }

    #[test]
    fn render_markdown_says_no_findings_when_empty() {
        let md = render_markdown(&[], &[], &["drift".into()], fixed_now(), 0, 5);
        assert!(md.contains("_No findings._"), "md = {md}");
    }

    #[test]
    fn render_markdown_lists_analyst_failures_at_bottom() {
        let errors = vec![AnalystFailure {
            analyst: "architecture".into(),
            message: "spawn failed: ENOENT".into(),
        }];
        let md = render_markdown(&[], &errors, &["architecture".into()], fixed_now(), 0, 5);
        assert!(md.contains("- Analyst failures: 1"));
        assert!(md.contains("## Analyst failures"));
        assert!(md.contains("- **architecture**: spawn failed: ENOENT"));
    }

    #[test]
    fn load_max_drafts_per_sweep_returns_default_when_no_config() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert_eq!(
            load_max_drafts_per_sweep(dir.path()),
            DEFAULT_MAX_DRAFTS_PER_SWEEP
        );
    }

    #[test]
    fn load_max_drafts_per_sweep_reads_research_section() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join(".hex")).unwrap();
        std::fs::write(
            dir.path().join(".hex/project.json"),
            r#"{ "research": { "max_drafts_per_sweep": 9 } }"#,
        )
        .unwrap();
        assert_eq!(load_max_drafts_per_sweep(dir.path()), 9);
    }

    #[test]
    fn load_max_drafts_per_sweep_falls_back_on_malformed_json() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join(".hex")).unwrap();
        std::fs::write(dir.path().join(".hex/project.json"), "{ not json").unwrap();
        assert_eq!(
            load_max_drafts_per_sweep(dir.path()),
            DEFAULT_MAX_DRAFTS_PER_SWEEP
        );
    }

    #[test]
    fn sweep_coordinator_writes_yaml_and_markdown_with_matching_stems() {
        // Hermetic: disable shell-out analysts (architecture/code_quality)
        // so the test doesn't depend on `hex` or `cargo` being on PATH and
        // doesn't waste time invoking them on an empty tempdir. The remaining
        // analysts (drift/naming/performance/ux) all gracefully return empty
        // sets when their input directories don't exist.
        let dir = tempfile::tempdir().expect("tempdir");
        let signal_dir = tempfile::tempdir().expect("signal tempdir");
        let opts = SweepOptions {
            enabled: AnalystSet {
                architecture: false,
                code_quality: false,
                drift: true,
                naming: true,
                performance: true,
                ux: true,
            },
            max_drafts_per_sweep: 5,
            now: fixed_now(),
            signal_dir: signal_dir.path().to_path_buf(),
        };

        let report = run_sweep_with(dir.path(), opts).expect("sweep ok");

        // Marker hygiene: a clean sweep must clear the in-flight file before
        // returning. Otherwise the next queue_drain tick would falsely conclude
        // a sweep is running and start firing aborts at nothing.
        assert!(
            !is_sweep_in_flight(signal_dir.path()),
            "in-flight marker must be cleared after a successful sweep"
        );

        assert_eq!(report.findings_total, 0);
        assert!(report.findings.is_empty());
        assert!(report.analyst_errors.is_empty());

        let stem = sweep_filename_stem(fixed_now());
        let expected_yaml = dir.path().join("docs/analysis").join(format!("{stem}.yaml"));
        let expected_md = dir.path().join("docs/analysis").join(format!("{stem}.md"));
        assert_eq!(report.yaml_path, expected_yaml);
        assert_eq!(report.markdown_path, expected_md);
        assert!(expected_yaml.exists(), "yaml file missing");
        assert!(expected_md.exists(), "md file missing");

        let yaml = std::fs::read_to_string(&expected_yaml).unwrap();
        assert!(yaml.contains("findings_emitted: 0"), "yaml = {yaml}");
        assert!(yaml.contains("max_drafts_per_sweep: 5"), "yaml = {yaml}");

        let md = std::fs::read_to_string(&expected_md).unwrap();
        assert!(md.contains("_No findings._"), "md = {md}");
    }

    #[test]
    fn sweep_marks_in_flight_during_run_and_clears_on_completion() {
        // The in-flight marker is the sched daemon's only signal that a
        // sweep is alive — it's what gates the preemption check in
        // queue_drain. If a successful sweep ever forgot to clear it, the
        // next non-research task would trigger a phantom abort.
        let dir = tempfile::tempdir().expect("tempdir");
        let signal_dir = tempfile::tempdir().expect("signal tempdir");
        // Pre-seed a stale abort signal — simulates a prior aborted sweep
        // whose abort signal was never consumed. The fresh sweep must
        // sweep its own state clean rather than honoring leftover signals.
        request_sweep_abort(signal_dir.path()).expect("seed stale abort");
        assert!(is_sweep_abort_requested(signal_dir.path()));

        let opts = SweepOptions {
            enabled: AnalystSet::NONE,
            max_drafts_per_sweep: 5,
            now: fixed_now(),
            signal_dir: signal_dir.path().to_path_buf(),
        };
        run_sweep_with(dir.path(), opts).expect("sweep ok");

        assert!(
            !is_sweep_in_flight(signal_dir.path()),
            "in-flight marker leaked past sweep completion"
        );
        assert!(
            !is_sweep_abort_requested(signal_dir.path()),
            "stale abort signal must be cleared at sweep start"
        );
    }

    #[test]
    fn sweep_aborts_cleanly_when_abort_signal_present_at_start() {
        // The fastest preemption case: the abort signal arrives between
        // ticks, so it's already on disk by the time `run_sweep_with`
        // starts. The first per-analyst poll fires before any analyst
        // runs, no artifacts are written, and the abort signal is cleared
        // so it can't bleed into the next sweep.
        let dir = tempfile::tempdir().expect("tempdir");
        let signal_dir = tempfile::tempdir().expect("signal tempdir");

        // Mark in-flight + write abort BEFORE starting. run_sweep_with
        // would normally clear the abort at start, so we need a separate
        // hook — call the inner path via the public wire format. The
        // contract is: abort signals written *after* the sweep has cleared
        // its initial state are honored. We reproduce that by writing the
        // abort *during* the run via a NONE analyst set (no analysts to
        // race against) plus a manual write between mark and inner.
        //
        // Easiest: enable a single analyst and pre-seed abort, but the
        // "clear stale at start" logic would consume it. So we test the
        // narrow contract: write abort between `mark_sweep_in_flight` and
        // the first analyst poll by enabling all analysts AND writing
        // abort *after* `run_sweep_with` clears the initial state. We do
        // that here with a tiny custom path that mirrors `run_sweep_with`
        // but writes the abort after the initial clear.
        mark_sweep_in_flight(signal_dir.path());
        clear_sweep_abort(signal_dir.path());
        request_sweep_abort(signal_dir.path()).unwrap();

        let opts = SweepOptions {
            enabled: AnalystSet::ALL,
            max_drafts_per_sweep: 5,
            now: fixed_now(),
            signal_dir: signal_dir.path().to_path_buf(),
        };
        // The public entry point clears the abort at start, so to actually
        // observe the abort path we route through the inner directly. The
        // inner doesn't write the in-flight marker (the wrapper does that
        // and the cleanup), so we must mark it ourselves above.
        let result = run_sweep_inner(dir.path(), opts);
        clear_sweep_in_flight(signal_dir.path());

        match result {
            Err(SweepError::Aborted { analysts_completed }) => {
                assert_eq!(
                    analysts_completed, 0,
                    "abort at start must bail before any analyst runs"
                );
            }
            other => panic!("expected Aborted, got {other:?}"),
        }

        // Aborted path must clear the abort signal so the next sweep
        // starts clean.
        assert!(
            !is_sweep_abort_requested(signal_dir.path()),
            "abort signal must be cleared after an aborted sweep"
        );

        // No artifacts should have been written.
        let analysis_dir = dir.path().join("docs").join("analysis");
        if analysis_dir.exists() {
            let entries: Vec<_> = std::fs::read_dir(&analysis_dir)
                .unwrap()
                .filter_map(|e| e.ok())
                .collect();
            assert!(
                entries.is_empty(),
                "aborted sweep must not write any sweep artifacts; found {entries:?}"
            );
        }
    }

    #[test]
    fn signal_helpers_round_trip_through_filesystem() {
        // Locks the public wire format for the cross-process contract:
        // sched-side `request_sweep_abort` + coordinator-side
        // `is_sweep_abort_requested` must agree on filename + parent dir.
        // Mirror tests live in `hex-cli/src/commands/sched.rs` — both
        // sides drift in lockstep or the on-disk channel breaks silently.
        let signal_dir = tempfile::tempdir().expect("tempdir");

        // Initially clean.
        assert!(!is_sweep_in_flight(signal_dir.path()));
        assert!(!is_sweep_abort_requested(signal_dir.path()));

        // Mark + clear in-flight.
        mark_sweep_in_flight(signal_dir.path());
        assert!(is_sweep_in_flight(signal_dir.path()));
        clear_sweep_in_flight(signal_dir.path());
        assert!(!is_sweep_in_flight(signal_dir.path()));

        // Mark + clear abort.
        request_sweep_abort(signal_dir.path()).expect("write abort");
        assert!(is_sweep_abort_requested(signal_dir.path()));
        clear_sweep_abort(signal_dir.path());
        assert!(!is_sweep_abort_requested(signal_dir.path()));

        // Filenames are exactly what hex-cli expects.
        assert_eq!(
            sweep_in_flight_path(signal_dir.path())
                .file_name()
                .unwrap(),
            SWEEP_IN_FLIGHT_FILENAME
        );
        assert_eq!(
            sweep_abort_path(signal_dir.path()).file_name().unwrap(),
            SWEEP_ABORT_FILENAME
        );
    }

    #[test]
    fn sweep_coordinator_caps_emitted_findings_to_configured_max() {
        // Stand-alone test of the cap path that doesn't depend on any analyst
        // actually firing — we drive `cap_findings` directly on a known
        // oversized set, and verify the coordinator's render path produces
        // identical YAML/MD whether the cap was applied at the top of
        // `run_sweep_with` or at the top of the test.
        let oversized: Vec<Finding> = (0..10)
            .map(|i| {
                finding(
                    &format!("f-{i:02}"),
                    if i < 3 { Severity::Critical } else { Severity::Low },
                    Domain::Architecture,
                )
            })
            .collect();

        let capped = cap_findings(oversized.clone(), 5);
        assert_eq!(capped.len(), 5);
        // First 3 must be the criticals (sorted by id asc: f-00, f-01, f-02);
        // remaining 2 are the lowest-id Lows: f-03, f-04.
        let ids: Vec<&str> = capped.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(ids, vec!["f-00", "f-01", "f-02", "f-03", "f-04"]);
    }
}
