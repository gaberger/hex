use spacetimedb::{table, reducer, ReducerContext, Table};

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
pub fn update_health(
    ctx: &ReducerContext,
    id: String,
    status: String,
) -> Result<(), String> {
    let existing = ctx.db.compute_node().id().find(&id);
    match existing {
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
pub fn increment_agents(ctx: &ReducerContext, id: String) -> Result<(), String> {
    let existing = ctx.db.compute_node().id().find(&id);
    match existing {
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
pub fn decrement_agents(ctx: &ReducerContext, id: String) -> Result<(), String> {
    let existing = ctx.db.compute_node().id().find(&id);
    match existing {
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
