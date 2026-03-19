//! File Lock Manager SpacetimeDB Module
//!
//! Provides distributed file locking for multi-agent development.
//! Supports exclusive locks and shared read locks with automatic TTL expiry.
//!
//! Tables:
//!   - `file_lock` (public) -- tracks which agent holds a lock on which file

use spacetimedb::{table, reducer, ReducerContext, Table};

// ─── File Lock (PUBLIC) ─────────────────────────────────────────────────────

#[table(name = file_lock, public)]
#[derive(Clone, Debug)]
pub struct FileLock {
    /// File path being locked (unique — one lock per file)
    #[unique]
    pub file_path: String,
    /// Agent that holds the lock
    pub agent_id: String,
    /// Lock type: "exclusive" or "shared_read"
    pub lock_type: String,
    /// ISO 8601 timestamp when the lock was acquired
    pub acquired_at: String,
    /// ISO 8601 timestamp when the lock expires (5 min TTL)
    pub expires_at: String,
    /// Optional worktree this lock is associated with
    pub worktree: String,
}

/// Acquire a lock on a file path.
///
/// - Exclusive locks block all other locks on the same file.
/// - Shared read locks allow concurrent reads but block exclusive locks.
/// - If a conflicting lock exists, logs an error and returns without panicking.
#[reducer]
pub fn acquire_lock(
    ctx: &ReducerContext,
    file_path: String,
    agent_id: String,
    lock_type: String,
    worktree: Option<String>,
    acquired_at: String,
    expires_at: String,
) -> Result<(), String> {
    // Validate lock_type
    if lock_type != "exclusive" && lock_type != "shared_read" {
        return Err(format!(
            "Invalid lock_type '{}'. Expected: exclusive, shared_read",
            lock_type
        ));
    }

    if let Some(existing) = ctx.db.file_lock().file_path().find(&file_path) {
        // Check if existing lock has expired (allow override)
        if existing.expires_at > acquired_at {
            // Lock is still valid — check for conflicts
            if existing.lock_type == "exclusive" {
                log::error!(
                    "Cannot acquire {} lock on '{}': exclusive lock held by '{}'",
                    lock_type, file_path, existing.agent_id
                );
                return Err(format!(
                    "File '{}' is exclusively locked by agent '{}'",
                    file_path, existing.agent_id
                ));
            }

            if lock_type == "exclusive" {
                log::error!(
                    "Cannot acquire exclusive lock on '{}': shared_read lock held by '{}'",
                    file_path, existing.agent_id
                );
                return Err(format!(
                    "File '{}' has a shared_read lock held by agent '{}'",
                    file_path, existing.agent_id
                ));
            }

            // Both are shared_read — allow concurrent reads
            // (SpacetimeDB unique constraint means we can't insert a second row
            //  for the same file_path, so we just log and return Ok)
            log::info!(
                "Shared read lock on '{}' already held by '{}', concurrent read allowed",
                file_path, existing.agent_id
            );
            return Ok(());
        }

        // Existing lock has expired — overwrite it
        ctx.db.file_lock().file_path().delete(&file_path);
    }

    ctx.db.file_lock().insert(FileLock {
        file_path,
        agent_id,
        lock_type,
        acquired_at,
        expires_at,
        worktree: worktree.unwrap_or_default(),
    });

    Ok(())
}

/// Release a lock on a file path. Only the agent that holds the lock can release it.
#[reducer]
pub fn release_lock(
    ctx: &ReducerContext,
    file_path: String,
    agent_id: String,
) -> Result<(), String> {
    match ctx.db.file_lock().file_path().find(&file_path) {
        Some(existing) => {
            if existing.agent_id != agent_id {
                return Err(format!(
                    "Agent '{}' cannot release lock on '{}' held by '{}'",
                    agent_id, file_path, existing.agent_id
                ));
            }
            ctx.db.file_lock().file_path().delete(&file_path);
            Ok(())
        }
        None => Err(format!("No lock found for file '{}'", file_path)),
    }
}

/// Expire all stale locks where expires_at < now.
/// `now` is an ISO 8601 timestamp representing the current time.
#[reducer]
pub fn expire_stale_locks(
    ctx: &ReducerContext,
    now: String,
) -> Result<(), String> {
    let stale: Vec<FileLock> = ctx
        .db
        .file_lock()
        .iter()
        .filter(|l| l.expires_at <= now)
        .collect();

    let count = stale.len();
    for lock in stale {
        ctx.db.file_lock().file_path().delete(&lock.file_path);
    }

    if count > 0 {
        log::info!("Expired {} stale file locks", count);
    }

    Ok(())
}

// ─── Pure logic helpers (testable without SpacetimeDB runtime) ───────────────

/// Check whether a lock has expired given an ISO 8601 `now` timestamp.
pub fn is_lock_expired(expires_at: &str, now: &str) -> bool {
    expires_at <= now
}

/// Validate a lock type string.
pub fn validate_lock_type(lock_type: &str) -> Result<(), String> {
    match lock_type {
        "exclusive" | "shared_read" => Ok(()),
        _ => Err(format!(
            "Invalid lock_type '{}'. Expected: exclusive, shared_read",
            lock_type
        )),
    }
}

/// Check whether two lock types conflict.
/// Exclusive conflicts with everything. Shared_read only conflicts with exclusive.
pub fn locks_conflict(existing_type: &str, requested_type: &str) -> bool {
    existing_type == "exclusive" || requested_type == "exclusive"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_not_expired_before_deadline() {
        assert!(!is_lock_expired("2025-01-01T12:05:00Z", "2025-01-01T12:00:00Z"));
    }

    #[test]
    fn lock_expired_at_exact_deadline() {
        assert!(is_lock_expired("2025-01-01T12:00:00Z", "2025-01-01T12:00:00Z"));
    }

    #[test]
    fn lock_expired_after_deadline() {
        assert!(is_lock_expired("2025-01-01T12:00:00Z", "2025-01-01T12:05:00Z"));
    }

    #[test]
    fn valid_lock_types_accepted() {
        assert!(validate_lock_type("exclusive").is_ok());
        assert!(validate_lock_type("shared_read").is_ok());
    }

    #[test]
    fn invalid_lock_type_rejected() {
        assert!(validate_lock_type("write").is_err());
        assert!(validate_lock_type("").is_err());
    }

    #[test]
    fn exclusive_conflicts_with_exclusive() {
        assert!(locks_conflict("exclusive", "exclusive"));
    }

    #[test]
    fn exclusive_conflicts_with_shared_read() {
        assert!(locks_conflict("exclusive", "shared_read"));
    }

    #[test]
    fn shared_read_conflicts_with_exclusive() {
        assert!(locks_conflict("shared_read", "exclusive"));
    }

    #[test]
    fn shared_read_does_not_conflict_with_shared_read() {
        assert!(!locks_conflict("shared_read", "shared_read"));
    }
}
