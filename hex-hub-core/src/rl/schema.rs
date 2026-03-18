use rusqlite::Connection;

/// Run RL-specific migrations on the shared hub database.
/// Safe to call multiple times (all statements use IF NOT EXISTS).
pub fn migrate_rl(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS rl_experiences (
            id          TEXT PRIMARY KEY,
            state       TEXT NOT NULL,   -- JSON-encoded state
            action      TEXT NOT NULL,
            reward      REAL NOT NULL,
            next_state  TEXT NOT NULL,   -- JSON-encoded next state
            timestamp   TEXT NOT NULL,
            task_type   TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS rl_q_table (
            state_key    TEXT NOT NULL,
            action       TEXT NOT NULL,
            q_value      REAL NOT NULL DEFAULT 0.0,
            visit_count  INTEGER NOT NULL DEFAULT 0,
            last_updated TEXT NOT NULL,
            PRIMARY KEY (state_key, action)
        );

        CREATE TABLE IF NOT EXISTS rl_patterns (
            id            TEXT PRIMARY KEY,
            category      TEXT NOT NULL,
            content       TEXT NOT NULL,
            confidence    REAL NOT NULL DEFAULT 1.0,
            created_at    TEXT NOT NULL,
            last_accessed TEXT NOT NULL,
            access_count  INTEGER NOT NULL DEFAULT 0,
            decay_rate    REAL NOT NULL DEFAULT 0.01
        );

        CREATE INDEX IF NOT EXISTS idx_rl_experiences_task_type ON rl_experiences(task_type);
        CREATE INDEX IF NOT EXISTS idx_rl_q_table_state ON rl_q_table(state_key);
        CREATE INDEX IF NOT EXISTS idx_rl_patterns_category ON rl_patterns(category);
        ",
    )
}

/// Seed the Q-table with sensible defaults so the RL engine doesn't start cold.
/// Safe to call multiple times — uses INSERT OR IGNORE.
///
/// Strategy: for small codebases/low token usage, prefer balanced context.
/// For large codebases/high token usage, prefer conservative (avoid blowing budget).
/// For multi-agent scenarios, prefer aggressive (maximize parallelism value).
pub fn seed_rl(conn: &Connection) -> Result<(), rusqlite::Error> {
    let now = chrono::Utc::now().to_rfc3339();

    // State key format: {task_type}:sz{0-4}:ag{N}:tk{0-4}
    // Seed initial Q-values encoding domain knowledge about what works
    let seeds: &[(&str, &str, f64)] = &[
        // Small codebase, single agent — balanced works well
        ("conversation:sz0:ag1:tk0", "context:balanced", 0.5),
        ("conversation:sz1:ag1:tk0", "context:balanced", 0.5),
        ("conversation:sz1:ag1:tk1", "context:balanced", 0.4),
        // Medium codebase — balanced still good, but aggressive has value
        ("conversation:sz2:ag1:tk1", "context:balanced", 0.4),
        ("conversation:sz2:ag1:tk1", "context:aggressive", 0.3),
        ("conversation:sz2:ag1:tk2", "context:balanced", 0.3),
        ("conversation:sz2:ag1:tk2", "context:conservative", 0.35),
        // Large codebase, high token usage — conservative to preserve budget
        ("conversation:sz3:ag1:tk2", "context:conservative", 0.4),
        ("conversation:sz3:ag1:tk3", "context:conservative", 0.5),
        ("conversation:sz4:ag1:tk3", "context:conservative", 0.5),
        ("conversation:sz4:ag1:tk4", "context:conservative", 0.6),
        // Multi-agent: aggressive packing extracts more value per agent
        ("conversation:sz2:ag2:tk1", "context:aggressive", 0.4),
        ("conversation:sz2:ag4:tk2", "context:aggressive", 0.45),
        ("conversation:sz3:ag2:tk2", "context:aggressive", 0.4),
        ("conversation:sz3:ag4:tk2", "context:aggressive", 0.45),
        // Build tasks — conservative by default (builds are token-heavy)
        ("build:sz2:ag1:tk1", "context:conservative", 0.4),
        ("build:sz3:ag1:tk2", "context:conservative", 0.5),
        ("build:sz4:ag1:tk3", "context:conservative", 0.5),
        // Build + multi-agent — balanced
        ("build:sz2:ag2:tk1", "context:balanced", 0.4),
        ("build:sz3:ag2:tk2", "context:balanced", 0.4),
        // Test tasks — balanced (tests need context for assertions)
        ("test:sz1:ag1:tk0", "context:balanced", 0.5),
        ("test:sz2:ag1:tk1", "context:balanced", 0.4),
        ("test:sz3:ag1:tk2", "context:balanced", 0.4),
    ];

    for (state_key, action, q_value) in seeds {
        conn.execute(
            "INSERT OR IGNORE INTO rl_q_table (state_key, action, q_value, visit_count, last_updated)
             VALUES (?1, ?2, ?3, 1, ?4)",
            rusqlite::params![state_key, action, q_value, now],
        )?;
    }

    // Seed initial patterns with domain knowledge
    let pattern_seeds: &[(&str, &str, f64)] = &[
        ("context", "Use conservative context for builds over 10k files to avoid token overflow", 0.8),
        ("context", "Aggressive context packing works well with multi-agent swarms (2+ agents)", 0.7),
        ("context", "Balanced context is the safe default for interactive conversation", 0.9),
        ("agent", "hex-coder agent is preferred for single-adapter code generation tasks", 0.8),
        ("agent", "planner agent is preferred for multi-phase decomposition before coding", 0.7),
    ];

    for (category, content, confidence) in pattern_seeds {
        let id = uuid::Uuid::new_v4().to_string();
        conn.execute(
            "INSERT OR IGNORE INTO rl_patterns (id, category, content, confidence, created_at, last_accessed, access_count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0)",
            rusqlite::params![id, category, content, confidence, now, now],
        )?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrate_rl_creates_tables() {
        let conn = Connection::open_in_memory().unwrap();
        migrate_rl(&conn).unwrap();

        // Verify tables exist by querying them
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM rl_experiences", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM rl_q_table", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM rl_patterns", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn migrate_rl_is_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        migrate_rl(&conn).unwrap();
        migrate_rl(&conn).unwrap(); // second call should not fail
    }

    #[test]
    fn seed_rl_populates_q_table() {
        let conn = Connection::open_in_memory().unwrap();
        migrate_rl(&conn).unwrap();
        seed_rl(&conn).unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM rl_q_table", [], |r| r.get(0))
            .unwrap();
        assert!(count >= 20, "Expected at least 20 seed Q-entries, got {}", count);

        let pattern_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM rl_patterns", [], |r| r.get(0))
            .unwrap();
        assert!(pattern_count >= 5, "Expected at least 5 seed patterns, got {}", pattern_count);
    }

    #[test]
    fn seed_rl_is_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        migrate_rl(&conn).unwrap();
        seed_rl(&conn).unwrap();

        let count1: i64 = conn
            .query_row("SELECT COUNT(*) FROM rl_q_table", [], |r| r.get(0))
            .unwrap();

        seed_rl(&conn).unwrap(); // second call should not duplicate

        let count2: i64 = conn
            .query_row("SELECT COUNT(*) FROM rl_q_table", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count1, count2, "Seed should be idempotent");
    }
}
