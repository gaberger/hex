use spacetimedb::{table, reducer, ReducerContext, Table};

#[table(name = workplan_execution, public)]
#[derive(Clone, Debug)]
pub struct WorkplanExecution {
    #[unique]
    pub id: String,
    pub workplan_path: String,
    pub status: String,
    pub current_phase: String,
    pub started_at: String,
    pub updated_at: String,
    /// When status is "superseded", the path of the absorbing workplan.
    pub superseded_by: String,
    /// Human-readable reason for supersession.
    pub supersession_reason: String,
}

#[table(name = workplan_task, public)]
#[derive(Clone, Debug)]
pub struct WorkplanTask {
    #[unique]
    pub id: String,
    pub workplan_id: String,
    pub name: String,
    pub layer: String,
    pub status: String,
    pub agent_id: String,
    pub result: String,
}

#[reducer]
pub fn start_workplan(
    ctx: &ReducerContext,
    id: String,
    path: String,
) -> Result<(), String> {
    ctx.db.workplan_execution().insert(WorkplanExecution {
        id,
        workplan_path: path,
        status: "running".to_string(),
        current_phase: "init".to_string(),
        started_at: String::new(),
        updated_at: String::new(),
        superseded_by: String::new(),
        supersession_reason: String::new(),
    });
    Ok(())
}

#[reducer]
pub fn supersede_workplan(
    ctx: &ReducerContext,
    workplan_id: String,
    superseded_by: String,
    reason: String,
) -> Result<(), String> {
    let existing = ctx.db.workplan_execution().id().find(&workplan_id);
    match existing {
        Some(old) => {
            let superseded_by_ref = superseded_by.clone();
            let updated = WorkplanExecution {
                status: "superseded".to_string(),
                superseded_by,
                supersession_reason: reason,
                ..old
            };
            ctx.db.workplan_execution().id().update(updated);
            // Mark all pending tasks as completed-by-supersession
            let tasks: Vec<_> = ctx.db.workplan_task().iter()
                .filter(|t| t.workplan_id == workplan_id && t.status == "pending")
                .collect();
            for task in tasks {
                let updated_task = WorkplanTask {
                    status: "completed".to_string(),
                    result: format!("Superseded by {}", superseded_by_ref),
                    ..task
                };
                ctx.db.workplan_task().id().update(updated_task);
            }
        }
        None => {
            return Err(format!("Workplan '{}' not found", workplan_id));
        }
    }
    Ok(())
}

#[reducer]
pub fn update_task(
    ctx: &ReducerContext,
    task_id: String,
    status: String,
    agent_id: String,
    result: String,
) -> Result<(), String> {
    let existing = ctx.db.workplan_task().id().find(&task_id);
    match existing {
        Some(old) => {
            let updated = WorkplanTask {
                status,
                agent_id,
                result,
                ..old
            };
            ctx.db.workplan_task().id().update(updated);
        }
        None => {
            return Err(format!("Task '{}' not found", task_id));
        }
    }
    Ok(())
}

#[reducer]
pub fn advance_phase(
    ctx: &ReducerContext,
    workplan_id: String,
    phase: String,
) -> Result<(), String> {
    let existing = ctx.db.workplan_execution().id().find(&workplan_id);
    match existing {
        Some(old) => {
            let updated = WorkplanExecution {
                current_phase: phase,
                updated_at: String::new(),
                ..old
            };
            ctx.db.workplan_execution().id().update(updated);
        }
        None => {
            return Err(format!("Workplan '{}' not found", workplan_id));
        }
    }
    Ok(())
}

/// Valid workplan execution statuses.
pub const VALID_EXECUTION_STATUSES: &[&str] = &["running", "completed", "failed", "superseded"];

/// Valid workplan task statuses.
pub const VALID_TASK_STATUSES: &[&str] = &["pending", "in_progress", "completed", "failed"];

/// Valid workplan phases (ordered pipeline).
pub const VALID_PHASES: &[&str] = &["init", "specs", "plan", "code", "validate", "integrate", "finalize"];

/// Check if an execution status is valid.
pub fn is_valid_execution_status(status: &str) -> bool {
    VALID_EXECUTION_STATUSES.contains(&status)
}

/// Check if a task status is valid.
pub fn is_valid_task_status(status: &str) -> bool {
    VALID_TASK_STATUSES.contains(&status)
}

/// Check if a phase name is valid.
pub fn is_valid_phase(phase: &str) -> bool {
    VALID_PHASES.contains(&phase)
}

/// Return the index of a phase in the pipeline, or None if invalid.
pub fn phase_index(phase: &str) -> Option<usize> {
    VALID_PHASES.iter().position(|p| *p == phase)
}

/// Check whether advancing from one phase to another moves forward.
pub fn is_phase_advance(from: &str, to: &str) -> bool {
    match (phase_index(from), phase_index(to)) {
        (Some(a), Some(b)) => b > a,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Execution status validation ─────────────────────────────────────

    #[test]
    fn valid_execution_statuses_accepted() {
        for s in VALID_EXECUTION_STATUSES {
            assert!(is_valid_execution_status(s), "expected '{}' valid", s);
        }
    }

    #[test]
    fn unknown_execution_status_rejected() {
        assert!(!is_valid_execution_status("paused"));
        assert!(!is_valid_execution_status(""));
        assert!(!is_valid_execution_status("RUNNING"));
    }

    #[test]
    fn initial_execution_status_is_running() {
        // Mirrors start_workplan reducer
        assert!(is_valid_execution_status("running"));
    }

    #[test]
    fn superseded_is_valid_terminal_status() {
        assert!(is_valid_execution_status("superseded"));
    }

    // ── Task status validation ──────────────────────────────────────────

    #[test]
    fn valid_task_statuses_accepted() {
        for s in VALID_TASK_STATUSES {
            assert!(is_valid_task_status(s), "expected '{}' valid", s);
        }
    }

    #[test]
    fn unknown_task_status_rejected() {
        assert!(!is_valid_task_status("blocked"));
        assert!(!is_valid_task_status("cancelled"));
    }

    // ── Phase validation ────────────────────────────────────────────────

    #[test]
    fn valid_phases_accepted() {
        for p in VALID_PHASES {
            assert!(is_valid_phase(p), "expected '{}' valid", p);
        }
    }

    #[test]
    fn unknown_phase_rejected() {
        assert!(!is_valid_phase("deploy"));
        assert!(!is_valid_phase(""));
    }

    #[test]
    fn phase_index_returns_correct_order() {
        assert_eq!(phase_index("init"), Some(0));
        assert_eq!(phase_index("specs"), Some(1));
        assert_eq!(phase_index("finalize"), Some(6));
        assert_eq!(phase_index("bogus"), None);
    }

    #[test]
    fn phase_advance_forward() {
        assert!(is_phase_advance("init", "specs"));
        assert!(is_phase_advance("code", "validate"));
        assert!(is_phase_advance("init", "finalize"));
    }

    #[test]
    fn phase_advance_backward_rejected() {
        assert!(!is_phase_advance("validate", "code"));
        assert!(!is_phase_advance("finalize", "init"));
    }

    #[test]
    fn phase_advance_same_rejected() {
        assert!(!is_phase_advance("code", "code"));
    }

    #[test]
    fn phase_advance_invalid_phase_rejected() {
        assert!(!is_phase_advance("init", "bogus"));
        assert!(!is_phase_advance("bogus", "init"));
    }

    // ── Struct construction ─────────────────────────────────────────────

    #[test]
    fn workplan_execution_defaults() {
        let wp = WorkplanExecution {
            id: "wp-1".to_string(),
            workplan_path: "/plans/feat-x.json".to_string(),
            status: "running".to_string(),
            current_phase: "init".to_string(),
            started_at: String::new(),
            updated_at: String::new(),
            superseded_by: String::new(),
            supersession_reason: String::new(),
        };
        assert!(is_valid_execution_status(&wp.status));
        assert!(is_valid_phase(&wp.current_phase));
        assert!(wp.superseded_by.is_empty());
    }

    #[test]
    fn workplan_task_defaults() {
        let task = WorkplanTask {
            id: "t-1".to_string(),
            workplan_id: "wp-1".to_string(),
            name: "Implement port".to_string(),
            layer: "ports".to_string(),
            status: "pending".to_string(),
            agent_id: String::new(),
            result: String::new(),
        };
        assert!(is_valid_task_status(&task.status));
        assert!(task.agent_id.is_empty());
    }

    #[test]
    fn supersession_sets_correct_fields() {
        let original = WorkplanExecution {
            id: "wp-1".to_string(),
            workplan_path: "/old.json".to_string(),
            status: "running".to_string(),
            current_phase: "code".to_string(),
            started_at: "t0".to_string(),
            updated_at: "t0".to_string(),
            superseded_by: String::new(),
            supersession_reason: String::new(),
        };
        let superseded = WorkplanExecution {
            status: "superseded".to_string(),
            superseded_by: "/new.json".to_string(),
            supersession_reason: "Scope changed".to_string(),
            ..original.clone()
        };
        assert_eq!(superseded.id, "wp-1");
        assert_eq!(superseded.status, "superseded");
        assert!(!superseded.superseded_by.is_empty());
    }

    #[test]
    fn task_status_update_preserves_identity() {
        let task = WorkplanTask {
            id: "t-1".to_string(),
            workplan_id: "wp-1".to_string(),
            name: "Build adapter".to_string(),
            layer: "adapters".to_string(),
            status: "pending".to_string(),
            agent_id: String::new(),
            result: String::new(),
        };
        let updated = WorkplanTask {
            status: "in_progress".to_string(),
            agent_id: "agent-42".to_string(),
            ..task.clone()
        };
        assert_eq!(updated.id, "t-1");
        assert_eq!(updated.name, "Build adapter");
        assert_eq!(updated.status, "in_progress");
    }
}
