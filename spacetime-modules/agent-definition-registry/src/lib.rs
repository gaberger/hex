#![allow(clippy::too_many_arguments, clippy::needless_borrows_for_generic_args)]

use spacetimedb::{reducer, table, ReducerContext, Table};

// ── Tables ──────────────────────────────────────────────

/// An agent definition — a persona the LLM adopts with specific constraints.
#[table(name = agent_definition, public)]
#[derive(Clone, Debug)]
pub struct AgentDefinition {
    #[unique]
    pub id: String,
    /// Unique agent name (e.g., "hex-coder", "planner")
    #[unique]
    pub name: String,
    pub description: String,
    /// System prompt additions specific to this agent's role
    pub role_prompt: String,
    /// JSON-encoded Vec<String> — tools this agent may use (empty = all)
    pub allowed_tools_json: String,
    /// JSON-encoded AgentConstraints
    pub constraints_json: String,
    /// Model override (empty = use global setting)
    pub model: String,
    /// Max turns before the agent must stop
    pub max_turns: u32,
    /// JSON-encoded HashMap<String, String>
    pub metadata_json: String,
    /// Monotonically increasing version number
    pub version: u32,
    pub created_at: String,
    pub updated_at: String,
}

/// Version history — snapshot of each definition revision for audit trail.
#[table(name = agent_definition_version, public)]
#[derive(Clone, Debug)]
pub struct AgentDefinitionVersion {
    pub definition_id: String,
    pub version: u32,
    /// Full JSON snapshot of the AgentDefinition at this version
    pub snapshot_json: String,
    pub created_at: String,
}

// ── Reducers ────────────────────────────────────────────

#[reducer]
pub fn register_definition(
    ctx: &ReducerContext,
    id: String,
    name: String,
    description: String,
    role_prompt: String,
    allowed_tools_json: String,
    constraints_json: String,
    model: String,
    max_turns: u32,
    metadata_json: String,
    timestamp: String,
) -> Result<(), String> {
    // Validate JSON fields
    validate_json(&allowed_tools_json, "allowed_tools_json")?;
    validate_json(&constraints_json, "constraints_json")?;
    validate_json(&metadata_json, "metadata_json")?;

    let def = AgentDefinition {
        id: id.clone(),
        name,
        description,
        role_prompt,
        allowed_tools_json,
        constraints_json,
        model,
        max_turns,
        metadata_json,
        version: 1,
        created_at: timestamp.clone(),
        updated_at: timestamp.clone(),
    };

    // Create version snapshot
    let snapshot = serde_json::to_string(&DefinitionSnapshot::from_def(&def))
        .map_err(|e| format!("Snapshot serialization failed: {}", e))?;

    ctx.db.agent_definition().insert(def);

    ctx.db
        .agent_definition_version()
        .insert(AgentDefinitionVersion {
            definition_id: id,
            version: 1,
            snapshot_json: snapshot,
            created_at: timestamp,
        });

    Ok(())
}

#[reducer]
pub fn update_definition(
    ctx: &ReducerContext,
    id: String,
    description: String,
    role_prompt: String,
    allowed_tools_json: String,
    constraints_json: String,
    model: String,
    max_turns: u32,
    metadata_json: String,
    timestamp: String,
) -> Result<(), String> {
    validate_json(&allowed_tools_json, "allowed_tools_json")?;
    validate_json(&constraints_json, "constraints_json")?;
    validate_json(&metadata_json, "metadata_json")?;

    let existing = ctx
        .db
        .agent_definition()
        .id()
        .find(&id)
        .ok_or_else(|| format!("AgentDefinition '{}' not found", id))?;

    let new_version = existing.version + 1;

    let updated = AgentDefinition {
        description,
        role_prompt,
        allowed_tools_json,
        constraints_json,
        model,
        max_turns,
        metadata_json,
        version: new_version,
        updated_at: timestamp.clone(),
        ..existing
    };

    // Create version snapshot before updating
    let snapshot = serde_json::to_string(&DefinitionSnapshot::from_def(&updated))
        .map_err(|e| format!("Snapshot serialization failed: {}", e))?;

    ctx.db.agent_definition().id().update(updated);

    ctx.db
        .agent_definition_version()
        .insert(AgentDefinitionVersion {
            definition_id: id,
            version: new_version,
            snapshot_json: snapshot,
            created_at: timestamp,
        });

    Ok(())
}

#[reducer]
pub fn remove_definition(ctx: &ReducerContext, id: String) -> Result<(), String> {
    let deleted = ctx.db.agent_definition().id().delete(&id);
    if !deleted {
        return Err(format!("AgentDefinition '{}' not found", id));
    }
    // Version history is kept for audit — not deleted
    Ok(())
}

/// Lookup helper — clients subscribe to the agent_definition table and filter
/// locally by name. This reducer exists for API completeness.
#[reducer]
pub fn get_definition_by_name(ctx: &ReducerContext, name: String) -> Result<(), String> {
    let _found = ctx
        .db
        .agent_definition()
        .name()
        .find(&name)
        .ok_or_else(|| format!("AgentDefinition with name '{}' not found", name))?;
    Ok(())
}

// ── Internal Types ──────────────────────────────────────

/// Serializable snapshot for version history.
#[derive(serde::Serialize)]
struct DefinitionSnapshot {
    name: String,
    description: String,
    role_prompt: String,
    allowed_tools_json: String,
    constraints_json: String,
    model: String,
    max_turns: u32,
    metadata_json: String,
}

impl DefinitionSnapshot {
    fn from_def(def: &AgentDefinition) -> Self {
        Self {
            name: def.name.clone(),
            description: def.description.clone(),
            role_prompt: def.role_prompt.clone(),
            allowed_tools_json: def.allowed_tools_json.clone(),
            constraints_json: def.constraints_json.clone(),
            model: def.model.clone(),
            max_turns: def.max_turns,
            metadata_json: def.metadata_json.clone(),
        }
    }
}

fn validate_json(value: &str, field_name: &str) -> Result<(), String> {
    if value.is_empty() {
        return Ok(()); // Empty is allowed — means "default"
    }
    serde_json::from_str::<serde_json::Value>(value)
        .map_err(|e| format!("Invalid JSON in {}: {}", field_name, e))?;
    Ok(())
}
