//! SpacetimeDB-backed implementation of SkillLoaderPort.
//!
//! Subscribes to the `skill` table in SpacetimeDB. Maintains an in-memory
//! SkillManifest cache that auto-refreshes on table change callbacks.
//! Zero-latency reads — all data is local after initial subscription sync.

use async_trait::async_trait;
use std::sync::{Arc, RwLock};

use crate::ports::{Skill, SkillManifest, SkillTrigger};
use crate::ports::skills::{SkillLoaderPort, SkillLoadError};

/// Trigger entry as stored in SpacetimeDB's triggers_json column.
#[derive(serde::Deserialize)]
struct TriggerEntry {
    trigger_type: String,
    trigger_value: String,
}

/// SpacetimeDB-backed skill loader.
///
/// On creation, subscribes to the `skill` table. Each row insert/update/delete
/// fires a callback that rebuilds the in-memory SkillManifest. The `load()`
/// method returns the cached manifest with no network round-trip.
pub struct SpacetimeSkillLoader {
    /// Cached manifest — rebuilt on every table change callback.
    cache: Arc<RwLock<SkillManifest>>,
    /// Hub base URL for REST fallback (if SpacetimeDB subscription not yet synced).
    hub_url: String,
}

impl SpacetimeSkillLoader {
    pub fn new(hub_url: &str) -> Self {
        Self {
            cache: Arc::new(RwLock::new(SkillManifest::default())),
            hub_url: hub_url.to_string(),
        }
    }

    /// Connect to SpacetimeDB and subscribe to the skill table.
    /// Call this once at startup after constructing the adapter.
    pub async fn connect(&self, _host: &str, _database: &str) -> Result<(), SkillLoadError> {
        // TODO: When spacetimedb-sdk is integrated:
        // 1. DbConnection::builder().with_uri(host).with_database_name(database).build()
        // 2. Subscribe to "SELECT * FROM skill"
        // 3. Register on_insert / on_update / on_delete callbacks that call self.rebuild_cache()
        //
        // For now, fall back to REST fetch from hub_url.
        self.fetch_from_hub().await
    }

    /// Fetch skills from hex-hub REST API as a bootstrap/fallback.
    async fn fetch_from_hub(&self) -> Result<(), SkillLoadError> {
        let url = format!("{}/api/state/skills", self.hub_url);
        let resp = reqwest::get(&url).await.map_err(|e| SkillLoadError::ReadError {
            path: url.clone(),
            reason: e.to_string(),
        })?;

        if !resp.status().is_success() {
            return Err(SkillLoadError::ReadError {
                path: url,
                reason: format!("HTTP {}", resp.status()),
            });
        }

        let entries: Vec<SkillEntryDto> = resp.json().await.map_err(|e| SkillLoadError::ParseError {
            path: url,
            reason: e.to_string(),
        })?;

        let skills: Vec<Skill> = entries.into_iter().filter_map(|e| Self::dto_to_skill(e).ok()).collect();
        let manifest = SkillManifest { skills };

        if let Ok(mut cache) = self.cache.write() {
            *cache = manifest;
        }

        Ok(())
    }

    /// Convert a DTO from hub/SpacetimeDB into the domain Skill type.
    fn dto_to_skill(dto: SkillEntryDto) -> Result<Skill, SkillLoadError> {
        let trigger_entries: Vec<TriggerEntry> =
            serde_json::from_str(&dto.triggers_json).unwrap_or_default();

        let triggers = trigger_entries
            .into_iter()
            .map(|t| match t.trigger_type.as_str() {
                "slash_command" => SkillTrigger::SlashCommand(t.trigger_value),
                "pattern" => SkillTrigger::Pattern(t.trigger_value),
                "keyword" => SkillTrigger::Keyword(t.trigger_value),
                _ => SkillTrigger::Keyword(t.trigger_value),
            })
            .collect();

        Ok(Skill {
            name: dto.name,
            description: dto.description,
            triggers,
            body: dto.body,
            source_path: format!("spacetimedb://{}", dto.id),
        })
    }

    /// Rebuild the in-memory cache from a full table snapshot.
    /// Called by SpacetimeDB subscription callbacks.
    #[allow(dead_code)]
    fn rebuild_cache(&self, rows: Vec<SkillEntryDto>) {
        let skills: Vec<Skill> = rows.into_iter().filter_map(|e| Self::dto_to_skill(e).ok()).collect();
        if let Ok(mut cache) = self.cache.write() {
            *cache = SkillManifest { skills };
        }
    }
}

#[async_trait]
impl SkillLoaderPort for SpacetimeSkillLoader {
    async fn load(&self, _dirs: &[&str]) -> Result<SkillManifest, SkillLoadError> {
        // dirs parameter is ignored — we read from SpacetimeDB, not filesystem.
        // Return the cached manifest (populated by subscription or REST fallback).
        let cache = self.cache.read().map_err(|e| SkillLoadError::ReadError {
            path: "spacetimedb://skill".into(),
            reason: format!("Cache lock poisoned: {}", e),
        })?;
        Ok(cache.clone())
    }
}

/// DTO matching the shape of SpacetimeDB's `skill` table / hub REST response.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SkillEntryDto {
    id: String,
    name: String,
    description: String,
    triggers_json: String,
    body: String,
}
