//! Key-value persistent memory store for HexFlo.
//!
//! Uses SwarmDb's SQLite connection with a dedicated `hexflo_memory` table.

use rusqlite::params;
use serde::{Deserialize, Serialize};

use super::HexFlo;

// ── Types ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryEntry {
    pub key: String,
    pub value: String,
    pub scope: String,
    pub updated_at: String,
}

// ── Memory operations on HexFlo ────────────────────────

impl HexFlo {
    /// Store a key-value pair. Uses INSERT OR REPLACE for upsert semantics.
    pub async fn memory_store(
        &self,
        key: &str,
        value: &str,
        scope: Option<&str>,
    ) -> Result<(), String> {
        let db = self.require_db()?;
        let conn = db.conn().clone();
        let k = key.to_string();
        let v = value.to_string();
        let s = scope.unwrap_or("global").to_string();
        let now = chrono::Utc::now().to_rfc3339();

        tokio::task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            conn.execute(
                "INSERT OR REPLACE INTO hexflo_memory (key, value, scope, updated_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![k, v, s, now],
            )
        })
        .await
        .expect("spawn_blocking join")
        .map_err(|e| e.to_string())?;

        Ok(())
    }

    /// Retrieve a value by key.
    pub async fn memory_retrieve(&self, key: &str) -> Result<Option<String>, String> {
        let db = self.require_db()?;
        let conn = db.conn().clone();
        let k = key.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            let result: Option<String> = conn
                .query_row(
                    "SELECT value FROM hexflo_memory WHERE key = ?1",
                    params![k],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|e| e.to_string())?;
            Ok(result)
        })
        .await
        .expect("spawn_blocking join")
    }

    /// Search memory entries by LIKE pattern on key and value.
    pub async fn memory_search(&self, query: &str) -> Result<Vec<MemoryEntry>, String> {
        let db = self.require_db()?;
        let conn = db.conn().clone();
        let pattern = format!("%{}%", query);

        tokio::task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            let mut stmt = conn
                .prepare(
                    "SELECT key, value, scope, updated_at FROM hexflo_memory
                     WHERE key LIKE ?1 OR value LIKE ?1
                     ORDER BY updated_at DESC",
                )
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map(params![pattern], |row| {
                    Ok(MemoryEntry {
                        key: row.get(0)?,
                        value: row.get(1)?,
                        scope: row.get(2)?,
                        updated_at: row.get(3)?,
                    })
                })
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            Ok(rows)
        })
        .await
        .expect("spawn_blocking join")
    }

    /// Delete a memory entry by key. Returns true if a row was deleted.
    pub async fn memory_delete(&self, key: &str) -> Result<bool, String> {
        let db = self.require_db()?;
        let conn = db.conn().clone();
        let k = key.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            let changed = conn
                .execute("DELETE FROM hexflo_memory WHERE key = ?1", params![k])
                .map_err(|e| e.to_string())?;
            Ok(changed > 0)
        })
        .await
        .expect("spawn_blocking join")
    }

    /// List all memory entries in a given scope.
    pub async fn memory_list(&self, scope: &str) -> Result<Vec<MemoryEntry>, String> {
        let db = self.require_db()?;
        let conn = db.conn().clone();
        let s = scope.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            let mut stmt = conn
                .prepare(
                    "SELECT key, value, scope, updated_at FROM hexflo_memory
                     WHERE scope = ?1
                     ORDER BY key ASC",
                )
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map(params![s], |row| {
                    Ok(MemoryEntry {
                        key: row.get(0)?,
                        value: row.get(1)?,
                        scope: row.get(2)?,
                        updated_at: row.get(3)?,
                    })
                })
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            Ok(rows)
        })
        .await
        .expect("spawn_blocking join")
    }
}

// We need the optional() extension
use rusqlite::OptionalExtension;
