//! SpacetimeDB-backed implementation of HookRunnerPort.
//!
//! Two compilation modes:
//! 1. Default (no feature): REST-only fallback fetching from hex-hub `/api/state/hooks`.
//! 2. `spacetimedb` feature: Real subscription via `spacetimedb-sdk`.
//!    Connects via WebSocket, subscribes to the `hook` table, caches configs
//!    locally, and logs execution results via the `log_execution` reducer.
//!    Falls back to REST if the SpacetimeDB connection fails.

use async_trait::async_trait;
use std::sync::{Arc, RwLock};
use std::time::Instant;

use crate::ports::{Hook, HookConfig, HookEvent, HookResult};
use crate::ports::hooks::{HookRunnerPort, HookError};

/// Handler configuration as stored in SpacetimeDB's handler_config_json.
#[derive(serde::Deserialize)]
struct HandlerConfig {
    #[serde(default)]
    command: Option<String>,
}

/// DTO matching the shape of hex-hub REST `/api/state/hooks` response
/// and SpacetimeDB's `hook` table.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct HookEntryDto {
    #[serde(default)]
    #[allow(dead_code)]
    id: String,
    event_type: String,
    #[allow(dead_code)]
    handler_type: String,
    handler_config_json: String,
    timeout_secs: u32,
    blocking: bool,
    tool_pattern: String,
    enabled: bool,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Feature-gated implementation (real SpacetimeDB SDK)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(feature = "spacetimedb")]
mod real {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use tokio::sync::RwLock as TokioRwLock;

    use spacetimedb_sdk::DbContext;
    use spacetimedb_sdk::Table as SdkTable;

    // Generated hook-registry bindings from hex-hub-core.
    use hex_hub_core::spacetime_bindings::hook_registry::{
        DbConnection,
        hook_table::HookTableAccess,
        log_execution_reducer::log_execution,
    };

    /// Convert a SpacetimeDB `hook` row into the domain `Hook` type.
    fn stdb_row_to_hook(
        row: &hex_hub_core::spacetime_bindings::hook_registry::Hook,
    ) -> Result<Hook, HookError> {
        let event = parse_event_type(&row.event_type)?;

        let command = serde_json::from_str::<HandlerConfig>(&row.handler_config_json)
            .ok()
            .and_then(|c| c.command)
            .unwrap_or_default();

        Ok(Hook {
            event,
            command,
            timeout_secs: row.timeout_secs,
            blocking: row.blocking,
            tool_pattern: if row.tool_pattern.is_empty() {
                None
            } else {
                Some(row.tool_pattern.clone())
            },
        })
    }

    /// SpacetimeDB-backed hook runner with real-time subscription.
    ///
    /// Architecture:
    /// - Connects to SpacetimeDB via WebSocket (`DbConnection::builder()`)
    /// - Subscribes to `SELECT * FROM hook`
    /// - `on_insert`/`on_delete` callbacks keep the local cache in sync
    /// - `log_execution` reducer writes audit trail back to SpacetimeDB
    /// - Falls back to REST if SpacetimeDB connection fails
    pub struct SpacetimeHookRunner {
        cache: Arc<RwLock<HookConfig>>,
        hub_url: String,
        agent_id: String,
        /// Whether SpacetimeDB subscription has applied initial rows.
        subscribed: Arc<AtomicBool>,
        /// The SpacetimeDB connection handle (None until connect() succeeds).
        connection: Arc<TokioRwLock<Option<DbConnection>>>,
    }

    impl SpacetimeHookRunner {
        pub fn new(hub_url: &str, agent_id: &str) -> Self {
            Self {
                cache: Arc::new(RwLock::new(HookConfig::default())),
                hub_url: hub_url.to_string(),
                agent_id: agent_id.to_string(),
                subscribed: Arc::new(AtomicBool::new(false)),
                connection: Arc::new(TokioRwLock::new(None)),
            }
        }

        /// Connect to SpacetimeDB and subscribe to the hook table.
        ///
        /// If the connection fails or the subscription does not apply within 5s,
        /// seeds the cache from REST as a fallback.
        pub async fn connect(&self, host: &str, database: &str) -> Result<(), HookError> {
            let cache = Arc::clone(&self.cache);
            let subscribed = Arc::clone(&self.subscribed);

            // Clones for the on_connect closure
            let cache_on_applied = Arc::clone(&cache);
            let subscribed_on_applied = Arc::clone(&subscribed);
            let cache_insert = Arc::clone(&cache);
            let cache_delete = Arc::clone(&cache);

            match DbConnection::builder()
                .with_uri(host)
                .with_module_name(database)
                .on_connect(move |conn, _identity, _token| {
                    // Register table callbacks before subscribing
                    conn.db().hook().on_insert({
                        let c = Arc::clone(&cache_insert);
                        move |_ctx, row| {
                            if !row.enabled {
                                return;
                            }
                            if let Ok(hook) = stdb_row_to_hook(row) {
                                if let Ok(mut cache) = c.write() {
                                    cache.hooks.push(hook);
                                }
                            }
                        }
                    });

                    conn.db().hook().on_delete({
                        let c = Arc::clone(&cache_delete);
                        move |_ctx, row| {
                            if let Ok(hook) = stdb_row_to_hook(row) {
                                if let Ok(mut cache) = c.write() {
                                    cache.hooks.retain(|h| {
                                        !(h.event == hook.event && h.command == hook.command)
                                    });
                                }
                            }
                        }
                    });

                    // Subscribe to the hook table
                    conn.subscription_builder()
                        .on_applied({
                            let c = Arc::clone(&cache_on_applied);
                            let s = Arc::clone(&subscribed_on_applied);
                            move |ctx| {
                                // Initial sync: populate cache from subscription snapshot
                                let hooks: Vec<Hook> = ctx.db.hook().iter()
                                    .filter(|h| h.enabled)
                                    .filter_map(|h| stdb_row_to_hook(&h).ok())
                                    .collect();
                                if let Ok(mut cache) = c.write() {
                                    *cache = HookConfig { hooks };
                                }
                                s.store(true, Ordering::Release);
                                tracing::info!("SpacetimeDB hook subscription applied");
                            }
                        })
                        .on_error(|_ctx| {
                            tracing::error!("SpacetimeDB hook subscription error");
                        })
                        .subscribe(["SELECT * FROM hook"]);
                })
                .on_connect_error(|_ctx, err| {
                    tracing::warn!(?err, "SpacetimeDB hook connection error");
                })
                .on_disconnect(|_ctx, err| {
                    if let Some(e) = err {
                        tracing::warn!(?e, "SpacetimeDB hook disconnected with error");
                    } else {
                        tracing::info!("SpacetimeDB hook disconnected cleanly");
                    }
                })
                .build()
            {
                Ok(conn) => {
                    // Spawn the async message loop to process subscription updates
                    let conn_for_loop = conn.clone();
                    tokio::spawn(async move {
                        if let Err(e) = conn_for_loop.run_async().await {
                            tracing::warn!("SpacetimeDB hook run_async ended: {}", e);
                        }
                    });

                    // Wait briefly for initial subscription to apply
                    let deadline =
                        tokio::time::Instant::now() + tokio::time::Duration::from_secs(5);
                    while tokio::time::Instant::now() < deadline {
                        if subscribed.load(Ordering::Acquire) {
                            break;
                        }
                        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
                    }

                    if subscribed.load(Ordering::Acquire) {
                        let count = self
                            .cache
                            .read()
                            .map(|c| c.hooks.len())
                            .unwrap_or(0);
                        tracing::info!(
                            count,
                            "SpacetimeDB hook subscription active, {} hooks cached",
                            count
                        );
                        *self.connection.write().await = Some(conn);
                        return Ok(());
                    }

                    // Subscription didn't apply in time -- store connection anyway
                    // (it may apply later), but also seed from REST
                    tracing::warn!(
                        "SpacetimeDB hook subscription did not apply within 5s, seeding from REST"
                    );
                    *self.connection.write().await = Some(conn);
                    self.seed_from_rest().await
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "SpacetimeDB hook connection failed, falling back to REST"
                    );
                    self.seed_from_rest().await
                }
            }
        }

        /// Seed the cache from the hex-hub REST API as a fallback.
        async fn seed_from_rest(&self) -> Result<(), HookError> {
            let url = format!("{}/api/state/hooks", self.hub_url);
            let resp = reqwest::get(&url)
                .await
                .map_err(|e| HookError::LoadError(e.to_string()))?;

            if !resp.status().is_success() {
                return Err(HookError::LoadError(format!("HTTP {}", resp.status())));
            }

            let entries: Vec<HookEntryDto> = resp
                .json()
                .await
                .map_err(|e| HookError::LoadError(e.to_string()))?;

            let hooks: Vec<Hook> = entries
                .into_iter()
                .filter_map(|e| dto_to_hook(e).ok())
                .collect();
            if let Ok(mut cache) = self.cache.write() {
                *cache = HookConfig { hooks };
            }
            Ok(())
        }

        /// Log a hook execution result via SpacetimeDB reducer.
        /// Falls back to REST POST if no SpacetimeDB connection is available.
        async fn log_execution_result(
            &self,
            hook_id: &str,
            event: &HookEvent,
            result: &HookResult,
        ) {
            let event_str = format!("{:?}", event).to_lowercase();
            let timestamp = chrono::Utc::now().to_rfc3339();

            // Try SpacetimeDB reducer first
            let logged_via_stdb = {
                let conn_guard = self.connection.read().await;
                if let Some(conn) = conn_guard.as_ref() {
                    conn.reducers()
                        .log_execution(
                            hook_id.to_string(),
                            self.agent_id.clone(),
                            event_str.clone(),
                            result.exit_code,
                            result.stdout.clone(),
                            result.stderr.clone(),
                            result.duration_ms,
                            result.timed_out,
                            timestamp.clone(),
                        )
                        .is_ok()
                } else {
                    false
                }
            };

            // Fall back to REST if reducer call was not possible
            if !logged_via_stdb {
                let payload = serde_json::json!({
                    "hookId": hook_id,
                    "agentId": self.agent_id,
                    "eventType": event_str,
                    "exitCode": result.exit_code,
                    "stdout": result.stdout,
                    "stderr": result.stderr,
                    "durationMs": result.duration_ms,
                    "timedOut": result.timed_out,
                    "timestamp": timestamp,
                });
                let url = format!("{}/api/state/hooks/log", self.hub_url);
                let _ = reqwest::Client::new().post(&url).json(&payload).send().await;
            }
        }
    }

    #[async_trait]
    impl HookRunnerPort for SpacetimeHookRunner {
        async fn load_config(&self, _settings_path: &str) -> Result<HookConfig, HookError> {
            let cache = self
                .cache
                .read()
                .map_err(|e| HookError::LoadError(format!("Cache lock poisoned: {}", e)))?;
            Ok(cache.clone())
        }

        async fn run_hooks(
            &self,
            config: &HookConfig,
            event: &HookEvent,
            env: &[(&str, &str)],
        ) -> Vec<HookResult> {
            let matching = config.for_event(event);
            let mut results = Vec::new();

            for hook in matching {
                if hook.command.is_empty() {
                    continue;
                }

                let start = Instant::now();
                let output = tokio::process::Command::new("sh")
                    .arg("-c")
                    .arg(&hook.command)
                    .envs(env.iter().map(|(k, v)| (k.to_string(), v.to_string())))
                    .output()
                    .await;

                let duration_ms = start.elapsed().as_millis() as u64;

                let result = match output {
                    Ok(out) => HookResult {
                        event: event.clone(),
                        command: hook.command.clone(),
                        exit_code: out.status.code().unwrap_or(-1),
                        stdout: String::from_utf8_lossy(&out.stdout).to_string(),
                        stderr: String::from_utf8_lossy(&out.stderr).to_string(),
                        duration_ms,
                        timed_out: false,
                    },
                    Err(e) => HookResult {
                        event: event.clone(),
                        command: hook.command.clone(),
                        exit_code: -1,
                        stdout: String::new(),
                        stderr: e.to_string(),
                        duration_ms,
                        timed_out: false,
                    },
                };

                // Log via SpacetimeDB reducer (or REST fallback)
                self.log_execution_result("", event, &result).await;
                results.push(result);
            }

            results
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Stub implementation (no SpacetimeDB SDK — REST only)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(not(feature = "spacetimedb"))]
mod stub {
    use super::*;

    /// REST-only hook runner (SpacetimeDB feature not enabled).
    pub struct SpacetimeHookRunner {
        cache: Arc<RwLock<HookConfig>>,
        hub_url: String,
        agent_id: String,
    }

    impl SpacetimeHookRunner {
        pub fn new(hub_url: &str, agent_id: &str) -> Self {
            Self {
                cache: Arc::new(RwLock::new(HookConfig::default())),
                hub_url: hub_url.to_string(),
                agent_id: agent_id.to_string(),
            }
        }

        pub async fn connect(&self, _host: &str, _database: &str) -> Result<(), HookError> {
            self.fetch_from_hub().await
        }

        async fn fetch_from_hub(&self) -> Result<(), HookError> {
            let url = format!("{}/api/state/hooks", self.hub_url);
            let resp = reqwest::get(&url)
                .await
                .map_err(|e| HookError::LoadError(e.to_string()))?;

            if !resp.status().is_success() {
                return Err(HookError::LoadError(format!("HTTP {}", resp.status())));
            }

            let entries: Vec<HookEntryDto> = resp
                .json()
                .await
                .map_err(|e| HookError::LoadError(e.to_string()))?;

            let hooks: Vec<Hook> = entries
                .into_iter()
                .filter_map(|e| dto_to_hook(e).ok())
                .collect();
            if let Ok(mut cache) = self.cache.write() {
                *cache = HookConfig { hooks };
            }
            Ok(())
        }

        async fn log_execution_rest(
            &self,
            hook_id: &str,
            event: &HookEvent,
            result: &HookResult,
        ) {
            let event_str = format!("{:?}", event).to_lowercase();
            let payload = serde_json::json!({
                "hookId": hook_id,
                "agentId": self.agent_id,
                "eventType": event_str,
                "exitCode": result.exit_code,
                "stdout": result.stdout,
                "stderr": result.stderr,
                "durationMs": result.duration_ms,
                "timedOut": result.timed_out,
                "timestamp": chrono::Utc::now().to_rfc3339(),
            });

            let url = format!("{}/api/state/hooks/log", self.hub_url);
            let _ = reqwest::Client::new().post(&url).json(&payload).send().await;
        }
    }

    #[async_trait]
    impl HookRunnerPort for SpacetimeHookRunner {
        async fn load_config(&self, _settings_path: &str) -> Result<HookConfig, HookError> {
            let cache = self
                .cache
                .read()
                .map_err(|e| HookError::LoadError(format!("Cache lock poisoned: {}", e)))?;
            Ok(cache.clone())
        }

        async fn run_hooks(
            &self,
            config: &HookConfig,
            event: &HookEvent,
            env: &[(&str, &str)],
        ) -> Vec<HookResult> {
            let matching = config.for_event(event);
            let mut results = Vec::new();

            for hook in matching {
                if hook.command.is_empty() {
                    continue;
                }

                let start = Instant::now();
                let output = tokio::process::Command::new("sh")
                    .arg("-c")
                    .arg(&hook.command)
                    .envs(env.iter().map(|(k, v)| (k.to_string(), v.to_string())))
                    .output()
                    .await;

                let duration_ms = start.elapsed().as_millis() as u64;

                let result = match output {
                    Ok(out) => HookResult {
                        event: event.clone(),
                        command: hook.command.clone(),
                        exit_code: out.status.code().unwrap_or(-1),
                        stdout: String::from_utf8_lossy(&out.stdout).to_string(),
                        stderr: String::from_utf8_lossy(&out.stderr).to_string(),
                        duration_ms,
                        timed_out: false,
                    },
                    Err(e) => HookResult {
                        event: event.clone(),
                        command: hook.command.clone(),
                        exit_code: -1,
                        stdout: String::new(),
                        stderr: e.to_string(),
                        duration_ms,
                        timed_out: false,
                    },
                };

                self.log_execution_rest("", event, &result).await;
                results.push(result);
            }

            results
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Shared helpers + re-export
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Parse an event_type string into the domain HookEvent enum.
fn parse_event_type(s: &str) -> Result<HookEvent, HookError> {
    match s {
        "session_start" => Ok(HookEvent::SessionStart),
        "session_end" => Ok(HookEvent::SessionEnd),
        "pre_task" => Ok(HookEvent::PreTask),
        "post_task" => Ok(HookEvent::PostTask),
        "pre_edit" => Ok(HookEvent::PreEdit),
        "post_edit" => Ok(HookEvent::PostEdit),
        "pre_tool_use" => Ok(HookEvent::PreToolUse),
        "post_tool_use" => Ok(HookEvent::PostToolUse),
        "user_prompt_submit" => Ok(HookEvent::UserPromptSubmit),
        other => Err(HookError::LoadError(format!("Unknown event: {}", other))),
    }
}

/// Convert a REST DTO into the domain Hook type.
fn dto_to_hook(dto: HookEntryDto) -> Result<Hook, HookError> {
    if !dto.enabled {
        return Err(HookError::LoadError("Hook disabled".into()));
    }

    let event = parse_event_type(&dto.event_type)?;

    let command = serde_json::from_str::<HandlerConfig>(&dto.handler_config_json)
        .ok()
        .and_then(|c| c.command)
        .unwrap_or_default();

    Ok(Hook {
        event,
        command,
        timeout_secs: dto.timeout_secs,
        blocking: dto.blocking,
        tool_pattern: if dto.tool_pattern.is_empty() {
            None
        } else {
            Some(dto.tool_pattern)
        },
    })
}

#[cfg(feature = "spacetimedb")]
pub use real::SpacetimeHookRunner;
#[cfg(not(feature = "spacetimedb"))]
pub use stub::SpacetimeHookRunner;
