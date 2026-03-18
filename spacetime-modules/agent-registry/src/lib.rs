use spacetimedb::{table, reducer, ReducerContext, Table};

#[table(name = agent, public)]
#[derive(Clone, Debug)]
pub struct Agent {
    #[unique]
    pub id: String,
    pub name: String,
    pub project_dir: String,
    pub model: String,
    pub status: String,
    pub started_at: String,
    pub ended_at: String,
    pub metrics_json: String,
}

#[table(name = agent_heartbeat, public)]
#[derive(Clone, Debug)]
pub struct AgentHeartbeat {
    #[unique]
    pub agent_id: String,
    pub last_seen: String,
    pub turn_count: u32,
    pub token_usage: u64,
}

#[reducer]
pub fn register_agent(
    ctx: &ReducerContext,
    id: String,
    name: String,
    project_dir: String,
    model: String,
) -> Result<(), String> {
    ctx.db.agent().insert(Agent {
        id: id.clone(),
        name,
        project_dir,
        model,
        status: "registered".to_string(),
        started_at: String::new(),
        ended_at: String::new(),
        metrics_json: "{}".to_string(),
    });

    ctx.db.agent_heartbeat().insert(AgentHeartbeat {
        agent_id: id,
        last_seen: String::new(),
        turn_count: 0,
        token_usage: 0,
    });

    Ok(())
}

#[reducer]
pub fn update_status(
    ctx: &ReducerContext,
    id: String,
    status: String,
    metrics_json: String,
) -> Result<(), String> {
    let existing = ctx.db.agent().id().find(&id);
    match existing {
        Some(old) => {
            let updated = Agent {
                status,
                metrics_json,
                ..old
            };
            ctx.db.agent().id().update(updated);
        }
        None => {
            return Err(format!("Agent '{}' not found", id));
        }
    }
    Ok(())
}

#[reducer]
pub fn heartbeat(
    ctx: &ReducerContext,
    agent_id: String,
    turn_count: u32,
    token_usage: u64,
) -> Result<(), String> {
    let existing = ctx.db.agent_heartbeat().agent_id().find(&agent_id);
    match existing {
        Some(old) => {
            let updated = AgentHeartbeat {
                last_seen: String::new(),
                turn_count,
                token_usage,
                ..old
            };
            ctx.db.agent_heartbeat().agent_id().update(updated);
        }
        None => {
            return Err(format!("Agent '{}' not registered", agent_id));
        }
    }
    Ok(())
}

#[reducer]
pub fn remove_agent(ctx: &ReducerContext, id: String) -> Result<(), String> {
    let deleted = ctx.db.agent().id().delete(&id);
    if !deleted {
        return Err(format!("Agent '{}' not found", id));
    }
    ctx.db.agent_heartbeat().agent_id().delete(&id);
    Ok(())
}
