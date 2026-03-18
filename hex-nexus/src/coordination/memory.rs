//! Key-value persistent memory store for HexFlo.
//!
//! Delegates to IStatePort — works with both SQLite and SpacetimeDB backends.

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
    /// Store a key-value pair. Upsert semantics via IStatePort.
    pub async fn memory_store(
        &self,
        key: &str,
        value: &str,
        scope: Option<&str>,
    ) -> Result<(), String> {
        let s = scope.unwrap_or("global");
        self.state
            .hexflo_memory_store(key, value, s)
            .await
            .map_err(|e| e.to_string())
    }

    /// Retrieve a value by key.
    pub async fn memory_retrieve(&self, key: &str) -> Result<Option<String>, String> {
        self.state
            .hexflo_memory_retrieve(key)
            .await
            .map_err(|e| e.to_string())
    }

    /// Search memory entries by pattern on key and value.
    pub async fn memory_search(&self, query: &str) -> Result<Vec<MemoryEntry>, String> {
        let results = self.state
            .hexflo_memory_search(query)
            .await
            .map_err(|e| e.to_string())?;

        Ok(results
            .into_iter()
            .map(|(k, v)| MemoryEntry {
                key: k,
                value: v,
                scope: "global".to_string(),
                updated_at: String::new(),
            })
            .collect())
    }

    /// Delete a memory entry by key. Returns true if a row was deleted.
    pub async fn memory_delete(&self, key: &str) -> Result<bool, String> {
        // Check if key exists first — IStatePort::hexflo_memory_delete
        // succeeds silently when the key doesn't exist, but callers
        // need the boolean to return 404.
        let exists = self.state
            .hexflo_memory_retrieve(key)
            .await
            .map_err(|e| e.to_string())?
            .is_some();

        if !exists {
            return Ok(false);
        }

        self.state
            .hexflo_memory_delete(key)
            .await
            .map(|()| true)
            .map_err(|e| e.to_string())
    }

    /// List all memory entries in a given scope.
    /// (Implemented as a search with scope filter — IStatePort search covers this.)
    pub async fn memory_list(&self, scope: &str) -> Result<Vec<MemoryEntry>, String> {
        // Search with scope prefix as query — the SQLite LIKE will match
        let results = self.state
            .hexflo_memory_search(scope)
            .await
            .map_err(|e| e.to_string())?;

        Ok(results
            .into_iter()
            .map(|(k, v)| MemoryEntry {
                key: k,
                value: v,
                scope: scope.to_string(),
                updated_at: String::new(),
            })
            .collect())
    }
}
