#![allow(clippy::too_many_arguments)]
use spacetimedb::{table, reducer, ReducerContext, Table};

// ============================================================
//  Tables
// ============================================================

/// A swarm — a named group of agents working on a coordinated task.
/// Ownership: each swarm has exactly one owner agent (1:1). An agent may only
/// have one active swarm at a time (enforced in swarm_init reducer).
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
    /// Agent ID that owns this swarm (ADR-2603241900). Authoritative owner —
    /// not just creator. Use swarm_transfer to change ownership.
    pub owner_agent_id: String,
    /// Kept for backward compatibility during migration; mirrors owner_agent_id.
    pub created_by: String,
    pub created_at: String,
    pub updated_at: String,
}

/// A task within a swarm — the unit of work assigned to agents.
/// CAS fields (ADR-2603241900): callers read `version` before assigning, then
/// pass it to task_assign. Mismatch → ConflictError; prevents double-assignment
/// across remote nodes without distributed locks.
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
    /// Comma-separated task IDs this task depends on (empty = no deps).
    /// SpacetimeDB doesn't support Vec in table columns, so we use CSV.
    pub depends_on: String,
    /// Monotonic version counter — incremented on every status transition.
    /// Used for optimistic locking in task_assign CAS.
    pub version: u64,
    /// Agent ID of the last claimer (for conflict error messages).
    pub claimed_by: String,
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

// ─── Unified Agent Registry (ADR-058) ────────────────────────────────────

/// A hex agent — unified identity for all agent types (local Claude Code
/// sessions, remote agents, swarm participants). Single source of truth
/// replacing the fragmented agent-registry + orchestration + swarm_agent systems.
#[table(name = hex_agent, public)]
#[derive(Clone, Debug)]
pub struct HexAgent {
    #[primary_key]
    pub id: String,
    pub name: String,
    pub host: String,
    pub project_id: String,
    pub project_dir: String,
    pub model: String,
    pub session_id: String,
    /// Status: "online", "idle", "stale", "dead", "completed"
    pub status: String,
    /// Current swarm assignment (empty if unassigned).
    pub swarm_id: String,
    /// Role within swarm: "coder", "planner", "reviewer", etc.
    pub role: String,
    pub worktree_path: String,
    pub registered_at: String,
    pub last_heartbeat: String,
    /// JSON-encoded capabilities (models, tools, GPU, etc.)
    pub capabilities_json: String,
}

/// Register or re-register an agent (upsert by ID).
/// Called by hex hook session-start via /api/hex-agents/connect.
#[reducer]
pub fn agent_connect(
    ctx: &ReducerContext,
    id: String,
    name: String,
    host: String,
    project_id: String,
    project_dir: String,
    model: String,
    session_id: String,
    capabilities_json: String,
    timestamp: String,
) -> Result<(), String> {
    if let Some(existing) = ctx.db.hex_agent().id().find(&id) {
        ctx.db.hex_agent().id().update(HexAgent {
            name, host, project_id, project_dir, model, session_id,
            capabilities_json,
            status: "online".to_string(),
            last_heartbeat: timestamp.clone(),
            registered_at: existing.registered_at, // keep original
            ..existing
        });
    } else {
        ctx.db.hex_agent().insert(HexAgent {
            id, name, host, project_id, project_dir, model, session_id,
            status: "online".to_string(),
            swarm_id: String::new(),
            role: String::new(),
            worktree_path: String::new(),
            registered_at: timestamp.clone(),
            last_heartbeat: timestamp,
            capabilities_json,
        });
    }
    Ok(())
}

/// Disconnect an agent (set status to completed).
#[reducer]
pub fn agent_disconnect(
    ctx: &ReducerContext,
    id: String,
    timestamp: String,
) -> Result<(), String> {
    let agent = ctx.db.hex_agent().id().find(&id)
        .ok_or_else(|| format!("Agent '{}' not found", id))?;
    ctx.db.hex_agent().id().update(HexAgent {
        status: "completed".to_string(),
        last_heartbeat: timestamp,
        ..agent
    });
    Ok(())
}

/// Update agent heartbeat — keeps status=online.
#[reducer]
pub fn agent_heartbeat_update(
    ctx: &ReducerContext,
    id: String,
    timestamp: String,
) -> Result<(), String> {
    let agent = ctx.db.hex_agent().id().find(&id)
        .ok_or_else(|| format!("Agent '{}' not found", id))?;
    ctx.db.hex_agent().id().update(HexAgent {
        status: "online".to_string(),
        last_heartbeat: timestamp,
        ..agent
    });
    Ok(())
}

/// Assign agent to a swarm with a role.
#[reducer]
pub fn agent_assign_swarm(
    ctx: &ReducerContext,
    id: String,
    swarm_id: String,
    role: String,
) -> Result<(), String> {
    let agent = ctx.db.hex_agent().id().find(&id)
        .ok_or_else(|| format!("Agent '{}' not found", id))?;
    ctx.db.hex_agent().id().update(HexAgent {
        swarm_id, role, ..agent
    });
    Ok(())
}

/// Evict dead agents — delete agents with status "dead" whose heartbeat
/// is older than the given threshold timestamp.
#[reducer]
pub fn agent_evict_dead(
    ctx: &ReducerContext,
) -> Result<(), String> {
    let to_remove: Vec<String> = ctx.db.hex_agent().iter()
        .filter(|a| a.status == "dead")
        .map(|a| a.id.clone())
        .collect();
    for id in &to_remove {
        ctx.db.hex_agent().id().delete(id);
    }
    Ok(())
}

/// Mark agents as stale/dead based on heartbeat age.
/// stale_threshold: agents without heartbeat since this time become "stale"
/// dead_threshold: agents without heartbeat since this time become "dead"
#[reducer]
pub fn agent_mark_inactive(
    ctx: &ReducerContext,
    stale_threshold: String,
    dead_threshold: String,
) -> Result<(), String> {
    // Normalize Z → +00:00 for consistent string comparison of RFC3339 timestamps
    let stale_t = stale_threshold.replace("Z", "+00:00");
    let dead_t = dead_threshold.replace("Z", "+00:00");
    let agents: Vec<HexAgent> = ctx.db.hex_agent().iter()
        .filter(|a| a.status == "online" || a.status == "idle" || a.status == "stale")
        .collect();
    for agent in agents {
        let hb = agent.last_heartbeat.replace("Z", "+00:00");
        if hb < dead_t && agent.status != "dead" {
            ctx.db.hex_agent().id().update(HexAgent {
                status: "dead".to_string(),
                ..agent
            });
        } else if hb < stale_t && agent.status == "online" {
            ctx.db.hex_agent().id().update(HexAgent {
                status: "stale".to_string(),
                ..agent
            });
        }
    }
    Ok(())
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
    pub description: String,
    pub path: String,
    pub registered_at: String,
}

/// Register or update a project.
#[reducer]
pub fn register_project(
    ctx: &ReducerContext,
    project_id: String,
    name: String,
    description: String,
    path: String,
    registered_at: String,
) -> Result<(), String> {
    if path.is_empty() {
        return Err("Project path is required".to_string());
    }
    if let Some(existing) = ctx.db.project().project_id().find(&project_id) {
        ctx.db.project().project_id().update(Project {
            name,
            description,
            path,
            registered_at,
            ..existing
        });
    } else {
        ctx.db.project().insert(Project {
            project_id,
            name,
            description,
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

// ─── MCP Tool Registry ──────────────────────────────────────────────────────

/// Registered MCP tool — synced from config/mcp-tools.json on nexus startup.
/// Dashboard and external clients subscribe to this table to discover available tools.
#[table(name = mcp_tool, public)]
#[derive(Clone, Debug)]
pub struct McpTool {
    #[primary_key]
    pub name: String,
    pub category: String,
    pub description: String,
    pub route_method: String,
    pub route_path: String,
    /// JSON-encoded inputSchema for the tool.
    pub input_schema: String,
    pub version: String,
    pub synced_at: String,
}

/// Upsert an MCP tool definition (called by config sync on nexus startup).
#[reducer]
pub fn mcp_tool_sync(
    ctx: &ReducerContext,
    name: String,
    category: String,
    description: String,
    route_method: String,
    route_path: String,
    input_schema: String,
    version: String,
    synced_at: String,
) -> Result<(), String> {
    if let Some(existing) = ctx.db.mcp_tool().name().find(&name) {
        ctx.db.mcp_tool().name().update(McpTool {
            category, description, route_method, route_path, input_schema, version, synced_at,
            ..existing
        });
    } else {
        ctx.db.mcp_tool().insert(McpTool {
            name, category, description, route_method, route_path, input_schema, version, synced_at,
        });
    }
    Ok(())
}

// ─── Remote Agent Registry (ADR-040) ─────────────────────────────────────

/// A remote agent connected via SSH tunnel + WebSocket.
#[table(name = remote_agent, public)]
#[derive(Clone, Debug)]
pub struct RemoteAgent {
    #[primary_key]
    pub agent_id: String,
    pub name: String,
    pub host: String,
    pub project_dir: String,
    /// Status: "connecting", "online", "busy", "stale", "dead"
    pub status: String,
    /// JSON-encoded AgentCapabilities (models, tools, max_concurrent, gpu_vram_mb)
    pub capabilities_json: String,
    pub last_heartbeat: String,
    pub connected_at: String,
    pub tunnel_id: String,
}

/// Register or update a remote agent.
#[reducer]
pub fn register_remote_agent(
    ctx: &ReducerContext,
    agent_id: String,
    name: String,
    host: String,
    project_dir: String,
    capabilities_json: String,
    tunnel_id: String,
    timestamp: String,
) -> Result<(), String> {
    if let Some(existing) = ctx.db.remote_agent().agent_id().find(&agent_id) {
        ctx.db.remote_agent().agent_id().update(RemoteAgent {
            name, host, project_dir, capabilities_json, tunnel_id,
            status: "online".to_string(),
            last_heartbeat: timestamp.clone(),
            connected_at: timestamp,
            ..existing
        });
    } else {
        ctx.db.remote_agent().insert(RemoteAgent {
            agent_id, name, host, project_dir, capabilities_json, tunnel_id,
            status: "online".to_string(),
            last_heartbeat: timestamp.clone(),
            connected_at: timestamp,
        });
    }
    Ok(())
}

/// Update a remote agent's heartbeat and optionally status.
#[reducer]
pub fn remote_agent_heartbeat(
    ctx: &ReducerContext,
    agent_id: String,
    status: String,
    timestamp: String,
) -> Result<(), String> {
    let agent = ctx.db.remote_agent().agent_id().find(&agent_id)
        .ok_or_else(|| format!("Remote agent '{}' not found", agent_id))?;
    ctx.db.remote_agent().agent_id().update(RemoteAgent {
        status,
        last_heartbeat: timestamp,
        ..agent
    });
    Ok(())
}

/// Remove a remote agent (on disconnect or death).
#[reducer]
pub fn remove_remote_agent(
    ctx: &ReducerContext,
    agent_id: String,
) -> Result<(), String> {
    if !ctx.db.remote_agent().agent_id().delete(&agent_id) {
        return Err(format!("Remote agent '{}' not found", agent_id));
    }
    Ok(())
}

// ─── Inference Server Registry ───────────────────────────────────────────

/// An inference server (Ollama, vLLM, OpenAI-compatible) available to agents.
#[table(name = inference_server, public)]
#[derive(Clone, Debug)]
pub struct InferenceServer {
    #[primary_key]
    pub server_id: String,
    pub name: String,
    pub host: String,
    pub provider: String,
    /// JSON-encoded list of available models
    pub models_json: String,
    /// Status: "online", "offline", "degraded"
    pub status: String,
    pub last_health_check: String,
    pub registered_at: String,
}

/// Register or update an inference server.
#[reducer]
pub fn register_inference_server(
    ctx: &ReducerContext,
    server_id: String,
    name: String,
    host: String,
    provider: String,
    models_json: String,
    timestamp: String,
) -> Result<(), String> {
    if let Some(existing) = ctx.db.inference_server().server_id().find(&server_id) {
        ctx.db.inference_server().server_id().update(InferenceServer {
            name, host, provider, models_json,
            status: "online".to_string(),
            last_health_check: timestamp,
            ..existing
        });
    } else {
        ctx.db.inference_server().insert(InferenceServer {
            server_id, name, host, provider, models_json,
            status: "online".to_string(),
            last_health_check: timestamp.clone(),
            registered_at: timestamp,
        });
    }
    Ok(())
}

/// Remove an inference server.
#[reducer]
pub fn remove_inference_server(
    ctx: &ReducerContext,
    server_id: String,
) -> Result<(), String> {
    if !ctx.db.inference_server().server_id().delete(&server_id) {
        return Err(format!("Inference server '{}' not found", server_id));
    }
    Ok(())
}

// ─── Agent Notification Inbox (ADR-060) ──────────────────────────────────

/// A notification message addressed to a specific agent.
/// Used for system events (restart, update, config change) that agents
/// must acknowledge before they can be dismissed.
#[table(name = agent_inbox, public)]
#[derive(Clone, Debug)]
pub struct AgentInbox {
    #[primary_key]
    #[auto_inc]
    pub id: u64,
    pub agent_id: String,
    /// Priority: 0=info, 1=warning, 2=critical
    pub priority: u8,
    /// Kind: "restart", "update", "shutdown", "config_change", "info"
    pub kind: String,
    /// JSON payload with event-specific data
    pub payload: String,
    pub created_at: String,
    /// Set when the target agent acknowledges the notification.
    pub acknowledged_at: String,
    /// Set when the notification expires (cleanup).
    pub expired_at: String,
}

/// Send a notification to a specific agent. Validates agent exists.
#[reducer]
pub fn notify_agent(
    ctx: &ReducerContext,
    agent_id: String,
    priority: u8,
    kind: String,
    payload: String,
    timestamp: String,
) -> Result<(), String> {
    // Validate agent exists in hex_agent table
    if ctx.db.hex_agent().id().find(&agent_id).is_none() {
        return Err(format!("Agent '{}' not found", agent_id));
    }
    if priority > 2 {
        return Err("Priority must be 0 (info), 1 (warning), or 2 (critical)".to_string());
    }

    ctx.db.agent_inbox().insert(AgentInbox {
        id: 0, // auto_inc
        agent_id,
        priority,
        kind,
        payload,
        created_at: timestamp,
        acknowledged_at: String::new(),
        expired_at: String::new(),
    });

    Ok(())
}

/// Broadcast a notification to all agents in a project.
#[reducer]
pub fn notify_all_agents(
    ctx: &ReducerContext,
    project_id: String,
    priority: u8,
    kind: String,
    payload: String,
    timestamp: String,
) -> Result<(), String> {
    if priority > 2 {
        return Err("Priority must be 0 (info), 1 (warning), or 2 (critical)".to_string());
    }

    let agents: Vec<String> = ctx.db.hex_agent().iter()
        .filter(|a| a.project_id == project_id && (a.status == "online" || a.status == "idle"))
        .map(|a| a.id.clone())
        .collect();

    for aid in agents {
        ctx.db.agent_inbox().insert(AgentInbox {
            id: 0,
            agent_id: aid,
            priority,
            kind: kind.clone(),
            payload: payload.clone(),
            created_at: timestamp.clone(),
            acknowledged_at: String::new(),
            expired_at: String::new(),
        });
    }

    Ok(())
}

/// Acknowledge a notification. Only the target agent can ack.
#[reducer]
pub fn acknowledge_notification(
    ctx: &ReducerContext,
    notification_id: u64,
    agent_id: String,
    timestamp: String,
) -> Result<(), String> {
    let notif = ctx.db.agent_inbox().id().find(notification_id)
        .ok_or_else(|| format!("Notification '{}' not found", notification_id))?;

    if notif.agent_id != agent_id {
        return Err(format!("Agent '{}' is not the target of notification '{}'", agent_id, notification_id));
    }

    if !notif.acknowledged_at.is_empty() {
        return Ok(()); // Already acked — idempotent
    }

    ctx.db.agent_inbox().id().update(AgentInbox {
        acknowledged_at: timestamp,
        ..notif
    });

    Ok(())
}

/// Expire notifications older than max_age_secs that were never acknowledged.
#[reducer]
pub fn expire_stale_notifications(
    ctx: &ReducerContext,
    threshold_timestamp: String,
) -> Result<(), String> {
    let expired: Vec<AgentInbox> = ctx.db.agent_inbox().iter()
        .filter(|n| {
            n.acknowledged_at.is_empty()
                && n.expired_at.is_empty()
                && n.created_at < threshold_timestamp
        })
        .collect();

    let now = threshold_timestamp;
    for notif in expired {
        ctx.db.agent_inbox().id().update(AgentInbox {
            expired_at: now.clone(),
            ..notif
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
    created_by: String,
    timestamp: String,
) -> Result<(), String> {
    // Prevent duplicate swarm IDs
    if ctx.db.swarm().id().find(&id).is_some() {
        return Err(format!("Swarm '{}' already exists", id));
    }

    // ADR-2603241900: enforce 1:1 agent↔swarm ownership.
    // An agent may not own more than one active swarm at a time.
    if !created_by.is_empty() {
        let already_owns = ctx.db.swarm().iter()
            .any(|s| s.owner_agent_id == created_by && s.status == "active");
        if already_owns {
            return Err(format!(
                "Agent '{}' already owns an active swarm — complete or transfer it first",
                created_by
            ));
        }
    }

    ctx.db.swarm().insert(Swarm {
        id: id.clone(),
        project_id,
        name,
        topology,
        status: "active".to_string(),
        owner_agent_id: created_by.clone(),
        created_by: created_by.clone(),
        created_at: timestamp.clone(),
        updated_at: timestamp,
    });

    // Update hex_agent.swarm_id to point at the owned swarm
    if let Some(agent) = ctx.db.hex_agent().id().find(&created_by) {
        ctx.db.hex_agent().id().update(HexAgent {
            swarm_id: id,
            ..agent
        });
    }

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
/// `depends_on` is a comma-separated list of task IDs that must complete before
/// this task can be assigned. Pass empty string for no dependencies.
#[reducer]
pub fn task_create(
    ctx: &ReducerContext,
    id: String,
    swarm_id: String,
    title: String,
    depends_on: String,
    timestamp: String,
) -> Result<(), String> {
    // Verify swarm exists and is active
    let swarm = ctx.db.swarm().id().find(&swarm_id)
        .ok_or_else(|| format!("Swarm '{}' not found", swarm_id))?;

    if swarm.status != "active" {
        return Err(format!("Swarm '{}' is not active (status: {})", swarm_id, swarm.status));
    }

    // Validate that all referenced dependency task IDs actually exist
    if !depends_on.is_empty() {
        for dep_id in depends_on.split(',') {
            let dep_id = dep_id.trim();
            if !dep_id.is_empty() && ctx.db.swarm_task().id().find(dep_id.to_string()).is_none() {
                return Err(format!("Dependency task '{}' not found", dep_id));
            }
        }
    }

    ctx.db.swarm_task().insert(SwarmTask {
        id,
        swarm_id,
        title,
        status: "pending".to_string(),
        agent_id: String::new(),
        result: String::new(),
        depends_on,
        version: 0,
        claimed_by: String::new(),
        created_at: timestamp,
        completed_at: String::new(),
    });

    Ok(())
}

/// Check whether all dependencies of a task have been completed.
/// Returns true if the task has no dependencies or all dependencies are completed.
fn dependencies_met(ctx: &ReducerContext, task: &SwarmTask) -> bool {
    if task.depends_on.is_empty() {
        return true;
    }
    for dep_id in task.depends_on.split(',') {
        let dep_id = dep_id.trim();
        if dep_id.is_empty() {
            continue;
        }
        match ctx.db.swarm_task().id().find(dep_id.to_string()) {
            Some(dep_task) => {
                if dep_task.status != "completed" {
                    return false;
                }
            }
            None => {
                // Dependency task not found — treat as unmet
                return false;
            }
        }
    }
    true
}

/// Assign a task to an agent using Compare-And-Swap (ADR-2603241900).
///
/// `expected_version` must match `task.version` at the time of the call.
/// Pass `u64::MAX` (18446744073709551615) to skip version check (legacy / force-assign).
/// On mismatch → error "version_mismatch:<expected>:<actual>".
/// If task is already claimed → error "already_claimed:<agent_id>".
#[reducer]
pub fn task_assign(
    ctx: &ReducerContext,
    task_id: String,
    agent_id: String,
    expected_version: u64,
    timestamp: String,
) -> Result<(), String> {
    let task = ctx.db.swarm_task().id().find(&task_id)
        .ok_or_else(|| format!("Task '{}' not found", task_id))?;

    // CAS version check (skip if caller passes u64::MAX)
    if expected_version != u64::MAX && task.version != expected_version {
        return Err(format!(
            "version_mismatch:{}:{}",
            expected_version, task.version
        ));
    }

    if task.status != "pending" {
        return Err(format!(
            "already_claimed:{}",
            task.claimed_by
        ));
    }

    // Check that all dependency tasks are completed before allowing assignment
    if !dependencies_met(ctx, &task) {
        return Err(format!(
            "Cannot assign task '{}' — dependencies not met (depends_on: {})",
            task_id, task.depends_on
        ));
    }

    let swarm_id = task.swarm_id.clone();
    let new_version = task.version + 1;

    ctx.db.swarm_task().id().update(SwarmTask {
        status: "in_progress".to_string(),
        agent_id: agent_id.clone(),
        claimed_by: agent_id.clone(),
        version: new_version,
        completed_at: timestamp.clone(),
        ..task
    });

    // ── Link participant agent ↔ swarm (ADR-058) ────────────────────
    // Note: this is participant membership, not ownership. Ownership is
    // set in swarm_init via owner_agent_id.
    if let Some(agent) = ctx.db.hex_agent().id().find(&agent_id) {
        if agent.swarm_id.is_empty() {
            ctx.db.hex_agent().id().update(HexAgent {
                swarm_id: swarm_id.clone(),
                ..agent
            });
        }
    }

    // Ensure a swarm_agent row exists for this agent in this swarm
    if ctx.db.swarm_agent().id().find(&agent_id).is_none() {
        let name = ctx.db.hex_agent().id().find(&agent_id)
            .map(|a| a.name.clone())
            .unwrap_or_else(|| agent_id.clone());
        ctx.db.swarm_agent().insert(SwarmAgent {
            id: agent_id,
            swarm_id,
            name,
            role: String::new(),
            status: "active".to_string(),
            worktree_path: String::new(),
            last_heartbeat: timestamp,
        });
    }

    Ok(())
}

/// Transfer swarm ownership to a new agent (ADR-2603241900).
/// Only the current owner or a call with no owner set may transfer.
#[reducer]
pub fn swarm_transfer(
    ctx: &ReducerContext,
    swarm_id: String,
    new_owner_agent_id: String,
    timestamp: String,
) -> Result<(), String> {
    let swarm = ctx.db.swarm().id().find(&swarm_id)
        .ok_or_else(|| format!("Swarm '{}' not found", swarm_id))?;

    if swarm.status != "active" {
        return Err(format!("Swarm '{}' is not active — cannot transfer", swarm_id));
    }

    // Verify new owner exists
    let new_owner = ctx.db.hex_agent().id().find(&new_owner_agent_id)
        .ok_or_else(|| format!("New owner agent '{}' not found", new_owner_agent_id))?;

    // New owner must not already own an active swarm
    let already_owns = ctx.db.swarm().iter()
        .any(|s| s.owner_agent_id == new_owner_agent_id && s.status == "active" && s.id != swarm_id);
    if already_owns {
        return Err(format!(
            "Agent '{}' already owns an active swarm — cannot receive transfer",
            new_owner_agent_id
        ));
    }

    let old_owner_id = swarm.owner_agent_id.clone();

    // Update swarm ownership
    ctx.db.swarm().id().update(Swarm {
        owner_agent_id: new_owner_agent_id.clone(),
        created_by: new_owner_agent_id.clone(),
        updated_at: timestamp.clone(),
        ..swarm
    });

    // Clear swarm_id on old owner (they no longer own it)
    if !old_owner_id.is_empty() {
        if let Some(old_owner) = ctx.db.hex_agent().id().find(&old_owner_id) {
            if old_owner.swarm_id == swarm_id {
                ctx.db.hex_agent().id().update(HexAgent {
                    swarm_id: String::new(),
                    ..old_owner
                });
            }
        }
    }

    // Set swarm_id on new owner
    ctx.db.hex_agent().id().update(HexAgent {
        swarm_id,
        ..new_owner
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
        let new_version = task.version + 1;
        ctx.db.swarm_task().id().update(SwarmTask {
            status: "pending".to_string(),
            agent_id: String::new(),
            claimed_by: String::new(),
            version: new_version,
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

// ============================================================
//  Quality Gate & Fix Task Tables (Swarm Gate Enforcement)
// ============================================================

/// A quality gate check within a swarm — validates code at a specific tier.
/// Created by the swarm gate enforcement pipeline to track compile, test,
/// and architecture analysis results per tier.
#[table(name = quality_gate_task, public)]
#[derive(Clone, Debug)]
pub struct QualityGateTask {
    #[primary_key]
    pub id: String,
    pub swarm_id: String,
    /// Hex architecture tier (0=domain+ports, 1=secondary, 2=primary, 3=usecases, 4=composition)
    pub tier: u32,
    /// Gate type: "compile", "test", "analyze", "full"
    pub gate_type: String,
    /// Output directory to analyze
    pub target_dir: String,
    /// Language: "typescript", "rust"
    pub language: String,
    /// Status: "pending", "running", "pass", "fail"
    pub status: String,
    /// Architecture score 0-100 (from hex analyze)
    pub score: u32,
    /// Letter grade: "A", "B", "C", "D", "F"
    pub grade: String,
    pub violations_count: u32,
    /// First 4KB of error output
    pub error_output: String,
    /// Which retry iteration (1, 2, 3)
    pub iteration: u32,
    pub created_at: String,
    /// ISO 8601 or empty if not yet completed
    pub completed_at: String,
}

/// A fix task spawned in response to a quality gate failure.
/// Tracks the automated fix attempt including model usage and cost.
#[table(name = fix_task, public)]
#[derive(Clone, Debug)]
pub struct FixTask {
    #[primary_key]
    pub id: String,
    /// Which quality gate triggered this fix
    pub gate_task_id: String,
    pub swarm_id: String,
    /// Fix type: "compile", "test", "violation"
    pub fix_type: String,
    /// File to fix
    pub target_file: String,
    /// The error/violation description
    pub error_context: String,
    /// Inference model used for the fix
    pub model_used: String,
    pub tokens: u64,
    /// Cost in USD stored as string for WASM compatibility
    pub cost_usd: String,
    /// Status: "pending", "running", "completed", "failed"
    pub status: String,
    /// Result: "fixed", "unchanged", or error message
    pub result: String,
    pub created_at: String,
    pub completed_at: String,
}

// ============================================================
//  Quality Gate Reducers
// ============================================================

/// Create a new quality gate task for a swarm tier.
#[reducer]
pub fn create_quality_gate(
    ctx: &ReducerContext,
    id: String,
    swarm_id: String,
    tier: u32,
    gate_type: String,
    target_dir: String,
    language: String,
    iteration: u32,
    timestamp: String,
) -> Result<(), String> {
    // Verify swarm exists
    if ctx.db.swarm().id().find(&swarm_id).is_none() {
        return Err(format!("Swarm '{}' not found", swarm_id));
    }

    if tier > 4 {
        return Err("Tier must be 0-4".to_string());
    }

    ctx.db.quality_gate_task().insert(QualityGateTask {
        id,
        swarm_id,
        tier,
        gate_type,
        target_dir,
        language,
        status: "pending".to_string(),
        score: 0,
        grade: String::new(),
        violations_count: 0,
        error_output: String::new(),
        iteration,
        created_at: timestamp,
        completed_at: String::new(),
    });

    Ok(())
}

/// Complete a quality gate task with results.
#[reducer]
pub fn complete_quality_gate(
    ctx: &ReducerContext,
    id: String,
    status: String,
    score: u32,
    grade: String,
    violations_count: u32,
    error_output: String,
    timestamp: String,
) -> Result<(), String> {
    let gate = ctx.db.quality_gate_task().id().find(&id)
        .ok_or_else(|| format!("QualityGateTask '{}' not found", id))?;

    if status != "pass" && status != "fail" {
        return Err(format!("Status must be 'pass' or 'fail', got '{}'", status));
    }

    ctx.db.quality_gate_task().id().update(QualityGateTask {
        status,
        score,
        grade,
        violations_count,
        error_output,
        completed_at: timestamp,
        ..gate
    });

    Ok(())
}

// ============================================================
//  Fix Task Reducers
// ============================================================

/// Create a fix task in response to a quality gate failure.
#[reducer]
pub fn create_fix_task(
    ctx: &ReducerContext,
    id: String,
    gate_task_id: String,
    swarm_id: String,
    fix_type: String,
    target_file: String,
    error_context: String,
    timestamp: String,
) -> Result<(), String> {
    // Verify the gate task exists
    if ctx.db.quality_gate_task().id().find(&gate_task_id).is_none() {
        return Err(format!("QualityGateTask '{}' not found", gate_task_id));
    }

    ctx.db.fix_task().insert(FixTask {
        id,
        gate_task_id,
        swarm_id,
        fix_type,
        target_file,
        error_context,
        model_used: String::new(),
        tokens: 0,
        cost_usd: String::new(),
        status: "pending".to_string(),
        result: String::new(),
        created_at: timestamp,
        completed_at: String::new(),
    });

    Ok(())
}

/// Complete a fix task with results and cost tracking.
#[reducer]
pub fn complete_fix_task(
    ctx: &ReducerContext,
    id: String,
    status: String,
    result: String,
    model_used: String,
    tokens: u64,
    cost_usd: String,
    timestamp: String,
) -> Result<(), String> {
    let fix = ctx.db.fix_task().id().find(&id)
        .ok_or_else(|| format!("FixTask '{}' not found", id))?;

    if status != "completed" && status != "failed" {
        return Err(format!("Status must be 'completed' or 'failed', got '{}'", status));
    }

    ctx.db.fix_task().id().update(FixTask {
        status,
        result,
        model_used,
        tokens,
        cost_usd,
        completed_at: timestamp,
        ..fix
    });

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

// ============================================================
//  Enforcement Rules (ADR-2603221959 P5)
// ============================================================

/// An enforcement rule — persisted in SpacetimeDB, synced from .hex/adr-rules.toml.
/// These rules are checked by MCP tool guards and nexus API middleware.
#[table(name = enforcement_rule, public)]
#[derive(Clone, Debug)]
pub struct EnforcementRule {
    #[unique]
    pub id: String,
    /// ADR reference (e.g. "ADR-056")
    pub adr: String,
    /// Operation this rule applies to: "edit", "spawn_agent", "bash", "*"
    pub operation: String,
    /// Condition: "requires_workplan", "requires_task", "boundary_check", "pattern_match"
    pub condition: String,
    /// Severity: "block", "warn", "info"
    pub severity: String,
    /// Whether this rule is active
    pub enabled: u8, // 1 = enabled, 0 = disabled (SpacetimeDB lacks bool in some contexts)
    /// Project ID this rule applies to (empty = global)
    pub project_id: String,
    /// Human-readable description
    pub message: String,
    /// File patterns to match (comma-separated, e.g. ".ts,.tsx")
    pub file_patterns: String,
    /// Violation patterns to detect (comma-separated literal strings)
    pub violation_patterns: String,
    pub created_at: String,
    pub updated_at: String,
}

#[reducer]
pub fn enforcement_rule_upsert(
    ctx: &ReducerContext,
    id: String,
    adr: String,
    operation: String,
    condition: String,
    severity: String,
    enabled: u8,
    project_id: String,
    message: String,
    file_patterns: String,
    violation_patterns: String,
    timestamp: String,
) -> Result<(), String> {
    // Delete existing if present (upsert)
    if ctx.db.enforcement_rule().id().find(&id).is_some() {
        ctx.db.enforcement_rule().id().delete(&id);
    }

    ctx.db.enforcement_rule().insert(EnforcementRule {
        id,
        adr,
        operation,
        condition,
        severity,
        enabled,
        project_id,
        message,
        file_patterns,
        violation_patterns,
        created_at: timestamp.clone(),
        updated_at: timestamp,
    });

    Ok(())
}

#[reducer]
pub fn enforcement_rule_toggle(
    ctx: &ReducerContext,
    id: String,
    enabled: u8,
    timestamp: String,
) -> Result<(), String> {
    let rule = ctx.db.enforcement_rule().id().find(&id)
        .ok_or_else(|| format!("Rule '{}' not found", id))?;

    ctx.db.enforcement_rule().id().delete(&id);
    ctx.db.enforcement_rule().insert(EnforcementRule {
        enabled,
        updated_at: timestamp,
        ..rule
    });

    Ok(())
}

#[reducer]
pub fn enforcement_rule_delete(
    ctx: &ReducerContext,
    id: String,
) -> Result<(), String> {
    ctx.db.enforcement_rule().id().delete(&id);
    Ok(())
}

/// Combined cleanup reducer called by hex-nexus periodically (ADR-042).
///
/// `cutoff` is an RFC3339 timestamp — agents and notifications older than
/// this value are marked stale/dead/expired. Inlines agent_mark_stale,
/// agent_mark_dead, and expire_stale_notifications in a single transaction.
#[reducer]
pub fn coordination_cleanup(
    ctx: &ReducerContext,
    cutoff: String,
) -> Result<(), String> {
    // 1. Mark active agents as stale if their heartbeat is before the cutoff.
    let stale: Vec<SwarmAgent> = ctx.db.swarm_agent().iter()
        .filter(|a| a.status == "active" && a.last_heartbeat < cutoff)
        .collect();
    for agent in stale {
        ctx.db.swarm_agent().id().update(SwarmAgent {
            status: "stale".to_string(),
            ..agent
        });
    }

    // 2. Mark stale agents as dead and reclaim their in-progress tasks.
    let dead: Vec<SwarmAgent> = ctx.db.swarm_agent().iter()
        .filter(|a| a.status == "stale" && a.last_heartbeat < cutoff)
        .collect();
    for agent in dead {
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

    // 3. Expire unacknowledged inbox notifications older than the cutoff.
    let expired: Vec<AgentInbox> = ctx.db.agent_inbox().iter()
        .filter(|n| {
            n.acknowledged_at.is_empty()
                && n.expired_at.is_empty()
                && n.created_at < cutoff
        })
        .collect();
    for notif in expired {
        ctx.db.agent_inbox().id().update(AgentInbox {
            expired_at: cutoff.clone(),
            ..notif
        });
    }

    Ok(())
}

// ─── Architecture Fingerprint ──────────────────────────────────────────────
//
// ADR-2603301200: Token-efficient architecture context injected into every
// LLM inference system prompt. Generated from go.mod/package.json/Cargo.toml,
// workplan, and active ADRs. Prevents models from hallucinating wrong stacks.

#[table(name = architecture_fingerprint, public)]
#[derive(Clone, Debug)]
pub struct ArchitectureFingerprint {
    #[primary_key]
    pub project_id: String,
    /// Primary language: "go", "typescript", "rust"
    pub language: String,
    /// Framework: "stdlib", "gin", "express", "axum", etc. or "none"
    pub framework: String,
    /// Output type: "cli", "web-api", "library", "standalone"
    pub output_type: String,
    /// Architecture style: "hexagonal", "standalone", "layered"
    pub architecture_style: String,
    /// JSON array of constraint strings (max 5)
    pub constraints: String,
    /// JSON array of {id, summary} objects (max 3 active ADRs)
    pub active_adrs: String,
    /// One-sentence description of what the project builds
    pub workplan_objective: String,
    /// Estimated token count of the formatted injection block
    pub fingerprint_tokens: u32,
    /// ISO 8601 timestamp of last generation
    pub generated_at: String,
}

/// Upsert an architecture fingerprint for a project.
#[reducer]
pub fn upsert_fingerprint(
    ctx: &ReducerContext,
    project_id: String,
    language: String,
    framework: String,
    output_type: String,
    architecture_style: String,
    constraints: String,
    active_adrs: String,
    workplan_objective: String,
    fingerprint_tokens: u32,
    generated_at: String,
) -> Result<(), String> {
    if project_id.is_empty() {
        return Err("project_id is required".to_string());
    }
    let fp = ArchitectureFingerprint {
        project_id: project_id.clone(),
        language,
        framework,
        output_type,
        architecture_style,
        constraints,
        active_adrs,
        workplan_objective,
        fingerprint_tokens,
        generated_at,
    };
    if ctx.db.architecture_fingerprint().project_id().find(&project_id).is_some() {
        ctx.db.architecture_fingerprint().project_id().update(fp);
    } else {
        ctx.db.architecture_fingerprint().insert(fp);
    }
    Ok(())
}

/// Remove a fingerprint when a project is deleted or reset.
#[reducer]
pub fn delete_fingerprint(
    ctx: &ReducerContext,
    project_id: String,
) -> Result<(), String> {
    if ctx.db.architecture_fingerprint().project_id().find(&project_id).is_some() {
        ctx.db.architecture_fingerprint().project_id().delete(&project_id);
        Ok(())
    } else {
        Err(format!("No fingerprint found for project '{}'", project_id))
    }
}
