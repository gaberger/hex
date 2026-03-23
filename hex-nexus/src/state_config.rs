//! Runtime backend configuration for IStatePort (ADR-025 Phase 4, ADR-032).
//!
//! SpacetimeDB is the only backend. SQLite has been removed.
//!
//! Priority: `HEX_STATE_BACKEND` env var > `.hex/state.json` file > default (SpacetimeDB localhost:3033, database hexflo-coordination).

use std::path::PathBuf;
use std::sync::Arc;

use crate::ports::state::{IStatePort, StateError};

// ── Configuration ────────────────────────────────────────

/// SpacetimeDB connection configuration.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub struct StateBackendConfig {
    #[serde(default = "default_stdb_host")]
    pub host: String,
    #[serde(default = "default_stdb_database")]
    pub database: String,
    #[serde(default)]
    pub auth_token: Option<String>,
}

impl Default for StateBackendConfig {
    fn default() -> Self {
        Self {
            host: default_stdb_host(),
            database: default_stdb_database(),
            auth_token: None,
        }
    }
}

fn default_stdb_host() -> String {
    "http://localhost:3033".to_string()
}

fn default_stdb_database() -> String {
    "hexflo-coordination".to_string()
}

// ── Config Loading ───────────────────────────────────────

/// Resolve SpacetimeDB connection configuration.
///
/// Priority:
/// 1. `HEX_STDB_HOST` / `HEX_STDB_DATABASE` / `HEX_STDB_AUTH_TOKEN` env vars
/// 2. `.hex/state.json` in the current working directory or `~/.hex/state.json`
/// 3. Default: SpacetimeDB at `http://localhost:3033` database `hexflo-coordination`
pub fn resolve_config() -> StateBackendConfig {
    // 1. Environment variables
    let has_env = std::env::var("HEX_STDB_HOST").is_ok()
        || std::env::var("HEX_STDB_DATABASE").is_ok();

    if has_env {
        let cfg = StateBackendConfig {
            host: std::env::var("HEX_STDB_HOST")
                .unwrap_or_else(|_| default_stdb_host()),
            database: std::env::var("HEX_STDB_DATABASE")
                .unwrap_or_else(|_| default_stdb_database()),
            auth_token: std::env::var("HEX_STDB_AUTH_TOKEN").ok(),
        };
        tracing::info!(
            host = %cfg.host,
            database = %cfg.database,
            "State backend: SpacetimeDB (from env vars)",
        );
        return cfg;
    }

    // 2. Config file
    if let Some(cfg) = load_config_file() {
        tracing::info!(
            host = %cfg.host,
            database = %cfg.database,
            "State backend: SpacetimeDB (from .hex/state.json)",
        );
        return cfg;
    }

    // 3. Default
    let cfg = StateBackendConfig::default();
    tracing::info!(
        host = %cfg.host,
        database = %cfg.database,
        "State backend: SpacetimeDB (default)",
    );
    cfg
}

/// Try to read `.hex/state.json` from the current working directory,
/// falling back to `~/.hex/state.json` for global configuration.
fn load_config_file() -> Option<StateBackendConfig> {
    let candidates = [
        PathBuf::from(".hex/state.json"),
        dirs_next().map(|h| h.join(".hex/state.json")).unwrap_or_default(),
    ];

    for path in &candidates {
        if let Ok(contents) = std::fs::read_to_string(path) {
            match serde_json::from_str::<StateBackendConfig>(&contents) {
                Ok(cfg) => {
                    tracing::info!(path = %path.display(), "Loaded state config");
                    return Some(cfg);
                }
                Err(e) => {
                    tracing::warn!(path = %path.display(), "Failed to parse state.json: {}", e);
                }
            }
        }
    }
    None
}

/// Return the user's home directory.
fn dirs_next() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .map(PathBuf::from)
}

// ── Factory ──────────────────────────────────────────────

/// Create the SpacetimeDB IStatePort implementation.
///
/// Returns an `Arc<dyn IStatePort>` ready for injection into AppState.
pub fn create_state_backend(
    config: &StateBackendConfig,
) -> Result<Arc<dyn IStatePort>, StateError> {
    use crate::adapters::spacetime_state::{SpacetimeConfig, SpacetimeStateAdapter};

    let stdb_config = SpacetimeConfig {
        host: config.host.clone(),
        database: config.database.clone(),
        auth_token: config.auth_token.clone(),
    };
    let adapter = SpacetimeStateAdapter::new(stdb_config);
    tracing::info!(
        host = %config.host,
        database = %config.database,
        "SpacetimeStateAdapter created (connect() must be called separately)",
    );
    Ok(Arc::new(adapter))
}

/// Convenience: resolve config and create the backend in one call.
pub fn create_default_state_backend() -> Result<Arc<dyn IStatePort>, StateError> {
    let config = resolve_config();
    create_state_backend(&config)
}
