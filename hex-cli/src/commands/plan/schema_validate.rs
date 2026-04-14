//! Authoring-time workplan schema validation (ADR-2604142200, wp-enforce-workplan-evidence E1.1).
//!
//! Rejects workplans that would silently false-positive through `hex plan reconcile`
//! because their tasks have no file-path evidence. Runs at JSON load time — before
//! dispatch, before reconciliation — so bad shapes never enter the execution path.
//!
//! Pairs with the runtime verifier in `reconcile_evidence.rs`: this module prevents
//! the broken input from existing; that module catches any that slip through anyway.
//!
//! The central rule is simple: every task must declare at least one file path in
//! `files[]`. Prose-only done-conditions (e.g. "Rename brain.rs -> sched.rs" with
//! no files listed) are the pattern that let wp-rename-brain-to-sched reconcile as
//! 11/11 done while `hex sched` did not compile. Closed here.
//!
//! `validate_workplan_evidence` is a pure function — easy to unit-test and reusable
//! from `hex plan lint` (E3), pre-commit hooks (E3.2), and the CLI dispatch paths.

use super::{PhaseTask, Step, Workplan};

/// A schema violation found during authoring-time validation.
///
/// `task_id` identifies the offending task (id, or synthesized from position
/// if the task itself has no id). `kind` is the failure class. `detail` is a
/// human-readable elaboration suitable for CLI error output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct EvidenceViolation {
    pub(super) task_id: String,
    pub(super) kind: ViolationKind,
    pub(super) detail: String,
}

/// Kinds of evidence violation. Kept narrow by design — each maps to a concrete
/// CLI message and (eventually) a suggested fix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ViolationKind {
    /// `files` field is missing entirely from the task (wouldn't serde-default to
    /// empty, but defensive — kept for future schema versions that make the field
    /// required at the JSON level).
    MissingFiles,
    /// `files` field is present but empty. The common case — most pre-verifier
    /// workplans have empty files[].
    EmptyFiles,
    /// `files` lists a path that references outside the repo root (absolute,
    /// parent-traversal, etc). Not enforced at author-time (we can't resolve
    /// without a repo_root), but the kind is reserved for future extension.
    NonExistentPath,
    /// Task id is also missing — prevents any downstream reference.
    MissingTaskId,
}

impl std::fmt::Display for EvidenceViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} [{}]: {}", self.task_id, self.kind_str(), self.detail)
    }
}

impl EvidenceViolation {
    fn kind_str(&self) -> &'static str {
        match self.kind {
            ViolationKind::MissingFiles => "missing-files",
            ViolationKind::EmptyFiles => "empty-files",
            ViolationKind::NonExistentPath => "non-existent-path",
            ViolationKind::MissingTaskId => "missing-task-id",
        }
    }
}

/// Validate a parsed workplan. Returns `Ok(())` if every task has at least one
/// file-path assertion; otherwise returns the full list of violations so the
/// caller can display them all at once rather than forcing N fix-then-rerun
/// cycles.
pub(super) fn validate_workplan_evidence(wp: &Workplan) -> Result<(), Vec<EvidenceViolation>> {
    let mut violations = Vec::new();
    for (idx, step) in wp.steps.iter().enumerate() {
        violations.extend(validate_step(step, idx));
    }
    for phase in &wp.phases {
        for (task_idx, task) in phase.tasks.iter().enumerate() {
            violations.extend(validate_phase_task(task, &phase.id, task_idx));
        }
    }
    if violations.is_empty() {
        Ok(())
    } else {
        Err(violations)
    }
}

fn validate_step(step: &Step, idx: usize) -> Vec<EvidenceViolation> {
    let task_id = if step.id.is_empty() {
        format!("step[{}]", idx)
    } else {
        step.id.clone()
    };
    let mut out = Vec::new();
    if step.id.is_empty() {
        out.push(EvidenceViolation {
            task_id: task_id.clone(),
            kind: ViolationKind::MissingTaskId,
            detail: "legacy step has no `id` — reconcile cannot match commits".to_string(),
        });
    }
    if step.files.is_empty() {
        out.push(EvidenceViolation {
            task_id,
            kind: ViolationKind::EmptyFiles,
            detail: format!(
                "step \"{}\" declares no files[]; reconcile will have no evidence to verify",
                truncate(&step.description, 60)
            ),
        });
    }
    out
}

fn validate_phase_task(task: &PhaseTask, phase_id: &str, idx: usize) -> Vec<EvidenceViolation> {
    let task_id = if task.id.is_empty() {
        format!("{}.task[{}]", phase_id, idx)
    } else {
        task.id.clone()
    };
    let mut out = Vec::new();
    if task.id.is_empty() {
        out.push(EvidenceViolation {
            task_id: task_id.clone(),
            kind: ViolationKind::MissingTaskId,
            detail: format!(
                "phase {} task at position {} has no `id`",
                phase_id, idx
            ),
        });
    }
    if task.files.is_empty() {
        out.push(EvidenceViolation {
            task_id,
            kind: ViolationKind::EmptyFiles,
            detail: format!(
                "task \"{}\" declares no files[]; reconcile will have no evidence to verify",
                truncate(&task.name, 60)
            ),
        });
    }
    out
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}

/// Render a violation list for CLI output.
pub(super) fn format_violations(violations: &[EvidenceViolation]) -> String {
    let mut s = format!(
        "workplan schema validation failed: {} violation(s)\n",
        violations.len()
    );
    for v in violations {
        s.push_str(&format!("  - {}\n", v));
    }
    s.push_str("\nhint: every task must declare at least one file path in `files[]`.\n");
    s.push_str("      `files` is what reconcile uses to verify the task completed.\n");
    s.push_str("      prose-only done-conditions trigger false-positive reconcile (ADR-2604142200).\n");
    s.push_str("      to override (not recommended), pass --accept-incomplete-evidence.\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::plan::Phase;

    fn wp_with_legacy_step(files: Vec<&str>) -> Workplan {
        Workplan {
            steps: vec![Step {
                id: "s1".into(),
                description: "do thing".into(),
                files: files.into_iter().map(String::from).collect(),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn wp_with_phase_task(files: Vec<&str>) -> Workplan {
        Workplan {
            phases: vec![Phase {
                id: "P1".into(),
                name: "phase 1".into(),
                tasks: vec![PhaseTask {
                    id: "P1.1".into(),
                    name: "task".into(),
                    files: files.into_iter().map(String::from).collect(),
                    ..Default::default()
                }],
            }],
            ..Default::default()
        }
    }

    #[test]
    fn legacy_step_with_files_passes() {
        let wp = wp_with_legacy_step(vec!["src/foo.rs"]);
        assert!(validate_workplan_evidence(&wp).is_ok());
    }

    #[test]
    fn legacy_step_without_files_fails() {
        let wp = wp_with_legacy_step(vec![]);
        let err = validate_workplan_evidence(&wp).unwrap_err();
        assert_eq!(err.len(), 1);
        assert_eq!(err[0].kind, ViolationKind::EmptyFiles);
        assert_eq!(err[0].task_id, "s1");
    }

    #[test]
    fn phase_task_with_files_passes() {
        let wp = wp_with_phase_task(vec!["src/bar.rs", "tests/bar.rs"]);
        assert!(validate_workplan_evidence(&wp).is_ok());
    }

    #[test]
    fn phase_task_without_files_fails() {
        let wp = wp_with_phase_task(vec![]);
        let err = validate_workplan_evidence(&wp).unwrap_err();
        assert_eq!(err.len(), 1);
        assert_eq!(err[0].kind, ViolationKind::EmptyFiles);
        assert_eq!(err[0].task_id, "P1.1");
    }

    #[test]
    fn missing_id_reported_separately() {
        let wp = Workplan {
            phases: vec![Phase {
                id: "P1".into(),
                name: "phase".into(),
                tasks: vec![PhaseTask {
                    id: "".into(),
                    name: "anon".into(),
                    files: vec![],
                    ..Default::default()
                }],
            }],
            ..Default::default()
        };
        let err = validate_workplan_evidence(&wp).unwrap_err();
        assert_eq!(err.len(), 2);
        assert!(err.iter().any(|v| v.kind == ViolationKind::MissingTaskId));
        assert!(err.iter().any(|v| v.kind == ViolationKind::EmptyFiles));
    }

    #[test]
    fn all_violations_collected_not_short_circuit() {
        let wp = Workplan {
            phases: vec![Phase {
                id: "P1".into(),
                name: "phase".into(),
                tasks: vec![
                    PhaseTask { id: "P1.1".into(), files: vec![], ..Default::default() },
                    PhaseTask { id: "P1.2".into(), files: vec![], ..Default::default() },
                    PhaseTask { id: "P1.3".into(), files: vec!["src/ok.rs".into()], ..Default::default() },
                ],
            }],
            ..Default::default()
        };
        let err = validate_workplan_evidence(&wp).unwrap_err();
        assert_eq!(err.len(), 2);
    }

    #[test]
    fn empty_workplan_passes() {
        let wp = Workplan::default();
        assert!(validate_workplan_evidence(&wp).is_ok());
    }
}
