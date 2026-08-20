//! Drift analyst (workplan `wp-idle-research-swarm`, P2.3).
//!
//! Detects mismatches between workplan claims and reality:
//! - **Status drift** — a task says `status: done` but its declared `files[]`
//!   are missing on disk (the failure mode that prompted ADR-2026-04-14-2200).
//! - **Missing files** — a task declares files that don't yet exist regardless
//!   of status (informational; these are tasks not yet started).
//!
//! The workplan task description is "shell out to
//! `hex plan reconcile --all --dry-run --json`". That JSON output mode does
//! not exist on the CLI yet, so we replicate the relevant subset of
//! reconcile's logic in-process against `docs/workplans/wp-*.json`. When the
//! `--json` flag lands the IO entry point can be swapped for a
//! `Command::new("hex")` call without touching [`detect_drift`], which stays
//! pure and unit-testable.

use std::path::{Path, PathBuf};

use hex_core::{ActionKind, Domain, Finding, Severity, SuggestedAction};
use serde::Deserialize;
use sha2::{Digest, Sha256};

/// Errors produced by [`analyze_drift`].
#[derive(Debug)]
pub enum AnalystError {
    /// Failed to read a workplan file the analyst was inspecting.
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
}

impl std::fmt::Display for AnalystError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AnalystError::Read { path, source } => {
                write!(f, "failed to read {}: {}", path.display(), source)
            }
        }
    }
}

impl std::error::Error for AnalystError {}

#[derive(Debug, Default, Deserialize)]
struct DriftWorkplan {
    #[serde(default)]
    id: String,
    #[serde(default)]
    phases: Vec<DriftPhase>,
    #[serde(default)]
    steps: Vec<DriftTask>,
}

#[derive(Debug, Default, Deserialize)]
struct DriftPhase {
    #[serde(default)]
    tasks: Vec<DriftTask>,
}

#[derive(Debug, Default, Deserialize)]
struct DriftTask {
    #[serde(default)]
    id: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    files: Vec<String>,
}

/// Run the deterministic drift analyst against `repo_root`.
///
/// Reads every `docs/workplans/wp-*.json` and, for each task, compares its
/// declared `files[]` against the filesystem. Findings are emitted for two
/// failure modes: status drift (claimed done with files missing — `High`
/// severity, suggests amending the workplan) and missing files (declared but
/// absent regardless of status — `Medium` severity, informational).
pub fn analyze_drift(repo_root: &Path) -> Result<Vec<Finding>, AnalystError> {
    let workplans_dir = repo_root.join("docs").join("workplans");
    let entries = match std::fs::read_dir(&workplans_dir) {
        Ok(e) => e,
        // No workplans directory means nothing to check — not an error.
        Err(_) => return Ok(Vec::new()),
    };

    let mut findings: Vec<Finding> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if !name.starts_with("wp-") || !name.ends_with(".json") {
            continue;
        }
        let content = std::fs::read_to_string(&path).map_err(|source| AnalystError::Read {
            path: path.clone(),
            source,
        })?;
        // Malformed workplans are not this analyst's responsibility to surface
        // (`hex plan lint` covers schema violations); skip silently.
        let Ok(wp) = serde_json::from_str::<DriftWorkplan>(&content) else {
            continue;
        };
        findings.extend(detect_drift(&wp, name, repo_root));
    }
    Ok(findings)
}

fn detect_drift(wp: &DriftWorkplan, workplan_name: &str, repo_root: &Path) -> Vec<Finding> {
    let mut out: Vec<Finding> = Vec::new();
    let tasks: Vec<&DriftTask> = if !wp.phases.is_empty() {
        wp.phases.iter().flat_map(|p| &p.tasks).collect()
    } else {
        wp.steps.iter().collect()
    };

    for task in tasks {
        let missing: Vec<String> = task
            .files
            .iter()
            .filter(|f| !repo_root.join(f).exists())
            .cloned()
            .collect();
        if missing.is_empty() {
            continue;
        }
        let is_done = task.status == "done" || task.status == "completed";
        let (severity, kind, title) = if is_done {
            (
                Severity::High,
                ActionKind::AmendWorkplan,
                format!(
                    "{}/{}: status=done but {} declared file(s) missing",
                    wp.id,
                    task.id,
                    missing.len()
                ),
            )
        } else {
            (
                Severity::Medium,
                ActionKind::Informational,
                format!(
                    "{}/{}: {} declared file(s) missing",
                    wp.id,
                    task.id,
                    missing.len()
                ),
            )
        };

        let mut evidence = Vec::with_capacity(missing.len() + 3);
        evidence.push(format!("workplan: {workplan_name}"));
        evidence.push(format!("task: {}", task.id));
        evidence.push(format!("status: {}", task.status));
        for m in &missing {
            evidence.push(format!("missing file: {m}"));
        }

        out.push(Finding {
            id: stable_id(&wp.id, &task.id, &missing, is_done),
            domain: Domain::Drift,
            severity,
            title,
            evidence,
            suggested_action: SuggestedAction {
                kind,
                draft_ref: Some(workplan_name.to_string()),
            },
        });
    }
    out
}

fn stable_id(wp_id: &str, task_id: &str, missing: &[String], is_done: bool) -> String {
    let mut hasher = Sha256::new();
    hasher.update(wp_id.as_bytes());
    hasher.update([0u8]);
    hasher.update(task_id.as_bytes());
    hasher.update([0u8]);
    hasher.update([is_done as u8]);
    for m in missing {
        hasher.update([0u8]);
        hasher.update(m.as_bytes());
    }
    let digest = hasher.finalize();
    format!(
        "f-drift-{:016x}",
        u64::from_be_bytes(digest[..8].try_into().unwrap())
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn write_workplan(dir: &Path, name: &str, body: &str) {
        let wp_dir = dir.join("docs").join("workplans");
        fs::create_dir_all(&wp_dir).unwrap();
        fs::write(wp_dir.join(name), body).unwrap();
    }

    #[test]
    fn flags_status_drift_when_done_task_files_missing() {
        let tmp = tempdir().unwrap();
        let body = serde_json::json!({
            "id": "wp-foo",
            "phases": [{
                "id": "P1",
                "tasks": [{
                    "id": "P1.1",
                    "status": "done",
                    "files": ["src/missing.rs"]
                }]
            }]
        })
        .to_string();
        write_workplan(tmp.path(), "wp-foo.json", &body);

        let findings = analyze_drift(tmp.path()).unwrap();
        assert_eq!(findings.len(), 1, "got: {findings:?}");
        let f = &findings[0];
        assert_eq!(f.domain, Domain::Drift);
        assert_eq!(f.severity, Severity::High);
        assert!(f.title.contains("status=done"));
        assert!(f
            .evidence
            .iter()
            .any(|e| e == "missing file: src/missing.rs"));
        assert_eq!(f.suggested_action.kind, ActionKind::AmendWorkplan);
        assert_eq!(
            f.suggested_action.draft_ref.as_deref(),
            Some("wp-foo.json")
        );
    }

    #[test]
    fn flags_missing_files_for_pending_task_at_lower_severity() {
        let tmp = tempdir().unwrap();
        let body = serde_json::json!({
            "id": "wp-bar",
            "phases": [{
                "tasks": [{
                    "id": "P1.1",
                    "status": "pending",
                    "files": ["src/missing.rs"]
                }]
            }]
        })
        .to_string();
        write_workplan(tmp.path(), "wp-bar.json", &body);

        let findings = analyze_drift(tmp.path()).unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Medium);
        assert_eq!(findings[0].suggested_action.kind, ActionKind::Informational);
    }

    #[test]
    fn no_finding_when_files_exist() {
        let tmp = tempdir().unwrap();
        let real = tmp.path().join("src");
        fs::create_dir_all(&real).unwrap();
        fs::write(real.join("present.rs"), "// stub").unwrap();
        let body = serde_json::json!({
            "id": "wp-baz",
            "phases": [{
                "tasks": [{
                    "id": "P1.1",
                    "status": "done",
                    "files": ["src/present.rs"]
                }]
            }]
        })
        .to_string();
        write_workplan(tmp.path(), "wp-baz.json", &body);

        let findings = analyze_drift(tmp.path()).unwrap();
        assert!(
            findings.is_empty(),
            "expected no drift; got {findings:?}"
        );
    }

    #[test]
    fn skips_non_workplan_files() {
        let tmp = tempdir().unwrap();
        // Drafts and ad-hoc JSONs in the same directory are ignored.
        write_workplan(tmp.path(), "draft-something.json", r#"{"id":"x"}"#);
        write_workplan(
            tmp.path(),
            "wp-real.json",
            &serde_json::json!({
                "id": "wp-real",
                "phases": [{ "tasks": [{
                    "id": "T1", "status": "done", "files": ["nope.rs"]
                }]}]
            })
            .to_string(),
        );

        let findings = analyze_drift(tmp.path()).unwrap();
        assert_eq!(findings.len(), 1);
        assert!(findings[0].title.starts_with("wp-real/T1"));
    }

    #[test]
    fn handles_missing_workplans_dir() {
        let tmp = tempdir().unwrap();
        let findings = analyze_drift(tmp.path()).unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn supports_legacy_steps_format() {
        let tmp = tempdir().unwrap();
        let body = serde_json::json!({
            "id": "wp-legacy",
            "steps": [{
                "id": "S1",
                "status": "done",
                "files": ["gone.rs"]
            }]
        })
        .to_string();
        write_workplan(tmp.path(), "wp-legacy.json", &body);
        let findings = analyze_drift(tmp.path()).unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::High);
    }

    #[test]
    fn finding_id_is_stable_for_same_input() {
        let tmp = tempdir().unwrap();
        let body = serde_json::json!({
            "id": "wp-stable",
            "phases": [{ "tasks": [{
                "id": "T", "status": "done", "files": ["x.rs"]
            }]}]
        })
        .to_string();
        write_workplan(tmp.path(), "wp-stable.json", &body);
        let a = analyze_drift(tmp.path()).unwrap();
        let b = analyze_drift(tmp.path()).unwrap();
        assert_eq!(a[0].id, b[0].id);
        assert!(a[0].id.starts_with("f-drift-"));
    }

    #[test]
    fn yaml_roundtrip_for_emitted_finding() {
        // Anything we emit must satisfy the on-disk YAML contract from
        // `hex-core::research_finding`.
        let tmp = tempdir().unwrap();
        let body = serde_json::json!({
            "id": "wp-yaml",
            "phases": [{ "tasks": [{
                "id": "T", "status": "done", "files": ["nope.rs"]
            }]}]
        })
        .to_string();
        write_workplan(tmp.path(), "wp-yaml.json", &body);
        let f = analyze_drift(tmp.path()).unwrap().pop().unwrap();
        let yaml = serde_yaml::to_string(&f).expect("serialize");
        let back: Finding = serde_yaml::from_str(&yaml).expect("deserialize");
        assert_eq!(f, back);
        assert!(yaml.contains("domain: drift"), "yaml = {yaml}");
    }
}
