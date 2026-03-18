//! Runtime backend selection for IStatePort (ADR-025 Phase 4).
//!
//! Priority: `HEX_STATE_BACKEND` env var > `.hex/state.json` file > default (SQLite).
//!
//! SQLite is always available. SpacetimeDB requires the `spacetimedb` feature.

use std::path::PathBuf;
use std::sync::Arc;

use crate::adapters::sqlite_state::SqliteStateAdapter;
use crate::ports::state::{IStatePort, StateError};

// ── Configuration ────────────────────────────────────────

/// Which state backend to use and how to connect to it.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "backend", rename_all = "lowercase")]
pub enum StateBackendConfig {
    Sqlite {
        #[serde(default = "default_sqlite_path")]
        path: PathBuf,
    },
    #[cfg(feature = "spacetimedb")]
    Spacetimedb {
        #[serde(default = "default_stdb_host")]
        host: String,
        #[serde(default = "default_stdb_database")]
        database: String,
        #[serde(default)]
        auth_token: Option<String>,
    },
}

impl Default for StateBackendConfig {
    fn default() -> Self {
        Self::Sqlite {
            path: default_sqlite_path(),
        }
    }
}

fn default_sqlite_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(format!("{}/.hex/hub.db", home))
}

#[cfg(feature = "spacetimedb")]
fn default_stdb_host() -> String {
    "http://localhost:3000".to_string()
}

#[cfg(feature = "spacetimedb")]
fn default_stdb_database() -> String {
    "hex-nexus".to_string()
}

// ── Config Loading ───────────────────────────────────────

/// Resolve which backend to use.
///
/// Priority:
/// 1. `HEX_STATE_BACKEND` env var ("sqlite" or "spacetimedb")
/// 2. `.hex/state.json` in the current working directory
/// 3. Default: SQLite at `~/.hex/hub.db`
pub fn resolve_config() -> StateBackendConfig {
    // 1. Environment variable
    if let Ok(backend) = std::env::var("HEX_STATE_BACKEND") {
        match backend.to_lowercase().as_str() {
            "sqlite" => {
                tracing::info!("State backend: SQLite (from HEX_STATE_BACKEND)");
                return StateBackendConfig::Sqlite {
                    path: default_sqlite_path(),
                };
            }
            #[cfg(feature = "spacetimedb")]
            "spacetimedb" => {
                tracing::info!("State backend: SpacetimeDB (from HEX_STATE_BACKEND)");
                // Try to load connection details from file, fall back to defaults
                if let Some(cfg) = load_config_file() {
                    return cfg;
                }
                return StateBackendConfig::Spacetimedb {
                    host: std::env::var("HEX_STDB_HOST")
                        .unwrap_or_else(|_| default_stdb_host()),
                    database: std::env::var("HEX_STDB_DATABASE")
                        .unwrap_or_else(|_| default_stdb_database()),
                    auth_token: std::env::var("HEX_STDB_AUTH_TOKEN").ok(),
                };
            }
            other => {
                tracing::warn!(
                    "Unknown HEX_STATE_BACKEND value '{}' — falling back to SQLite",
                    other,
                );
                return StateBackendConfig::default();
            }
        }
    }

    // 2. Config file
    if let Some(cfg) = load_config_file() {
        tracing::info!("State backend loaded from .hex/state.json");
        return cfg;
    }

    // 3. Default
    tracing::info!("State backend: SQLite (default)");
    StateBackendConfig::default()
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

/// Create the appropriate IStatePort implementation based on config.
///
/// Returns an `Arc<dyn IStatePort>` ready for injection into AppState.
pub fn create_state_backend(
    config: &StateBackendConfig,
) -> Result<Arc<dyn IStatePort>, StateError> {
    match config {
        StateBackendConfig::Sqlite { path } => {
            let path_str = path.to_string_lossy();
            let adapter = SqliteStateAdapter::new(&path_str)?;
            tracing::info!(path = %path_str, "SqliteStateAdapter created");
            Ok(Arc::new(adapter))
        }
        #[cfg(feature = "spacetimedb")]
        StateBackendConfig::Spacetimedb {
            host,
            database,
            auth_token,
        } => {
            use crate::adapters::spacetime_state::{SpacetimeConfig, SpacetimeStateAdapter};

            let stdb_config = SpacetimeConfig {
                host: host.clone(),
                database: database.clone(),
                auth_token: auth_token.clone(),
            };
            let adapter = SpacetimeStateAdapter::new(stdb_config);
            tracing::info!(
                host = %host,
                database = %database,
                "SpacetimeStateAdapter created (connect() must be called separately)",
            );
            Ok(Arc::new(adapter))
        }
    }
}

/// Convenience: resolve config and create the backend in one call.
pub fn create_default_state_backend() -> Result<Arc<dyn IStatePort>, StateError> {
    let config = resolve_config();
    create_state_backend(&config)
}
