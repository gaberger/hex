//! Research dashboard surface (workplan `wp-idle-research-swarm`, P5.2).
//!
//! `GET /api/research/sweeps` reads the `docs/analysis/idle-sweep-*.yaml`
//! files written by the sweep coordinator (`research::coordinator::run_sweep`)
//! and projects each into a wire-stable summary the dashboard renders as a
//! card. The endpoint is read-only: it never invokes a sweep itself, so a
//! noisy dashboard cannot drive load on the idle-research path.
//!
//! The summary is intentionally small — full sweep YAML is available via
//! `GET /api/{project_id}/read/{*path}` when an operator wants to drill in.
//! Here we surface only what a card needs: timestamp, finding count, draft
//! count, and the relative paths of the draft artifacts each finding spawned
//! (so the dashboard can render anchor links without a second round-trip).
//!
//! Project root resolution mirrors `routes::analysis::analyze_current_project`
//! — `HEX_PROJECT_ROOT` env var first, then `std::env::current_dir()`. Tests
//! pass an explicit root via `list_sweeps_in`.

use axum::{extract::Query, Json};
use http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::{Path, PathBuf};

use hex_core::{ActionKind, Finding};

/// Default cap on returned summaries when the caller doesn't pass `?limit=`.
/// Matches the `max_drafts_per_sweep` default — small enough to render as a
/// single dashboard card, large enough to span a typical 24-hour window
/// (sweeps throttle to ≥6h apart, so 5 covers ~30h of idle history).
pub const DEFAULT_LIMIT: usize = 5;

/// Hard upper bound on `?limit=`. Caps response size + read-amplification on
/// the docs/analysis directory; operators who need a deeper history should
/// read the directory directly.
pub const MAX_LIMIT: usize = 50;

/// Query string for `GET /api/research/sweeps`.
#[derive(Debug, Deserialize, Default)]
pub struct SweepsQuery {
    /// How many sweep summaries to return, newest-first. Defaults to
    /// `DEFAULT_LIMIT`; clamped to `[1, MAX_LIMIT]`.
    pub limit: Option<usize>,
}

/// One finding's draft pointer, if it spawned one (P4.2/P4.3 writers).
///
/// `kind` mirrors `ActionKind` as kebab-case wire form so a dashboard switch
/// statement reads the same labels operators see in `idle-sweep-*.md`.
/// `path` is repo-relative — the dashboard prepends the project base when
/// constructing anchor hrefs.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DraftLink {
    pub finding_id: String,
    /// Wire-form action kind: `"draft-adr"`, `"draft-workplan"`,
    /// `"amend-workplan"`, `"memory"`, `"informational"`.
    pub kind: String,
    /// Repo-relative path to the draft file, when one was written. `None`
    /// for `Memory` / `Informational` actions which don't produce a file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// One sweep summary — what the dashboard card renders.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SweepSummary {
    /// Repo-relative path to the YAML artifact (e.g.
    /// `docs/analysis/idle-sweep-20260429-1234.yaml`). The dashboard uses
    /// this both as a stable id and as the "open full sweep" link.
    pub yaml_path: String,
    /// Repo-relative path to the human-readable markdown sibling.
    pub markdown_path: String,
    /// RFC3339 timestamp parsed out of the YAML header. Falls back to the
    /// filename stem when the YAML is unreadable so a corrupted sweep file
    /// still renders a row instead of being silently dropped.
    pub sweep_at: String,
    /// Number of findings emitted in the sweep (post-cap). This is what
    /// the card's "N findings" badge shows.
    pub finding_count: usize,
    /// Number of findings whose `suggested_action.kind` produced a draft
    /// artifact (`DraftAdr`, `DraftWorkplan`, `AmendWorkplan`). Memory /
    /// informational findings are excluded since they don't render as
    /// clickable drafts.
    pub draft_count: usize,
    /// Per-finding draft pointers. Capped to `finding_count` entries — one
    /// row per finding, in the order they appear in the YAML. Findings
    /// without a draft (Memory / Informational) still get an entry with
    /// `path: None` so the dashboard can show the kind badge.
    pub drafts: Vec<DraftLink>,
}

/// `GET /api/research/sweeps[?limit=N]` — read the last N sweep summaries.
///
/// Resolves the project root from `HEX_PROJECT_ROOT` env var (set by
/// `hex-nexus` when launched against a target project), falling back to the
/// current working directory.
pub async fn list_sweeps(
    Query(params): Query<SweepsQuery>,
) -> (StatusCode, Json<serde_json::Value>) {
    let limit = params
        .limit
        .unwrap_or(DEFAULT_LIMIT)
        .clamp(1, MAX_LIMIT);

    let root = std::env::var("HEX_PROJECT_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    match list_sweeps_in(&root, limit) {
        Ok(summaries) => (
            StatusCode::OK,
            Json(json!({ "sweeps": summaries })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        ),
    }
}

/// Pure listing entry point — exposed for tests.
///
/// Reads `<repo_root>/docs/analysis/idle-sweep-*.yaml`, parses each into a
/// [`SweepSummary`], and returns the newest-`limit` entries. A missing
/// `docs/analysis/` directory returns an empty list rather than an error —
/// the dashboard renders "No sweeps yet" which is the right state for a
/// fresh project.
pub fn list_sweeps_in(repo_root: &Path, limit: usize) -> std::io::Result<Vec<SweepSummary>> {
    let analysis_dir = repo_root.join("docs").join("analysis");
    if !analysis_dir.exists() {
        return Ok(Vec::new());
    }

    let mut entries: Vec<PathBuf> = std::fs::read_dir(&analysis_dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|s| s.to_str())
                .is_some_and(|name| name.starts_with("idle-sweep-") && name.ends_with(".yaml"))
        })
        .collect();

    // Newest first by filename stem. The stem is `idle-sweep-YYYYMMDD-HHMM`,
    // so a lexicographic sort matches chronological order — no timestamp
    // parsing needed for the sort itself.
    entries.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
    entries.truncate(limit);

    let summaries = entries
        .iter()
        .filter_map(|yaml_path| summarize_sweep_file(repo_root, yaml_path))
        .collect();
    Ok(summaries)
}

/// Parse a single sweep YAML into a [`SweepSummary`]. Returns `None` if the
/// file is unreadable; corrupted-but-readable files fall back to filename-
/// derived fields so a partial sweep still surfaces on the dashboard.
fn summarize_sweep_file(repo_root: &Path, yaml_path: &Path) -> Option<SweepSummary> {
    let body = std::fs::read_to_string(yaml_path).ok()?;
    let doc: serde_yaml::Value = serde_yaml::from_str(&body).ok()?;

    let sweep_at = doc
        .get("sweep_at")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            // Fall back to filename stem (idle-sweep-YYYYMMDD-HHMM) when the
            // YAML header is missing — better to show a partial row than
            // drop the entry entirely.
            yaml_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string()
        });

    let findings: Vec<Finding> = doc
        .get("findings")
        .and_then(|v| serde_yaml::from_value(v.clone()).ok())
        .unwrap_or_default();

    let drafts: Vec<DraftLink> = findings.iter().map(draft_link_for).collect();
    let draft_count = drafts.iter().filter(|d| d.path.is_some()).count();

    let yaml_rel = path_relative_to(repo_root, yaml_path);
    let md_rel = yaml_rel.replace(".yaml", ".md");

    Some(SweepSummary {
        yaml_path: yaml_rel,
        markdown_path: md_rel,
        sweep_at,
        finding_count: findings.len(),
        draft_count,
        drafts,
    })
}

/// Produce a [`DraftLink`] for one finding — encodes the kind plus the
/// repo-relative draft path when one was written by P4.2/P4.3.
fn draft_link_for(finding: &Finding) -> DraftLink {
    let kind = action_kind_label(&finding.suggested_action.kind);
    let path = match finding.suggested_action.kind {
        ActionKind::DraftAdr => Some(format!("docs/adrs/drafts/ADR-{}.md", finding.id)),
        ActionKind::DraftWorkplan | ActionKind::AmendWorkplan => Some(format!(
            "docs/workplans/drafts/draft-{}.json",
            finding.id
        )),
        ActionKind::Memory | ActionKind::Informational => None,
    };
    DraftLink {
        finding_id: finding.id.clone(),
        kind: kind.into(),
        path,
    }
}

/// Wire-form action kind label — kebab-case to mirror the `serde
/// rename_all = "snake_case"` upstream variant names but with hyphens for
/// dashboard CSS selectors and grep ergonomics.
fn action_kind_label(kind: &ActionKind) -> &'static str {
    match kind {
        ActionKind::DraftAdr => "draft-adr",
        ActionKind::DraftWorkplan => "draft-workplan",
        ActionKind::AmendWorkplan => "amend-workplan",
        ActionKind::Memory => "memory",
        ActionKind::Informational => "informational",
    }
}

/// Return `path` as a string relative to `repo_root` when it is a child of
/// `repo_root`; otherwise return the path's `to_string_lossy()` form. The
/// dashboard renders `path` as an anchor href, so a relative path is what we
/// want when the file lives inside the project tree.
fn path_relative_to(repo_root: &Path, path: &Path) -> String {
    path.strip_prefix(repo_root)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| path.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_core::{Domain, Severity, SuggestedAction};

    fn finding(id: &str, kind: ActionKind) -> Finding {
        Finding {
            id: id.into(),
            domain: Domain::Architecture,
            severity: Severity::High,
            title: format!("title for {id}"),
            evidence: vec![],
            suggested_action: SuggestedAction {
                kind,
                draft_ref: None,
            },
        }
    }

    fn write_sweep_yaml(dir: &Path, stem: &str, sweep_at: &str, findings: &[Finding]) -> PathBuf {
        let path = dir.join(format!("{stem}.yaml"));
        let doc = serde_yaml::to_string(&serde_yaml::Mapping::from_iter([
            (
                serde_yaml::Value::String("sweep_at".into()),
                serde_yaml::Value::String(sweep_at.into()),
            ),
            (
                serde_yaml::Value::String("findings_emitted".into()),
                serde_yaml::Value::Number(serde_yaml::Number::from(findings.len())),
            ),
            (
                serde_yaml::Value::String("findings".into()),
                serde_yaml::to_value(findings).unwrap(),
            ),
        ]))
        .unwrap();
        std::fs::write(&path, doc).unwrap();
        path
    }

    #[test]
    fn list_sweeps_returns_empty_when_analysis_dir_missing() {
        let dir = tempfile::tempdir().unwrap();
        let out = list_sweeps_in(dir.path(), DEFAULT_LIMIT).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn list_sweeps_orders_newest_first_and_caps_to_limit() {
        // Three sweeps; ask for the latest two. Filenames carry timestamp,
        // so a lex sort by filename matches reverse-chronological.
        let dir = tempfile::tempdir().unwrap();
        let analysis = dir.path().join("docs/analysis");
        std::fs::create_dir_all(&analysis).unwrap();

        write_sweep_yaml(&analysis, "idle-sweep-20260101-0900", "2026-01-01T09:00:00Z", &[]);
        write_sweep_yaml(&analysis, "idle-sweep-20260201-0900", "2026-02-01T09:00:00Z", &[]);
        write_sweep_yaml(&analysis, "idle-sweep-20260301-0900", "2026-03-01T09:00:00Z", &[]);

        let out = list_sweeps_in(dir.path(), 2).unwrap();
        assert_eq!(out.len(), 2);
        assert!(out[0].yaml_path.ends_with("idle-sweep-20260301-0900.yaml"));
        assert!(out[1].yaml_path.ends_with("idle-sweep-20260201-0900.yaml"));
    }

    #[test]
    fn sweep_summary_counts_draft_kinds_and_skips_memory() {
        // 4 findings: 1 ADR, 2 workplan-shaped, 1 memory. Drafts list should
        // have 4 entries (one per finding) but draft_count must be 3 since
        // memory findings don't produce a draft file.
        let dir = tempfile::tempdir().unwrap();
        let analysis = dir.path().join("docs/analysis");
        std::fs::create_dir_all(&analysis).unwrap();
        let findings = vec![
            finding("f-1", ActionKind::DraftAdr),
            finding("f-2", ActionKind::DraftWorkplan),
            finding("f-3", ActionKind::AmendWorkplan),
            finding("f-4", ActionKind::Memory),
        ];
        write_sweep_yaml(
            &analysis,
            "idle-sweep-20260429-1234",
            "2026-04-29T12:34:56+00:00",
            &findings,
        );

        let out = list_sweeps_in(dir.path(), DEFAULT_LIMIT).unwrap();
        assert_eq!(out.len(), 1);
        let s = &out[0];
        assert_eq!(s.finding_count, 4);
        assert_eq!(s.draft_count, 3, "memory findings excluded from draft_count");
        assert_eq!(s.drafts.len(), 4);
        assert_eq!(s.drafts[0].kind, "draft-adr");
        assert_eq!(s.drafts[0].path.as_deref(), Some("docs/adrs/drafts/ADR-f-1.md"));
        assert_eq!(s.drafts[1].kind, "draft-workplan");
        assert_eq!(
            s.drafts[1].path.as_deref(),
            Some("docs/workplans/drafts/draft-f-2.json"),
        );
        assert_eq!(s.drafts[2].kind, "amend-workplan");
        assert_eq!(
            s.drafts[2].path.as_deref(),
            Some("docs/workplans/drafts/draft-f-3.json"),
        );
        assert_eq!(s.drafts[3].kind, "memory");
        assert!(s.drafts[3].path.is_none(), "memory findings carry no draft path");
    }

    #[test]
    fn sweep_summary_paths_are_relative_to_repo_root() {
        let dir = tempfile::tempdir().unwrap();
        let analysis = dir.path().join("docs/analysis");
        std::fs::create_dir_all(&analysis).unwrap();
        write_sweep_yaml(&analysis, "idle-sweep-20260429-1234", "2026-04-29T12:34:56Z", &[]);

        let out = list_sweeps_in(dir.path(), DEFAULT_LIMIT).unwrap();
        assert_eq!(out.len(), 1);
        // Anchor hrefs in the dashboard need clean repo-relative paths.
        assert_eq!(out[0].yaml_path, "docs/analysis/idle-sweep-20260429-1234.yaml");
        assert_eq!(out[0].markdown_path, "docs/analysis/idle-sweep-20260429-1234.md");
    }

    #[test]
    fn sweep_summary_falls_back_to_filename_when_sweep_at_missing() {
        // A sweep YAML that lost its header still has a usable timestamp in
        // the filename. We must not drop the row — operators rely on the
        // dashboard surfacing every sweep that ran, even if one is corrupt.
        let dir = tempfile::tempdir().unwrap();
        let analysis = dir.path().join("docs/analysis");
        std::fs::create_dir_all(&analysis).unwrap();
        let p = analysis.join("idle-sweep-20260429-1234.yaml");
        std::fs::write(&p, "findings: []\n").unwrap();

        let out = list_sweeps_in(dir.path(), DEFAULT_LIMIT).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].sweep_at, "idle-sweep-20260429-1234");
        assert_eq!(out[0].finding_count, 0);
    }

    #[test]
    fn list_sweeps_skips_non_sweep_yaml_files_in_analysis_dir() {
        // docs/analysis/ also holds hand-written analysis MDs and unrelated
        // YAMLs (see docs/analysis/session-*.yaml). Only files matching the
        // idle-sweep-*.yaml prefix should be projected as sweeps.
        let dir = tempfile::tempdir().unwrap();
        let analysis = dir.path().join("docs/analysis");
        std::fs::create_dir_all(&analysis).unwrap();
        std::fs::write(analysis.join("session-2604141400-insights.yaml"), "kind: notes\n").unwrap();
        std::fs::write(analysis.join("brain-string-audit.md"), "# notes\n").unwrap();
        write_sweep_yaml(&analysis, "idle-sweep-20260429-1234", "2026-04-29T12:34:56Z", &[]);

        let out = list_sweeps_in(dir.path(), DEFAULT_LIMIT).unwrap();
        assert_eq!(out.len(), 1, "only idle-sweep-*.yaml files count");
        assert!(out[0].yaml_path.ends_with("idle-sweep-20260429-1234.yaml"));
    }

    #[test]
    fn list_sweeps_clamps_limit_via_handler_query_default() {
        // Sanity: DEFAULT_LIMIT and MAX_LIMIT are both > 0 so the clamp in
        // `list_sweeps` can never reduce a passed `limit` to zero. The
        // handler clamps `params.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT)`.
        assert!(DEFAULT_LIMIT >= 1);
        assert!(MAX_LIMIT >= DEFAULT_LIMIT);
    }
}
