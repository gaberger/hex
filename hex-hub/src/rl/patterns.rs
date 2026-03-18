use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pattern {
    pub id: String,
    pub category: String,
    pub content: String,
    pub confidence: f64,
    pub created_at: String,
    pub last_accessed: String,
    pub access_count: i64,
}

/// Stateless pattern store — all data lives in SQLite.
pub struct PatternStore;

impl PatternStore {
    /// Store a new pattern. Returns its generated ID.
    pub fn store(
        conn: &Connection,
        category: &str,
        content: &str,
        initial_confidence: f64,
    ) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO rl_patterns (id, category, content, confidence, created_at, last_accessed, access_count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0)",
            params![id, category, content, initial_confidence, now, now],
        )
        .expect("Failed to store pattern");
        id
    }

    /// Search patterns by category and optional content substring (LIKE query).
    /// Updates `last_accessed` and `access_count` for returned rows.
    pub fn search(
        conn: &Connection,
        category: &str,
        query_text: &str,
        limit: u32,
    ) -> Vec<Pattern> {
        let like_pattern = format!("%{}%", query_text);
        let mut stmt = conn
            .prepare(
                "SELECT id, category, content, confidence, created_at, last_accessed, access_count
                 FROM rl_patterns
                 WHERE category = ?1 AND content LIKE ?2
                 ORDER BY confidence DESC
                 LIMIT ?3",
            )
            .expect("Failed to prepare search query");

        let patterns: Vec<Pattern> = stmt
            .query_map(params![category, like_pattern, limit], |row| {
                Ok(Pattern {
                    id: row.get(0)?,
                    category: row.get(1)?,
                    content: row.get(2)?,
                    confidence: row.get(3)?,
                    created_at: row.get(4)?,
                    last_accessed: row.get(5)?,
                    access_count: row.get(6)?,
                })
            })
            .expect("Failed to execute search query")
            .filter_map(|r| r.ok())
            .collect();

        // Update access metadata for returned patterns
        let now = chrono::Utc::now().to_rfc3339();
        for p in &patterns {
            conn.execute(
                "UPDATE rl_patterns SET last_accessed = ?1, access_count = access_count + 1 WHERE id = ?2",
                params![now, p.id],
            )
            .ok();
        }

        patterns
    }

    /// Increase (or decrease) a pattern's confidence by `delta`.
    /// Clamps to [0.0, 1.0].
    pub fn reinforce(conn: &Connection, id: &str, delta: f64) {
        conn.execute(
            "UPDATE rl_patterns
             SET confidence = MIN(1.0, MAX(0.0, confidence + ?1)),
                 last_accessed = ?2
             WHERE id = ?3",
            params![delta, chrono::Utc::now().to_rfc3339(), id],
        )
        .expect("Failed to reinforce pattern");
    }

    /// Apply temporal decay to all patterns: confidence *= (1.0 - decay_rate).
    pub fn decay_all(conn: &Connection) {
        conn.execute(
            "UPDATE rl_patterns SET confidence = confidence * (1.0 - decay_rate)",
            [],
        )
        .expect("Failed to decay patterns");
    }

    /// Get the top patterns by confidence for a given category.
    pub fn get_top(conn: &Connection, category: &str, limit: u32) -> Vec<Pattern> {
        let mut stmt = conn
            .prepare(
                "SELECT id, category, content, confidence, created_at, last_accessed, access_count
                 FROM rl_patterns
                 WHERE category = ?1
                 ORDER BY confidence DESC
                 LIMIT ?2",
            )
            .expect("Failed to prepare get_top query");

        stmt.query_map(params![category, limit], |row| {
            Ok(Pattern {
                id: row.get(0)?,
                category: row.get(1)?,
                content: row.get(2)?,
                confidence: row.get(3)?,
                created_at: row.get(4)?,
                last_accessed: row.get(5)?,
                access_count: row.get(6)?,
            })
        })
        .expect("Failed to execute get_top query")
        .filter_map(|r| r.ok())
        .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rl::schema::migrate_rl;

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        migrate_rl(&conn).unwrap();
        conn
    }

    #[test]
    fn store_and_search() {
        let conn = setup();
        PatternStore::store(&conn, "code", "use async/await for IO", 0.9);
        PatternStore::store(&conn, "code", "prefer iterators over loops", 0.8);
        PatternStore::store(&conn, "test", "mock external deps", 0.7);

        let results = PatternStore::search(&conn, "code", "async", 10);
        assert_eq!(results.len(), 1);
        assert!(results[0].content.contains("async"));

        // Search all in category
        let results = PatternStore::search(&conn, "code", "", 10);
        assert_eq!(results.len(), 2);
        // Ordered by confidence desc
        assert!(results[0].confidence >= results[1].confidence);
    }

    #[test]
    fn reinforce_updates_confidence() {
        let conn = setup();
        let id = PatternStore::store(&conn, "code", "test pattern", 0.5);

        PatternStore::reinforce(&conn, &id, 0.3);
        let results = PatternStore::search(&conn, "code", "test", 10);
        assert!((results[0].confidence - 0.8).abs() < 1e-9);

        // Clamp to max 1.0
        PatternStore::reinforce(&conn, &id, 0.5);
        let results = PatternStore::search(&conn, "code", "test", 10);
        assert!((results[0].confidence - 1.0).abs() < 1e-9);

        // Clamp to min 0.0
        PatternStore::reinforce(&conn, &id, -2.0);
        let results = PatternStore::search(&conn, "code", "test", 10);
        assert!((results[0].confidence - 0.0).abs() < 1e-9);
    }

    #[test]
    fn decay_all_reduces_confidence() {
        let conn = setup();
        PatternStore::store(&conn, "code", "pattern A", 1.0);
        // default decay_rate = 0.01 → after decay: 1.0 * 0.99 = 0.99
        PatternStore::decay_all(&conn);

        let results = PatternStore::get_top(&conn, "code", 10);
        assert!((results[0].confidence - 0.99).abs() < 1e-9);
    }

    #[test]
    fn get_top_returns_sorted() {
        let conn = setup();
        PatternStore::store(&conn, "arch", "low conf", 0.2);
        PatternStore::store(&conn, "arch", "high conf", 0.9);
        PatternStore::store(&conn, "arch", "mid conf", 0.5);

        let results = PatternStore::get_top(&conn, "arch", 2);
        assert_eq!(results.len(), 2);
        assert!(results[0].confidence >= results[1].confidence);
        assert!(results[0].content.contains("high"));
    }

    #[test]
    fn search_updates_access_count() {
        let conn = setup();
        PatternStore::store(&conn, "code", "findme", 0.5);

        PatternStore::search(&conn, "code", "findme", 10);
        PatternStore::search(&conn, "code", "findme", 10);

        // After 2 searches, the access_count stored is 2
        // (but the returned value in the second search shows 1 because we read before updating)
        let top = PatternStore::get_top(&conn, "code", 10);
        assert!(top[0].access_count >= 2);
    }
}
