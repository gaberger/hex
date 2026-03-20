//! hexflo-cleanup — SpacetimeDB scheduled procedures for agent lifecycle management.
//!
//! Replaces the tokio-based cleanup loop in hex-nexus with server-side scheduled
//! reducers that run inside SpacetimeDB. This means cleanup happens even when
//! hex-nexus is offline, and works across multiple hex-nexus instances.
//!
//! ADR-039 Phase 7: SpacetimeDB 2.0 Procedures
//!
//! Tables referenced (from hexflo-coordination + agent-registry modules):
//!   - swarm_agent: agent tracking with last_heartbeat timestamp
//!   - swarm_task: tasks with assigned agent and status
//!
//! Since SpacetimeDB modules are isolated, this module defines its own copies
//! of the relevant tables. In production, these would be in the same module
//! or use cross-module references when SpacetimeDB supports them.

use spacetimedb::{table, reducer, schedule, ReducerContext, Table, Timestamp};

// ── Tables ──────────────────────────────────────────────

/// Agent record with heartbeat tracking.
#[table(name = agent_health, public)]
pub struct AgentHealth {
    #[primary_key]
    pub agent_id: String,
    pub swarm_id: String,
    pub agent_name: String,
    pub status: String, // "active", "stale", "dead"
    pub last_heartbeat: Timestamp,
    pub registered_at: Timestamp,
}

/// Task record for reclamation.
#[table(name = reclaimable_task, public)]
pub struct ReclaimableTask {
    #[primary_key]
    pub task_id: String,
    pub swarm_id: String,
    pub title: String,
    pub status: String, // "pending", "in_progress", "completed", "failed"
    pub assigned_to: String, // agent_id
    pub updated_at: Timestamp,
}

/// Cleanup run log — tracks when cleanup last ran and what it did.
#[table(name = cleanup_log, public)]
pub struct CleanupLog {
    #[primary_key]
    #[auto_inc]
    pub id: u64,
    pub ran_at: Timestamp,
    pub stale_count: u32,
    pub dead_count: u32,
    pub reclaimed_tasks: u32,
}

/// Scheduler anchor — holds the scheduled reducer reference.
#[table(name = cleanup_schedule, public)]
pub struct CleanupSchedule {
    #[primary_key]
    pub id: u64,
    pub scheduled_id: schedule::ScheduleAt,
    pub interval_secs: u64,
}

// ── Constants ───────────────────────────────────────────

/// Agent is "stale" after 45 seconds without heartbeat.
const STALE_THRESHOLD_MICROS: u64 = 45_000_000;
/// Agent is "dead" after 120 seconds without heartbeat.
const DEAD_THRESHOLD_MICROS: u64 = 120_000_000;

// ── Heartbeat reducer (called by agents) ────────────────

/// Agents call this to report they're alive.
#[reducer]
pub fn agent_heartbeat(ctx: &ReducerContext, agent_id: String) {
    let now = ctx.timestamp;
    if let Some(mut agent) = ctx.db.agent_health().agent_id().find(&agent_id) {
        agent.last_heartbeat = now;
        agent.status = "active".to_string();
        ctx.db.agent_health().agent_id().update(agent);
    }
}

/// Register a new agent for health tracking.
#[reducer]
pub fn register_agent_health(
    ctx: &ReducerContext,
    agent_id: String,
    swarm_id: String,
    agent_name: String,
) {
    let now = ctx.timestamp;
    ctx.db.agent_health().insert(AgentHealth {
        agent_id,
        swarm_id,
        agent_name,
        status: "active".to_string(),
        last_heartbeat: now,
        registered_at: now,
    });
}

// ── Scheduled cleanup reducer ───────────────────────────

/// Initialize the cleanup schedule. Call once on module publish.
#[reducer(init)]
pub fn init(ctx: &ReducerContext) {
    // Schedule cleanup to run every 30 seconds
    let schedule_at = schedule::ScheduleAt::Interval(std::time::Duration::from_secs(30).into());
    ctx.db.cleanup_schedule().insert(CleanupSchedule {
        id: 1,
        scheduled_id: schedule_at,
        interval_secs: 30,
    });
    log::info!("hexflo-cleanup: scheduled every 30s");
}

/// Scheduled reducer: mark stale/dead agents and reclaim their tasks.
///
/// Runs every 30 seconds inside SpacetimeDB — no hex-nexus needed.
///
/// Process:
/// 1. Scan all agents with status "active"
/// 2. If last_heartbeat > 45s ago → mark "stale"
/// 3. If last_heartbeat > 120s ago → mark "dead"
/// 4. For all "dead" agents → reset their in_progress tasks to "pending"
#[reducer]
pub fn run_cleanup(ctx: &ReducerContext) {
    let now = ctx.timestamp;
    let mut stale_count: u32 = 0;
    let mut dead_count: u32 = 0;
    let mut reclaimed_tasks: u32 = 0;

    // Collect agents that need status changes
    let agents: Vec<AgentHealth> = ctx.db.agent_health().iter().collect();

    for agent in agents {
        let elapsed = now
            .duration_since(agent.last_heartbeat)
            .map(|d| d.as_micros() as u64)
            .unwrap_or(0);

        if elapsed >= DEAD_THRESHOLD_MICROS && agent.status != "dead" {
            // Mark dead
            let mut updated = agent.clone();
            updated.status = "dead".to_string();
            ctx.db.agent_health().agent_id().update(updated);
            dead_count += 1;

            // Reclaim tasks assigned to this dead agent
            let tasks: Vec<ReclaimableTask> = ctx
                .db
                .reclaimable_task()
                .iter()
                .filter(|t| t.assigned_to == agent.agent_id && t.status == "in_progress")
                .collect();

            for task in tasks {
                let mut reclaimed = task;
                reclaimed.status = "pending".to_string();
                reclaimed.assigned_to = String::new();
                reclaimed.updated_at = now;
                ctx.db.reclaimable_task().task_id().update(reclaimed);
                reclaimed_tasks += 1;
            }
        } else if elapsed >= STALE_THRESHOLD_MICROS && agent.status == "active" {
            // Mark stale
            let mut updated = agent.clone();
            updated.status = "stale".to_string();
            ctx.db.agent_health().agent_id().update(updated);
            stale_count += 1;
        }
    }

    // Log the cleanup run
    if stale_count > 0 || dead_count > 0 || reclaimed_tasks > 0 {
        ctx.db.cleanup_log().insert(CleanupLog {
            id: 0, // auto_inc
            ran_at: now,
            stale_count,
            dead_count,
            reclaimed_tasks,
        });
        log::info!(
            "hexflo-cleanup: stale={}, dead={}, reclaimed={}",
            stale_count,
            dead_count,
            reclaimed_tasks
        );
    }
}

// ── Manual cleanup trigger ──────────────────────────────

/// Manual trigger for cleanup (e.g., from hex-nexus REST API).
#[reducer]
pub fn trigger_cleanup(ctx: &ReducerContext) {
    run_cleanup(ctx);
}

/// Remove a dead agent from tracking entirely.
#[reducer]
pub fn remove_dead_agent(ctx: &ReducerContext, agent_id: String) {
    if let Some(agent) = ctx.db.agent_health().agent_id().find(&agent_id) {
        if agent.status == "dead" {
            ctx.db.agent_health().agent_id().delete(&agent_id);
        }
    }
}
