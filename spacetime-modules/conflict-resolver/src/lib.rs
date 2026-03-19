//! Conflict Resolver SpacetimeDB Module
//!
//! Tracks and resolves file conflicts between agents in multi-agent
//! development workflows (parallel worktrees, concurrent edits).
//!
//! Tables:
//!   - `conflict_event` (public) -- records of detected conflicts and their resolutions

use spacetimedb::{table, reducer, ReducerContext, Table};

// ─── Conflict Event (PUBLIC) ────────────────────────────────────────────────

#[table(name = conflict_event, public)]
#[derive(Clone, Debug)]
pub struct ConflictEvent {
    /// Auto-incrementing conflict identifier
    #[auto_inc]
    #[primary_key]
    pub conflict_id: u64,
    /// File path where the conflict was detected
    pub file_path: String,
    /// JSON array of agent IDs involved in the conflict
    pub agents_json: String,
    /// Resolution strategy: "priority", "merge", "escalate", or "pending"
    pub resolution: String,
    /// Agent or "system" that resolved the conflict
    pub resolved_by: String,
    /// ISO 8601 timestamp when the conflict was reported
    pub created_at: String,
    /// ISO 8601 timestamp when the conflict was resolved (empty if pending)
    pub resolved_at: String,
}

/// Report a new conflict between agents on a file.
/// Creates a ConflictEvent with status "pending".
#[reducer]
pub fn report_conflict(
    ctx: &ReducerContext,
    file_path: String,
    agents_json: String,
    created_at: String,
) -> Result<(), String> {
    if file_path.is_empty() {
        return Err("file_path cannot be empty".to_string());
    }

    ctx.db.conflict_event().insert(ConflictEvent {
        conflict_id: 0, // auto_inc will assign
        file_path: file_path.clone(),
        agents_json: agents_json.clone(),
        resolution: "pending".to_string(),
        resolved_by: String::new(),
        created_at,
        resolved_at: String::new(),
    });

    log::info!(
        "Conflict reported on '{}' between agents: {}",
        file_path, agents_json
    );

    Ok(())
}

/// Resolve a conflict by updating its resolution strategy and resolver.
#[reducer]
pub fn resolve_conflict(
    ctx: &ReducerContext,
    conflict_id: u64,
    resolution: String,
    resolved_by: String,
    resolved_at: String,
) -> Result<(), String> {
    // Validate resolution type
    match resolution.as_str() {
        "priority" | "merge" | "escalate" => {}
        _ => {
            return Err(format!(
                "Invalid resolution '{}'. Expected: priority, merge, escalate",
                resolution
            ));
        }
    }

    match ctx.db.conflict_event().conflict_id().find(&conflict_id) {
        Some(existing) => {
            if existing.resolution != "pending" {
                return Err(format!(
                    "Conflict {} is already resolved with '{}'",
                    conflict_id, existing.resolution
                ));
            }

            ctx.db.conflict_event().conflict_id().update(ConflictEvent {
                resolution: resolution.clone(),
                resolved_by: resolved_by.clone(),
                resolved_at,
                ..existing
            });

            log::info!(
                "Conflict {} resolved via '{}' by '{}'",
                conflict_id, resolution, resolved_by
            );

            Ok(())
        }
        None => Err(format!("Conflict {} not found", conflict_id)),
    }
}

// ─── Pure logic helpers (testable without SpacetimeDB runtime) ───────────────

/// Validate a resolution strategy string.
pub fn validate_resolution(resolution: &str) -> Result<(), String> {
    match resolution {
        "priority" | "merge" | "escalate" | "pending" => Ok(()),
        _ => Err(format!(
            "Invalid resolution '{}'. Expected: priority, merge, escalate, pending",
            resolution
        )),
    }
}

/// Check if a conflict is still pending resolution.
pub fn is_pending(resolution: &str) -> bool {
    resolution == "pending"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_resolutions_accepted() {
        for r in &["priority", "merge", "escalate", "pending"] {
            assert!(validate_resolution(r).is_ok(), "Resolution '{}' should be valid", r);
        }
    }

    #[test]
    fn invalid_resolution_rejected() {
        assert!(validate_resolution("unknown").is_err());
        assert!(validate_resolution("").is_err());
        assert!(validate_resolution("retry").is_err());
    }

    #[test]
    fn pending_conflict_is_pending() {
        assert!(is_pending("pending"));
    }

    #[test]
    fn resolved_conflict_is_not_pending() {
        assert!(!is_pending("priority"));
        assert!(!is_pending("merge"));
        assert!(!is_pending("escalate"));
    }

    #[test]
    fn conflict_event_fields() {
        let event = ConflictEvent {
            conflict_id: 1,
            file_path: "src/domain/foo.ts".to_string(),
            agents_json: "[\"agent-1\",\"agent-2\"]".to_string(),
            resolution: "pending".to_string(),
            resolved_by: String::new(),
            created_at: "2025-01-01T00:00:00Z".to_string(),
            resolved_at: String::new(),
        };
        assert_eq!(event.file_path, "src/domain/foo.ts");
        assert!(is_pending(&event.resolution));
        assert!(event.resolved_at.is_empty());
    }
}
