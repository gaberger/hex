//! hexflo-lifecycle — Triggered procedures for swarm phase transitions.
//!
//! ADR-039 Phase 8: Automatic swarm lifecycle management.
//!
//! When a task completes, this module checks if the entire tier is done.
//! If so, it advances the swarm to the next phase and unblocks dependent tasks.
//!
//! Phase progression: SPECS → PLAN → CODE → VALIDATE → INTEGRATE → COMPLETE
//!
//! This runs inside SpacetimeDB as a reducer — no hex-nexus polling needed.
//! The browser sees phase changes instantly via SpacetimeDB subscription.

use spacetimedb::{table, reducer, ReducerContext, Table, Timestamp};

// ── Tables ──────────────────────────────────────────────

#[table(name = swarm_lifecycle, public)]
pub struct SwarmLifecycle {
    #[primary_key]
    pub swarm_id: String,
    pub name: String,
    pub phase: String,       // "specs", "plan", "code", "validate", "integrate", "complete"
    pub phase_index: u32,    // 0-5
    pub total_tasks: u32,
    pub completed_tasks: u32,
    pub status: String,      // "active", "completed", "failed"
    pub updated_at: Timestamp,
}

#[table(name = lifecycle_task, public)]
pub struct LifecycleTask {
    #[primary_key]
    pub task_id: String,
    pub swarm_id: String,
    pub tier: u32,           // 0-5, maps to phase
    pub status: String,      // "pending", "in_progress", "completed", "failed"
    pub depends_on: String,  // comma-separated task IDs
    pub updated_at: Timestamp,
}

/// Event log for phase transitions.
#[table(name = phase_transition_log, public)]
pub struct PhaseTransitionLog {
    #[primary_key]
    #[auto_inc]
    pub id: u64,
    pub swarm_id: String,
    pub from_phase: String,
    pub to_phase: String,
    pub triggered_by_task: String,
    pub transitioned_at: Timestamp,
}

// ── Constants ───────────────────────────────────────────

const PHASES: &[&str] = &["specs", "plan", "code", "validate", "integrate", "complete"];

// ── Reducers ────────────────────────────────────────────

/// Register a swarm for lifecycle tracking.
#[reducer]
pub fn register_swarm_lifecycle(
    ctx: &ReducerContext,
    swarm_id: String,
    name: String,
    total_tasks: u32,
) {
    ctx.db.swarm_lifecycle().insert(SwarmLifecycle {
        swarm_id,
        name,
        phase: "specs".to_string(),
        phase_index: 0,
        total_tasks,
        completed_tasks: 0,
        status: "active".to_string(),
        updated_at: ctx.timestamp,
    });
}

/// Register a task for lifecycle tracking.
#[reducer]
pub fn register_lifecycle_task(
    ctx: &ReducerContext,
    task_id: String,
    swarm_id: String,
    tier: u32,
    depends_on: String,
) {
    ctx.db.lifecycle_task().insert(LifecycleTask {
        task_id,
        swarm_id,
        tier,
        status: "pending".to_string(),
        depends_on,
        updated_at: ctx.timestamp,
    });
}

/// Called when a task completes. Triggers phase transition check.
///
/// This is the core trigger: when the last task in a tier completes,
/// the swarm advances to the next phase, and tasks in the next tier
/// become unblocked.
#[reducer]
pub fn on_task_complete(ctx: &ReducerContext, task_id: String, swarm_id: String) {
    let now = ctx.timestamp;

    // Update the task status
    if let Some(mut task) = ctx.db.lifecycle_task().task_id().find(&task_id) {
        task.status = "completed".to_string();
        task.updated_at = now;
        ctx.db.lifecycle_task().task_id().update(task);
    }

    // Get the swarm
    let Some(mut swarm) = ctx.db.swarm_lifecycle().swarm_id().find(&swarm_id) else {
        return;
    };

    // Count completed tasks
    let all_tasks: Vec<LifecycleTask> = ctx
        .db
        .lifecycle_task()
        .iter()
        .filter(|t| t.swarm_id == swarm_id)
        .collect();

    let completed = all_tasks.iter().filter(|t| t.status == "completed").count() as u32;
    swarm.completed_tasks = completed;

    // Check if current phase tier is fully complete
    let current_tier = swarm.phase_index;
    let tier_tasks: Vec<&LifecycleTask> = all_tasks
        .iter()
        .filter(|t| t.tier == current_tier)
        .collect();

    let tier_done = !tier_tasks.is_empty()
        && tier_tasks.iter().all(|t| t.status == "completed");

    if tier_done && (current_tier as usize) < PHASES.len() - 1 {
        // Advance to next phase
        let old_phase = swarm.phase.clone();
        let new_index = current_tier + 1;
        let new_phase = PHASES[new_index as usize].to_string();

        swarm.phase = new_phase.clone();
        swarm.phase_index = new_index;
        swarm.updated_at = now;

        // Log the transition
        ctx.db.phase_transition_log().insert(PhaseTransitionLog {
            id: 0, // auto_inc
            swarm_id: swarm_id.clone(),
            from_phase: old_phase,
            to_phase: new_phase,
            triggered_by_task: task_id,
            transitioned_at: now,
        });

        log::info!(
            "Phase transition: swarm {} → phase {} (tier {})",
            swarm_id,
            swarm.phase,
            new_index
        );
    }

    // Check if swarm is fully complete
    if completed == swarm.total_tasks && swarm.total_tasks > 0 {
        swarm.phase = "complete".to_string();
        swarm.phase_index = 5;
        swarm.status = "completed".to_string();
    }

    swarm.updated_at = now;
    ctx.db.swarm_lifecycle().swarm_id().update(swarm);
}

/// Called when a task fails. Marks swarm as failed if critical.
#[reducer]
pub fn on_task_fail(ctx: &ReducerContext, task_id: String, swarm_id: String) {
    let now = ctx.timestamp;

    if let Some(mut task) = ctx.db.lifecycle_task().task_id().find(&task_id) {
        task.status = "failed".to_string();
        task.updated_at = now;
        ctx.db.lifecycle_task().task_id().update(task);
    }

    // Mark swarm as failed
    if let Some(mut swarm) = ctx.db.swarm_lifecycle().swarm_id().find(&swarm_id) {
        swarm.status = "failed".to_string();
        swarm.updated_at = now;
        ctx.db.swarm_lifecycle().swarm_id().update(swarm);
    }
}

/// Check which tasks in the next tier are now unblocked.
/// Returns task IDs that have all dependencies satisfied.
#[reducer]
pub fn check_unblocked_tasks(ctx: &ReducerContext, swarm_id: String) {
    let all_tasks: Vec<LifecycleTask> = ctx
        .db
        .lifecycle_task()
        .iter()
        .filter(|t| t.swarm_id == swarm_id)
        .collect();

    let completed_ids: std::collections::HashSet<&str> = all_tasks
        .iter()
        .filter(|t| t.status == "completed")
        .map(|t| t.task_id.as_str())
        .collect();

    for task in &all_tasks {
        if task.status != "pending" {
            continue;
        }

        // Check if all dependencies are met
        let deps_met = if task.depends_on.is_empty() {
            true
        } else {
            task.depends_on
                .split(',')
                .all(|dep| completed_ids.contains(dep.trim()))
        };

        if deps_met {
            log::info!("Task {} unblocked in swarm {}", task.task_id, swarm_id);
            // Task is ready — agents can pick it up via SpacetimeDB subscription
        }
    }
}
