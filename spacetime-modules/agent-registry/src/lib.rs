use spacetimedb::{table, reducer, ReducerContext, Table};

#[table(name = agent, public)]
#[derive(Clone, Debug)]
pub struct Agent {
    #[unique]
    pub id: String,
    pub name: String,
    pub project_id: String,
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
    project_id: String,
    project_dir: String,
    model: String,
    started_at: String,
) -> Result<(), String> {
    ctx.db.agent().insert(Agent {
        id: id.clone(),
        name,
        project_id,
        project_dir,
        model,
        status: "registered".to_string(),
        started_at,
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

/// Valid agent statuses used by the heartbeat protocol.
pub const VALID_AGENT_STATUSES: &[&str] = &["registered", "active", "stale", "dead", "disconnected"];

/// Check whether a status string is a recognized agent status.
pub fn is_valid_status(status: &str) -> bool {
    VALID_AGENT_STATUSES.contains(&status)
}

/// Validate that an agent ID looks like a UUID v4 (36 chars, 4 hyphens).
pub fn is_valid_agent_id(id: &str) -> bool {
    id.len() == 36 && id.chars().filter(|c| *c == '-').count() == 4
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Status validation ───────────────────────────────────────────────

    #[test]
    fn valid_statuses_are_accepted() {
        for status in VALID_AGENT_STATUSES {
            assert!(is_valid_status(status), "expected '{}' to be valid", status);
        }
    }

    #[test]
    fn unknown_status_is_rejected() {
        assert!(!is_valid_status("running"));
        assert!(!is_valid_status(""));
        assert!(!is_valid_status("ACTIVE"));
    }

    #[test]
    fn initial_status_is_registered() {
        // Mirrors the register_agent reducer: initial status must be "registered"
        let initial = "registered";
        assert!(is_valid_status(initial));
    }

    // ── Agent ID format ─────────────────────────────────────────────────

    #[test]
    fn valid_uuid_format() {
        let id = "550e8400-e29b-41d4-a716-446655440000";
        assert!(is_valid_agent_id(id));
    }

    #[test]
    fn rejects_short_id() {
        assert!(!is_valid_agent_id("abc-123"));
    }

    #[test]
    fn rejects_id_without_hyphens() {
        let id = "550e8400e29b41d4a716446655440000xxxx";
        assert!(!is_valid_agent_id(id));
    }

    #[test]
    fn rejects_empty_id() {
        assert!(!is_valid_agent_id(""));
    }

    // ── Struct construction ─────────────────────────────────────────────

    #[test]
    fn agent_struct_defaults() {
        let agent = Agent {
            id: "test-id-00000000-0000-0000-000000000000".to_string(),
            name: "coder-1".to_string(),
            project_id: "proj-1".to_string(),
            project_dir: "/tmp/proj".to_string(),
            model: "claude-opus-4-20250514".to_string(),
            status: "registered".to_string(),
            started_at: "2025-01-01T00:00:00Z".to_string(),
            ended_at: String::new(),
            metrics_json: "{}".to_string(),
        };
        assert_eq!(agent.status, "registered");
        assert!(agent.ended_at.is_empty());
        assert_eq!(agent.metrics_json, "{}");
    }

    #[test]
    fn heartbeat_struct_initial_values() {
        let hb = AgentHeartbeat {
            agent_id: "agent-1".to_string(),
            last_seen: String::new(),
            turn_count: 0,
            token_usage: 0,
        };
        assert_eq!(hb.turn_count, 0);
        assert_eq!(hb.token_usage, 0);
        assert!(hb.last_seen.is_empty());
    }

    #[test]
    fn agent_clone_preserves_fields() {
        let agent = Agent {
            id: "a1".to_string(),
            name: "n1".to_string(),
            project_id: "p1".to_string(),
            project_dir: "/d".to_string(),
            model: "m1".to_string(),
            status: "active".to_string(),
            started_at: "t1".to_string(),
            ended_at: "t2".to_string(),
            metrics_json: "{\"x\":1}".to_string(),
        };
        let cloned = agent.clone();
        assert_eq!(agent.id, cloned.id);
        assert_eq!(agent.metrics_json, cloned.metrics_json);
    }

    // ── Status update via struct spread ──────────────────────────────────

    #[test]
    fn status_update_preserves_identity() {
        let original = Agent {
            id: "agent-abc".to_string(),
            name: "worker".to_string(),
            project_id: "proj".to_string(),
            project_dir: "/proj".to_string(),
            model: "gpt-4".to_string(),
            status: "registered".to_string(),
            started_at: "t0".to_string(),
            ended_at: String::new(),
            metrics_json: "{}".to_string(),
        };
        let updated = Agent {
            status: "active".to_string(),
            metrics_json: "{\"turns\":5}".to_string(),
            ..original.clone()
        };
        assert_eq!(updated.id, "agent-abc");
        assert_eq!(updated.name, "worker");
        assert_eq!(updated.status, "active");
        assert_eq!(updated.metrics_json, "{\"turns\":5}");
    }
}
