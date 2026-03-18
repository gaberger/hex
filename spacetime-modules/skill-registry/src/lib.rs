use spacetimedb::{table, reducer, ReducerContext, Table};

// ── Tables ──────────────────────────────────────────────

/// A skill definition — a prompt template triggered by user input patterns.
#[table(name = skill, public)]
#[derive(Clone, Debug)]
pub struct Skill {
    #[unique]
    pub id: String,
    /// Unique skill name (e.g., "hex-scaffold")
    #[unique]
    pub name: String,
    pub description: String,
    /// JSON-encoded Vec<SkillTrigger> — slash commands, regex patterns, keywords
    pub triggers_json: String,
    /// The full prompt body (markdown content)
    pub body: String,
    /// Origin: "filesystem", "hub-ui", "migrate-config"
    pub source: String,
    pub created_at: String,
    pub updated_at: String,
}

/// Denormalized trigger index for fast matching.
/// One row per trigger per skill — allows efficient lookups by trigger type.
#[table(name = skill_trigger_index, public)]
#[derive(Clone, Debug)]
pub struct SkillTriggerIndex {
    pub skill_id: String,
    /// "slash_command", "pattern", or "keyword"
    pub trigger_type: String,
    /// The trigger value (e.g., "/hex-scaffold", "scaffold.*hex", "scaffold")
    pub trigger_value: String,
}

// ── Reducers ────────────────────────────────────────────

#[reducer]
pub fn register_skill(
    ctx: &ReducerContext,
    id: String,
    name: String,
    description: String,
    triggers_json: String,
    body: String,
    source: String,
    timestamp: String,
) -> Result<(), String> {
    // Parse triggers to populate the index
    let triggers: Vec<TriggerEntry> = serde_json::from_str(&triggers_json)
        .map_err(|e| format!("Invalid triggers_json: {}", e))?;

    ctx.db.skill().insert(Skill {
        id: id.clone(),
        name,
        description,
        triggers_json,
        body,
        source,
        created_at: timestamp.clone(),
        updated_at: timestamp,
    });

    // Populate trigger index
    for trigger in triggers {
        ctx.db.skill_trigger_index().insert(SkillTriggerIndex {
            skill_id: id.clone(),
            trigger_type: trigger.trigger_type,
            trigger_value: trigger.trigger_value,
        });
    }

    Ok(())
}

#[reducer]
pub fn update_skill(
    ctx: &ReducerContext,
    id: String,
    description: String,
    triggers_json: String,
    body: String,
    timestamp: String,
) -> Result<(), String> {
    let existing = ctx.db.skill().id().find(&id)
        .ok_or_else(|| format!("Skill '{}' not found", id))?;

    let triggers: Vec<TriggerEntry> = serde_json::from_str(&triggers_json)
        .map_err(|e| format!("Invalid triggers_json: {}", e))?;

    let updated = Skill {
        description,
        triggers_json,
        body,
        updated_at: timestamp,
        ..existing
    };
    ctx.db.skill().id().update(updated);

    // Rebuild trigger index — delete old entries, insert new
    let old_triggers: Vec<_> = ctx.db.skill_trigger_index()
        .iter()
        .filter(|t| t.skill_id == id)
        .collect();
    for old in old_triggers {
        ctx.db.skill_trigger_index().delete(old);
    }
    for trigger in triggers {
        ctx.db.skill_trigger_index().insert(SkillTriggerIndex {
            skill_id: id.clone(),
            trigger_type: trigger.trigger_type,
            trigger_value: trigger.trigger_value,
        });
    }

    Ok(())
}

#[reducer]
pub fn remove_skill(ctx: &ReducerContext, id: String) -> Result<(), String> {
    let deleted = ctx.db.skill().id().delete(&id);
    if !deleted {
        return Err(format!("Skill '{}' not found", id));
    }

    // Clean up trigger index
    let triggers: Vec<_> = ctx.db.skill_trigger_index()
        .iter()
        .filter(|t| t.skill_id == id)
        .collect();
    for trigger in triggers {
        ctx.db.skill_trigger_index().delete(trigger);
    }

    Ok(())
}

/// Search skills by trigger type and value.
/// Returns all matching skill IDs via the trigger index.
#[reducer]
pub fn search_skills(
    ctx: &ReducerContext,
    trigger_type: String,
    query: String,
) -> Result<(), String> {
    // SpacetimeDB reducers can't return values directly — clients read via subscriptions.
    // This reducer exists as a no-op query hint; clients subscribe to skill table
    // and filter locally. Kept for API completeness and future server-side filtering.
    let _matches: Vec<_> = ctx.db.skill_trigger_index()
        .iter()
        .filter(|t| {
            t.trigger_type == trigger_type
                && t.trigger_value.contains(&query)
        })
        .collect();
    Ok(())
}

// ── Internal Types ──────────────────────────────────────

/// Intermediate type for deserializing trigger entries from JSON.
#[derive(serde::Deserialize)]
struct TriggerEntry {
    trigger_type: String,
    trigger_value: String,
}
