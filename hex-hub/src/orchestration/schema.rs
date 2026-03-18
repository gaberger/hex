use rusqlite::Connection;

/// Run orchestration-specific migrations on the shared hub database.
/// Safe to call multiple times (all statements use IF NOT EXISTS).
pub fn migrate_orchestration(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS hex_agents (
            id          TEXT PRIMARY KEY,
            process_id  INTEGER NOT NULL,
            agent_name  TEXT NOT NULL,
            project_dir TEXT NOT NULL,
            model       TEXT NOT NULL DEFAULT 'default',
            status      TEXT NOT NULL DEFAULT 'spawning',
            started_at  TEXT NOT NULL,
            ended_at    TEXT,
            metrics_json TEXT
        );

        CREATE TABLE IF NOT EXISTS workplan_executions (
            id             TEXT PRIMARY KEY,
            workplan_path  TEXT NOT NULL,
            status         TEXT NOT NULL DEFAULT 'running',
            current_phase  TEXT NOT NULL DEFAULT '',
            started_at     TEXT NOT NULL,
            updated_at     TEXT NOT NULL,
            result_json    TEXT
        );

        CREATE INDEX IF NOT EXISTS idx_hex_agents_status ON hex_agents(status);
        CREATE INDEX IF NOT EXISTS idx_workplan_executions_status ON workplan_executions(status);
        ",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrate_orchestration_creates_tables() {
        let conn = Connection::open_in_memory().unwrap();
        migrate_orchestration(&conn).unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM hex_agents", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM workplan_executions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn migrate_orchestration_is_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        migrate_orchestration(&conn).unwrap();
        migrate_orchestration(&conn).unwrap();
    }
}
