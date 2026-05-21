//! Runtime backend configuration for IStatePort (ADR-025 Phase 4, ADR-032,
//! ADR-2026-05-19-0900 P4.3).
//!
//! SpacetimeDB is the only backend. SQLite has been removed.
//!
//! `host` precedence is delegated to `adapters::stdb_endpoint::discover_endpoint`
//! (the canonical hierarchy: HEX_SPACETIMEDB_HOST / HEX_STDB_HOST env →
//! `.hex/project.json` coordination.host → localhost:3033 default →
//! HEX_STDB_FALLBACK_HOST). `.hex/state.json` is NO LONGER a configuration
//! source — it remains write-only telemetry for the dashboard (current
//! STDB host, swarm count, etc.). The cache-drift class of bugs from
//! 2026-05-19 was rooted in state.json being treated as config.
//!
//! `database` and `auth_token` still come from env vars (HEX_STDB_DATABASE,
//! HEX_STDB_AUTH_TOKEN) with module defaults.

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
    hex_core::stdb_database_for_module("hexflo-coordination").to_string()
}

// ── Config Loading ───────────────────────────────────────

/// Resolve SpacetimeDB connection configuration.
///
/// Host comes from the canonical hierarchy in
/// [`crate::adapters::stdb_endpoint::discover_endpoint`]:
///
/// 1. `HEX_SPACETIMEDB_HOST` / `HEX_STDB_HOST` env vars
/// 2. `.hex/project.json` → `coordination.host`
/// 3. `http://127.0.0.1:3033` default
/// 4. `HEX_STDB_FALLBACK_HOST` env var (operator escape hatch)
///
/// Database + auth come from env vars + module defaults. `.hex/state.json`
/// is **not** read as config (ADR-2026-05-19-0900 P4.3); it survives as the
/// dashboard's write-only telemetry surface. If a state.json with a
/// config-shaped payload is found, we warn so the operator knows to
/// clean it up.
pub fn resolve_config() -> StateBackendConfig {
    let host = crate::adapters::stdb_endpoint::discover_endpoint();
    let database = std::env::var("HEX_STDB_DATABASE")
        .unwrap_or_else(|_| default_stdb_database());
    let auth_token = std::env::var("HEX_STDB_AUTH_TOKEN")
        .ok()
        .filter(|s| !s.is_empty());

    warn_on_legacy_state_json_config();

    tracing::info!(
        host = %host,
        database = %database,
        "State backend: SpacetimeDB (host via stdb_endpoint::discover_endpoint)",
    );
    StateBackendConfig { host, database, auth_token }
}

/// Detect the legacy "state.json with backend config in it" shape and
/// log a deprecation warning. Doesn't mutate the file — operators clean
/// it up on their own schedule. Best-effort: file missing / unparseable
/// is silent.
fn warn_on_legacy_state_json_config() {
    let candidates = [
        PathBuf::from(".hex/state.json"),
        dirs_next().map(|h| h.join(".hex/state.json")).unwrap_or_default(),
    ];
    for path in &candidates {
        let Ok(contents) = std::fs::read_to_string(path) else { continue };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&contents) else { continue };
        let has_legacy = value.get("host").and_then(|h| h.as_str()).is_some()
            || value.get("backend").is_some()
            || value.get("database").and_then(|d| d.as_str()).is_some();
        if has_legacy {
            tracing::warn!(
                path = %path.display(),
                "Legacy .hex/state.json config fields detected (host/backend/database). \
                 These are IGNORED — endpoint is resolved via stdb_endpoint::discover_endpoint. \
                 Move host config to HEX_SPACETIMEDB_HOST or .hex/project.json coordination.host \
                 to silence this warning (ADR-2026-05-19-0900 P4.3)."
            );
            return; // one warning is enough
        }
    }
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

// ── Decision Deadline Configuration (ADR-2026-04-13-1500 P1.2) ───

/// Default decision deadline in seconds (2 hours per ADR).
const DEFAULT_DECISION_DEADLINE_SECS: u64 = 7200;

/// Resolve the decision auto-resolution deadline in seconds.
///
/// Priority:
/// 1. `HEX_DECISION_DEADLINE_SECS` env var
/// 2. `.hex/project.json` → `decision.deadline_secs`
/// 3. Default: 7200 (2 hours per ADR-2026-04-13-1500)
pub fn resolve_decision_deadline_secs() -> u64 {
    // 1. Environment variable (highest precedence)
    if let Ok(val) = std::env::var("HEX_DECISION_DEADLINE_SECS") {
        if let Ok(secs) = val.parse::<u64>() {
            if secs > 0 {
                tracing::info!(deadline_secs = secs, "Decision deadline from env var");
                return secs;
            }
            tracing::warn!("HEX_DECISION_DEADLINE_SECS must be > 0, ignoring");
        } else {
            tracing::warn!(
                value = %val,
                "HEX_DECISION_DEADLINE_SECS is not a valid integer, ignoring"
            );
        }
    }

    // 2. .hex/project.json → decision.deadline_secs
    let project_dir = std::env::var("CLAUDE_PROJECT_DIR")
        .or_else(|_| std::env::var("HEX_PROJECT_DIR"))
        .unwrap_or_else(|_| ".".to_string());
    let project_json = std::path::Path::new(&project_dir).join(".hex/project.json");
    if let Ok(content) = std::fs::read_to_string(&project_json) {
        if let Ok(project) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(secs) = project["decision"]["deadline_secs"].as_u64() {
                if secs > 0 {
                    tracing::info!(
                        deadline_secs = secs,
                        "Decision deadline from .hex/project.json"
                    );
                    return secs;
                }
            }
        }
    }

    // 3. Default
    tracing::debug!(
        deadline_secs = DEFAULT_DECISION_DEADLINE_SECS,
        "Decision deadline: using default"
    );
    DEFAULT_DECISION_DEADLINE_SECS
}

/// Like `create_default_state_backend` but wires an `InferenceTxBus` so that
/// `inference_task_create` broadcasts to /ws/inference subscribers immediately
/// on insert (ADR-2026-04-01-1200 P2.T3).
pub fn create_default_state_backend_with_inference(
    inference_tx: crate::state::InferenceTxBus,
) -> Result<Arc<dyn IStatePort>, StateError> {
    use crate::adapters::spacetime_state::{SpacetimeConfig, SpacetimeStateAdapter};

    let config = resolve_config();
    let stdb_config = SpacetimeConfig {
        host: config.host.clone(),
        database: config.database.clone(),
        auth_token: config.auth_token.clone(),
    };
    let adapter = SpacetimeStateAdapter::new(stdb_config)
        .with_inference_tx(inference_tx);
    tracing::info!(
        host = %config.host,
        database = %config.database,
        "SpacetimeStateAdapter created with inference_tx (connect() must be called separately)",
    );
    Ok(Arc::new(adapter))
}
