#![allow(clippy::too_many_arguments)]
use spacetimedb::{reducer, table, ReducerContext, ScheduleAt, Table, Timestamp};

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
    /// Agent ID that owns this swarm (ADR-2026-03-24-1900). Authoritative owner —
    /// not just creator. Use swarm_transfer to change ownership.
    pub owner_agent_id: String,
    /// Kept for backward compatibility during migration; mirrors owner_agent_id.
    pub created_by: String,
    pub created_at: String,
    pub updated_at: String,
}

/// A task within a swarm — the unit of work assigned to agents.
/// CAS fields (ADR-2026-03-24-1900): callers read `version` before assigning, then
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

/// An inference task dispatched by the workplan executor to a Claude Code agent.
/// Replaces hexflo_memory "inference:queue:*" keys with first-class STDB rows.
#[table(name = inference_task, public)]
#[derive(Clone, Debug)]
pub struct InferenceTask {
    #[primary_key]
    pub id: String,
    pub workplan_id: String,
    pub task_id: String,
    pub phase: String,
    pub prompt: String,
    pub role: String,
    /// "Pending" | "InProgress" | "Completed" | "Failed"
    pub status: String,
    /// Agent that claimed this task (empty until claimed)
    pub agent_id: String,
    pub result: String,
    pub error: String,
    pub created_at: String,
    pub updated_at: String,
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
    role: String,
    capabilities_json: String,
    timestamp: String,
) -> Result<(), String> {
    if let Some(existing) = ctx.db.hex_agent().id().find(&id) {
        ctx.db.hex_agent().id().update(HexAgent {
            name,
            host,
            project_id,
            project_dir,
            model,
            session_id,
            role: if !role.is_empty() { role } else { existing.role },
            capabilities_json,
            status: "online".to_string(),
            last_heartbeat: timestamp.clone(),
            registered_at: existing.registered_at, // keep original
            ..existing
        });
    } else {
        ctx.db.hex_agent().insert(HexAgent {
            id: id.clone(),
            name,
            host,
            project_id,
            project_dir,
            model,
            session_id,
            status: "online".to_string(),
            swarm_id: String::new(),
            role,
            worktree_path: String::new(),
            registered_at: timestamp.clone(),
            last_heartbeat: timestamp.clone(),
            capabilities_json,
        });
    }

    // Revive any dead swarm_agent entries for this agent (TLA+ finding:
    // after agent_evict_dead deletes the hex_agent row, a reconnecting
    // agent re-creates hex_agent but the orphaned swarm_agent stays "dead").
    let dead_swarm_agents: Vec<SwarmAgent> = ctx
        .db
        .swarm_agent()
        .iter()
        .filter(|sa| sa.id == id && sa.status == "dead")
        .collect();
    for sa in dead_swarm_agents {
        ctx.db.swarm_agent().id().update(SwarmAgent {
            status: "active".to_string(),
            last_heartbeat: timestamp.clone(),
            ..sa
        });
    }

    Ok(())
}

/// Disconnect an agent (set status to completed).
#[reducer]
pub fn agent_disconnect(ctx: &ReducerContext, id: String, timestamp: String) -> Result<(), String> {
    let agent = ctx
        .db
        .hex_agent()
        .id()
        .find(&id)
        .ok_or_else(|| format!("Agent '{}' not found", id))?;
    ctx.db.hex_agent().id().update(HexAgent {
        status: "completed".to_string(),
        last_heartbeat: timestamp,
        ..agent
    });
    Ok(())
}

/// Update agent capabilities (models, tok/s, provider) without full re-registration.
/// Called by worker after inference discovery (ADR-2026-04-13-0010 P2.1).
#[reducer]
pub fn agent_update_capabilities(
    ctx: &ReducerContext,
    id: String,
    capabilities_json: String,
    timestamp: String,
) -> Result<(), String> {
    let agent = ctx
        .db
        .hex_agent()
        .id()
        .find(&id)
        .ok_or_else(|| format!("Agent '{}' not found", id))?;
    ctx.db.hex_agent().id().update(HexAgent {
        capabilities_json,
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
    let agent = ctx
        .db
        .hex_agent()
        .id()
        .find(&id)
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
    let agent = ctx
        .db
        .hex_agent()
        .id()
        .find(&id)
        .ok_or_else(|| format!("Agent '{}' not found", id))?;
    ctx.db.hex_agent().id().update(HexAgent {
        swarm_id,
        role,
        ..agent
    });
    Ok(())
}

/// Evict dead agents — delete agents with status "dead" whose heartbeat
/// is older than the given threshold timestamp.
#[reducer]
pub fn agent_evict_dead(ctx: &ReducerContext) -> Result<(), String> {
    let to_remove: Vec<String> = ctx
        .db
        .hex_agent()
        .iter()
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
    let agents: Vec<HexAgent> = ctx
        .db
        .hex_agent()
        .iter()
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

// ─── Cleanup Log (absorbed from hexflo-cleanup) ─────────────────────────────

/// Cleanup run log — tracks when cleanup last ran and what it did.
/// Absorbed from hexflo-cleanup module for consolidated observability.
#[table(name = cleanup_log, public)]
#[derive(Clone, Debug)]
pub struct CleanupLog {
    #[primary_key]
    #[auto_inc]
    pub id: u64,
    pub ran_at: String,
    pub stale_count: u32,
    pub dead_count: u32,
    pub reclaimed_tasks: u32,
    pub expired_notifications: u32,
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
    pub ast_is_stub: bool,
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
    ast_is_stub: bool,
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
            ast_is_stub,
            ..existing
        });
    } else {
        ctx.db.project().insert(Project {
            project_id,
            name,
            description,
            path,
            registered_at,
            ast_is_stub,
        });
    }
    Ok(())
}

/// Remove a project by ID.
#[reducer]
pub fn remove_project(ctx: &ReducerContext, project_id: String) -> Result<(), String> {
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
            value_json,
            source_file,
            synced_at,
            ..existing
        });
    } else {
        ctx.db.project_config().insert(ProjectConfig {
            key,
            project_id,
            value_json,
            source_file,
            synced_at,
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
            name,
            trigger_cmd,
            description,
            source_path,
            synced_at,
            ..existing
        });
    } else {
        ctx.db.skill_registry().insert(SkillEntry {
            skill_id,
            project_id,
            name,
            trigger_cmd,
            description,
            source_path,
            synced_at,
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
            name,
            role,
            model,
            capabilities_json,
            tools_json,
            source_path,
            synced_at,
            ..existing
        });
    } else {
        ctx.db.agent_definition().insert(AgentDef {
            agent_def_id,
            project_id,
            name,
            role,
            model,
            capabilities_json,
            tools_json,
            source_path,
            synced_at,
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
            category,
            description,
            route_method,
            route_path,
            input_schema,
            version,
            synced_at,
            ..existing
        });
    } else {
        ctx.db.mcp_tool().insert(McpTool {
            name,
            category,
            description,
            route_method,
            route_path,
            input_schema,
            version,
            synced_at,
        });
    }
    Ok(())
}

// ============================================================
//  Remote Agent Registry (ADR-2026-04-05-0900 P4.1)
//
//  Replaces in-memory RemoteRegistryAdapter with SpacetimeDB-backed state.
//  Dashboard subscribes to this table for real-time fleet visibility.
//  Agents on any host see the full fleet via WebSocket subscription.
// ============================================================

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
            name,
            host,
            project_dir,
            capabilities_json,
            tunnel_id,
            status: "online".to_string(),
            last_heartbeat: timestamp.clone(),
            connected_at: timestamp,
            ..existing
        });
    } else {
        ctx.db.remote_agent().insert(RemoteAgent {
            agent_id,
            name,
            host,
            project_dir,
            capabilities_json,
            tunnel_id,
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
    let agent = ctx
        .db
        .remote_agent()
        .agent_id()
        .find(&agent_id)
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
pub fn remove_remote_agent(ctx: &ReducerContext, agent_id: String) -> Result<(), String> {
    if !ctx.db.remote_agent().agent_id().delete(&agent_id) {
        return Err(format!("Remote agent '{}' not found", agent_id));
    }
    Ok(())
}

/// Update heartbeat timestamp and set status to "online" (P4.1 convenience reducer).
#[reducer]
pub fn update_remote_heartbeat(
    ctx: &ReducerContext,
    agent_id: String,
    timestamp: String,
) -> Result<(), String> {
    let agent = ctx
        .db
        .remote_agent()
        .agent_id()
        .find(&agent_id)
        .ok_or_else(|| format!("Remote agent '{}' not found", agent_id))?;
    ctx.db.remote_agent().agent_id().update(RemoteAgent {
        status: "online".to_string(),
        last_heartbeat: timestamp,
        ..agent
    });
    Ok(())
}

/// Update only the status of a remote agent (e.g. "busy", "stale", "dead").
#[reducer]
pub fn update_remote_status(
    ctx: &ReducerContext,
    agent_id: String,
    status: String,
) -> Result<(), String> {
    let agent = ctx
        .db
        .remote_agent()
        .agent_id()
        .find(&agent_id)
        .ok_or_else(|| format!("Remote agent '{}' not found", agent_id))?;
    ctx.db
        .remote_agent()
        .agent_id()
        .update(RemoteAgent { status, ..agent });
    Ok(())
}

/// Delete a remote agent row (alias used by P4.1 fleet management).
#[reducer]
pub fn deregister_remote_agent(ctx: &ReducerContext, agent_id: String) -> Result<(), String> {
    if !ctx.db.remote_agent().agent_id().delete(&agent_id) {
        return Err(format!("Remote agent '{}' not found", agent_id));
    }
    log::info!("Remote agent deregistered: {}", agent_id);
    Ok(())
}

/// Log remote agents for a given host (enables subscription filtering).
#[reducer]
pub fn list_remote_agents_by_host(ctx: &ReducerContext, host: String) {
    let agents: Vec<RemoteAgent> = ctx
        .db
        .remote_agent()
        .iter()
        .filter(|a| a.host == host)
        .collect();
    log::info!("Host '{}' has {} remote agent(s)", host, agents.len());
    for agent in &agents {
        log::info!(
            "  agent={} name={} status={}",
            agent.agent_id,
            agent.name,
            agent.status
        );
    }
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
        ctx.db
            .inference_server()
            .server_id()
            .update(InferenceServer {
                name,
                host,
                provider,
                models_json,
                status: "online".to_string(),
                last_health_check: timestamp,
                ..existing
            });
    } else {
        ctx.db.inference_server().insert(InferenceServer {
            server_id,
            name,
            host,
            provider,
            models_json,
            status: "online".to_string(),
            last_health_check: timestamp.clone(),
            registered_at: timestamp,
        });
    }
    Ok(())
}

/// Remove an inference server.
#[reducer]
pub fn remove_inference_server(ctx: &ReducerContext, server_id: String) -> Result<(), String> {
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

    let agents: Vec<String> = ctx
        .db
        .hex_agent()
        .iter()
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
    let notif = ctx
        .db
        .agent_inbox()
        .id()
        .find(notification_id)
        .ok_or_else(|| format!("Notification '{}' not found", notification_id))?;

    if notif.agent_id != agent_id {
        return Err(format!(
            "Agent '{}' is not the target of notification '{}'",
            agent_id, notification_id
        ));
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
    let expired: Vec<AgentInbox> = ctx
        .db
        .agent_inbox()
        .iter()
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

    // ADR-2026-03-24-1900: enforce 1:1 agent↔swarm ownership.
    // An agent may not own more than one active swarm at a time.
    if !created_by.is_empty() {
        let already_owns = ctx
            .db
            .swarm()
            .iter()
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
pub fn swarm_complete(ctx: &ReducerContext, id: String, timestamp: String) -> Result<(), String> {
    let existing = ctx
        .db
        .swarm()
        .id()
        .find(&id)
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
    let existing = ctx
        .db
        .swarm()
        .id()
        .find(&id)
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
    let swarm = ctx
        .db
        .swarm()
        .id()
        .find(&swarm_id)
        .ok_or_else(|| format!("Swarm '{}' not found", swarm_id))?;

    if swarm.status != "active" {
        return Err(format!(
            "Swarm '{}' is not active (status: {})",
            swarm_id, swarm.status
        ));
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

/// Assign a task to an agent using Compare-And-Swap (ADR-2026-03-24-1900).
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
    let task = ctx
        .db
        .swarm_task()
        .id()
        .find(&task_id)
        .ok_or_else(|| format!("Task '{}' not found", task_id))?;

    // CAS version check (skip if caller passes u64::MAX)
    if expected_version != u64::MAX && task.version != expected_version {
        return Err(format!(
            "version_mismatch:{}:{}",
            expected_version, task.version
        ));
    }

    if task.status != "pending" {
        return Err(format!("already_claimed:{}", task.claimed_by));
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
        let name = ctx
            .db
            .hex_agent()
            .id()
            .find(&agent_id)
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

/// Transfer swarm ownership to a new agent (ADR-2026-03-24-1900).
/// Only the current owner or a call with no owner set may transfer.
#[reducer]
pub fn swarm_transfer(
    ctx: &ReducerContext,
    swarm_id: String,
    new_owner_agent_id: String,
    timestamp: String,
) -> Result<(), String> {
    let swarm = ctx
        .db
        .swarm()
        .id()
        .find(&swarm_id)
        .ok_or_else(|| format!("Swarm '{}' not found", swarm_id))?;

    if swarm.status != "active" {
        return Err(format!(
            "Swarm '{}' is not active — cannot transfer",
            swarm_id
        ));
    }

    // Verify new owner exists
    let new_owner = ctx
        .db
        .hex_agent()
        .id()
        .find(&new_owner_agent_id)
        .ok_or_else(|| format!("New owner agent '{}' not found", new_owner_agent_id))?;

    // New owner must not already own an active swarm
    let already_owns = ctx.db.swarm().iter().any(|s| {
        s.owner_agent_id == new_owner_agent_id && s.status == "active" && s.id != swarm_id
    });
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
    let task = ctx
        .db
        .swarm_task()
        .id()
        .find(&task_id)
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
    let task = ctx
        .db
        .swarm_task()
        .id()
        .find(&task_id)
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
pub fn task_reclaim(ctx: &ReducerContext, agent_id: String) -> Result<(), String> {
    let tasks: Vec<SwarmTask> = ctx
        .db
        .swarm_task()
        .iter()
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
//  Inference Task Reducers
// ============================================================

/// Create a new inference task with status=Pending.
#[reducer]
pub fn inference_task_create(
    ctx: &ReducerContext,
    id: String,
    workplan_id: String,
    task_id: String,
    phase: String,
    prompt: String,
    role: String,
    timestamp: String,
) -> Result<(), String> {
    if ctx.db.inference_task().id().find(&id).is_some() {
        return Err(format!("InferenceTask '{}' already exists", id));
    }

    ctx.db.inference_task().insert(InferenceTask {
        id,
        workplan_id,
        task_id,
        phase,
        prompt,
        role,
        status: "Pending".to_string(),
        agent_id: String::new(),
        result: String::new(),
        error: String::new(),
        created_at: timestamp.clone(),
        updated_at: timestamp,
    });

    Ok(())
}

/// Claim an inference task: CAS Pending→InProgress.
/// Returns Err("already_claimed:<agent_id>") if not Pending.
#[reducer]
pub fn inference_task_claim(
    ctx: &ReducerContext,
    id: String,
    agent_id: String,
    timestamp: String,
) -> Result<(), String> {
    let task = ctx
        .db
        .inference_task()
        .id()
        .find(&id)
        .ok_or_else(|| format!("InferenceTask '{}' not found", id))?;

    if task.status != "Pending" {
        return Err(format!("already_claimed:{}", task.agent_id));
    }

    ctx.db.inference_task().id().update(InferenceTask {
        status: "InProgress".to_string(),
        agent_id,
        updated_at: timestamp,
        ..task
    });

    Ok(())
}

/// Gate a Pending inference_task → PendingReview so workers cannot claim
/// it until the operator approves. Used by the brain-chat auto-followup
/// path: every code-touching task gets human approval before running.
/// CAS-checked: only Pending tasks can be gated; rejecting Completed /
/// InProgress / Failed prevents accidental state corruption.
#[reducer]
pub fn inference_task_gate(
    ctx: &ReducerContext,
    id: String,
    timestamp: String,
) -> Result<(), String> {
    let task = ctx
        .db
        .inference_task()
        .id()
        .find(&id)
        .ok_or_else(|| format!("InferenceTask '{}' not found", id))?;

    if task.status != "Pending" {
        return Err(format!("cannot_gate: task is {}", task.status));
    }

    ctx.db.inference_task().id().update(InferenceTask {
        status: "PendingReview".to_string(),
        updated_at: timestamp,
        ..task
    });

    Ok(())
}

/// Promote a PendingReview inference_task to Pending so workers can claim it.
/// Used by the brain-dispatch surface when an operator approves a dispatch
/// whose brief touched a critical-path token. CAS-checks the current status
/// so a stray promote on a Completed/Failed task is a no-op error rather than
/// a state corruption.
#[reducer]
pub fn inference_task_promote(
    ctx: &ReducerContext,
    id: String,
    timestamp: String,
) -> Result<(), String> {
    let task = ctx
        .db
        .inference_task()
        .id()
        .find(&id)
        .ok_or_else(|| format!("InferenceTask '{}' not found", id))?;

    if task.status != "PendingReview" {
        return Err(format!("cannot_promote: task is {}", task.status));
    }

    ctx.db.inference_task().id().update(InferenceTask {
        status: "Pending".to_string(),
        updated_at: timestamp,
        ..task
    });

    Ok(())
}

/// Mark an inference task as completed with a result.
#[reducer]
pub fn inference_task_complete(
    ctx: &ReducerContext,
    id: String,
    result: String,
    timestamp: String,
) -> Result<(), String> {
    let task = ctx
        .db
        .inference_task()
        .id()
        .find(&id)
        .ok_or_else(|| format!("InferenceTask '{}' not found", id))?;

    ctx.db.inference_task().id().update(InferenceTask {
        status: "Completed".to_string(),
        result,
        updated_at: timestamp,
        ..task
    });

    Ok(())
}

/// Mark an inference task as failed with an error message.
#[reducer]
pub fn inference_task_fail(
    ctx: &ReducerContext,
    id: String,
    error: String,
    timestamp: String,
) -> Result<(), String> {
    let task = ctx
        .db
        .inference_task()
        .id()
        .find(&id)
        .ok_or_else(|| format!("InferenceTask '{}' not found", id))?;

    // ADR-2026-04-24-1630: sanitize empty error strings - never store empty
    let sanitized_error = if error.trim().is_empty() {
        "unknown error".to_string()
    } else {
        error
    };

    ctx.db.inference_task().id().update(InferenceTask {
        status: "Failed".to_string(),
        error: sanitized_error,
        updated_at: timestamp,
        ..task
    });

    Ok(())
}

// ============================================================
//  Workplan event log — ADR-2026-04-27-1000 §2 (state-model-v2)
// ============================================================
//
// Append-only event log for workplan task state transitions. Replaces the
// mutable `status` field on workplan tasks: every transition becomes a row
// here, and current state is a fold over the log. The JSON workplan file
// becomes a projection rebuildable from this table (see `hex plan project`,
// `hex plan replay`).
//
// Caller-supplied `id` (UUID v4 string) follows the pattern set by
// `inference_task` — SpacetimeDB reducers can only return Result<(), String>,
// so the caller already knows the id it submitted. `kind` is a string keyed
// to the accepted-set defined below; validated by the reducer rather than a
// Rust enum to keep the WASM boundary text-only.

/// Workplan-event kind: Dispatched | AgentStopped | EvidenceChecked |
/// GateRun | Demoted | ManualMark | Migrated.
pub const WORKPLAN_EVENT_KINDS: &[&str] = &[
    "Dispatched",
    "AgentStopped",
    "EvidenceChecked",
    "GateRun",
    "Demoted",
    "ManualMark",
    "Migrated",
];

/// Append-only event row for workplan task state transitions.
/// One row per transition — current `is_done(task)` is a fold over rows
/// matching (workplan_id, task_id) plus on-disk evidence.
#[table(name = workplan_event, public)]
#[derive(Clone, Debug)]
pub struct WorkplanEvent {
    /// Caller-supplied UUID (v4 recommended). Unique across the table.
    #[primary_key]
    pub id: String,
    pub workplan_id: String,
    pub task_id: String,
    /// One of WORKPLAN_EVENT_KINDS — validated on append.
    pub kind: String,
    /// RFC3339 timestamp string, matching the rest of this module.
    pub occurred_at: String,
    /// Writer identity, e.g. "executor:nexus@host", "reconcile:cli",
    /// "human:gary", "migrate:v1-snapshot".
    pub actor: String,
    /// Kind-specific JSON payload, serialized as a string at the WASM
    /// boundary (SpacetimeDB has no native jsonb type). Empty string is
    /// treated as "no payload".
    pub payload: String,
}

/// Append a workplan event. Validates `id` is non-empty and unique, and
/// `kind` is in the accepted set. Returns `Ok(())` — caller already holds
/// the id it submitted.
#[reducer]
pub fn workplan_event_append(
    ctx: &ReducerContext,
    id: String,
    workplan_id: String,
    task_id: String,
    kind: String,
    occurred_at: String,
    actor: String,
    payload: String,
) -> Result<(), String> {
    if id.trim().is_empty() {
        return Err("workplan_event id must be non-empty".to_string());
    }
    if workplan_id.trim().is_empty() {
        return Err("workplan_event workplan_id must be non-empty".to_string());
    }
    if !WORKPLAN_EVENT_KINDS.iter().any(|k| *k == kind) {
        return Err(format!(
            "workplan_event kind '{}' not in accepted set: {:?}",
            kind, WORKPLAN_EVENT_KINDS
        ));
    }
    if ctx.db.workplan_event().id().find(&id).is_some() {
        return Err(format!("WorkplanEvent '{}' already exists", id));
    }

    ctx.db.workplan_event().insert(WorkplanEvent {
        id,
        workplan_id,
        task_id,
        kind,
        occurred_at,
        actor,
        payload,
    });

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
pub fn agent_heartbeat(ctx: &ReducerContext, id: String, timestamp: String) -> Result<(), String> {
    let agent = ctx
        .db
        .swarm_agent()
        .id()
        .find(&id)
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
pub fn agent_mark_stale(ctx: &ReducerContext, threshold_timestamp: String) -> Result<(), String> {
    let stale: Vec<SwarmAgent> = ctx
        .db
        .swarm_agent()
        .iter()
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
pub fn agent_mark_dead(ctx: &ReducerContext, threshold_timestamp: String) -> Result<(), String> {
    let dead: Vec<SwarmAgent> = ctx
        .db
        .swarm_agent()
        .iter()
        .filter(|a| a.status == "stale" && a.last_heartbeat < threshold_timestamp)
        .collect();

    for agent in dead {
        // Reclaim tasks from this dead agent
        let orphaned: Vec<SwarmTask> = ctx
            .db
            .swarm_task()
            .iter()
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
pub fn agent_remove(ctx: &ReducerContext, id: String) -> Result<(), String> {
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
pub fn memory_delete(ctx: &ReducerContext, key: String) -> Result<(), String> {
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
    let gate = ctx
        .db
        .quality_gate_task()
        .id()
        .find(&id)
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
    if ctx
        .db
        .quality_gate_task()
        .id()
        .find(&gate_task_id)
        .is_none()
    {
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
    let fix = ctx
        .db
        .fix_task()
        .id()
        .find(&id)
        .ok_or_else(|| format!("FixTask '{}' not found", id))?;

    if status != "completed" && status != "failed" {
        return Err(format!(
            "Status must be 'completed' or 'failed', got '{}'",
            status
        ));
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
pub fn memory_clear_scope(ctx: &ReducerContext, scope: String) -> Result<(), String> {
    let to_delete: Vec<HexFloMemory> = ctx
        .db
        .hexflo_memory()
        .iter()
        .filter(|m| m.scope == scope)
        .collect();

    for entry in to_delete {
        ctx.db.hexflo_memory().key().delete(&entry.key);
    }

    Ok(())
}

// ============================================================
//  Dev Session & Inference Log (ADR-2026-04-07-1300)
//  Tracks hex dev pipeline sessions with full audit trail.
//  Dashboard subscribes for real-time progress visibility.
// ============================================================

/// A hex dev session — the top-level aggregate for a pipeline run.
/// Links swarm tasks, quality gates, and inference calls together.
#[table(name = dev_session, public)]
#[derive(Clone, Debug)]
pub struct DevSession {
    #[primary_key]
    pub id: String,
    pub project_id: String,
    pub feature_description: String,
    /// "pending", "adr", "workplan", "scaffold", "code", "validate", "completed", "failed", "paused"
    pub status: String,
    pub current_phase: String,
    pub model: String,
    pub provider: String,
    pub adr_path: String,
    pub workplan_path: String,
    pub swarm_id: String,
    pub output_dir: String,
    pub agent_id: String,
    pub total_tokens: u64,
    /// Cost stored as string for WASM f64 compatibility
    pub total_cost_usd: String,
    pub architecture_grade: String,
    pub architecture_score: u32,
    /// Comma-separated completed step IDs
    pub completed_steps: String,
    /// Comma-separated objective verdicts: "CodeGenerated:pass,CodeCompiles:pass,..."
    pub objective_results: String,
    pub created_at: String,
    pub updated_at: String,
}

/// Per-inference-call audit log entry linked to a dev session.
#[table(name = inference_log, public)]
#[derive(Clone, Debug)]
pub struct InferenceLog {
    #[primary_key]
    pub id: String,
    pub session_id: String,
    pub phase: String,
    pub model: String,
    pub provider: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Cost stored as string for WASM f64 compatibility
    pub cost_usd: String,
    pub duration_ms: u64,
    /// Context window size of the model
    pub context_window: u64,
    /// What was generated: file path, ADR path, workplan path
    pub artifact: String,
    /// "ok", "error"
    pub status: String,
    pub created_at: String,
}

// ── Dev Session Reducers ────────────────────────────────────

#[reducer]
pub fn session_create(
    ctx: &ReducerContext,
    id: String,
    project_id: String,
    feature_description: String,
    model: String,
    provider: String,
    agent_id: String,
    timestamp: String,
) -> Result<(), String> {
    if id.is_empty() {
        return Err("session id required".into());
    }
    if ctx.db.dev_session().id().find(&id).is_some() {
        return Err(format!("session {} already exists", id));
    }
    ctx.db.dev_session().insert(DevSession {
        id,
        project_id,
        feature_description,
        status: "pending".into(),
        current_phase: "adr".into(),
        model,
        provider,
        adr_path: String::new(),
        workplan_path: String::new(),
        swarm_id: String::new(),
        output_dir: String::new(),
        agent_id,
        total_tokens: 0,
        total_cost_usd: "0.0".into(),
        architecture_grade: String::new(),
        architecture_score: 0,
        completed_steps: String::new(),
        objective_results: String::new(),
        created_at: timestamp.clone(),
        updated_at: timestamp,
    });
    Ok(())
}

#[reducer]
pub fn session_update_phase(
    ctx: &ReducerContext,
    id: String,
    phase: String,
    timestamp: String,
) -> Result<(), String> {
    let session = ctx
        .db
        .dev_session()
        .id()
        .find(&id)
        .ok_or_else(|| format!("session {} not found", id))?;
    ctx.db.dev_session().id().update(DevSession {
        status: phase.clone(),
        current_phase: phase,
        updated_at: timestamp,
        ..session
    });
    Ok(())
}

#[reducer]
pub fn session_complete_step(
    ctx: &ReducerContext,
    id: String,
    step_id: String,
    timestamp: String,
) -> Result<(), String> {
    let session = ctx
        .db
        .dev_session()
        .id()
        .find(&id)
        .ok_or_else(|| format!("session {} not found", id))?;
    let mut steps = session.completed_steps.clone();
    if !steps.is_empty() {
        steps.push(',');
    }
    steps.push_str(&step_id);
    ctx.db.dev_session().id().update(DevSession {
        completed_steps: steps,
        updated_at: timestamp,
        ..session
    });
    Ok(())
}

#[reducer]
pub fn session_set_quality(
    ctx: &ReducerContext,
    id: String,
    grade: String,
    score: u32,
    objectives: String,
    total_tokens: u64,
    total_cost_usd: String,
    timestamp: String,
) -> Result<(), String> {
    let session = ctx
        .db
        .dev_session()
        .id()
        .find(&id)
        .ok_or_else(|| format!("session {} not found", id))?;
    ctx.db.dev_session().id().update(DevSession {
        architecture_grade: grade,
        architecture_score: score,
        objective_results: objectives,
        total_tokens,
        total_cost_usd,
        updated_at: timestamp,
        ..session
    });
    Ok(())
}

#[reducer]
pub fn session_finalize(
    ctx: &ReducerContext,
    id: String,
    status: String,
    timestamp: String,
) -> Result<(), String> {
    let session = ctx
        .db
        .dev_session()
        .id()
        .find(&id)
        .ok_or_else(|| format!("session {} not found", id))?;
    ctx.db.dev_session().id().update(DevSession {
        status,
        updated_at: timestamp,
        ..session
    });
    Ok(())
}

#[reducer]
pub fn session_set_paths(
    ctx: &ReducerContext,
    id: String,
    adr_path: String,
    workplan_path: String,
    swarm_id: String,
    output_dir: String,
    timestamp: String,
) -> Result<(), String> {
    let session = ctx
        .db
        .dev_session()
        .id()
        .find(&id)
        .ok_or_else(|| format!("session {} not found", id))?;
    ctx.db.dev_session().id().update(DevSession {
        adr_path,
        workplan_path,
        swarm_id,
        output_dir,
        updated_at: timestamp,
        ..session
    });
    Ok(())
}

// ── Inference Log Reducers ──────────────────────────────────

#[reducer]
pub fn inference_log_create(
    ctx: &ReducerContext,
    id: String,
    session_id: String,
    phase: String,
    model: String,
    provider: String,
    input_tokens: u64,
    output_tokens: u64,
    cost_usd: String,
    duration_ms: u64,
    context_window: u64,
    artifact: String,
    status: String,
    timestamp: String,
) -> Result<(), String> {
    ctx.db.inference_log().insert(InferenceLog {
        id,
        session_id,
        phase,
        model,
        provider,
        input_tokens,
        output_tokens,
        cost_usd,
        duration_ms,
        context_window,
        artifact,
        status,
        created_at: timestamp,
    });
    Ok(())
}

// ============================================================
//  Enforcement Rules (ADR-2026-03-22-1959 P5)
// ============================================================

/// An enforcement rule — persisted in SpacetimeDB, synced from .hex/ADR-rules.toml.
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
    let rule = ctx
        .db
        .enforcement_rule()
        .id()
        .find(&id)
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
pub fn enforcement_rule_delete(ctx: &ReducerContext, id: String) -> Result<(), String> {
    ctx.db.enforcement_rule().id().delete(&id);
    Ok(())
}

/// Combined cleanup reducer called by hex-nexus periodically (ADR-042).
///
/// `cutoff` is an RFC3339 timestamp — agents and notifications older than
/// this value are marked stale/dead/expired. Inlines agent_mark_stale,
/// agent_mark_dead, and expire_stale_notifications in a single transaction.
///
/// Logs each run to `cleanup_log` when any work is done (absorbed from
/// hexflo-cleanup module).
#[reducer]
pub fn coordination_cleanup(ctx: &ReducerContext, cutoff: String) -> Result<(), String> {
    let mut stale_count: u32 = 0;
    let mut dead_count: u32 = 0;
    let mut reclaimed_tasks: u32 = 0;
    let mut expired_notifications: u32 = 0;

    // 1. Mark active swarm agents as stale if their heartbeat is before the cutoff.
    let stale: Vec<SwarmAgent> = ctx
        .db
        .swarm_agent()
        .iter()
        .filter(|a| a.status == "active" && a.last_heartbeat < cutoff)
        .collect();
    for agent in stale {
        ctx.db.swarm_agent().id().update(SwarmAgent {
            status: "stale".to_string(),
            ..agent
        });
        stale_count += 1;
    }

    // 2. Mark stale swarm agents as dead and reclaim their in-progress tasks.
    let dead: Vec<SwarmAgent> = ctx
        .db
        .swarm_agent()
        .iter()
        .filter(|a| a.status == "stale" && a.last_heartbeat < cutoff)
        .collect();
    for agent in dead {
        let orphaned: Vec<SwarmTask> = ctx
            .db
            .swarm_task()
            .iter()
            .filter(|t| t.agent_id == agent.id && t.status == "in_progress")
            .collect();
        for task in orphaned {
            ctx.db.swarm_task().id().update(SwarmTask {
                status: "pending".to_string(),
                agent_id: String::new(),
                ..task
            });
            reclaimed_tasks += 1;
        }
        ctx.db.swarm_agent().id().update(SwarmAgent {
            status: "dead".to_string(),
            ..agent
        });
        dead_count += 1;
    }

    // 3. Also mark hex_agent entries as stale (unified agent registry).
    //    Normalize Z → +00:00 for consistent RFC3339 string comparison.
    let cutoff_normalized = cutoff.replace('Z', "+00:00");
    let hex_agents: Vec<HexAgent> = ctx
        .db
        .hex_agent()
        .iter()
        .filter(|a| a.status == "online" || a.status == "idle" || a.status == "stale")
        .collect();
    for agent in hex_agents {
        let hb = agent.last_heartbeat.replace('Z', "+00:00");
        if hb < cutoff_normalized && (agent.status == "online" || agent.status == "idle") {
            ctx.db.hex_agent().id().update(HexAgent {
                status: "stale".to_string(),
                ..agent
            });
            stale_count += 1;
        }
    }

    // 4. Expire unacknowledged inbox notifications older than the cutoff.
    let expired: Vec<AgentInbox> = ctx
        .db
        .agent_inbox()
        .iter()
        .filter(|n| {
            n.acknowledged_at.is_empty() && n.expired_at.is_empty() && n.created_at < cutoff
        })
        .collect();
    for notif in expired {
        ctx.db.agent_inbox().id().update(AgentInbox {
            expired_at: cutoff.clone(),
            ..notif
        });
        expired_notifications += 1;
    }

    // 5. Log the cleanup run if any work was done (absorbed from hexflo-cleanup).
    if stale_count > 0 || dead_count > 0 || reclaimed_tasks > 0 || expired_notifications > 0 {
        ctx.db.cleanup_log().insert(CleanupLog {
            id: 0, // auto_inc
            ran_at: cutoff.clone(),
            stale_count,
            dead_count,
            reclaimed_tasks,
            expired_notifications,
        });
        log::info!(
            "coordination_cleanup: stale={}, dead={}, reclaimed={}, expired_notifs={}",
            stale_count,
            dead_count,
            reclaimed_tasks,
            expired_notifications
        );
    }

    Ok(())
}

// ─── Cleanup reducers absorbed from hexflo-cleanup ──────────────────────────

/// Remove a dead swarm agent from tracking entirely.
/// Only removes agents with status "dead" — active/stale agents are preserved.
/// Absorbed from hexflo-cleanup's `remove_dead_agent` reducer.
#[reducer]
pub fn remove_dead_swarm_agent(ctx: &ReducerContext, agent_id: String) -> Result<(), String> {
    if let Some(agent) = ctx.db.swarm_agent().id().find(&agent_id) {
        if agent.status == "dead" {
            ctx.db.swarm_agent().id().delete(&agent_id);
            Ok(())
        } else {
            Err(format!(
                "Agent '{}' is not dead (status: '{}') — only dead agents can be removed",
                agent_id, agent.status
            ))
        }
    } else {
        Err(format!("Swarm agent '{}' not found", agent_id))
    }
}

/// Manual trigger for a cleanup pass — delegates to coordination_cleanup.
/// Absorbed from hexflo-cleanup's `trigger_cleanup` reducer for use by
/// the hex-nexus REST API (POST /api/hexflo/cleanup).
#[reducer]
pub fn trigger_cleanup(ctx: &ReducerContext, cutoff: String) -> Result<(), String> {
    coordination_cleanup(ctx, cutoff)
}

// ─── Architecture Fingerprint ──────────────────────────────────────────────
//
// ADR-2026-03-30-1200: Token-efficient architecture context injected into every
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
    if ctx
        .db
        .architecture_fingerprint()
        .project_id()
        .find(&project_id)
        .is_some()
    {
        ctx.db.architecture_fingerprint().project_id().update(fp);
    } else {
        ctx.db.architecture_fingerprint().insert(fp);
    }
    Ok(())
}

/// Remove a fingerprint when a project is deleted or reset.
#[reducer]
pub fn delete_fingerprint(ctx: &ReducerContext, project_id: String) -> Result<(), String> {
    if ctx
        .db
        .architecture_fingerprint()
        .project_id()
        .find(&project_id)
        .is_some()
    {
        ctx.db
            .architecture_fingerprint()
            .project_id()
            .delete(&project_id);
        Ok(())
    } else {
        Err(format!("No fingerprint found for project '{}'", project_id))
    }
}

// ============================================================
//  Fleet State (absorbed from fleet-state module — ADR-2026-04-05-0900)
// ============================================================

/// A compute node in the fleet — tracks capacity for multi-host agent dispatch.
#[table(name = compute_node, public)]
#[derive(Clone, Debug)]
pub struct ComputeNode {
    #[unique]
    pub id: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub status: String,
    pub max_agents: u32,
    pub active_agents: u32,
    pub last_health_check: String,
}

#[reducer]
pub fn register_node(
    ctx: &ReducerContext,
    id: String,
    host: String,
    port: u16,
    username: String,
    max_agents: u32,
) -> Result<(), String> {
    ctx.db.compute_node().insert(ComputeNode {
        id,
        host,
        port,
        username,
        status: "online".to_string(),
        max_agents,
        active_agents: 0,
        last_health_check: String::new(),
    });
    Ok(())
}

#[reducer]
pub fn update_node_health(ctx: &ReducerContext, id: String, status: String) -> Result<(), String> {
    match ctx.db.compute_node().id().find(&id) {
        Some(old) => {
            let updated = ComputeNode {
                status,
                last_health_check: String::new(),
                ..old
            };
            ctx.db.compute_node().id().update(updated);
        }
        None => {
            return Err(format!("Node '{}' not found", id));
        }
    }
    Ok(())
}

#[reducer]
pub fn increment_node_agents(ctx: &ReducerContext, id: String) -> Result<(), String> {
    match ctx.db.compute_node().id().find(&id) {
        Some(old) => {
            if old.active_agents >= old.max_agents {
                return Err(format!(
                    "Node '{}' at capacity ({}/{})",
                    id, old.active_agents, old.max_agents
                ));
            }
            let updated = ComputeNode {
                active_agents: old.active_agents + 1,
                ..old
            };
            ctx.db.compute_node().id().update(updated);
        }
        None => {
            return Err(format!("Node '{}' not found", id));
        }
    }
    Ok(())
}

#[reducer]
pub fn decrement_node_agents(ctx: &ReducerContext, id: String) -> Result<(), String> {
    match ctx.db.compute_node().id().find(&id) {
        Some(old) => {
            let updated = ComputeNode {
                active_agents: old.active_agents.saturating_sub(1),
                ..old
            };
            ctx.db.compute_node().id().update(updated);
        }
        None => {
            return Err(format!("Node '{}' not found", id));
        }
    }
    Ok(())
}

#[reducer]
pub fn remove_node(ctx: &ReducerContext, id: String) -> Result<(), String> {
    let deleted = ctx.db.compute_node().id().delete(&id);
    if !deleted {
        return Err(format!("Node '{}' not found", id));
    }
    Ok(())
}

// ============================================================
//  Lifecycle Events (absorbed from hexflo-lifecycle module)
//
//  ADR-039 Phase 8: Automatic swarm lifecycle management.
//  When a task completes, these reducers check if the entire tier
//  is done. If so, the swarm advances to the next phase and
//  unblocks dependent tasks.
//
//  Phase progression: SPECS → PLAN → CODE → VALIDATE → INTEGRATE → COMPLETE
// ============================================================

// ── Lifecycle Tables ───────────────────────────────────────

/// Tracks the current lifecycle phase of a swarm.
#[table(name = swarm_lifecycle, public)]
#[derive(Clone, Debug)]
pub struct SwarmLifecycle {
    #[primary_key]
    pub swarm_id: String,
    pub name: String,
    pub phase: String, // "specs", "plan", "code", "validate", "integrate", "complete"
    pub phase_index: u32, // 0-5
    pub total_tasks: u32,
    pub completed_tasks: u32,
    pub status: String, // "active", "completed", "failed"
    pub updated_at: String,
}

/// A task tracked for lifecycle phase progression.
#[table(name = lifecycle_task, public)]
#[derive(Clone, Debug)]
pub struct LifecycleTask {
    #[primary_key]
    pub task_id: String,
    pub swarm_id: String,
    pub tier: u32,          // 0-5, maps to phase
    pub status: String,     // "pending", "in_progress", "completed", "failed"
    pub depends_on: String, // comma-separated task IDs
    pub updated_at: String,
}

/// Event log for phase transitions.
#[table(name = phase_transition_log, public)]
#[derive(Clone, Debug)]
pub struct PhaseTransitionLog {
    #[primary_key]
    #[auto_inc]
    pub id: u64,
    pub swarm_id: String,
    pub from_phase: String,
    pub to_phase: String,
    pub triggered_by_task: String,
    pub transitioned_at: String,
}

// ── Lifecycle Constants ────────────────────────────────────

const LIFECYCLE_PHASES: &[&str] = &["specs", "plan", "code", "validate", "integrate", "complete"];

// ── Lifecycle Reducers ─────────────────────────────────────

/// Register a swarm for lifecycle tracking.
#[reducer]
pub fn lifecycle_register_swarm(
    ctx: &ReducerContext,
    swarm_id: String,
    name: String,
    total_tasks: u32,
    timestamp: String,
) {
    ctx.db.swarm_lifecycle().insert(SwarmLifecycle {
        swarm_id,
        name,
        phase: "specs".to_string(),
        phase_index: 0,
        total_tasks,
        completed_tasks: 0,
        status: "active".to_string(),
        updated_at: timestamp,
    });
}

/// Register a task for lifecycle tracking.
#[reducer]
pub fn lifecycle_register_task(
    ctx: &ReducerContext,
    task_id: String,
    swarm_id: String,
    tier: u32,
    depends_on: String,
    timestamp: String,
) {
    ctx.db.lifecycle_task().insert(LifecycleTask {
        task_id,
        swarm_id,
        tier,
        status: "pending".to_string(),
        depends_on,
        updated_at: timestamp,
    });
}

/// Called when a task completes. Triggers phase transition check.
///
/// This is the core trigger: when the last task in a tier completes,
/// the swarm advances to the next phase, and tasks in the next tier
/// become unblocked.
#[reducer]
pub fn lifecycle_on_task_complete(
    ctx: &ReducerContext,
    task_id: String,
    swarm_id: String,
    timestamp: String,
) {
    // Update the task status
    if let Some(mut task) = ctx.db.lifecycle_task().task_id().find(&task_id) {
        task.status = "completed".to_string();
        task.updated_at = timestamp.clone();
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

    let tier_done = !tier_tasks.is_empty() && tier_tasks.iter().all(|t| t.status == "completed");

    if tier_done && (current_tier as usize) < LIFECYCLE_PHASES.len() - 1 {
        // Advance to next phase
        let old_phase = swarm.phase.clone();
        let new_index = current_tier + 1;
        let new_phase = LIFECYCLE_PHASES[new_index as usize].to_string();

        swarm.phase = new_phase.clone();
        swarm.phase_index = new_index;
        swarm.updated_at = timestamp.clone();

        // Log the transition
        ctx.db.phase_transition_log().insert(PhaseTransitionLog {
            id: 0, // auto_inc
            swarm_id: swarm_id.clone(),
            from_phase: old_phase,
            to_phase: new_phase,
            triggered_by_task: task_id,
            transitioned_at: timestamp,
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

    swarm.updated_at = swarm.updated_at.clone();
    ctx.db.swarm_lifecycle().swarm_id().update(swarm);
}

/// Called when a task fails. Marks swarm as failed if critical.
#[reducer]
pub fn lifecycle_on_task_fail(
    ctx: &ReducerContext,
    task_id: String,
    swarm_id: String,
    timestamp: String,
) {
    if let Some(mut task) = ctx.db.lifecycle_task().task_id().find(&task_id) {
        task.status = "failed".to_string();
        task.updated_at = timestamp.clone();
        ctx.db.lifecycle_task().task_id().update(task);
    }

    // Mark swarm as failed
    if let Some(mut swarm) = ctx.db.swarm_lifecycle().swarm_id().find(&swarm_id) {
        swarm.status = "failed".to_string();
        swarm.updated_at = timestamp;
        ctx.db.swarm_lifecycle().swarm_id().update(swarm);
    }
}

/// Check which tasks in the next tier are now unblocked.
/// Returns task IDs that have all dependencies satisfied.
#[reducer]
pub fn lifecycle_check_unblocked(ctx: &ReducerContext, swarm_id: String) {
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

// ─── Developer Decision Inbox (ADR-2026-04-13-1500) ──────────────────────────

/// A decision that requires developer input. hex surfaces these with a
/// recommended default action; if the developer doesn't respond before the
/// deadline, the default auto-applies.
#[table(name = developer_inbox, public)]
#[derive(Clone, Debug)]
pub struct DeveloperInbox {
    #[primary_key]
    #[auto_inc]
    pub id: u64,
    pub project_id: String,
    /// Type: "taste", "dependency", "architecture", "escalation", "budget"
    pub decision_type: String,
    pub title: String,
    pub description: String,
    /// What hex will do if the developer doesn't respond
    pub default_action: String,
    /// Why hex chose this default
    pub default_reason: String,
    /// JSON array of alternative actions
    pub alternatives: String,
    /// ISO timestamp — after this, default_action auto-applies
    pub deadline_at: String,
    pub auto_resolved: bool,
    /// What action was taken
    pub resolved_action: String,
    /// "human" or "auto"
    pub resolved_by: String,
    pub resolved_at: String,
    pub created_at: String,
}

/// Surface a decision to the developer inbox.
#[reducer]
pub fn surface_decision(
    ctx: &ReducerContext,
    project_id: String,
    decision_type: String,
    title: String,
    description: String,
    default_action: String,
    default_reason: String,
    alternatives: String,
    deadline_at: String,
    timestamp: String,
) -> Result<(), String> {
    let valid_types = [
        "taste",
        "dependency",
        "architecture",
        "escalation",
        "budget",
    ];
    if !valid_types.contains(&decision_type.as_str()) {
        return Err(format!(
            "Invalid decision_type '{}'. Must be one of: {}",
            decision_type,
            valid_types.join(", ")
        ));
    }

    ctx.db.developer_inbox().insert(DeveloperInbox {
        id: 0, // auto_inc
        project_id,
        decision_type,
        title,
        description,
        default_action,
        default_reason,
        alternatives,
        deadline_at,
        auto_resolved: false,
        resolved_action: String::new(),
        resolved_by: String::new(),
        resolved_at: String::new(),
        created_at: timestamp,
    });

    Ok(())
}

/// Resolve a decision (by human or programmatically).
#[reducer]
pub fn resolve_decision(
    ctx: &ReducerContext,
    id: u64,
    action: String,
    resolved_by: String,
    timestamp: String,
) -> Result<(), String> {
    let entry = ctx
        .db
        .developer_inbox()
        .id()
        .find(id)
        .ok_or_else(|| format!("Decision '{}' not found", id))?;

    if !entry.resolved_action.is_empty() {
        return Ok(()); // Already resolved — idempotent
    }

    ctx.db.developer_inbox().id().update(DeveloperInbox {
        resolved_action: action,
        resolved_by,
        resolved_at: timestamp,
        ..entry
    });

    Ok(())
}

/// Auto-resolve all decisions past their deadline with their default_action.
#[reducer]
pub fn expire_decisions(ctx: &ReducerContext, current_time: String) -> Result<(), String> {
    let expired: Vec<DeveloperInbox> = ctx
        .db
        .developer_inbox()
        .iter()
        .filter(|d| {
            d.resolved_action.is_empty()
                && !d.deadline_at.is_empty()
                && d.deadline_at < current_time
        })
        .collect();

    for entry in expired {
        let default_action = entry.default_action.clone();
        ctx.db.developer_inbox().id().update(DeveloperInbox {
            auto_resolved: true,
            resolved_action: default_action,
            resolved_by: "auto".to_string(),
            resolved_at: current_time.clone(),
            ..entry
        });
    }

    Ok(())
}

// ─── Delegation Trust Model (ADR-2026-04-13-1500) ─────────────────────────────

/// Per-scope trust level that controls how much autonomy hex has in a given
/// area. Trust can be elevated by consistent good outcomes and decayed when
/// something goes wrong.
#[table(name = delegation_trust, public)]
#[derive(Clone, Debug)]
pub struct DelegationTrust {
    #[primary_key]
    #[auto_inc]
    pub id: u64,
    pub project_id: String,
    /// Hierarchical scope like "domain", "adapters/secondary/stripe"
    pub scope: String,
    /// "observe", "suggest", "act", "silent"
    pub trust_level: String,
    pub last_elevated_at: String,
    pub last_decayed_at: String,
    pub decay_reason: String,
    /// If true, auto-decay is disabled for this scope
    pub pinned: bool,
}

/// Set (upsert) trust level for a project scope.
#[reducer]
pub fn set_trust(
    ctx: &ReducerContext,
    project_id: String,
    scope: String,
    level: String,
    timestamp: String,
) -> Result<(), String> {
    let valid_levels = ["observe", "suggest", "act", "silent"];
    if !valid_levels.contains(&level.as_str()) {
        return Err(format!(
            "Invalid trust level '{}'. Must be one of: {}",
            level,
            valid_levels.join(", ")
        ));
    }

    // Find existing entry for this project_id + scope
    let existing: Option<DelegationTrust> = ctx
        .db
        .delegation_trust()
        .iter()
        .find(|t| t.project_id == project_id && t.scope == scope);

    if let Some(old) = existing {
        ctx.db.delegation_trust().id().delete(old.id);
    }

    ctx.db.delegation_trust().insert(DelegationTrust {
        id: 0, // auto_inc
        project_id,
        scope,
        trust_level: level,
        last_elevated_at: timestamp,
        last_decayed_at: String::new(),
        decay_reason: String::new(),
        pinned: false,
    });

    Ok(())
}

/// Decay trust one level: silent → act → suggest → observe.
/// No-op if already at "observe" or if the scope is pinned.
#[reducer]
pub fn decay_trust(
    ctx: &ReducerContext,
    project_id: String,
    scope: String,
    reason: String,
    timestamp: String,
) -> Result<(), String> {
    let entry = ctx
        .db
        .delegation_trust()
        .iter()
        .find(|t| t.project_id == project_id && t.scope == scope)
        .ok_or_else(|| {
            format!(
                "No trust entry for project '{}' scope '{}'",
                project_id, scope
            )
        })?;

    if entry.pinned {
        return Ok(()); // Pinned — no decay
    }

    let new_level = match entry.trust_level.as_str() {
        "silent" => "act",
        "act" => "suggest",
        "suggest" => "observe",
        "observe" => return Ok(()), // Already at lowest — no-op
        other => return Err(format!("Unknown trust level '{}'", other)),
    };

    ctx.db.delegation_trust().id().update(DelegationTrust {
        trust_level: new_level.to_string(),
        last_decayed_at: timestamp,
        decay_reason: reason,
        ..entry
    });

    Ok(())
}

/// Pin a trust scope so it cannot be auto-decayed.
#[reducer]
pub fn pin_trust(ctx: &ReducerContext, project_id: String, scope: String) -> Result<(), String> {
    let entry = ctx
        .db
        .delegation_trust()
        .iter()
        .find(|t| t.project_id == project_id && t.scope == scope)
        .ok_or_else(|| {
            format!(
                "No trust entry for project '{}' scope '{}'",
                project_id, scope
            )
        })?;

    if entry.pinned {
        return Ok(()); // Already pinned — idempotent
    }

    ctx.db.delegation_trust().id().update(DelegationTrust {
        pinned: true,
        ..entry
    });

    Ok(())
}

/// Initialize default trust rows for a project (all scopes at "suggest").
#[reducer]
pub fn init_project_trust(
    ctx: &ReducerContext,
    project_id: String,
    timestamp: String,
) -> Result<(), String> {
    let default_scopes = [
        "domain",
        "ports",
        "adapters/primary",
        "adapters/secondary",
        "dependencies",
        "deployment",
    ];

    for scope in &default_scopes {
        // Skip if already exists
        let exists = ctx
            .db
            .delegation_trust()
            .iter()
            .any(|t| t.project_id == project_id && t.scope == *scope);
        if exists {
            continue;
        }

        ctx.db.delegation_trust().insert(DelegationTrust {
            id: 0, // auto_inc
            project_id: project_id.clone(),
            scope: scope.to_string(),
            trust_level: "suggest".to_string(),
            last_elevated_at: timestamp.clone(),
            last_decayed_at: String::new(),
            decay_reason: String::new(),
            pinned: false,
        });
    }

    Ok(())
}

// ─── Briefing Buffer (ADR-2026-04-13-1500) ────────────────────────────────────

/// Accumulated events for the developer briefing. Events are logged
/// continuously and consumed when the developer opens a session or asks
/// "what happened?"
#[table(name = briefing_buffer, public)]
#[derive(Clone, Debug)]
pub struct BriefingBuffer {
    #[primary_key]
    #[auto_inc]
    pub id: u64,
    pub project_id: String,
    /// "nominal", "notable", "decision", "critical"
    pub severity: String,
    /// "architecture", "swarm", "build", "inference", "anomaly"
    pub category: String,
    pub title: String,
    pub body: String,
    pub related_task_id: String,
    pub related_agent_id: String,
    pub seen: bool,
    pub created_at: String,
}

/// Log a briefing event.
#[reducer]
pub fn log_briefing_event(
    ctx: &ReducerContext,
    project_id: String,
    severity: String,
    category: String,
    title: String,
    body: String,
    related_task_id: String,
    related_agent_id: String,
    timestamp: String,
) -> Result<(), String> {
    let valid_severities = ["nominal", "notable", "decision", "critical"];
    if !valid_severities.contains(&severity.as_str()) {
        return Err(format!(
            "Invalid severity '{}'. Must be one of: {}",
            severity,
            valid_severities.join(", ")
        ));
    }

    let valid_categories = ["architecture", "swarm", "build", "inference", "anomaly"];
    if !valid_categories.contains(&category.as_str()) {
        return Err(format!(
            "Invalid category '{}'. Must be one of: {}",
            category,
            valid_categories.join(", ")
        ));
    }

    ctx.db.briefing_buffer().insert(BriefingBuffer {
        id: 0, // auto_inc
        project_id,
        severity,
        category,
        title,
        body,
        related_task_id,
        related_agent_id,
        seen: false,
        created_at: timestamp,
    });

    Ok(())
}

/// Mark a briefing event as seen.
#[reducer]
pub fn mark_briefing_seen(ctx: &ReducerContext, id: u64) -> Result<(), String> {
    let entry = ctx
        .db
        .briefing_buffer()
        .id()
        .find(id)
        .ok_or_else(|| format!("Briefing event '{}' not found", id))?;

    if entry.seen {
        return Ok(()); // Already seen — idempotent
    }

    ctx.db.briefing_buffer().id().update(BriefingBuffer {
        seen: true,
        ..entry
    });

    Ok(())
}

/// Archive (delete) old briefing events that have been seen.
#[reducer]
pub fn archive_old_briefings(ctx: &ReducerContext, cutoff_time: String) -> Result<(), String> {
    let to_delete: Vec<u64> = ctx
        .db
        .briefing_buffer()
        .iter()
        .filter(|b| b.seen && b.created_at < cutoff_time)
        .map(|b| b.id)
        .collect();

    for id in to_delete {
        ctx.db.briefing_buffer().id().delete(id);
    }

    Ok(())
}

// ============================================================
//  Substrate — swap-ticket + shadow-sample (ADR-2026-04-26-1500 P6, wp-substrate-shadow-promotion P1)
// ============================================================
//
// `swap_ticket` records every proposed swap of a port -> adapter binding in
// the runtime composition. `shadow_sample` records the per-call comparison
// between incumbent and candidate while a ticket is in `shadow` state. The
// promotion judge (hex-nexus, wp-substrate-shadow-promotion P4) reads
// `shadow_sample` rows for a ticket and transitions it to `shadow_green` /
// `shadow_red`. STDB-only — no SQLite path.
//
// State machine (enforced in `swap_ticket_transition`):
//   candidate     -> shadow
//   shadow        -> shadow_green | shadow_red
//   shadow_green  -> promoted
//   promoted      -> rolled_back
// All other transitions are rejected. Terminal states (shadow_red,
// rolled_back) cannot be re-opened.

/// A proposed swap of an adapter binding for a port. One row per ticket.
/// Fields that the substrate models as `Option<String>` (incumbent for the
/// first adapter on a port; shadow_started_at before shadow begins) are
/// stored as `""` and treated as absent — STDB favours flat scalars.
#[table(name = swap_ticket, public)]
#[derive(Clone, Debug)]
pub struct SwapTicket {
    #[primary_key]
    pub id: String,
    pub project_id: String,
    pub port_id: String,
    /// Empty string when there is no prior binding (first adapter on the port).
    pub incumbent_adapter_id: String,
    pub candidate_adapter_id: String,
    /// Serialized `AdapterManifest` from hex-core (JSON).
    pub candidate_manifest_json: String,
    /// One of: "candidate" | "shadow" | "shadow_green" | "shadow_red"
    /// | "promoted" | "rolled_back".
    pub state: String,
    pub shadow_traffic_fraction: f32,
    pub shadow_window_seconds: u64,
    /// RFC3339; empty string until `swap_ticket_set_shadow_started` is called.
    pub shadow_started_at: String,
    /// Serialized `Vec<SuccessCriterion>` from hex-core (JSON).
    pub success_criteria_json: String,
    pub created_at: String,
    pub updated_at: String,
}

/// One incumbent-vs-candidate comparison recorded by the shadow router.
#[table(name = shadow_sample, public)]
#[derive(Clone, Debug)]
pub struct ShadowSample {
    #[primary_key]
    #[auto_inc]
    pub id: u64,
    pub ticket_id: String,
    /// Monotonic per-ticket call sequence assigned by the router.
    pub call_seq: u64,
    pub incumbent_adapter_id: String,
    pub candidate_adapter_id: String,
    /// Serialized `PortTelemetry::Metrics` for the incumbent's call (JSON).
    pub incumbent_metrics_json: String,
    /// Serialized `PortTelemetry::Metrics` for the candidate's call (JSON).
    pub candidate_metrics_json: String,
    /// Judge's call on response equivalence for this pair.
    pub agreed: bool,
    /// Populated when `agreed=false`; empty string otherwise.
    pub reason: String,
    pub recorded_at: String,
}

const SWAP_STATE_CANDIDATE: &str = "candidate";
const SWAP_STATE_SHADOW: &str = "shadow";
const SWAP_STATE_SHADOW_GREEN: &str = "shadow_green";
const SWAP_STATE_SHADOW_RED: &str = "shadow_red";
const SWAP_STATE_PROMOTED: &str = "promoted";
const SWAP_STATE_ROLLED_BACK: &str = "rolled_back";

fn swap_state_transition_allowed(from: &str, to: &str) -> bool {
    matches!(
        (from, to),
        (SWAP_STATE_CANDIDATE, SWAP_STATE_SHADOW)
            | (SWAP_STATE_SHADOW, SWAP_STATE_SHADOW_GREEN)
            | (SWAP_STATE_SHADOW, SWAP_STATE_SHADOW_RED)
            | (SWAP_STATE_SHADOW_GREEN, SWAP_STATE_PROMOTED)
            | (SWAP_STATE_PROMOTED, SWAP_STATE_ROLLED_BACK)
    )
}

/// Create a new swap ticket in `candidate` state. Caller supplies the UUID
/// (STDB reducers don't return values cleanly; the caller already needs the
/// id to subscribe to the row).
#[reducer]
pub fn swap_ticket_create(
    ctx: &ReducerContext,
    id: String,
    project_id: String,
    port_id: String,
    incumbent_adapter_id: String,
    candidate_adapter_id: String,
    candidate_manifest_json: String,
    shadow_traffic_fraction: f32,
    shadow_window_seconds: u64,
    success_criteria_json: String,
    timestamp: String,
) -> Result<(), String> {
    if ctx.db.swap_ticket().id().find(&id).is_some() {
        return Err(format!("swap_ticket {} already exists", id));
    }
    if !(0.0..=1.0).contains(&shadow_traffic_fraction) {
        return Err(format!(
            "shadow_traffic_fraction {} out of range [0.0, 1.0]",
            shadow_traffic_fraction
        ));
    }
    ctx.db.swap_ticket().insert(SwapTicket {
        id,
        project_id,
        port_id,
        incumbent_adapter_id,
        candidate_adapter_id,
        candidate_manifest_json,
        state: SWAP_STATE_CANDIDATE.to_string(),
        shadow_traffic_fraction,
        shadow_window_seconds,
        shadow_started_at: String::new(),
        success_criteria_json,
        created_at: timestamp.clone(),
        updated_at: timestamp,
    });
    Ok(())
}

/// Move a ticket to a new state. Rejects transitions not in the allowed set.
#[reducer]
pub fn swap_ticket_transition(
    ctx: &ReducerContext,
    id: String,
    new_state: String,
    timestamp: String,
) -> Result<(), String> {
    let existing = ctx
        .db
        .swap_ticket()
        .id()
        .find(&id)
        .ok_or_else(|| format!("swap_ticket {} not found", id))?;
    if !swap_state_transition_allowed(&existing.state, &new_state) {
        return Err(format!(
            "swap_ticket {}: transition {} -> {} not allowed",
            id, existing.state, new_state
        ));
    }
    ctx.db.swap_ticket().id().update(SwapTicket {
        state: new_state,
        updated_at: timestamp,
        ..existing
    });
    Ok(())
}

/// Update the operator-configurable fields (success_criteria, traffic
/// fraction, window) on a non-terminal ticket. Allowed in candidate or
/// shadow state — operator may adjust mid-shadow before the judge ticks.
/// Rejected for terminal states (shadow_green/shadow_red/promoted/rolled_back).
#[reducer]
pub fn swap_ticket_set_config(
    ctx: &ReducerContext,
    id: String,
    success_criteria_json: String,
    shadow_traffic_fraction: f32,
    shadow_window_seconds: u64,
    timestamp: String,
) -> Result<(), String> {
    let existing = ctx
        .db
        .swap_ticket()
        .id()
        .find(&id)
        .ok_or_else(|| format!("swap_ticket {} not found", id))?;
    if !matches!(existing.state.as_str(), SWAP_STATE_CANDIDATE | SWAP_STATE_SHADOW) {
        return Err(format!(
            "swap_ticket {}: cannot update config in state {}",
            id, existing.state
        ));
    }
    if !(0.0..=1.0).contains(&shadow_traffic_fraction) {
        return Err(format!(
            "shadow_traffic_fraction {} out of range [0.0, 1.0]",
            shadow_traffic_fraction
        ));
    }
    ctx.db.swap_ticket().id().update(SwapTicket {
        success_criteria_json,
        shadow_traffic_fraction,
        shadow_window_seconds,
        updated_at: timestamp,
        ..existing
    });
    Ok(())
}

/// Stamp `shadow_started_at` on a ticket. Called when the shadow router
/// begins routing mirrored traffic — separate from the state transition so
/// the judge can compute "time in shadow" deterministically.
#[reducer]
pub fn swap_ticket_set_shadow_started(
    ctx: &ReducerContext,
    id: String,
    timestamp: String,
) -> Result<(), String> {
    let existing = ctx
        .db
        .swap_ticket()
        .id()
        .find(&id)
        .ok_or_else(|| format!("swap_ticket {} not found", id))?;
    if existing.state != SWAP_STATE_SHADOW {
        return Err(format!(
            "swap_ticket {}: cannot set shadow_started_at in state {}",
            id, existing.state
        ));
    }
    ctx.db.swap_ticket().id().update(SwapTicket {
        shadow_started_at: timestamp.clone(),
        updated_at: timestamp,
        ..existing
    });
    Ok(())
}

/// Record one incumbent-vs-candidate comparison for a shadow ticket.
#[reducer]
pub fn shadow_sample_record(
    ctx: &ReducerContext,
    ticket_id: String,
    call_seq: u64,
    incumbent_adapter_id: String,
    candidate_adapter_id: String,
    incumbent_metrics_json: String,
    candidate_metrics_json: String,
    agreed: bool,
    reason: String,
    timestamp: String,
) -> Result<(), String> {
    if ctx.db.swap_ticket().id().find(&ticket_id).is_none() {
        return Err(format!(
            "shadow_sample_record: ticket {} not found",
            ticket_id
        ));
    }
    ctx.db.shadow_sample().insert(ShadowSample {
        id: 0, // auto_inc
        ticket_id,
        call_seq,
        incumbent_adapter_id,
        candidate_adapter_id,
        incumbent_metrics_json,
        candidate_metrics_json,
        agreed,
        reason,
        recorded_at: timestamp,
    });
    Ok(())
}

#[cfg(test)]
mod swap_ticket_state_tests {
    use super::*;

    #[test]
    fn allowed_transitions_form_the_state_machine() {
        for (from, to) in [
            (SWAP_STATE_CANDIDATE, SWAP_STATE_SHADOW),
            (SWAP_STATE_SHADOW, SWAP_STATE_SHADOW_GREEN),
            (SWAP_STATE_SHADOW, SWAP_STATE_SHADOW_RED),
            (SWAP_STATE_SHADOW_GREEN, SWAP_STATE_PROMOTED),
            (SWAP_STATE_PROMOTED, SWAP_STATE_ROLLED_BACK),
        ] {
            assert!(
                swap_state_transition_allowed(from, to),
                "{} -> {} should be allowed",
                from,
                to
            );
        }
    }

    #[test]
    fn forbidden_transitions_are_rejected() {
        for (from, to) in [
            (SWAP_STATE_CANDIDATE, SWAP_STATE_PROMOTED),       // skip shadow
            (SWAP_STATE_SHADOW, SWAP_STATE_PROMOTED),          // skip judge
            (SWAP_STATE_SHADOW_RED, SWAP_STATE_PROMOTED),      // can't promote a red
            (SWAP_STATE_PROMOTED, SWAP_STATE_SHADOW),          // no going back
            (SWAP_STATE_ROLLED_BACK, SWAP_STATE_CANDIDATE),    // terminal
            (SWAP_STATE_SHADOW_GREEN, SWAP_STATE_SHADOW_RED),  // judge is monotonic
        ] {
            assert!(
                !swap_state_transition_allowed(from, to),
                "{} -> {} should NOT be allowed",
                from,
                to
            );
        }
    }
}

// ============================================================
//  Brain-task history — wire type (ADR-2026-04-14-1400 §1 P1, wp-sched-queue-history P1.1)
// ============================================================
//
// Brain tasks (the `hex sched` / `hex brain queue` pipeline) are NOT stored in
// a dedicated SpacetimeDB table. They live in `hexflo_memory` keyed by
// `brain-task:<uuid>` with the full task record serialized as JSON in the
// value column. This keeps the queue schemaless at the DB layer — daemons can
// evolve the task record (adding lease/evidence-guard fields per
// ADR-2026-04-14-1400) without WASM re-publish gates.
//
// Adding a parallel `brain_task` table here would duplicate that state and
// invite drift between "the canonical task record" (hexflo_memory) and "the
// projection used by /api/brain/queue/history". Instead, the history endpoint
// (wp-sched-queue-history P1.2) reads directly from hexflo_memory_search,
// filters/sorts/truncates in nexus, and returns `BrainTaskSummary` wire shape.
//
// `BrainTaskSummary` is re-declared on the nexus side (with serde) rather than
// shared across the WASM boundary. When/if brain tasks are ever promoted to a
// first-class SpacetimeDB table, this module should add the table + a
// `brain_task_list_recent` reducer mirroring the shape below. Until then, the
// comment is the contract.
//
// Expected wire shape (matches hex-nexus/src/routes/brain.rs::BrainTaskSummary):
//   id: String
//   kind: String                  // "workplan" | "hex-command" | "shell" | "remote-shell"
//   status: String                // "pending" | "in_progress" | "completed" | "failed"
//   payload_truncated: String     // first 80 chars of payload
//   result_truncated: String      // first 300 chars of result (empty if null)
//   created_at_us: i64            // RFC3339 → microseconds since epoch
//   completed_at_us: i64          // 0 if not completed

// ─── Experimental Loop (ADR-2026-05-02-1400) ──────────────────────────────────
//
// Storage for the loop-closing trio of target-app representations:
// Objective (what to maximize), Hypothesis (predicted Δ on Objective),
// and Verdict (measured outcome + graduate/hold/rollback decision).
//
// Persona / Workload / Trial / Failure tables land in a follow-up phase
// (wp-experiment-loop-p2-extras, ADR-2026-05-02-1400 §Implementation P5–P8).
//
// Enums are stored as String columns (SpacetimeDB cannot store nested
// Rust enums in columns). The hex-side adapter (ADR-2026-05-02-1400 §P3)
// converts between these row shapes and `hex_core::domain::experiment::*`.

/// A target-app objective — what the application is trying to maximize/
/// minimize over its workload (ADR-2026-05-02-1400 §Decision).
#[table(name = objective, public)]
#[derive(Clone, Debug)]
pub struct Objective {
    #[primary_key]
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub description: String,
    /// Empty string = top-level objective (no parent).
    pub parent_id: String,
    /// "critical" | "high" | "medium" | "low"
    pub priority: String,
    pub target_value: f64,
    /// "greater_than" | "greater_than_or_equal" | "less_than" |
    /// "less_than_or_equal" | "equal" | "within_range"
    pub comparison: String,
    /// Tolerance for the `within_range` comparison; 0.0 otherwise.
    pub comparison_tolerance: f64,
    pub unit: String,
    /// "active" | "achieved" | "abandoned" | "superseded"
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

/// A falsifiable predicted effect on an Objective.
#[table(name = hypothesis, public)]
#[derive(Clone, Debug)]
pub struct Hypothesis {
    #[primary_key]
    pub id: String,
    pub project_id: String,
    pub content: String,
    pub target_objective_id: String,
    pub predicted_delta: f64,
    pub predicted_confidence: f64,
    pub verification_plan: String,
    /// Empty string = unattached.
    pub adr_id: String,
    /// "untested" | "confirmed" | "rejected" | "inconclusive"
    pub status: String,
    /// Timestamp of the most recent status transition; empty for "untested".
    pub status_at: String,
    /// Populated when status is "rejected".
    pub status_reason: String,
    pub created_at: String,
}

/// Quantified outcome record — closes the experimental loop.
#[table(name = verdict, public)]
#[derive(Clone, Debug)]
pub struct Verdict {
    #[primary_key]
    pub id: String,
    pub project_id: String,
    /// String stub until the Trial table lands (wp-experiment-loop-p2-extras).
    pub trial_id: String,
    pub hypothesis_id: String,
    pub objective_id: String,
    pub baseline_score: f64,
    pub trial_score: f64,
    pub delta: f64,
    pub confidence: f64,
    /// "graduate" | "hold" | "rollback" | "inconclusive"
    pub decision: String,
    /// Populated when decision is "hold" — re-evaluate timestamp.
    pub decision_until: String,
    /// Populated when decision is "rollback".
    pub decision_reason: String,
    pub archived_at: String,
    pub notes: String,
}

const VALID_OBJECTIVE_PRIORITIES: [&str; 4] = ["critical", "high", "medium", "low"];
const VALID_OBJECTIVE_STATUSES: [&str; 4] = ["active", "achieved", "abandoned", "superseded"];
const VALID_OBJECTIVE_COMPARISONS: [&str; 6] = [
    "greater_than",
    "greater_than_or_equal",
    "less_than",
    "less_than_or_equal",
    "equal",
    "within_range",
];
const VALID_HYPOTHESIS_STATUSES: [&str; 4] =
    ["untested", "confirmed", "rejected", "inconclusive"];
const VALID_VERDICT_DECISIONS: [&str; 4] = ["graduate", "hold", "rollback", "inconclusive"];

/// Insert or update an objective.
#[reducer]
pub fn objective_create(
    ctx: &ReducerContext,
    id: String,
    project_id: String,
    name: String,
    description: String,
    parent_id: String,
    priority: String,
    target_value: f64,
    comparison: String,
    comparison_tolerance: f64,
    unit: String,
    created_at: String,
) -> Result<(), String> {
    if id.is_empty() {
        return Err("Objective id is required".to_string());
    }
    if !VALID_OBJECTIVE_PRIORITIES.contains(&priority.as_str()) {
        return Err(format!(
            "Invalid priority '{}'. Must be one of: {}",
            priority,
            VALID_OBJECTIVE_PRIORITIES.join(", ")
        ));
    }
    if !VALID_OBJECTIVE_COMPARISONS.contains(&comparison.as_str()) {
        return Err(format!(
            "Invalid comparison '{}'. Must be one of: {}",
            comparison,
            VALID_OBJECTIVE_COMPARISONS.join(", ")
        ));
    }
    if !parent_id.is_empty() && ctx.db.objective().id().find(&parent_id).is_none() {
        return Err(format!("Parent objective '{}' not found", parent_id));
    }
    let row = Objective {
        id: id.clone(),
        project_id,
        name,
        description,
        parent_id,
        priority,
        target_value,
        comparison,
        comparison_tolerance,
        unit,
        status: "active".to_string(),
        created_at: created_at.clone(),
        updated_at: created_at,
    };
    if ctx.db.objective().id().find(&id).is_some() {
        ctx.db.objective().id().update(row);
    } else {
        ctx.db.objective().insert(row);
    }
    Ok(())
}

/// Update an objective's lifecycle status.
#[reducer]
pub fn objective_update_status(
    ctx: &ReducerContext,
    id: String,
    status: String,
    updated_at: String,
) -> Result<(), String> {
    if !VALID_OBJECTIVE_STATUSES.contains(&status.as_str()) {
        return Err(format!(
            "Invalid status '{}'. Must be one of: {}",
            status,
            VALID_OBJECTIVE_STATUSES.join(", ")
        ));
    }
    let existing = ctx
        .db
        .objective()
        .id()
        .find(&id)
        .ok_or_else(|| format!("Objective '{}' not found", id))?;
    ctx.db.objective().id().update(Objective {
        status,
        updated_at,
        ..existing
    });
    Ok(())
}

/// Insert a new hypothesis attached to an existing objective.
#[reducer]
pub fn hypothesis_create(
    ctx: &ReducerContext,
    id: String,
    project_id: String,
    content: String,
    target_objective_id: String,
    predicted_delta: f64,
    predicted_confidence: f64,
    verification_plan: String,
    adr_id: String,
    created_at: String,
) -> Result<(), String> {
    if id.is_empty() {
        return Err("Hypothesis id is required".to_string());
    }
    if target_objective_id.is_empty() {
        return Err("target_objective_id is required".to_string());
    }
    if ctx
        .db
        .objective()
        .id()
        .find(&target_objective_id)
        .is_none()
    {
        return Err(format!(
            "Target objective '{}' not found",
            target_objective_id
        ));
    }
    if !(0.0..=1.0).contains(&predicted_confidence) {
        return Err(format!(
            "predicted_confidence {} must be in [0.0, 1.0]",
            predicted_confidence
        ));
    }
    let row = Hypothesis {
        id: id.clone(),
        project_id,
        content,
        target_objective_id,
        predicted_delta,
        predicted_confidence,
        verification_plan,
        adr_id,
        status: "untested".to_string(),
        status_at: String::new(),
        status_reason: String::new(),
        created_at,
    };
    if ctx.db.hypothesis().id().find(&id).is_some() {
        ctx.db.hypothesis().id().update(row);
    } else {
        ctx.db.hypothesis().insert(row);
    }
    Ok(())
}

/// Transition a hypothesis to confirmed / rejected / inconclusive.
#[reducer]
pub fn hypothesis_update_status(
    ctx: &ReducerContext,
    id: String,
    status: String,
    status_at: String,
    status_reason: String,
) -> Result<(), String> {
    if !VALID_HYPOTHESIS_STATUSES.contains(&status.as_str()) {
        return Err(format!(
            "Invalid status '{}'. Must be one of: {}",
            status,
            VALID_HYPOTHESIS_STATUSES.join(", ")
        ));
    }
    let existing = ctx
        .db
        .hypothesis()
        .id()
        .find(&id)
        .ok_or_else(|| format!("Hypothesis '{}' not found", id))?;
    ctx.db.hypothesis().id().update(Hypothesis {
        status,
        status_at,
        status_reason,
        ..existing
    });
    Ok(())
}

/// Record a verdict against a hypothesis + objective. Insert-or-update on id.
#[reducer]
pub fn verdict_record(
    ctx: &ReducerContext,
    id: String,
    project_id: String,
    trial_id: String,
    hypothesis_id: String,
    objective_id: String,
    baseline_score: f64,
    trial_score: f64,
    delta: f64,
    confidence: f64,
    decision: String,
    decision_until: String,
    decision_reason: String,
    archived_at: String,
    notes: String,
) -> Result<(), String> {
    if id.is_empty() {
        return Err("Verdict id is required".to_string());
    }
    if !VALID_VERDICT_DECISIONS.contains(&decision.as_str()) {
        return Err(format!(
            "Invalid decision '{}'. Must be one of: {}",
            decision,
            VALID_VERDICT_DECISIONS.join(", ")
        ));
    }
    if ctx.db.hypothesis().id().find(&hypothesis_id).is_none() {
        return Err(format!("Hypothesis '{}' not found", hypothesis_id));
    }
    if ctx.db.objective().id().find(&objective_id).is_none() {
        return Err(format!("Objective '{}' not found", objective_id));
    }
    if !(0.0..=1.0).contains(&confidence) {
        return Err(format!(
            "confidence {} must be in [0.0, 1.0]",
            confidence
        ));
    }
    let row = Verdict {
        id: id.clone(),
        project_id,
        trial_id,
        hypothesis_id,
        objective_id,
        baseline_score,
        trial_score,
        delta,
        confidence,
        decision,
        decision_until,
        decision_reason,
        archived_at,
        notes,
    };
    if ctx.db.verdict().id().find(&id).is_some() {
        ctx.db.verdict().id().update(row);
    } else {
        ctx.db.verdict().insert(row);
    }
    Ok(())
}

// ============================================================
//  STDB-as-supervisor (wp-stdb-supervisor P1)
//
//  Replaces the naïve "spawn-and-pray" hex-agent restart with an OTP-style
//  supervisor: declarative desired state (worker_pool_intent), actual
//  state (worker_process), and an event log (supervisor_event) that
//  hex-nexus subscribes to for spawn / crash-loop handling.
//
//  P1.1 (this commit): just the worker_pool_intent table + a setter reducer.
//  P1.2/P1.3 (follow-up): worker_process and supervisor_event tables.
//  P2.* (follow-up): scheduled supervisor_tick reducer that does the
//  desired-vs-alive reconciliation + crash-loop accounting.
// ============================================================

/// Operator's declared intent for a worker pool. "I want N workers of role X
/// running, with Y restart strategy". The supervisor_tick scheduled reducer
/// (P2.1) reconciles actual `worker_process` rows against this intent and
/// emits `supervisor_event::spawn_request` when alive < desired.
///
/// `restart_strategy`:
///   - "permanent" — always respawn on exit (default for long-running pools)
///   - "transient" — respawn only on abnormal exit (exit_reason != "normal")
///   - "temporary" — never respawn (one-shot tasks)
///
/// `paused=true` + `desired_count=0` is how operators temporarily disable
/// a pool without deleting its config (CLI: `hex pool pause <id>`).
#[table(name = worker_pool_intent, public)]
#[derive(Clone, Debug)]
pub struct WorkerPoolIntent {
    #[primary_key]
    pub id: String,
    /// Persona role that this pool runs (matches a YAML in
    /// hex-cli/assets/agents/hex/hex/<role>.yml).
    pub role: String,
    /// How many workers of this role should be alive at any time.
    pub desired_count: u32,
    /// "permanent" | "transient" | "temporary".
    pub restart_strategy: String,
    /// Crash-loop circuit-breaker: max restarts inside the window before
    /// the supervisor stops respawning + alerts the operator.
    pub max_restarts: u32,
    pub max_restart_window_secs: u32,
    /// When true, supervisor stops emitting spawn_request for this pool.
    /// Set by operator (`hex pool pause`) or by supervisor_tick when the
    /// crash-loop circuit-breaker trips.
    pub paused: bool,
    /// Set by supervisor_tick when restart accounting trips the breaker.
    /// Operator must `hex pool resume <id>` to clear (also resets restart
    /// accounting window).
    pub in_crash_loop: bool,
    pub created_at: String,
    pub updated_at: String,
    /// Agent ID of the operator that created/last-updated this pool. Used
    /// for audit + ownership transfer (operator who owns the registration
    /// is the one whose inbox gets crash-loop alerts).
    pub owner_agent_id: String,
}

/// Create or update a worker pool intent. Idempotent — same `id` overwrites.
///
/// Inputs default-friendly so a minimal call (`worker_pool_intent_set` with
/// `id`, `role`, `desired_count`) gets sensible behaviour:
/// permanent + 5 restarts in 60s window + not paused.
#[reducer]
pub fn worker_pool_intent_set(
    ctx: &ReducerContext,
    id: String,
    role: String,
    desired_count: u32,
    restart_strategy: String,
    max_restarts: u32,
    max_restart_window_secs: u32,
    paused: bool,
    owner_agent_id: String,
) -> Result<(), String> {
    if id.is_empty() { return Err("id is required".into()); }
    if role.is_empty() { return Err("role is required".into()); }
    let strategy = restart_strategy.trim().to_lowercase();
    if !matches!(strategy.as_str(), "permanent" | "transient" | "temporary") {
        return Err(format!(
            "invalid restart_strategy '{}': must be permanent | transient | temporary",
            restart_strategy
        ));
    }

    let now = format!("{:?}", ctx.timestamp);
    let existing = ctx.db.worker_pool_intent().id().find(&id);
    let row = WorkerPoolIntent {
        id: id.clone(),
        role,
        desired_count,
        restart_strategy: strategy,
        max_restarts: if max_restarts == 0 { 5 } else { max_restarts },
        max_restart_window_secs: if max_restart_window_secs == 0 { 60 } else { max_restart_window_secs },
        paused,
        // Updating an intent always clears the crash-loop flag — operator
        // is taking deliberate action, give the pool another chance.
        in_crash_loop: false,
        created_at: existing.as_ref().map(|e| e.created_at.clone()).unwrap_or_else(|| now.clone()),
        updated_at: now,
        owner_agent_id,
    };
    if existing.is_some() {
        ctx.db.worker_pool_intent().id().update(row);
    } else {
        ctx.db.worker_pool_intent().insert(row);
    }
    Ok(())
}

/// Pause/resume a pool without recreating it. Convenience reducer for
/// `hex pool pause <id>` / `hex pool resume <id>`.
#[reducer]
pub fn worker_pool_intent_set_paused(
    ctx: &ReducerContext,
    id: String,
    paused: bool,
) -> Result<(), String> {
    let mut row = ctx.db.worker_pool_intent().id().find(&id)
        .ok_or_else(|| format!("worker pool '{}' not found", id))?;
    row.paused = paused;
    if !paused {
        // Resuming a pool clears any sticky crash-loop flag too.
        row.in_crash_loop = false;
    }
    row.updated_at = format!("{:?}", ctx.timestamp);
    ctx.db.worker_pool_intent().id().update(row);
    Ok(())
}

/// Delete a pool intent. Does NOT terminate any currently-running workers
/// of that role — they continue until they exit naturally. After delete,
/// the supervisor_tick stops emitting spawn_request for this pool.
#[reducer]
pub fn worker_pool_intent_delete(ctx: &ReducerContext, id: String) -> Result<(), String> {
    if ctx.db.worker_pool_intent().id().find(&id).is_none() {
        return Err(format!("worker pool '{}' not found", id));
    }
    ctx.db.worker_pool_intent().id().delete(&id);
    Ok(())
}

// ============================================================
//  STDB-as-supervisor — P1.2 worker_process + P1.3 supervisor_event
//  + P2.1 supervisor_tick scheduled reducer.
// ============================================================

/// Actual state of one worker process. Created by hex-nexus when it acts on
/// a `supervisor_event::spawn_request`; lifecycle (heartbeat, exit) updated
/// by the same. Read by `supervisor_tick` to compute alive vs desired.
///
/// `last_heartbeat` is a string-encoded RFC3339 timestamp (matches the
/// existing hex_agent table convention). Empty = never heartbeated.
///
/// `exited_at` non-empty marks the process as terminated. The supervisor
/// uses `exited_at` + `restart_count` + the parent pool's
/// `max_restarts`/`max_restart_window_secs` to decide if the pool is in a
/// crash loop.
#[table(name = worker_process, public)]
#[derive(Clone, Debug)]
pub struct WorkerProcess {
    #[primary_key]
    pub id: String,
    pub pool_id: String,
    pub role: String,
    pub host: String,
    pub pid: i64,
    pub started_at: String,
    pub last_heartbeat: String,
    /// Bumped when this row is replaced by a new spawn for the same pool.
    /// Crash-loop accounting in supervisor_tick reads
    /// `recent_restarts_for(pool_id, window)` from this counter across rows.
    pub restart_count: u32,
    pub in_crash_loop: bool,
    pub exited_at: String,
    pub exit_reason: String,
    /// Self-reported liveness ("healthy" | "degraded" | "stopping"). Added
    /// 2026-05-19 for ADR-2605190900 P1.2 — IHeartbeatPort lets components
    /// downgrade themselves proactively when they know they can't fully
    /// serve their contract (e.g. STDB downstream unreachable). Supervisor
    /// reads this to escalate Degraded → operator after a TTL even if
    /// last_heartbeat is fresh.
    pub status: String,
    /// Free-form note attached to the latest beat — surfaced to the
    /// dashboard. Empty string when unused (STDB has no Option<String>).
    pub evidence: String,
}

/// Register a freshly-spawned worker. Called by hex-nexus subscriber after
/// it acts on a spawn_request event.
#[reducer]
pub fn worker_process_register(
    ctx: &ReducerContext,
    id: String,
    pool_id: String,
    role: String,
    host: String,
    pid: i64,
) -> Result<(), String> {
    if id.is_empty() || pool_id.is_empty() {
        return Err("id and pool_id are required".into());
    }
    let now = format!("{:?}", ctx.timestamp);
    let row = WorkerProcess {
        id, pool_id, role, host, pid,
        started_at: now.clone(),
        last_heartbeat: now,
        restart_count: 0,
        in_crash_loop: false,
        exited_at: String::new(),
        exit_reason: String::new(),
        status: "healthy".to_string(),
        evidence: String::new(),
    };
    ctx.db.worker_process().insert(row);
    Ok(())
}

/// Record a heartbeat. The agent's existing heartbeat path can mirror to
/// this table when its agent_id maps to a worker_process row. Best-effort —
/// supervisor_tick treats missing heartbeats as "stale", not "missing
/// metadata is fatal".
#[reducer]
pub fn worker_process_heartbeat(ctx: &ReducerContext, id: String) -> Result<(), String> {
    let mut row = ctx.db.worker_process().id().find(&id)
        .ok_or_else(|| format!("worker_process '{}' not found", id))?;
    row.last_heartbeat = format!("{:?}", ctx.timestamp);
    ctx.db.worker_process().id().update(row);
    Ok(())
}

/// Self-reported status update. The component publishes `healthy`, downgrades
/// to `degraded` when an upstream is unreachable, or to `stopping` on graceful
/// shutdown. Same row as last_heartbeat — supervisor reads both together.
/// Added 2026-05-19 for ADR-2605190900 P1.2 to match the IHeartbeatPort
/// contract (hex-core/src/ports/heartbeat.rs).
#[reducer]
pub fn worker_process_status(
    ctx: &ReducerContext,
    id: String,
    status: String,
    evidence: String,
) -> Result<(), String> {
    if !matches!(status.as_str(), "healthy" | "degraded" | "stopping") {
        return Err(format!("invalid status '{}' — expected healthy|degraded|stopping", status));
    }
    let mut row = ctx.db.worker_process().id().find(&id)
        .ok_or_else(|| format!("worker_process '{}' not found", id))?;
    row.last_heartbeat = format!("{:?}", ctx.timestamp);
    row.status = status;
    row.evidence = evidence;
    ctx.db.worker_process().id().update(row);
    Ok(())
}

/// Graceful deregistration — removes the row outright. Idempotent: calling
/// twice or on an unknown id returns Ok so the adapter's Drop impl can be
/// best-effort. Distinct from worker_process_record_exit which keeps the
/// row for crash-loop accounting; deregister is the clean-shutdown path.
#[reducer]
pub fn worker_process_deregister(ctx: &ReducerContext, id: String) -> Result<(), String> {
    // The unique column accessor's delete() takes the column value, not
    // the row — pass &id directly. Find-then-delete is unnecessary; STDB
    // returns false on missing rows so idempotence is built in.
    let _existed = ctx.db.worker_process().id().delete(&id);
    Ok(())
}

// ============================================================
// dead_letter — bounded-retry quarantine for brain-tasks (ADR-2605190900 P2.1).
// ============================================================
// A dedicated audit row for brain-tasks that exceeded their retry budget.
// Distinct from BrainTaskStatus::DeadLetter — that's a status flag on the
// brain-task row; this table is the durable record of WHY a task was
// quarantined and HOW to replay it.
//
// The dispatcher (hex-nexus/src/orchestration/brain_dispatch_reconciler.rs)
// writes one row here on the Nth consecutive failure of a brain-task. The
// dashboard's #/dead-letter view reads from here; `dead_letter_replay` is
// the operator-driven "try again" path that moves the row back into the
// sched_task pending queue.
//
// Why a separate table, not just a status:
//   1) BrainTaskStatus::DeadLetter doesn't capture HOW MANY tries or WHEN
//      they happened — both are needed for the dispatch_retry_quota
//      hypothesis the improver consumes (P2.4).
//   2) The 2026-05-19 postmortem observed brain-tasks getting NEW ids
//      every minute instead of being marked failed — the existing
//      retry_count on the brain-task row never incremented for those.
//      A dedicated table sidesteps the bug regardless of how the
//      dispatcher counts.
//   3) Replay should be auditable. Moving the row out of dead_letter
//      back to sched_task pending is one reducer; the operation is
//      visible in the event log.
// ============================================================

#[table(name = dead_letter, public)]
#[derive(Clone, Debug)]
pub struct DeadLetter {
    /// Original brain-task / sched_task id. Same handle the dashboard
    /// already shows in #/queue.
    #[primary_key]
    pub task_id: String,
    /// "workplan" | "hex-command" | "shell" — matches the brain-task kind.
    pub kind: String,
    /// The original payload (workplan path, command args, shell line).
    pub payload: String,
    /// Most recent error message from the dispatcher / executor. Bounded
    /// to ~1 KB at write time — long stack traces get truncated.
    pub last_error: String,
    /// How many attempts before quarantine. Single source of truth — the
    /// brain-task's retry_count gets reset by reschedules, this stays
    /// monotonic.
    pub attempt_count: u32,
    /// RFC 3339 timestamps. Useful for "how old is this and is it worth
    /// bothering" triage on the dashboard.
    pub first_failed_at: String,
    pub last_failed_at: String,
    /// Operator-tunable priority at the time of quarantine — `replay`
    /// preserves it so the re-enqueued task lands in the same bucket.
    pub original_priority: i32,
}

/// Append a dead-letter row. Called by the dispatcher when a brain-task's
/// attempt_count exceeds the configured threshold (HEX_DEAD_LETTER_THRESHOLD).
/// On duplicate task_id the row is upserted — attempt_count bumps to the
/// new value, last_failed_at refreshes. first_failed_at preserved.
#[reducer]
pub fn dead_letter_record(
    ctx: &ReducerContext,
    task_id: String,
    kind: String,
    payload: String,
    last_error: String,
    attempt_count: u32,
    original_priority: i32,
    timestamp: String,
) -> Result<(), String> {
    if task_id.is_empty() {
        return Err("empty task_id".to_string());
    }
    // Bound the error string so an oversize traceback doesn't blow up the
    // STDB row size limit (see ADR-2026-05-08-2600).
    const MAX_ERR_LEN: usize = 1024;
    let last_error = if last_error.len() > MAX_ERR_LEN {
        let mut truncated = last_error[..MAX_ERR_LEN].to_string();
        truncated.push_str("… [truncated]");
        truncated
    } else {
        last_error
    };
    if let Some(existing) = ctx.db.dead_letter().task_id().find(&task_id) {
        ctx.db.dead_letter().task_id().update(DeadLetter {
            kind,
            payload,
            last_error,
            attempt_count,
            last_failed_at: timestamp,
            first_failed_at: existing.first_failed_at,
            original_priority,
            ..existing
        });
    } else {
        ctx.db.dead_letter().insert(DeadLetter {
            task_id,
            kind,
            payload,
            last_error,
            attempt_count,
            first_failed_at: timestamp.clone(),
            last_failed_at: timestamp,
            original_priority,
        });
    }
    Ok(())
}

// ============================================================
// improver_event — append-only log of MAPE-K transitions
// (ADR-2605190721 P4.1 + ADR-2605190900 P2.4).
// ============================================================
// The improver loop's K phase: every hypothesis discover / propose /
// judge / act / dead-letter transition writes one row here. The
// improver_judge reads back via `historical_reject_rate` so the system
// learns which detector + scope patterns the operator tends to overrule.
//
// Distinct from supervisor_event — supervisor_event is the OTP-style
// pool reconciliation log; improver_event is the MAPE-K learning log.
// Different consumers, different cadences.
// ============================================================

#[table(name = improver_event, public)]
#[derive(Clone, Debug)]
pub struct ImproverEvent {
    #[primary_key]
    #[auto_inc]
    pub id: u64,
    pub ts: String,
    /// "discover" | "propose" | "judge" | "act" | "retry_quota_exceeded" | "pong" | …
    /// Loose enum — detector authors invent new kinds; the dashboard
    /// groups by kind without enumerating them up front.
    pub kind: String,
    /// Where the event originated. Same shape as Hypothesis.source —
    /// e.g. "AdrDoctor", "DispatchRetryQuota", "GodTypes", "Liveness".
    pub source: String,
    /// Scope of the event — typically a task_id, ADR id, or component
    /// role. Together with source it acts as the dedup key for
    /// historical_reject_rate computation.
    pub scope: String,
    /// JSON blob with kind-specific fields. Kept under ~2 KB per row
    /// to stay below STDB's per-row size cap (ADR-2026-05-08-2600).
    pub payload: String,
    /// Optional cross-reference — e.g. a `pong` event's `related` points
    /// to the `ping` event's id; an `act` event's `related` points to
    /// the originating `discover`.
    pub related: u64,
}

/// Append one improver_event row. Loose schema — callers are
/// responsible for their kind/source/scope conventions. Returns the
/// auto-incremented id so the caller can include it in a downstream
/// `related` reference.
#[reducer]
pub fn improver_event_record(
    ctx: &ReducerContext,
    kind: String,
    source: String,
    scope: String,
    payload: String,
    related: u64,
    timestamp: String,
) -> Result<(), String> {
    if kind.is_empty() {
        return Err("kind is required".to_string());
    }
    // Bound payload to ~2 KB so a runaway emitter can't blow up STDB's
    // per-row size cap.
    const MAX_PAYLOAD_LEN: usize = 2048;
    let payload = if payload.len() > MAX_PAYLOAD_LEN {
        let mut t = payload[..MAX_PAYLOAD_LEN].to_string();
        t.push_str("… [truncated]");
        t
    } else {
        payload
    };
    ctx.db.improver_event().insert(ImproverEvent {
        id: 0, // auto_inc fills this
        ts: timestamp,
        kind,
        source,
        scope,
        payload,
        related,
    });
    Ok(())
}

/// Operator-driven replay — copy the dead-letter row back into the
/// sched_task pending queue (or whichever queue the dispatcher reads
/// from) and remove the dead_letter row. Returns the rehydrated kind +
/// payload + priority so the caller can re-enqueue at the API layer.
/// Idempotent: calling on an unknown task_id returns an empty result
/// rather than erroring, so the dashboard "Replay" button is safe to
/// double-click.
#[reducer]
pub fn dead_letter_replay(ctx: &ReducerContext, task_id: String) -> Result<(), String> {
    if let Some(row) = ctx.db.dead_letter().task_id().find(&task_id) {
        // The actual re-enqueue happens on the nexus side — the dispatcher
        // owns sched_task creation. This reducer just opens the gate by
        // removing the dead-letter quarantine; nexus subscribes to deletes
        // and re-enqueues. Wiring lives in P2.3.
        ctx.db.dead_letter().task_id().delete(&row.task_id);
    }
    Ok(())
}

/// Record process exit. nexus calls this when it observes the spawned
/// hex-agent terminate. exit_reason: "normal" (status 0), "crashed" (any
/// non-zero), "killed" (signal), "unknown" (fall-through).
#[reducer]
pub fn worker_process_record_exit(
    ctx: &ReducerContext,
    id: String,
    exit_reason: String,
) -> Result<(), String> {
    let mut row = ctx.db.worker_process().id().find(&id)
        .ok_or_else(|| format!("worker_process '{}' not found", id))?;
    if !row.exited_at.is_empty() {
        // Idempotent — already recorded
        return Ok(());
    }
    row.exited_at = format!("{:?}", ctx.timestamp);
    row.exit_reason = exit_reason;
    ctx.db.worker_process().id().update(row);
    Ok(())
}

/// One supervisor decision or observation. Grows append-only; nexus
/// subscribes and acts on `kind = "spawn_request"`. Other kinds are
/// observability / alerts — `crash_loop` triggers a priority-2 inbox post
/// from the nexus side.
///
/// Why a row instead of a reactive subscription event: tasks can race the
/// subscription window; durably-stored events let nexus reconnect and
/// replay any unhandled work.
#[table(name = supervisor_event, public)]
#[derive(Clone, Debug)]
pub struct SupervisorEvent {
    #[primary_key]
    #[auto_inc]
    pub id: u64,
    pub ts: String,
    /// "spawn_request" | "crash_loop" | "process_exited" | "tick"
    pub kind: String,
    pub pool_id: String,
    pub worker_id: String,
    /// JSON blob with kind-specific fields (target_count, exit_reason,
    /// restart_count_in_window, etc.). Keep small — STDB rows are
    /// length-bounded.
    pub payload: String,
    /// nexus marks an event as handled so we don't double-spawn.
    pub handled: bool,
    pub handled_at: String,
    pub handled_by: String,
}

/// Mark a supervisor_event as handled. Called by hex-nexus subscriber after
/// it acts on the event (e.g. after spawning the requested worker).
#[reducer]
pub fn supervisor_event_handle(
    ctx: &ReducerContext,
    id: u64,
    handled_by: String,
) -> Result<(), String> {
    let mut row = ctx.db.supervisor_event().id().find(id)
        .ok_or_else(|| format!("supervisor_event {} not found", id))?;
    if row.handled { return Ok(()); }
    row.handled = true;
    row.handled_at = format!("{:?}", ctx.timestamp);
    row.handled_by = handled_by;
    ctx.db.supervisor_event().id().update(row);
    Ok(())
}

/// Schedule anchor for the supervisor_tick scheduled reducer. Inserted by
/// `supervisor_init` once. STDB calls supervisor_tick at every interval.
#[table(name = supervisor_tick_schedule, public, scheduled(supervisor_tick))]
#[derive(Clone, Debug)]
pub struct SupervisorTickSchedule {
    #[primary_key]
    #[auto_inc]
    pub scheduled_id: u64,
    pub scheduled_at: ScheduleAt,
}

/// One-shot init: seed the supervisor_tick schedule. Idempotent — calling
/// twice is a no-op (the schedule row already exists). Operators run this
/// after `spacetime publish` of a fresh deployment.
#[reducer]
pub fn supervisor_init(ctx: &ReducerContext) -> Result<(), String> {
    let already = ctx.db.supervisor_tick_schedule().iter().next().is_some();
    if already {
        log::info!("supervisor_init: schedule already exists, skipping");
        return Ok(());
    }
    let interval = ScheduleAt::Interval(std::time::Duration::from_secs(10).into());
    ctx.db.supervisor_tick_schedule().insert(SupervisorTickSchedule {
        scheduled_id: 0, // auto_inc fills this
        scheduled_at: interval,
    });
    log::info!("supervisor_init: tick scheduled every 10s");
    Ok(())
}

/// THE supervisor. Fires every 10s.
///
/// For each non-paused worker_pool_intent:
///   1. Count alive worker_process rows (no exited_at, in_crash_loop=false,
///      pool_id matches).
///   2. If alive < desired_count, emit a spawn_request event.
///   3. Count recent restarts in the window. If > max_restarts:
///      - Mark the pool in_crash_loop = true (sticky until operator resumes).
///      - Emit crash_loop event so nexus can post priority-2 inbox.
///      - STOP emitting spawn_request for this pool.
///   4. Honour restart_strategy:
///      - permanent → respawn always
///      - transient → respawn only if recent exits had exit_reason != "normal"
///      - temporary → never respawn
///
/// Side note on cost: this reducer scans worker_process every 10s. With
/// hundreds of workers + months of history that scan grows. Adding a
/// `(pool_id, exited_at IS NULL)` index would help; for now keep it simple
/// and add the index when worker_process > 1k rows.
#[reducer]
pub fn supervisor_tick(
    ctx: &ReducerContext,
    _schedule: SupervisorTickSchedule,
) -> Result<(), String> {
    let now_str = format!("{:?}", ctx.timestamp);
    // Extract the integer micros from the Debug-format timestamp so we can
    // do window-relative comparisons. STDB's Debug format is:
    //   "Timestamp { __timestamp_micros_since_unix_epoch__: 1234567890 }"
    // Returns None on any parse mismatch — caller falls back to counting
    // all rows in that case (safer than skipping detection entirely).
    fn parse_ts_micros(s: &str) -> Option<i64> {
        let key = "__timestamp_micros_since_unix_epoch__:";
        let pos = s.find(key)?;
        let tail = &s[pos + key.len()..];
        let end = tail.find(|c: char| !c.is_ascii_digit() && c != '-' && c != ' ')
            .unwrap_or(tail.len());
        tail[..end].trim().parse::<i64>().ok()
    }
    let now_micros = parse_ts_micros(&now_str).unwrap_or(0);

    // ── Stale-heartbeat reap pass (ADR-2605190900 P3.2) ──
    //
    // Before the spawn/crash accounting runs, mark any worker whose
    // last_heartbeat is older than STALE_HEARTBEAT_SECS as exited. Without
    // this pass, a worker that stops beating WITHOUT setting exited_at
    // (the 11-day zombie sched daemon pattern from the 2026-05-19
    // postmortem) is counted as `alive_count` forever and the supervisor
    // never asks for a respawn.
    //
    // Threshold rationale: heartbeats land every 15s (IHeartbeatPort
    // recommended cadence). 60s = 4 missed beats — enough margin to
    // tolerate GC pauses or transient STDB reconnects without false
    // reaping, narrow enough that an actually-dead worker triggers
    // respawn within 1-2 minutes.
    const STALE_HEARTBEAT_SECS: i64 = 60;
    let stale_cutoff_micros = now_micros - (STALE_HEARTBEAT_SECS * 1_000_000);
    if now_micros != 0 {
        let alive_rows: Vec<WorkerProcess> = ctx.db.worker_process().iter()
            .filter(|p| p.exited_at.is_empty())
            .collect();
        for row in alive_rows {
            let last_beat_micros = parse_ts_micros(&row.last_heartbeat).unwrap_or(0);
            if last_beat_micros == 0 || last_beat_micros >= stale_cutoff_micros {
                continue;
            }
            let mut updated = row.clone();
            updated.exited_at = now_str.clone();
            updated.exit_reason = "stale_heartbeat".to_string();
            updated.status = "stopping".to_string();
            ctx.db.worker_process().id().update(updated);
            ctx.db.supervisor_event().insert(SupervisorEvent {
                id: 0,
                ts: now_str.clone(),
                kind: "stale_heartbeat".to_string(),
                pool_id: row.pool_id.clone(),
                worker_id: row.id.clone(),
                payload: format!(
                    r#"{{"last_heartbeat":"{}","threshold_secs":{}}}"#,
                    row.last_heartbeat, STALE_HEARTBEAT_SECS
                ),
                handled: false,
                handled_at: String::new(),
                handled_by: String::new(),
            });
            log::warn!(
                "supervisor: reaped stale worker {} (pool {}, last_heartbeat {})",
                row.id, row.pool_id, row.last_heartbeat
            );
        }
    }

    // Snapshot all worker_process rows once; we'll scan per-pool.
    // Re-snapshot after the reap pass so the alive_count below sees the
    // freshly-set exited_at values.
    let processes: Vec<WorkerProcess> = ctx.db.worker_process().iter().collect();
    let pools: Vec<WorkerPoolIntent> = ctx.db.worker_pool_intent().iter().collect();

    for pool in pools {
        // paused is operator-controlled; in_crash_loop is sticky-set by the
        // crash-loop detector below. Both stop spawning. Operator clears
        // in_crash_loop via worker_pool_intent_set_paused(false) which
        // toggles it as a side-effect.
        if pool.paused || pool.in_crash_loop {
            continue;
        }

        // Alive: pool match, no exit recorded, not flagged as crashed at the
        // process-row level (pool-level in_crash_loop is checked separately).
        let alive_count: u32 = processes.iter()
            .filter(|p| p.pool_id == pool.id && p.exited_at.is_empty() && !p.in_crash_loop)
            .count() as u32;

        // Crash-loop accounting: count exits within the configured window.
        // Without this scoping, ANY pool that ever had >max_restarts exits in
        // its lifetime can never recover — the supervisor would re-trip the
        // breaker every tick because all-time exits dwarfs max_restarts.
        // Window: max_restart_window_secs from the pool config.
        let window_micros: i64 = (pool.max_restart_window_secs as i64) * 1_000_000;
        let cutoff_micros = now_micros - window_micros;
        let exited_in_pool: Vec<&WorkerProcess> = processes.iter()
            .filter(|p| p.pool_id == pool.id && !p.exited_at.is_empty())
            .filter(|p| {
                // If we can't parse the timestamp (or now_micros was 0), include
                // the row — safer to over-count than under-count for a breaker.
                if now_micros == 0 { return true; }
                match parse_ts_micros(&p.exited_at) {
                    Some(t) => t >= cutoff_micros,
                    None => true,
                }
            })
            .collect();
        let recent_exits = exited_in_pool.len() as u32;

        // Crash-loop check (pool-level, sticky until operator resumes).
        if !pool.in_crash_loop && recent_exits > pool.max_restarts {
            log::warn!(
                "supervisor: pool {} entering crash-loop ({} exits > max_restarts {})",
                pool.id, recent_exits, pool.max_restarts
            );
            // Flip the flag on the pool row.
            let mut updated = pool.clone();
            updated.in_crash_loop = true;
            updated.updated_at = now_str.clone();
            ctx.db.worker_pool_intent().id().update(updated);

            ctx.db.supervisor_event().insert(SupervisorEvent {
                id: 0,
                ts: now_str.clone(),
                kind: "crash_loop".to_string(),
                pool_id: pool.id.clone(),
                worker_id: String::new(),
                payload: format!(
                    r#"{{"recent_exits":{},"max_restarts":{},"window_secs":{}}}"#,
                    recent_exits, pool.max_restarts, pool.max_restart_window_secs
                ),
                handled: false,
                handled_at: String::new(),
                handled_by: String::new(),
            });
            continue; // skip spawn_request for crash-looped pools
        }

        // Spawn request when alive < desired.
        if alive_count < pool.desired_count {
            // Honour restart_strategy when there's a recent exit:
            //   "temporary" → never respawn
            //   "transient" → only on abnormal exit (last exit_reason != "normal")
            //   "permanent" → always (default)
            let should_spawn = match pool.restart_strategy.as_str() {
                "temporary" => alive_count < pool.desired_count && recent_exits == 0, // initial spawn only
                "transient" => {
                    if recent_exits == 0 {
                        true
                    } else {
                        // Look at the most recent exit; if normal, skip.
                        let last_normal = exited_in_pool.last()
                            .map(|p| p.exit_reason == "normal")
                            .unwrap_or(false);
                        !last_normal
                    }
                }
                _ => true, // permanent or unknown → always
            };

            if should_spawn {
                let needed = pool.desired_count - alive_count;
                ctx.db.supervisor_event().insert(SupervisorEvent {
                    id: 0,
                    ts: now_str.clone(),
                    kind: "spawn_request".to_string(),
                    pool_id: pool.id.clone(),
                    worker_id: String::new(),
                    payload: format!(
                        r#"{{"role":"{}","needed":{},"alive":{},"desired":{}}}"#,
                        pool.role, needed, alive_count, pool.desired_count
                    ),
                    handled: false,
                    handled_at: String::new(),
                    handled_by: String::new(),
                });
                log::info!(
                    "supervisor: spawn_request emitted for pool {} (need {} of {})",
                    pool.id, needed, pool.desired_count
                );
            }
        }
    }

    Ok(())
}

// ============================================================
//  Persona Supervisor (OTP-style for executive personas)
//
//  Mirrors the worker_pool_intent / supervisor_tick design above, but for
//  VIRTUAL agents (cto, cpo, coo, ciso, chief-visionary, engineering-lead,
//  product-lead, sre-lead). Personas are conversation entities answered by
//  hex-nexus's org_responder — they have no process. Instead of emitting
//  spawn_request events, persona_tick directly upserts hex_agent rows and
//  refreshes their last_heartbeat to keep the persona "online" in the
//  dashboard / `hex agent list`.
//
//  persona_health gives the org_responder an inference circuit-breaker:
//  3 failures in 60s → 5 minute ban. Persistent across nexus restarts.
// ============================================================

pub const PERSONA_TICK_INTERVAL_SECS: u64 = 25;
pub const PERSONA_FAILURE_THRESHOLD: u32 = 3;
pub const PERSONA_FAILURE_WINDOW_SECS: i64 = 60;
pub const PERSONA_BAN_DURATION_SECS: i64 = 300;

#[table(name = persona_pool, public)]
#[derive(Clone, Debug)]
pub struct PersonaPool {
    #[primary_key]
    pub role: String,
    pub display_name: String,
    pub tier: String,
    pub paused: bool,
    pub last_tick_at: String,
    pub created_at: String,
    pub updated_at: String,
}

#[table(name = persona_health, public)]
#[derive(Clone, Debug)]
pub struct PersonaHealth {
    #[primary_key]
    pub role: String,
    pub recent_failures: u32,
    pub last_failure_at: String,
    pub last_failure_model: String,
    pub last_failure_status: u32,
    pub banned_until: String,
}

#[table(name = persona_event, public)]
#[derive(Clone, Debug)]
pub struct PersonaEvent {
    #[primary_key]
    #[auto_inc]
    pub id: u64,
    pub ts: String,
    pub kind: String,
    pub role: String,
    pub payload: String,
}

#[table(name = persona_tick_schedule, public, scheduled(persona_tick))]
#[derive(Clone, Debug)]
pub struct PersonaTickSchedule {
    #[primary_key]
    #[auto_inc]
    pub scheduled_id: u64,
    pub scheduled_at: ScheduleAt,
}

// ============================================================
// User-defined SOUL personas (ADR-2605131849, wp-user-defined-soul-personas P1)
// ============================================================
// Flat-peer personas that coexist with the built-in c-suite. Storage on
// disk at ~/.hex/personas/<name>/SOUL.md; this table tracks the rows so
// the dashboard + routing can enumerate them. Separate from persona_pool
// so the c-suite supervisor (25s tick, ban-after-3-fails) does NOT
// operate over user-personas — they are invoked on demand and have no
// health-state machine.
//
// name validation: ^[a-z][a-z0-9-]{0,63}$ enforced in the create reducer.
// collision with persona_pool (c-suite + IC roles) is rejected.
// ============================================================

#[table(name = user_persona, public)]
#[derive(Clone, Debug)]
pub struct UserPersona {
    #[primary_key]
    pub name: String,
    pub soul_hash: String,
    pub created_at: String,
    pub last_used_at: Option<String>,
    pub tools_override: Option<String>,
    pub tier_models_override: Option<String>,
}

fn user_persona_name_valid(name: &str) -> bool {
    if name.is_empty() || name.len() > 64 {
        return false;
    }
    let mut chars = name.chars();
    let first = match chars.next() {
        Some(c) => c,
        None => return false,
    };
    if !first.is_ascii_lowercase() {
        return false;
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Belt-and-suspenders c-suite + IC role collision check. The reducer
/// also queries persona_pool dynamically, but if persona_pool was
/// transiently emptied (e.g. by a `--delete-data=on-conflict` republish
/// before the supervisor's first tick re-seeds it), the dynamic check
/// alone would let a user-persona named `cto` slip in. The hardcoded
/// list closes that window.
fn user_persona_name_reserved(name: &str) -> bool {
    matches!(
        name,
        // C-suite
        "ceo" | "cto" | "cpo" | "coo" | "ciso" | "chief-architect"
        | "chief-visionary" | "cmo"
        // Leads
        | "engineering-lead" | "product-lead" | "sre-lead" | "validation-judge"
        // ICs and tooling agents
        | "hex-coder" | "hex-tester" | "hex-fixer" | "hex-reviewer" | "hex-ux"
        | "hex-documenter" | "rust-refactorer" | "dead-code-analyzer"
        | "scaffold-validator" | "planner" | "integrator" | "ux-designer"
        | "cli-designer" | "pm-agent" | "behavioral-spec-writer"
        | "adversarial-red" | "adversarial-blue" | "adr-reviewer"
        | "platform-engineer" | "sre-engineer"
    )
}

#[reducer]
pub fn user_persona_create(
    ctx: &ReducerContext,
    name: String,
    soul_hash: String,
    tools_override: Option<String>,
    tier_models_override: Option<String>,
) -> Result<(), String> {
    if !user_persona_name_valid(&name) {
        return Err(format!(
            "user_persona_create: invalid name '{}' (must match ^[a-z][a-z0-9-]{{0,63}}$)",
            name
        ));
    }
    if soul_hash.is_empty() {
        return Err("user_persona_create: soul_hash is required".into());
    }
    if user_persona_name_reserved(&name) {
        return Err(format!(
            "user_persona_create: '{}' is a reserved c-suite or IC role identifier",
            name
        ));
    }
    if ctx.db.persona_pool().role().find(&name).is_some() {
        return Err(format!(
            "user_persona_create: name '{}' collides with a built-in c-suite or IC role in persona_pool",
            name
        ));
    }
    if ctx.db.user_persona().name().find(&name).is_some() {
        return Err(format!(
            "user_persona_create: user_persona '{}' already exists; use user_persona_update_soul to refresh",
            name
        ));
    }
    let now = format!("{:?}", ctx.timestamp);
    ctx.db.user_persona().insert(UserPersona {
        name,
        soul_hash,
        created_at: now,
        last_used_at: None,
        tools_override,
        tier_models_override,
    });
    Ok(())
}

#[reducer]
pub fn user_persona_delete(ctx: &ReducerContext, name: String) -> Result<(), String> {
    if ctx.db.user_persona().name().find(&name).is_none() {
        return Err(format!("user_persona_delete: '{}' not found", name));
    }
    ctx.db.user_persona().name().delete(&name);
    Ok(())
}

#[reducer]
pub fn user_persona_touch(ctx: &ReducerContext, name: String) -> Result<(), String> {
    let mut row = ctx
        .db
        .user_persona()
        .name()
        .find(&name)
        .ok_or_else(|| format!("user_persona_touch: '{}' not found", name))?;
    row.last_used_at = Some(format!("{:?}", ctx.timestamp));
    ctx.db.user_persona().name().update(row);
    Ok(())
}

#[reducer]
pub fn user_persona_update_soul(
    ctx: &ReducerContext,
    name: String,
    soul_hash: String,
) -> Result<(), String> {
    if soul_hash.is_empty() {
        return Err("user_persona_update_soul: soul_hash is required".into());
    }
    let mut row = ctx
        .db
        .user_persona()
        .name()
        .find(&name)
        .ok_or_else(|| format!("user_persona_update_soul: '{}' not found", name))?;
    row.soul_hash = soul_hash;
    ctx.db.user_persona().name().update(row);
    Ok(())
}

#[reducer]
pub fn persona_pool_set(
    ctx: &ReducerContext,
    role: String,
    display_name: String,
    tier: String,
    paused: bool,
) -> Result<(), String> {
    if role.is_empty() {
        return Err("role is required".into());
    }
    let now = format!("{:?}", ctx.timestamp);
    let existing = ctx.db.persona_pool().role().find(&role);
    let row = PersonaPool {
        role: role.clone(),
        display_name,
        tier,
        paused,
        last_tick_at: existing
            .as_ref()
            .map(|e| e.last_tick_at.clone())
            .unwrap_or_default(),
        created_at: existing
            .as_ref()
            .map(|e| e.created_at.clone())
            .unwrap_or_else(|| now.clone()),
        updated_at: now,
    };
    if existing.is_some() {
        ctx.db.persona_pool().role().update(row);
    } else {
        ctx.db.persona_pool().insert(row);
    }
    Ok(())
}

#[reducer]
pub fn persona_pool_set_paused(
    ctx: &ReducerContext,
    role: String,
    paused: bool,
) -> Result<(), String> {
    let mut row = ctx
        .db
        .persona_pool()
        .role()
        .find(&role)
        .ok_or_else(|| format!("persona pool '{}' not found", role))?;
    row.paused = paused;
    row.updated_at = format!("{:?}", ctx.timestamp);
    ctx.db.persona_pool().role().update(row);
    ctx.db.persona_event().insert(PersonaEvent {
        id: 0,
        ts: format!("{:?}", ctx.timestamp),
        kind: if paused { "pause" } else { "resume" }.to_string(),
        role,
        payload: String::new(),
    });
    Ok(())
}

#[reducer]
pub fn persona_record_inference_failure(
    ctx: &ReducerContext,
    role: String,
    model_id: String,
    status_code: u32,
) -> Result<(), String> {
    let now_str = format!("{:?}", ctx.timestamp);
    let now_micros = parse_persona_ts_micros(&now_str).unwrap_or(0);
    let window_micros = PERSONA_FAILURE_WINDOW_SECS * 1_000_000;

    let existing = ctx.db.persona_health().role().find(&role);
    let recent = match &existing {
        Some(h) => {
            let last = parse_persona_ts_micros(&h.last_failure_at).unwrap_or(0);
            if now_micros > 0 && last > 0 && (now_micros - last) > window_micros {
                1
            } else {
                h.recent_failures + 1
            }
        }
        None => 1,
    };

    let banned_until = if recent >= PERSONA_FAILURE_THRESHOLD {
        let ban_until_micros = now_micros + (PERSONA_BAN_DURATION_SECS * 1_000_000);
        format!(
            "Timestamp {{ __timestamp_micros_since_unix_epoch__: {} }}",
            ban_until_micros
        )
    } else {
        existing
            .as_ref()
            .map(|h| h.banned_until.clone())
            .unwrap_or_default()
    };

    let row = PersonaHealth {
        role: role.clone(),
        recent_failures: recent,
        last_failure_at: now_str.clone(),
        last_failure_model: model_id.clone(),
        last_failure_status: status_code,
        banned_until: banned_until.clone(),
    };
    if existing.is_some() {
        ctx.db.persona_health().role().update(row);
    } else {
        ctx.db.persona_health().insert(row);
    }

    if recent >= PERSONA_FAILURE_THRESHOLD {
        ctx.db.persona_event().insert(PersonaEvent {
            id: 0,
            ts: now_str,
            kind: "ban".to_string(),
            role,
            payload: format!(
                r#"{{"model":"{}","status":{},"recent_failures":{},"banned_until":"{}"}}"#,
                model_id, status_code, recent, banned_until
            ),
        });
    }
    Ok(())
}

#[reducer]
pub fn persona_record_inference_success(
    ctx: &ReducerContext,
    role: String,
) -> Result<(), String> {
    if let Some(mut h) = ctx.db.persona_health().role().find(&role) {
        h.recent_failures = 0;
        h.banned_until = String::new();
        ctx.db.persona_health().role().update(h);
    }
    Ok(())
}

#[reducer]
pub fn persona_init(ctx: &ReducerContext) -> Result<(), String> {
    let now = format!("{:?}", ctx.timestamp);
    let seeds: &[(&str, &str, &str)] = &[
        ("cto", "Chief Technology Officer", "executive"),
        ("cpo", "Chief Product Officer", "executive"),
        ("coo", "Chief Operating Officer", "executive"),
        ("ciso", "Chief Information Security Officer", "executive"),
        ("chief-visionary", "Chief Visionary", "executive"),
        ("engineering-lead", "Engineering Lead", "lead"),
        ("product-lead", "Product Lead", "lead"),
        ("sre-lead", "SRE Lead", "lead"),
    ];
    for (role, name, tier) in seeds {
        if ctx.db.persona_pool().role().find(&role.to_string()).is_none() {
            ctx.db.persona_pool().insert(PersonaPool {
                role: role.to_string(),
                display_name: name.to_string(),
                tier: tier.to_string(),
                paused: false,
                last_tick_at: String::new(),
                created_at: now.clone(),
                updated_at: now.clone(),
            });
        }
    }

    let already = ctx.db.persona_tick_schedule().iter().next().is_some();
    if !already {
        let interval = ScheduleAt::Interval(
            std::time::Duration::from_secs(PERSONA_TICK_INTERVAL_SECS).into(),
        );
        ctx.db.persona_tick_schedule().insert(PersonaTickSchedule {
            scheduled_id: 0,
            scheduled_at: interval,
        });
        log::info!(
            "persona_init: tick scheduled every {}s",
            PERSONA_TICK_INTERVAL_SECS
        );
    }
    Ok(())
}

#[reducer]
pub fn persona_tick(
    ctx: &ReducerContext,
    _schedule: PersonaTickSchedule,
) -> Result<(), String> {
    let now_str = format!("{:?}", ctx.timestamp);
    let pools: Vec<PersonaPool> = ctx.db.persona_pool().iter().collect();

    for mut pool in pools {
        if pool.paused {
            continue;
        }

        let agent_id = format!("persona-{}", pool.role);

        if let Some(existing) = ctx.db.hex_agent().id().find(&agent_id) {
            ctx.db.hex_agent().id().update(HexAgent {
                status: "online".to_string(),
                last_heartbeat: now_str.clone(),
                ..existing
            });
        } else {
            let caps = format!(
                r#"{{"persona":true,"tier":"{}","display_name":"{}"}}"#,
                pool.tier, pool.display_name
            );
            ctx.db.hex_agent().insert(HexAgent {
                id: agent_id.clone(),
                name: pool.role.clone(),
                host: "stdb-supervised".to_string(),
                project_id: String::new(),
                project_dir: String::new(),
                model: String::new(),
                session_id: String::new(),
                status: "online".to_string(),
                swarm_id: String::new(),
                role: pool.role.clone(),
                worktree_path: String::new(),
                registered_at: now_str.clone(),
                last_heartbeat: now_str.clone(),
                capabilities_json: caps,
            });
            ctx.db.persona_event().insert(PersonaEvent {
                id: 0,
                ts: now_str.clone(),
                kind: "register".to_string(),
                role: pool.role.clone(),
                payload: format!(r#"{{"agent_id":"{}"}}"#, agent_id),
            });
        }

        pool.last_tick_at = now_str.clone();
        ctx.db.persona_pool().role().update(pool);
    }
    Ok(())
}

fn parse_persona_ts_micros(s: &str) -> Option<i64> {
    let key = "__timestamp_micros_since_unix_epoch__:";
    let pos = s.find(key)?;
    let tail = &s[pos + key.len()..];
    let end = tail
        .find(|c: char| !c.is_ascii_digit() && c != '-' && c != ' ')
        .unwrap_or(tail.len());
    tail[..end].trim().parse::<i64>().ok()
}

// ============================================================
//  Merge-Team Safety Gate (ADR-2026-05-08-1126 P1)
//
//  No agent writes to trunk. Every change to a hex-internal source file
//  originates inside a git worktree, opens a merge_request, accumulates
//  merge_vote rows from validation-judge + adversarial-red + adversarial-blue,
//  and lands via `hex worktree merge` only when the integrator subscriber
//  observes 2-of-3 PASS plus validation-judge=pass.
//
//  Per-pool quorum policies live in merge_quorum_policy: high-trust pools
//  can drop to 1-of-3, low-trust pools can require 3-of-3. Operator override
//  via `hex worktree approve` writes voter=operator verdict=pass and bypasses
//  the quorum (logged in merge_vote for audit).
// ============================================================

/// One merge request — opened by the daemon at end of CODE phase, consumed
/// by the integrator subscriber that orchestrates voting + merge.
///
/// `worktree_path` is the absolute path to the git worktree directory; used
/// as the primary key because each worktree carries at most one open merge
/// request at a time.
///
/// `status` lifecycle: pending → voting → (approved | rejected) → merged.
/// Once merged or rejected, the row is retained for audit (TTL purge can
/// be added later if `merge_request` grows large).
#[table(name = merge_request, public)]
#[derive(Clone, Debug)]
pub struct MergeRequest {
    #[primary_key]
    pub worktree_path: String,
    pub branch: String,
    /// Persona role that produced the change (e.g. "hex-coder", "rust-refactorer").
    pub role: String,
    pub opened_at: String,
    /// "pending" | "voting" | "approved" | "rejected" | "merged"
    pub status: String,
    /// Workplan id that drove the change, empty if ad-hoc.
    pub related_workplan: String,
    /// hex_agent.id of the agent that opened the request — for audit + alerting
    /// when a request stalls and needs an operator nudge.
    pub agent_id: String,
}

/// Open a merge request. Idempotent on `worktree_path` — re-opening just
/// updates the metadata + resets status to "pending" (voters can still
/// re-vote).
#[reducer]
pub fn merge_request_open(
    ctx: &ReducerContext,
    worktree_path: String,
    branch: String,
    role: String,
    related_workplan: String,
    agent_id: String,
) -> Result<(), String> {
    if worktree_path.is_empty() {
        return Err("worktree_path is required".into());
    }
    if branch.is_empty() {
        return Err("branch is required".into());
    }
    let now = format!("{:?}", ctx.timestamp);
    let row = MergeRequest {
        worktree_path: worktree_path.clone(),
        branch,
        role,
        opened_at: now,
        status: "pending".to_string(),
        related_workplan,
        agent_id,
    };
    if ctx.db.merge_request().worktree_path().find(&worktree_path).is_some() {
        ctx.db.merge_request().worktree_path().update(row);
    } else {
        ctx.db.merge_request().insert(row);
    }
    Ok(())
}

/// Update merge request status — called by the integrator subscriber as
/// votes accumulate. Allowed transitions enforced here so the integrator
/// can't mark something merged that wasn't approved.
#[reducer]
pub fn merge_request_set_status(
    ctx: &ReducerContext,
    worktree_path: String,
    new_status: String,
) -> Result<(), String> {
    let mut row = ctx
        .db
        .merge_request()
        .worktree_path()
        .find(&worktree_path)
        .ok_or_else(|| format!("merge_request '{}' not found", worktree_path))?;
    let allowed = matches!(
        (row.status.as_str(), new_status.as_str()),
        ("pending", "voting")
            | ("voting", "approved")
            | ("voting", "rejected")
            | ("approved", "merged")
            | ("pending", "rejected") // operator reject before voting starts
    );
    if !allowed {
        return Err(format!(
            "invalid transition: {} → {}",
            row.status, new_status
        ));
    }
    row.status = new_status;
    ctx.db.merge_request().worktree_path().update(row);
    Ok(())
}

/// One vote on a merge request. STDB lacks composite primary keys via the
/// `#[primary_key]` attribute, so we synthesize a string key
/// `<worktree_path>::<voter>` and enforce the (worktree, voter) uniqueness
/// at the reducer level. This pattern matches `(swarm, role)` keys
/// elsewhere in this module.
///
/// `voter`: "validation-judge" | "adversarial-red" | "adversarial-blue"
///        | "integrator" | "operator"
/// `verdict`: "pass" | "fail" | "abstain"
#[table(name = merge_vote, public)]
#[derive(Clone, Debug)]
pub struct MergeVote {
    /// Synthetic composite key: format!("{worktree_path}::{voter}").
    #[primary_key]
    pub key: String,
    pub worktree_path: String,
    pub voter: String,
    pub verdict: String,
    /// Free-form reason — judge writes spec failures here, adversarials
    /// write the boundary/correctness concern, operator writes override
    /// rationale. Cap at 4 KB at the reducer.
    pub reason: String,
    pub voted_at: String,
}

/// Cast a vote. Idempotent on (worktree_path, voter) — re-casting overwrites.
/// Validates voter + verdict against allowed sets; rejects unknown values
/// so the integrator never has to handle phantom roles.
#[reducer]
pub fn merge_vote_cast(
    ctx: &ReducerContext,
    worktree_path: String,
    voter: String,
    verdict: String,
    reason: String,
) -> Result<(), String> {
    if worktree_path.is_empty() {
        return Err("worktree_path is required".into());
    }
    let valid_voters = [
        "validation-judge",
        "adversarial-red",
        "adversarial-blue",
        "integrator",
        "operator",
    ];
    if !valid_voters.contains(&voter.as_str()) {
        return Err(format!(
            "invalid voter '{}': must be one of {:?}",
            voter, valid_voters
        ));
    }
    let valid_verdicts = ["pass", "fail", "abstain"];
    if !valid_verdicts.contains(&verdict.as_str()) {
        return Err(format!(
            "invalid verdict '{}': must be one of {:?}",
            verdict, valid_verdicts
        ));
    }
    if reason.len() > 4096 {
        return Err(format!(
            "reason too large: {} bytes (max 4096)",
            reason.len()
        ));
    }
    // Verify the merge_request exists — voting on a phantom worktree is
    // either a bug or a hijack attempt; either way we want to surface it.
    if ctx.db.merge_request().worktree_path().find(&worktree_path).is_none() {
        return Err(format!(
            "no merge_request open for worktree '{}'",
            worktree_path
        ));
    }
    let key = format!("{}::{}", worktree_path, voter);
    let now = format!("{:?}", ctx.timestamp);
    let row = MergeVote {
        key: key.clone(),
        worktree_path,
        voter,
        verdict,
        reason,
        voted_at: now,
    };
    if ctx.db.merge_vote().key().find(&key).is_some() {
        ctx.db.merge_vote().key().update(row);
    } else {
        ctx.db.merge_vote().insert(row);
    }
    Ok(())
}

/// Per-pool quorum policy. Default policy is encoded as `pool_id="*"` row.
/// Specific pool policies override the default. The integrator subscriber
/// reads policy by pool_id when tallying votes.
///
/// `min_pass_votes`: how many "pass" verdicts are required for approval.
///   Default 2 (out of 3 voters: judge + red + blue).
///
/// `require_judge_pass`: when true, validation-judge MUST vote pass.
///   Even if 3-of-3 adversarials pass, a judge=fail blocks merge.
///   Default true — the judge runs behavioral specs, that's load-bearing.
///
/// `allow_operator_override`: when true, voter=operator with verdict=pass
///   bypasses the quorum (the operator is acknowledging risk).
///   Default true; settable false for high-trust pools where override
///   would defeat the gate.
#[table(name = merge_quorum_policy, public)]
#[derive(Clone, Debug)]
pub struct MergeQuorumPolicy {
    /// `*` = default, otherwise matches worker_pool_intent.id or persona role.
    #[primary_key]
    pub pool_id: String,
    pub min_pass_votes: u32,
    pub require_judge_pass: bool,
    pub allow_operator_override: bool,
    pub created_at: String,
    pub updated_at: String,
}

/// Set or replace a quorum policy for a pool. Idempotent.
#[reducer]
pub fn merge_quorum_policy_set(
    ctx: &ReducerContext,
    pool_id: String,
    min_pass_votes: u32,
    require_judge_pass: bool,
    allow_operator_override: bool,
) -> Result<(), String> {
    if pool_id.is_empty() {
        return Err("pool_id is required (use '*' for default)".into());
    }
    if min_pass_votes == 0 || min_pass_votes > 5 {
        return Err(format!(
            "min_pass_votes={} out of range (1-5)",
            min_pass_votes
        ));
    }
    let now = format!("{:?}", ctx.timestamp);
    let existing = ctx.db.merge_quorum_policy().pool_id().find(&pool_id);
    let row = MergeQuorumPolicy {
        pool_id: pool_id.clone(),
        min_pass_votes,
        require_judge_pass,
        allow_operator_override,
        created_at: existing
            .as_ref()
            .map(|e| e.created_at.clone())
            .unwrap_or_else(|| now.clone()),
        updated_at: now,
    };
    if existing.is_some() {
        ctx.db.merge_quorum_policy().pool_id().update(row);
    } else {
        ctx.db.merge_quorum_policy().insert(row);
    }
    Ok(())
}

/// One-shot init: seed the default merge_quorum_policy row (`*` →
/// 2-of-3 with judge=pass required, operator override allowed). Idempotent.
/// Operators tune per-pool policies via merge_quorum_policy_set after init.
#[reducer]
pub fn merge_team_init(ctx: &ReducerContext) -> Result<(), String> {
    if ctx
        .db
        .merge_quorum_policy()
        .pool_id()
        .find(&"*".to_string())
        .is_none()
    {
        let now = format!("{:?}", ctx.timestamp);
        ctx.db.merge_quorum_policy().insert(MergeQuorumPolicy {
            pool_id: "*".to_string(),
            min_pass_votes: 2,
            require_judge_pass: true,
            allow_operator_override: true,
            created_at: now.clone(),
            updated_at: now,
        });
        log::info!("merge_team_init: seeded default policy (2-of-3, judge=pass required)");
    }
    Ok(())
}

/// Compute the merge decision for a worktree based on current votes vs
/// applicable quorum policy, and write it back to `merge_request.status`.
///
/// Status transitions on call:
///   pending/voting → "voting"            (votes still needed)
///   pending/voting → "approved"          (quorum + judge=pass)
///   pending/voting → "rejected"          (judge=fail OR pass quorum unreachable)
///
/// Caller side-effects:
///   - Integrator subscriber polls `merge_request WHERE status = 'approved'`
///     and runs `hex worktree merge`, then transitions to "merged".
///   - Operator override (voter=operator verdict=pass with policy allowing
///     it) writes status="approved" with reason embedded so the integrator
///     can audit the override path distinctly.
///
/// Idempotent — calling multiple times with no new votes yields the same
/// status. Already-merged or already-rejected requests are not transitioned.
#[reducer]
pub fn merge_decision_tally(
    ctx: &ReducerContext,
    worktree_path: String,
) -> Result<(), String> {
    let mr = ctx
        .db
        .merge_request()
        .worktree_path()
        .find(&worktree_path)
        .ok_or_else(|| format!("merge_request '{}' not found", worktree_path))?;

    // Terminal states are sticky.
    if mr.status == "merged" || mr.status == "rejected" {
        return Ok(());
    }

    // Resolve policy: pool-specific row first, fall back to default.
    let policy = ctx
        .db
        .merge_quorum_policy()
        .pool_id()
        .find(&mr.role)
        .or_else(|| {
            ctx.db
                .merge_quorum_policy()
                .pool_id()
                .find(&"*".to_string())
        })
        .ok_or_else(|| {
            "no merge_quorum_policy configured (call merge_team_init)".to_string()
        })?;

    let votes: Vec<MergeVote> = ctx
        .db
        .merge_vote()
        .iter()
        .filter(|v| v.worktree_path == worktree_path)
        .collect();

    let mut pass_count: u32 = 0;
    let mut fail_count: u32 = 0;
    let mut judge_verdict: Option<String> = None;
    let mut operator_pass = false;
    let mut operator_fail = false;

    for v in &votes {
        match v.verdict.as_str() {
            "pass" => pass_count += 1,
            "fail" => fail_count += 1,
            _ => {}
        }
        if v.voter == "validation-judge" {
            judge_verdict = Some(v.verdict.clone());
        }
        if v.voter == "operator" && v.verdict == "pass" {
            operator_pass = true;
        }
        if v.voter == "operator" && v.verdict == "fail" {
            operator_fail = true;
        }
    }

    let new_status = if operator_fail {
        "rejected"
    } else if operator_pass && policy.allow_operator_override {
        "approved"
    } else if policy.require_judge_pass && judge_verdict.as_deref() == Some("fail") {
        "rejected"
    } else if pass_count >= policy.min_pass_votes
        && (!policy.require_judge_pass || judge_verdict.as_deref() == Some("pass"))
    {
        "approved"
    } else {
        // Can the request still possibly approve? max 4 voters (judge+red+blue+integrator).
        let max_voters: u32 = 4;
        let remaining = max_voters.saturating_sub(pass_count + fail_count);
        if pass_count + remaining < policy.min_pass_votes {
            "rejected"
        } else {
            "voting"
        }
    };

    if new_status != mr.status {
        let mut updated = mr.clone();
        updated.status = new_status.to_string();
        ctx.db.merge_request().worktree_path().update(updated);
        log::info!(
            "merge_decision_tally: {} → {} (passes={}, fails={}, judge={:?})",
            mr.status,
            new_status,
            pass_count,
            fail_count,
            judge_verdict
        );
    }
    Ok(())
}

// ──────────────────────────────────────────────────────────────────────
// ADR-2026-05-08-2200 — Resource-aware supervisor
//
// Adds /proc-derived process observations (RSS, CPU, ppid, state, argv
// signature) plus a 60s tick that emits resource_anomaly rows for
// duplicates, oversize RSS, zombies, and CPU pin. No auto-kill.
// ──────────────────────────────────────────────────────────────────────

#[table(name = process_observation, public)]
#[derive(Clone, Debug)]
pub struct ProcessObservation {
    /// OS pid. One row per live PID.
    #[primary_key]
    pub pid: u32,
    /// Hostname the observation came from. Multi-host stretch goal.
    pub host: String,
    /// SHA-256 of the full argv joined by NUL — the "this is the same
    /// program invocation" key for duplicate detection.
    pub argv_sha: String,
    /// First ~120 chars of the argv, for human readability.
    pub argv_first: String,
    /// /proc/<pid>/stat field 3 — R, S, D, Z, T, …
    pub state: String,
    pub ppid: u32,
    /// Process start time (Unix micros). Stable identity for the PID.
    pub started_micros: i64,
    pub rss_kb: u64,
    /// CPU % over the last observation window (0..n_cores*100).
    pub cpu_pct: f32,
    /// When this row was last upserted. Used by prune.
    pub observed_at: Timestamp,
}

#[table(name = resource_anomaly, public)]
#[derive(Clone, Debug)]
pub struct ResourceAnomaly {
    #[primary_key]
    #[auto_inc]
    pub id: u64,
    pub detected_at: Timestamp,
    /// duplicate_argv | rss_oversize | zombie | cpu_pin
    pub kind: String,
    /// info | warn | critical
    pub severity: String,
    /// JSON list of involved PIDs.
    pub pids: String,
    /// Human-readable explanation (argv excerpt, threshold crossed, …).
    pub note: String,
    pub handled: bool,
    pub handled_at: String,
    pub handled_by: String,
}

/// Schedule anchor for `resource_supervisor_tick`.
#[table(name = resource_supervisor_tick_schedule, public, scheduled(resource_supervisor_tick))]
#[derive(Clone, Debug)]
pub struct ResourceSupervisorTickSchedule {
    #[primary_key]
    #[auto_inc]
    pub scheduled_id: u64,
    pub scheduled_at: ScheduleAt,
}

/// Upsert one process observation. Called by the nexus /proc walker.
/// Identity is `pid` — STDB keeps one row per PID.
#[reducer]
pub fn process_observation_upsert(
    ctx: &ReducerContext,
    pid: u32,
    host: String,
    argv_sha: String,
    argv_first: String,
    state: String,
    ppid: u32,
    started_micros: i64,
    rss_kb: u64,
    cpu_pct: f32,
) -> Result<(), String> {
    let now = ctx.timestamp;
    let row = ProcessObservation {
        pid,
        host,
        argv_sha,
        argv_first,
        state,
        ppid,
        started_micros,
        rss_kb,
        cpu_pct,
        observed_at: now,
    };
    if ctx.db.process_observation().pid().find(&pid).is_some() {
        ctx.db.process_observation().pid().update(row);
    } else {
        ctx.db.process_observation().insert(row);
    }
    Ok(())
}

/// Drop process_observation rows whose `observed_at` is older than
/// `stale_seconds`. Caller invokes this each tick — by definition any
/// PID whose entry isn't refreshed has either died or fallen out of the
/// observer's allow-list.
#[reducer]
pub fn process_observation_prune(
    ctx: &ReducerContext,
    stale_seconds: u32,
) -> Result<(), String> {
    let now_micros = parse_ts_micros_resource(&format!("{:?}", ctx.timestamp)).unwrap_or(0);
    let cutoff = now_micros - (stale_seconds as i64 * 1_000_000);
    let stale_pids: Vec<u32> = ctx
        .db
        .process_observation()
        .iter()
        .filter(|p| {
            parse_ts_micros_resource(&format!("{:?}", p.observed_at))
                .map(|m| m < cutoff)
                .unwrap_or(false)
        })
        .map(|p| p.pid)
        .collect();
    for pid in &stale_pids {
        ctx.db.process_observation().pid().delete(pid);
    }
    if !stale_pids.is_empty() {
        log::info!("process_observation_prune: dropped {} stale PIDs", stale_pids.len());
    }
    Ok(())
}

/// Idempotent init for the resource supervisor schedule. Calls from
/// nexus on every boot — duplicate inserts are guarded.
#[reducer]
pub fn resource_supervisor_init(ctx: &ReducerContext) -> Result<(), String> {
    if ctx.db.resource_supervisor_tick_schedule().iter().next().is_some() {
        log::info!("resource_supervisor_init: schedule already exists, skipping");
        return Ok(());
    }
    let interval = ScheduleAt::Interval(std::time::Duration::from_secs(60).into());
    ctx.db.resource_supervisor_tick_schedule().insert(ResourceSupervisorTickSchedule {
        scheduled_id: 0,
        scheduled_at: interval,
    });
    log::info!("resource_supervisor_init: tick scheduled every 60s");
    Ok(())
}

/// THE resource tick. Fires every 60 s.
///
///   1. Prune observations older than 120 s (PID gone).
///   2. Group live observations by argv_sha. Any sha with > 1 alive
///      pid → duplicate_argv anomaly (severity warn).
///   3. RSS thresholds: > 30 GiB → critical, > 20 GiB → warn.
///   4. state == "Z" → zombie critical.
///   5. cpu_pct > 800 % → cpu_pin warn.
///
/// Anomaly de-dup: before inserting, scan the most recent UNHANDLED
/// anomaly of (kind, sorted_pids) tuple. If one exists within the last
/// 5 minutes, suppress the new emit. This keeps the inbox quiet while
/// the operator is investigating.
#[reducer]
pub fn resource_supervisor_tick(
    ctx: &ReducerContext,
    _schedule: ResourceSupervisorTickSchedule,
) -> Result<(), String> {
    process_observation_prune(ctx, 120)?;

    let now = ctx.timestamp;
    let now_micros = parse_ts_micros_resource(&format!("{:?}", now)).unwrap_or(0);
    let suppress_window_micros: i64 = 5 * 60 * 1_000_000;

    let observations: Vec<ProcessObservation> = ctx.db.process_observation().iter().collect();
    let recent_anomalies: Vec<ResourceAnomaly> = ctx
        .db
        .resource_anomaly()
        .iter()
        .filter(|a| {
            !a.handled
                && parse_ts_micros_resource(&format!("{:?}", a.detected_at))
                    .map(|m| (now_micros - m) < suppress_window_micros)
                    .unwrap_or(false)
        })
        .collect();

    let already_open = |kind: &str, pids: &[u32]| -> bool {
        let mut sorted = pids.to_vec();
        sorted.sort_unstable();
        let key = sorted
            .iter()
            .map(|p| p.to_string())
            .collect::<Vec<_>>()
            .join(",");
        recent_anomalies.iter().any(|a| {
            a.kind == kind
                && {
                    // a.pids is `[1,2,3]` or `[]` JSON; cheap exact match
                    let want = format!("[{}]", key);
                    a.pids == want
                }
        })
    };

    let mut to_emit: Vec<(String, String, Vec<u32>, String)> = Vec::new();

    // Duplicate argv detection.
    use std::collections::HashMap;
    let mut by_sha: HashMap<String, Vec<&ProcessObservation>> = HashMap::new();
    for o in &observations {
        by_sha.entry(o.argv_sha.clone()).or_default().push(o);
    }
    for (sha, group) in &by_sha {
        if group.len() <= 1 {
            continue;
        }
        let pids: Vec<u32> = group.iter().map(|p| p.pid).collect();
        if already_open("duplicate_argv", &pids) {
            continue;
        }
        let argv_excerpt = group
            .first()
            .map(|p| p.argv_first.clone())
            .unwrap_or_default();
        to_emit.push((
            "duplicate_argv".to_string(),
            "warn".to_string(),
            pids,
            format!("{} processes share argv_sha={} argv={}", group.len(), sha, argv_excerpt),
        ));
    }

    for o in &observations {
        let single = vec![o.pid];

        if o.state == "Z" && !already_open("zombie", &single) {
            to_emit.push((
                "zombie".to_string(),
                "critical".to_string(),
                single.clone(),
                format!("zombie PID {} (parent {}) — needs reaping", o.pid, o.ppid),
            ));
        }

        let rss_gib = (o.rss_kb as f64) / (1024.0 * 1024.0);
        if rss_gib > 30.0 && !already_open("rss_oversize", &single) {
            to_emit.push((
                "rss_oversize".to_string(),
                "critical".to_string(),
                single.clone(),
                format!("PID {} RSS {:.1} GiB ({})", o.pid, rss_gib, o.argv_first),
            ));
        } else if rss_gib > 20.0 && !already_open("rss_oversize", &single) {
            to_emit.push((
                "rss_oversize".to_string(),
                "warn".to_string(),
                single.clone(),
                format!("PID {} RSS {:.1} GiB ({})", o.pid, rss_gib, o.argv_first),
            ));
        }

        if o.cpu_pct > 800.0 && !already_open("cpu_pin", &single) {
            to_emit.push((
                "cpu_pin".to_string(),
                "warn".to_string(),
                single.clone(),
                format!("PID {} CPU {:.0}% ({})", o.pid, o.cpu_pct, o.argv_first),
            ));
        }
    }

    let emit_count = to_emit.len();
    for (kind, severity, pids, note) in to_emit {
        let mut sorted = pids.clone();
        sorted.sort_unstable();
        let pids_json = format!(
            "[{}]",
            sorted
                .iter()
                .map(|p| p.to_string())
                .collect::<Vec<_>>()
                .join(",")
        );
        ctx.db.resource_anomaly().insert(ResourceAnomaly {
            id: 0,
            detected_at: now,
            kind,
            severity,
            pids: pids_json,
            note,
            handled: false,
            handled_at: String::new(),
            handled_by: String::new(),
        });
    }
    if emit_count > 0 {
        log::info!("resource_supervisor_tick: emitted {} anomalies", emit_count);
    }
    Ok(())
}

#[reducer]
pub fn resource_anomaly_ack(
    ctx: &ReducerContext,
    id: u64,
    handled_by: String,
) -> Result<(), String> {
    let mut row = ctx
        .db
        .resource_anomaly()
        .id()
        .find(&id)
        .ok_or_else(|| format!("resource_anomaly id={} not found", id))?;
    if row.handled {
        return Ok(());
    }
    row.handled = true;
    row.handled_at = format!("{:?}", ctx.timestamp);
    row.handled_by = handled_by;
    ctx.db.resource_anomaly().id().update(row);
    Ok(())
}

/// Local helper — same shape as the supervisor_tick parser, kept private
/// so this module owns its own ts parsing in case the upstream one
/// changes.
fn parse_ts_micros_resource(s: &str) -> Option<i64> {
    let key = "__timestamp_micros_since_unix_epoch__:";
    let pos = s.find(key)?;
    let tail = &s[pos + key.len()..];
    let end = tail
        .find(|c: char| !c.is_ascii_digit() && c != '-' && c != ' ')
        .unwrap_or(tail.len());
    tail[..end].trim().parse::<i64>().ok()
}

// ──────────────────────────────────────────────────────────────────────
// Commitment ledger
//
// Every persona "Confirm: I will X by Y, success = Z" reply in a board /
// group thread is parsed by nexus and written here. The 10-minute
// commitment_check_tick flips overdue rows so the operator sees who
// said-but-didn't on the dashboard, and emits a resource_anomaly so
// nothing slips silently.
// ──────────────────────────────────────────────────────────────────────

#[table(name = commitment, public)]
#[derive(Clone, Debug)]
pub struct Commitment {
    #[primary_key]
    #[auto_inc]
    pub id: u64,
    /// Persona role that made the promise.
    pub role: String,
    /// Verbatim Confirm/PLAN line as written by the persona — operator
    /// audit trail.
    pub raw_text: String,
    /// Extracted action ("draft ADR-XXX", "post status update", …).
    pub action: String,
    /// Unix micros. 0 = no explicit deadline (default 1h applied at check).
    pub deadline_micros: i64,
    /// What "done" looks like: file path, dashboard hashroute, STDB
    /// table name, or the literal "requires-operator-action".
    pub success_artifact: String,
    /// "verifiable_path" | "verifiable_route" | "operator_action" | "none"
    pub artifact_kind: String,
    pub thread_id: String,
    pub related_msg_id: u64,
    pub created_at: Timestamp,
    /// "open" | "satisfied" | "overdue" | "abandoned"
    pub status: String,
    pub last_checked: Timestamp,
    pub note: String,
}

#[table(name = commitment_check_schedule, public, scheduled(commitment_check_tick))]
#[derive(Clone, Debug)]
pub struct CommitmentCheckSchedule {
    #[primary_key]
    #[auto_inc]
    pub scheduled_id: u64,
    pub scheduled_at: ScheduleAt,
}

#[reducer]
pub fn commitment_init(ctx: &ReducerContext) -> Result<(), String> {
    if ctx.db.commitment_check_schedule().iter().next().is_some() {
        log::info!("commitment_init: schedule already exists, skipping");
        return Ok(());
    }
    let interval = ScheduleAt::Interval(std::time::Duration::from_secs(600).into());
    ctx.db.commitment_check_schedule().insert(CommitmentCheckSchedule {
        scheduled_id: 0,
        scheduled_at: interval,
    });
    log::info!("commitment_init: tick scheduled every 600s");
    Ok(())
}

#[reducer]
pub fn commitment_open(
    ctx: &ReducerContext,
    role: String,
    raw_text: String,
    action: String,
    deadline_micros: i64,
    success_artifact: String,
    artifact_kind: String,
    thread_id: String,
    related_msg_id: u64,
) -> Result<(), String> {
    let now = ctx.timestamp;
    ctx.db.commitment().insert(Commitment {
        id: 0,
        role,
        raw_text,
        action,
        deadline_micros,
        success_artifact,
        artifact_kind,
        thread_id,
        related_msg_id,
        created_at: now,
        status: "open".to_string(),
        last_checked: now,
        note: String::new(),
    });
    Ok(())
}

#[reducer]
pub fn commitment_satisfy(
    ctx: &ReducerContext,
    id: u64,
    evidence: String,
) -> Result<(), String> {
    let mut row = ctx
        .db
        .commitment()
        .id()
        .find(&id)
        .ok_or_else(|| format!("commitment id={} not found", id))?;
    if row.status == "satisfied" {
        return Ok(());
    }
    row.status = "satisfied".to_string();
    row.note = evidence;
    row.last_checked = ctx.timestamp;
    ctx.db.commitment().id().update(row);
    Ok(())
}

#[reducer]
pub fn commitment_abandon(
    ctx: &ReducerContext,
    id: u64,
    reason: String,
) -> Result<(), String> {
    let mut row = ctx
        .db
        .commitment()
        .id()
        .find(&id)
        .ok_or_else(|| format!("commitment id={} not found", id))?;
    if row.status == "abandoned" {
        return Ok(());
    }
    row.status = "abandoned".to_string();
    row.note = reason;
    row.last_checked = ctx.timestamp;
    ctx.db.commitment().id().update(row);
    Ok(())
}

/// Scheduled every 600s. For each open commitment whose deadline has
/// passed (or whose default 1h grace expired), flip to "overdue" and
/// emit a resource_anomaly so the operator sees it without checking the
/// commitments page.
#[reducer]
pub fn commitment_check_tick(
    ctx: &ReducerContext,
    _schedule: CommitmentCheckSchedule,
) -> Result<(), String> {
    let now = ctx.timestamp;
    let now_micros = parse_ts_micros_resource(&format!("{:?}", now)).unwrap_or(0);
    let default_grace_micros: i64 = 60 * 60 * 1_000_000; // 1h fallback

    let open: Vec<Commitment> = ctx
        .db
        .commitment()
        .iter()
        .filter(|c| c.status == "open")
        .collect();

    let mut flipped = 0u32;
    for c in open {
        let effective_deadline = if c.deadline_micros > 0 {
            c.deadline_micros
        } else {
            // No explicit deadline — give 1h from creation, then flag.
            parse_ts_micros_resource(&format!("{:?}", c.created_at))
                .map(|m| m + default_grace_micros)
                .unwrap_or(0)
        };
        if effective_deadline > 0 && now_micros > effective_deadline {
            let mut row = c.clone();
            row.status = "overdue".to_string();
            row.last_checked = now;
            ctx.db.commitment().id().update(row);
            flipped += 1;

            // Surface in the same anomaly inbox the operator already
            // watches — keep one alerting surface.
            let pids = format!("[{}]", c.id);
            let note = format!(
                "{} promised: {} (artifact={}, thread={})",
                c.role, c.action, c.success_artifact, c.thread_id
            );
            ctx.db.resource_anomaly().insert(ResourceAnomaly {
                id: 0,
                detected_at: now,
                kind: "overdue_commitment".to_string(),
                severity: "warn".to_string(),
                pids,
                note,
                handled: false,
                handled_at: String::new(),
                handled_by: String::new(),
            });
        }
    }
    if flipped > 0 {
        log::info!("commitment_check_tick: flipped {} commitments to overdue", flipped);
    }
    Ok(())
}

// ──────────────────────────────────────────────────────────────────────
// Digital-twin action queue (ADR-2026-05-08-2300)
//
// Personas can't write files. The drafter task picks up open commitments
// with verifiable_path artifacts, asks the proposer for the actual
// content, and writes a proposed_action row. The twin reviews, approves
// or rejects against operator memory + standards. The executor runs
// approved actions. Escalations land in the inbox.
// ──────────────────────────────────────────────────────────────────────

#[table(name = proposed_action, public)]
#[derive(Clone, Debug)]
pub struct ProposedAction {
    #[primary_key]
    #[auto_inc]
    pub id: u64,
    /// "file_write" today; "shell_exec" / "dm_send" / etc. in follow-on ADRs.
    pub kind: String,
    /// JSON-encoded action payload. For file_write: {"path":"...","content":"..."}.
    pub payload_json: String,
    pub proposed_by: String,
    /// 0 if not derived from a commitment.
    pub related_commitment_id: u64,
    /// "pending" | "approved" | "rejected" | "escalated" | "executed" | "execution_failed"
    pub status: String,
    /// Twin verdict ("approve"/"reject"/"escalate") + rationale.
    pub twin_verdict: String,
    pub twin_rationale: String,
    pub escalate_reason: String,
    pub proposed_at: Timestamp,
    pub decided_at: String,
    pub executed_at: String,
    /// Operator overrode the twin? (audit trail)
    pub operator_override: bool,
    pub operator_reason: String,
}

#[table(name = executed_action, public)]
#[derive(Clone, Debug)]
pub struct ExecutedAction {
    #[primary_key]
    #[auto_inc]
    pub id: u64,
    pub proposed_action_id: u64,
    pub kind: String,
    pub payload_json: String,
    pub success: bool,
    pub error: String,
    pub executed_at: Timestamp,
    /// What evidence was written back to the commitment.
    pub evidence: String,
}

#[reducer]
pub fn proposed_action_open(
    ctx: &ReducerContext,
    kind: String,
    payload_json: String,
    proposed_by: String,
    related_commitment_id: u64,
) -> Result<(), String> {
    // Idempotency: if a pending row for the same (related_commitment_id,
    // kind) exists, no-op. Prevents the drafter from racing itself.
    if related_commitment_id > 0 {
        let dup = ctx
            .db
            .proposed_action()
            .iter()
            .any(|p| {
                p.related_commitment_id == related_commitment_id
                    && p.kind == kind
                    && (p.status == "pending" || p.status == "approved" || p.status == "executed")
            });
        if dup {
            return Ok(());
        }
    }
    ctx.db.proposed_action().insert(ProposedAction {
        id: 0,
        kind,
        payload_json,
        proposed_by,
        related_commitment_id,
        status: "pending".to_string(),
        twin_verdict: String::new(),
        twin_rationale: String::new(),
        escalate_reason: String::new(),
        proposed_at: ctx.timestamp,
        decided_at: String::new(),
        executed_at: String::new(),
        operator_override: false,
        operator_reason: String::new(),
    });
    Ok(())
}

#[reducer]
pub fn proposed_action_twin_decide(
    ctx: &ReducerContext,
    id: u64,
    verdict: String,
    rationale: String,
    escalate_reason: String,
) -> Result<(), String> {
    let mut row = ctx
        .db
        .proposed_action()
        .id()
        .find(&id)
        .ok_or_else(|| format!("proposed_action id={} not found", id))?;
    if row.status != "pending" {
        return Ok(()); // already decided
    }
    let new_status = match verdict.as_str() {
        "approve" => "approved",
        "reject" => "rejected",
        "escalate" => "escalated",
        other => return Err(format!("invalid verdict: {}", other)),
    };
    row.status = new_status.to_string();
    row.twin_verdict = verdict;
    row.twin_rationale = rationale;
    row.escalate_reason = escalate_reason;
    row.decided_at = format!("{:?}", ctx.timestamp);
    ctx.db.proposed_action().id().update(row);
    Ok(())
}

#[reducer]
pub fn proposed_action_mark_executed(
    ctx: &ReducerContext,
    id: u64,
    success: bool,
    error: String,
    evidence: String,
) -> Result<(), String> {
    let mut row = ctx
        .db
        .proposed_action()
        .id()
        .find(&id)
        .ok_or_else(|| format!("proposed_action id={} not found", id))?;
    row.status = if success {
        "executed".to_string()
    } else {
        "execution_failed".to_string()
    };
    row.executed_at = format!("{:?}", ctx.timestamp);
    let kind = row.kind.clone();
    let payload = row.payload_json.clone();
    ctx.db.proposed_action().id().update(row);

    ctx.db.executed_action().insert(ExecutedAction {
        id: 0,
        proposed_action_id: id,
        kind,
        payload_json: payload,
        success,
        error,
        executed_at: ctx.timestamp,
        evidence,
    });
    Ok(())
}

#[reducer]
pub fn proposed_action_operator_override(
    ctx: &ReducerContext,
    id: u64,
    new_status: String,
    reason: String,
) -> Result<(), String> {
    let mut row = ctx
        .db
        .proposed_action()
        .id()
        .find(&id)
        .ok_or_else(|| format!("proposed_action id={} not found", id))?;
    if !matches!(
        new_status.as_str(),
        "approved" | "rejected" | "pending" | "escalated"
    ) {
        return Err(format!("invalid override status: {}", new_status));
    }
    row.status = new_status;
    row.operator_override = true;
    row.operator_reason = reason;
    row.decided_at = format!("{:?}", ctx.timestamp);
    ctx.db.proposed_action().id().update(row);
    Ok(())
}

// ──────────────────────────────────────────────────────────────────────
// Persona turn claim (ADR-2026-05-08-2400)
//
// Closes the 5-PLANs-simultaneously race. The first persona to call
// commitment_thread_claim wins the right to emit a Confirm row for that
// thread; subsequent claimants get an error and stay silent. STDB
// unique-on-thread_id enforces this atomically — no read-by lag matters.
// ──────────────────────────────────────────────────────────────────────

#[table(name = commitment_thread_claim, public)]
#[derive(Clone, Debug)]
pub struct CommitmentThreadClaim {
    /// One row per thread; PK enforces single-claim.
    #[primary_key]
    pub thread_id: String,
    pub claimed_by: String,
    pub claimed_at: Timestamp,
    /// The CEO message id that started the thread, for audit + dashboard.
    pub originating_msg_id: u64,
}

#[reducer]
pub fn claim_persona_turn(
    ctx: &ReducerContext,
    thread_id: String,
    claimed_by: String,
    originating_msg_id: u64,
) -> Result<(), String> {
    if thread_id.is_empty() {
        return Err("empty thread_id".to_string());
    }
    if ctx
        .db
        .commitment_thread_claim()
        .thread_id()
        .find(&thread_id)
        .is_some()
    {
        return Err(format!("thread {} already claimed", thread_id));
    }
    ctx.db.commitment_thread_claim().insert(CommitmentThreadClaim {
        thread_id,
        claimed_by,
        claimed_at: ctx.timestamp,
        originating_msg_id,
    });
    Ok(())
}
