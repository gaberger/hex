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
    });
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
