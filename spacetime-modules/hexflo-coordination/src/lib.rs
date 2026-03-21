use spacetimedb::{table, reducer, ReducerContext, Table};

// ============================================================
//  Tables
// ============================================================

/// A swarm — a named group of agents working on a coordinated task.
#[table(name = swarm, public)]
#[derive(Clone, Debug)]
pub struct Swarm {
    #[unique]
    pub id: String,
    pub project_id: String,
    pub name: String,
    /// Topology: "hierarchical", "mesh", "pipeline", "star"
    pub topology: String,
    /// Status: "active", "completed", "failed"
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

/// A task within a swarm — the unit of work assigned to agents.
#[table(name = swarm_task, public)]
#[derive(Clone, Debug)]
pub struct SwarmTask {
    #[unique]
    pub id: String,
    pub swarm_id: String,
    pub title: String,
    /// Status: "pending", "in_progress", "completed", "failed"
    pub status: String,
    /// Agent currently assigned to this task (empty if unassigned).
    pub agent_id: String,
    /// Result summary (populated on completion).
    pub result: String,
    pub created_at: String,
    pub completed_at: String,
}

/// An agent participating in a swarm.
#[table(name = swarm_agent, public)]
#[derive(Clone, Debug)]
pub struct SwarmAgent {
    #[unique]
    pub id: String,
    pub swarm_id: String,
    pub name: String,
    pub role: String,
    /// Status: "active", "stale", "dead", "disconnected"
    pub status: String,
    pub worktree_path: String,
    pub last_heartbeat: String,
}

/// Key-value memory store scoped to global, swarm, or agent level.
#[table(name = hexflo_memory, public)]
#[derive(Clone, Debug)]
pub struct HexFloMemory {
    #[unique]
    pub key: String,
    pub value: String,
    /// Scope: "global", "swarm:<id>", "agent:<id>"
    pub scope: String,
    pub updated_at: String,
}

// ─── Project Registry ──────────────────────────────────────────────────────

#[table(name = project, public)]
#[derive(Clone, Debug)]
pub struct Project {
    #[primary_key]
    pub project_id: String,
    pub name: String,
    pub path: String,
    pub registered_at: String,
}

/// Register or update a project.
#[reducer]
pub fn register_project(
    ctx: &ReducerContext,
    project_id: String,
    name: String,
    path: String,
    registered_at: String,
) -> Result<(), String> {
    if path.is_empty() {
        return Err("Project path is required".to_string());
    }
    if let Some(existing) = ctx.db.project().project_id().find(&project_id) {
        ctx.db.project().project_id().update(Project {
            name,
            path,
            registered_at,
            ..existing
        });
    } else {
        ctx.db.project().insert(Project {
            project_id,
            name,
            path,
            registered_at,
        });
    }
    Ok(())
}

/// Remove a project by ID.
#[reducer]
pub fn remove_project(
    ctx: &ReducerContext,
    project_id: String,
) -> Result<(), String> {
    if ctx.db.project().project_id().find(&project_id).is_some() {
        ctx.db.project().project_id().delete(&project_id);
        Ok(())
    } else {
        Err(format!("Project '{}' not found", project_id))
    }
}

// ─── Project Configuration ──────────────────────────────────────────────────

#[table(name = project_config, public)]
#[derive(Clone, Debug)]
pub struct ProjectConfig {
    #[primary_key]
    pub key: String,
    pub project_id: String,
    pub value_json: String,
    pub source_file: String,
    pub synced_at: String,
}

#[reducer]
pub fn sync_config(
    ctx: &ReducerContext,
    key: String,
    project_id: String,
    value_json: String,
    source_file: String,
    synced_at: String,
) -> Result<(), String> {
    if let Some(existing) = ctx.db.project_config().key().find(&key) {
        ctx.db.project_config().key().update(ProjectConfig {
            value_json, source_file, synced_at, ..existing
        });
    } else {
        ctx.db.project_config().insert(ProjectConfig {
            key, project_id, value_json, source_file, synced_at,
        });
    }
    Ok(())
}

// ─── Skill Registry ──────────────────────────────────────────────────────

#[table(name = skill_registry, public)]
#[derive(Clone, Debug)]
pub struct SkillEntry {
    #[primary_key]
    pub skill_id: String,
    pub project_id: String,
    pub name: String,
    pub trigger_cmd: String,
    pub description: String,
    pub source_path: String,
    pub synced_at: String,
}

#[reducer]
pub fn sync_skill(
    ctx: &ReducerContext,
    skill_id: String,
    project_id: String,
    name: String,
    trigger_cmd: String,
    description: String,
    source_path: String,
    synced_at: String,
) -> Result<(), String> {
    if let Some(existing) = ctx.db.skill_registry().skill_id().find(&skill_id) {
        ctx.db.skill_registry().skill_id().update(SkillEntry {
            name, trigger_cmd, description, source_path, synced_at, ..existing
        });
    } else {
        ctx.db.skill_registry().insert(SkillEntry {
            skill_id, project_id, name, trigger_cmd, description, source_path, synced_at,
        });
    }
    Ok(())
}

// ─── Agent Definition ──────────────────────────────────────────────────────

#[table(name = agent_definition, public)]
#[derive(Clone, Debug)]
pub struct AgentDef {
    #[primary_key]
    pub agent_def_id: String,
    pub project_id: String,
    pub name: String,
    pub role: String,
    pub model: String,
    pub capabilities_json: String,
    pub tools_json: String,
    pub source_path: String,
    pub synced_at: String,
}

#[reducer]
pub fn sync_agent_def(
    ctx: &ReducerContext,
    agent_def_id: String,
    project_id: String,
    name: String,
    role: String,
    model: String,
    capabilities_json: String,
    tools_json: String,
    source_path: String,
    synced_at: String,
) -> Result<(), String> {
    if let Some(existing) = ctx.db.agent_definition().agent_def_id().find(&agent_def_id) {
        ctx.db.agent_definition().agent_def_id().update(AgentDef {
            name, role, model, capabilities_json, tools_json, source_path, synced_at, ..existing
        });
    } else {
        ctx.db.agent_definition().insert(AgentDef {
            agent_def_id, project_id, name, role, model, capabilities_json, tools_json, source_path, synced_at,
        });
    }
    Ok(())
}

// ============================================================
//  Swarm Lifecycle Reducers
// ============================================================

/// Initialize a new swarm.
#[reducer]
pub fn swarm_init(
    ctx: &ReducerContext,
    id: String,
    name: String,
    topology: String,
    project_id: String,
    timestamp: String,
) -> Result<(), String> {
    // Prevent duplicate swarm IDs
    if ctx.db.swarm().id().find(&id).is_some() {
        return Err(format!("Swarm '{}' already exists", id));
    }

    ctx.db.swarm().insert(Swarm {
        id,
        project_id,
        name,
        topology,
        status: "active".to_string(),
        created_at: timestamp.clone(),
        updated_at: timestamp,
    });

    Ok(())
}

/// Mark a swarm as completed.
#[reducer]
pub fn swarm_complete(
    ctx: &ReducerContext,
    id: String,
    timestamp: String,
) -> Result<(), String> {
    let existing = ctx.db.swarm().id().find(&id)
        .ok_or_else(|| format!("Swarm '{}' not found", id))?;

    ctx.db.swarm().id().update(Swarm {
        status: "completed".to_string(),
        updated_at: timestamp,
        ..existing
    });

    Ok(())
}

/// Mark a swarm as failed.
#[reducer]
pub fn swarm_fail(
    ctx: &ReducerContext,
    id: String,
    reason: String,
    timestamp: String,
) -> Result<(), String> {
    let existing = ctx.db.swarm().id().find(&id)
        .ok_or_else(|| format!("Swarm '{}' not found", id))?;

    ctx.db.swarm().id().update(Swarm {
        status: "failed".to_string(),
        updated_at: timestamp,
        ..existing
    });

    // Store failure reason in memory
    ctx.db.hexflo_memory().insert(HexFloMemory {
        key: format!("swarm:{}:failure_reason", id),
        value: reason,
        scope: format!("swarm:{}", id),
        updated_at: existing.updated_at,
    });

    Ok(())
}

// ============================================================
//  Task Management Reducers
// ============================================================

/// Create a new task in a swarm.
#[reducer]
pub fn task_create(
    ctx: &ReducerContext,
    id: String,
    swarm_id: String,
    title: String,
    timestamp: String,
) -> Result<(), String> {
    // Verify swarm exists and is active
    let swarm = ctx.db.swarm().id().find(&swarm_id)
        .ok_or_else(|| format!("Swarm '{}' not found", swarm_id))?;

    if swarm.status != "active" {
        return Err(format!("Swarm '{}' is not active (status: {})", swarm_id, swarm.status));
    }

    ctx.db.swarm_task().insert(SwarmTask {
        id,
        swarm_id,
        title,
        status: "pending".to_string(),
        agent_id: String::new(),
        result: String::new(),
        created_at: timestamp,
        completed_at: String::new(),
    });

    Ok(())
}

/// Assign a task to an agent.
#[reducer]
pub fn task_assign(
    ctx: &ReducerContext,
    task_id: String,
    agent_id: String,
    timestamp: String,
) -> Result<(), String> {
    let task = ctx.db.swarm_task().id().find(&task_id)
        .ok_or_else(|| format!("Task '{}' not found", task_id))?;

    if task.status != "pending" {
        return Err(format!("Task '{}' is not pending (status: {})", task_id, task.status));
    }

    ctx.db.swarm_task().id().update(SwarmTask {
        status: "in_progress".to_string(),
        agent_id,
        completed_at: timestamp,
        ..task
    });

    Ok(())
}

/// Mark a task as completed with a result.
#[reducer]
pub fn task_complete(
    ctx: &ReducerContext,
    task_id: String,
    result: String,
    timestamp: String,
) -> Result<(), String> {
    let task = ctx.db.swarm_task().id().find(&task_id)
        .ok_or_else(|| format!("Task '{}' not found", task_id))?;

    ctx.db.swarm_task().id().update(SwarmTask {
        status: "completed".to_string(),
        result,
        completed_at: timestamp,
        ..task
    });

    Ok(())
}

/// Mark a task as failed.
#[reducer]
pub fn task_fail(
    ctx: &ReducerContext,
    task_id: String,
    reason: String,
    timestamp: String,
) -> Result<(), String> {
    let task = ctx.db.swarm_task().id().find(&task_id)
        .ok_or_else(|| format!("Task '{}' not found", task_id))?;

    ctx.db.swarm_task().id().update(SwarmTask {
        status: "failed".to_string(),
        result: reason,
        completed_at: timestamp,
        ..task
    });

    Ok(())
}

/// Reclaim all tasks assigned to a dead agent back to pending.
#[reducer]
pub fn task_reclaim(
    ctx: &ReducerContext,
    agent_id: String,
) -> Result<(), String> {
    let tasks: Vec<SwarmTask> = ctx.db.swarm_task().iter()
        .filter(|t| t.agent_id == agent_id && t.status == "in_progress")
        .collect();

    for task in tasks {
        ctx.db.swarm_task().id().update(SwarmTask {
            status: "pending".to_string(),
            agent_id: String::new(),
            ..task
        });
    }

    Ok(())
}

// ============================================================
//  Agent Lifecycle Reducers
// ============================================================

/// Register an agent in a swarm.
#[reducer]
pub fn agent_register(
    ctx: &ReducerContext,
    id: String,
    swarm_id: String,
    name: String,
    role: String,
    worktree_path: String,
    timestamp: String,
) -> Result<(), String> {
    // Verify swarm exists
    if ctx.db.swarm().id().find(&swarm_id).is_none() {
        return Err(format!("Swarm '{}' not found", swarm_id));
    }

    ctx.db.swarm_agent().insert(SwarmAgent {
        id,
        swarm_id,
        name,
        role,
        status: "active".to_string(),
        worktree_path,
        last_heartbeat: timestamp,
    });

    Ok(())
}

/// Update an agent's heartbeat timestamp.
#[reducer]
pub fn agent_heartbeat(
    ctx: &ReducerContext,
    id: String,
    timestamp: String,
) -> Result<(), String> {
    let agent = ctx.db.swarm_agent().id().find(&id)
        .ok_or_else(|| format!("Agent '{}' not found", id))?;

    ctx.db.swarm_agent().id().update(SwarmAgent {
        last_heartbeat: timestamp,
        status: "active".to_string(),
        ..agent
    });

    Ok(())
}

/// Mark agents as stale if their heartbeat is older than the threshold.
/// Called periodically by the nexus cleanup task.
///
/// `threshold_timestamp` is the cutoff — any agent with last_heartbeat
/// before this value is marked stale.
#[reducer]
pub fn agent_mark_stale(
    ctx: &ReducerContext,
    threshold_timestamp: String,
) -> Result<(), String> {
    let stale: Vec<SwarmAgent> = ctx.db.swarm_agent().iter()
        .filter(|a| a.status == "active" && a.last_heartbeat < threshold_timestamp)
        .collect();

    for agent in stale {
        ctx.db.swarm_agent().id().update(SwarmAgent {
            status: "stale".to_string(),
            ..agent
        });
    }

    Ok(())
}

/// Mark stale agents as dead and reclaim their tasks.
/// `threshold_timestamp` is the cutoff for dead (stricter than stale).
#[reducer]
pub fn agent_mark_dead(
    ctx: &ReducerContext,
    threshold_timestamp: String,
) -> Result<(), String> {
    let dead: Vec<SwarmAgent> = ctx.db.swarm_agent().iter()
        .filter(|a| a.status == "stale" && a.last_heartbeat < threshold_timestamp)
        .collect();

    for agent in dead {
        // Reclaim tasks from this dead agent
        let orphaned: Vec<SwarmTask> = ctx.db.swarm_task().iter()
            .filter(|t| t.agent_id == agent.id && t.status == "in_progress")
            .collect();

        for task in orphaned {
            ctx.db.swarm_task().id().update(SwarmTask {
                status: "pending".to_string(),
                agent_id: String::new(),
                ..task
            });
        }

        ctx.db.swarm_agent().id().update(SwarmAgent {
            status: "dead".to_string(),
            ..agent
        });
    }

    Ok(())
}

/// Remove a disconnected agent from the swarm.
#[reducer]
pub fn agent_remove(
    ctx: &ReducerContext,
    id: String,
) -> Result<(), String> {
    if !ctx.db.swarm_agent().id().delete(&id) {
        return Err(format!("Agent '{}' not found", id));
    }
    Ok(())
}

// ============================================================
//  Memory Reducers
// ============================================================

/// Store a key-value pair (upsert semantics).
#[reducer]
pub fn memory_store(
    ctx: &ReducerContext,
    key: String,
    value: String,
    scope: String,
    timestamp: String,
) -> Result<(), String> {
    // Upsert: delete existing, then insert
    if ctx.db.hexflo_memory().key().find(&key).is_some() {
        ctx.db.hexflo_memory().key().delete(&key);
    }

    ctx.db.hexflo_memory().insert(HexFloMemory {
        key,
        value,
        scope,
        updated_at: timestamp,
    });

    Ok(())
}

/// Delete a key from memory.
#[reducer]
pub fn memory_delete(
    ctx: &ReducerContext,
    key: String,
) -> Result<(), String> {
    if !ctx.db.hexflo_memory().key().delete(&key) {
        return Err(format!("Key '{}' not found", key));
    }
    Ok(())
}

/// Clear all memory entries for a given scope.
#[reducer]
pub fn memory_clear_scope(
    ctx: &ReducerContext,
    scope: String,
) -> Result<(), String> {
    let to_delete: Vec<HexFloMemory> = ctx.db.hexflo_memory().iter()
        .filter(|m| m.scope == scope)
        .collect();

    for entry in to_delete {
        ctx.db.hexflo_memory().key().delete(&entry.key);
    }

    Ok(())
}
