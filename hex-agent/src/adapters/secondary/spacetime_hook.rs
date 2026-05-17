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
// REST-only implementation
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// NOTE(ADR-2604050900): The hook-registry WASM module has been deleted.
// Hooks are loaded from local YAML files and synced via config_sync.rs.
// This adapter uses REST to fetch hooks from hex-hub until a hexflo-coordination
// subscription is implemented.

mod inner {
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

pub use inner::SpacetimeHookRunner;
