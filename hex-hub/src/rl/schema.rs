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
}
