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

// ─── Agent Cleanup ──────────────────────────────────────────────────────────

/// Stale threshold in seconds — agents without heartbeat for this long become "stale".
pub const STALE_THRESHOLD_SECS: u64 = 45;

/// Dead threshold in seconds — agents without heartbeat for this long become "dead".
pub const DEAD_THRESHOLD_SECS: u64 = 120;

/// Cleanup run log — tracks when agent cleanup last ran and what it did.
#[table(name = agent_cleanup_log, public)]
#[derive(Clone, Debug)]
pub struct AgentCleanupLog {
    #[auto_inc]
    #[primary_key]
    pub id: u64,
    pub ran_at: String,
    pub stale_count: u32,
    pub dead_count: u32,
    pub reclaimed_count: u32,
}

/// Run agent health cleanup.
///
/// `now` is an RFC3339 timestamp representing the current time.
/// `stale_cutoff` is an RFC3339 timestamp = now - STALE_THRESHOLD_SECS (45s).
/// `dead_cutoff` is an RFC3339 timestamp = now - DEAD_THRESHOLD_SECS (120s).
///
/// Agents whose `last_seen` is non-empty and older than `stale_cutoff` are
/// marked "stale". Agents already "stale" whose `last_seen` is older than
/// `dead_cutoff` are marked "dead".
///
/// This is a regular reducer — called periodically by hex-nexus or manually.
/// When SpacetimeDB scheduled-procedure support matures, this can be wired
/// to a cron-style schedule directly.
#[reducer]
pub fn run_agent_cleanup(
    ctx: &ReducerContext,
    now: String,
    stale_cutoff: String,
    dead_cutoff: String,
) -> Result<(), String> {
    let mut stale_count: u32 = 0;
    let mut dead_count: u32 = 0;
    let mut reclaimed_count: u32 = 0;

    // Normalize Z → +00:00 for consistent RFC3339 string comparison.
    let stale_c = stale_cutoff.replace('Z', "+00:00");
    let dead_c = dead_cutoff.replace('Z', "+00:00");

    // Collect all heartbeats for agents that are in active-ish states.
    let agents: Vec<(Agent, AgentHeartbeat)> = ctx.db.agent().iter()
        .filter(|a| a.status == "registered" || a.status == "active" || a.status == "stale")
        .filter_map(|a| {
            ctx.db.agent_heartbeat().agent_id().find(&a.id).map(|hb| (a, hb))
        })
        .collect();

    for (agent, hb) in agents {
        // Skip agents with no heartbeat yet (empty last_seen).
        if hb.last_seen.is_empty() {
            continue;
        }

        let last = hb.last_seen.replace('Z', "+00:00");

        if last < dead_c && (agent.status == "stale") {
            // Stale → Dead
            ctx.db.agent().id().update(Agent {
                status: "dead".to_string(),
                ..agent
            });
            dead_count += 1;
            reclaimed_count += 1; // agent slot reclaimed
        } else if last < stale_c && (agent.status == "registered" || agent.status == "active") {
            // Active/Registered → Stale
            ctx.db.agent().id().update(Agent {
                status: "stale".to_string(),
                ..agent
            });
            stale_count += 1;
        }
    }

    // Log cleanup run if any work was done.
    if stale_count > 0 || dead_count > 0 || reclaimed_count > 0 {
        ctx.db.agent_cleanup_log().insert(AgentCleanupLog {
            id: 0, // auto_inc
            ran_at: now.clone(),
            stale_count,
            dead_count,
            reclaimed_count,
        });
        log::info!(
            "run_agent_cleanup: stale={}, dead={}, reclaimed={}",
            stale_count, dead_count, reclaimed_count
        );
    }

    Ok(())
}

/// Manual trigger for agent cleanup — convenience wrapper around run_agent_cleanup.
///
/// Takes the same parameters and simply delegates. Useful for dashboard buttons
/// or ad-hoc maintenance.
#[reducer]
pub fn trigger_agent_cleanup(
    ctx: &ReducerContext,
    now: String,
    stale_cutoff: String,
    dead_cutoff: String,
) -> Result<(), String> {
    run_agent_cleanup(ctx, now, stale_cutoff, dead_cutoff)
}

// ─── Validation Helpers ─────────────────────────────────────────────────────

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
