//! StdbTaskPoller — push-based task claim using SpacetimeDB subscriptions.
//!
//! Primary path: subscribes to `code_gen_task` in the `remote-agent-registry`
//! SpacetimeDB module. New tasks are pushed via WebSocket → `on_insert` callback
//! → DashMap cache → `Notify` wakeup. Calling `poll_next()` returns the first
//! pending task and calls the `assign_task` reducer to claim it.
//!
//! Fallback path: when SpacetimeDB is unavailable (`is_connected()` false), falls
//! back to `TaskExecutor::poll_task()` REST polling (existing behaviour).

use std::sync::Arc;
use hex_core::{TaskCompletionBody, TaskStatus};
use serde::{Deserialize, Serialize};

use super::stdb_connection::StdbConnection;
use super::task_executor::{HexFloTask, TaskExecutor};

/// A task that has been successfully claimed by this agent.
///
/// Wraps either the SpacetimeDB `CodeGenTask.request_json` payload or
/// the REST-polled `HexFloTask` title (legacy), normalised into a common shape.
#[derive(Debug, Clone)]
pub struct ClaimedTask {
    pub task_id: String,
    pub description: String,
    /// True when claimed via StDB `assign_task` reducer; false for REST claim.
    pub via_stdb: bool,
}

/// Task payload encoded in `code_gen_task.request_json` by the supervisor.
///
/// Schema defined in ADR-2603300100 P4.1. The `description` field is the
/// human-readable step title passed to the code phase.
#[derive(Debug, Deserialize, Serialize)]
pub struct TaskPayload {
    pub step_id: String,
    pub description: String,
    #[serde(default)]
    pub model_hint: Option<String>,
    #[serde(default)]
    pub output_dir: Option<String>,
    /// Agent role hint from the supervisor (e.g. `"hex-coder"`).
    /// When present and equal to `"hex-coder"`, `TaskExecutor` routes the task
    /// directly to `CodePhaseWorker` instead of `SwarmSpawner`.
    #[serde(default)]
    pub role: Option<String>,
}

/// Polls for the next available `CodeGenTask` to execute.
///
/// Constructed once at daemon startup. Call `initialize()` before the poll loop
/// to attempt the SpacetimeDB connection; then call `poll_next()` inside the loop.
pub struct StdbTaskPoller {
    stdb: Arc<StdbConnection>,
    rest: TaskExecutor,
}

impl StdbTaskPoller {
    /// Create from environment variables. Reads:
    /// - `SPACETIMEDB_URL` (e.g. `ws://localhost:3033`) — for StDB connection
    /// - `SPACETIMEDB_DATABASE` — SpacetimeDB module name (default: `remote-agent-registry`)
    /// - `SPACETIMEDB_TOKEN` — auth token (optional)
    /// - All vars consumed by `TaskExecutor::from_env()` for REST fallback
    pub fn from_env() -> Self {
        let rest = TaskExecutor::from_env();
        let agent_id = std::env::var("HEX_AGENT_ID").unwrap_or_else(|_| "unknown".into());
        let stdb = Arc::new(StdbConnection::new(agent_id));
        Self { stdb, rest }
    }

    /// Connect to SpacetimeDB. Call once before the poll loop.
    ///
    /// On failure, logs a warning and continues in REST-only mode — no panic.
    pub async fn initialize(&self) {
        let ws_url = std::env::var("SPACETIMEDB_URL")
            .unwrap_or_else(|_| "ws://localhost:3033".into());
        let database = std::env::var("SPACETIMEDB_DATABASE")
            .unwrap_or_else(|_| "remote-agent-registry".into());
        let token = std::env::var("SPACETIMEDB_TOKEN").ok();

        if let Err(e) = self
            .stdb
            .connect(&ws_url, &database, token.as_deref())
            .await
        {
            tracing::warn!(error = %e, "StdbTaskPoller: StDB connect error, REST fallback");
        }

        if self.stdb.is_connected() {
            tracing::info!(
                "StdbTaskPoller: SpacetimeDB connected — push-based task claiming active"
            );
        } else {
            tracing::info!("StdbTaskPoller: REST polling fallback active");
        }
    }

    /// Claim the next available task. Returns `None` if no task is available yet.
    ///
    /// **StDB path**: checks the in-memory cache for `status=pending` tasks; calls
    /// `assign_task` reducer to claim the first one.
    ///
    /// **REST path**: delegates to `TaskExecutor::poll_task()`.
    pub async fn poll_next(&self) -> Option<ClaimedTask> {
        if self.stdb.is_connected() {
            self.poll_via_stdb().await
        } else {
            self.poll_via_rest().await
        }
    }

    async fn poll_via_stdb(&self) -> Option<ClaimedTask> {
        let tasks = self.stdb.pending_tasks();

        // Iterate through all cached pending tasks so a failed claim on one
        // task (race with another agent) does not stall the loop — we try the
        // next cached task immediately rather than returning None and sleeping.
        for task in tasks {
            match self.stdb.assign_task(&task.task_id).await {
                Ok(()) => {
                    self.stdb.evict(&task.task_id);

                    // Decode the request_json payload, fall back to raw description field
                    let description = serde_json::from_str::<TaskPayload>(&task.request_json)
                        .map(|p| p.description)
                        .unwrap_or_else(|_| task.request_json.clone());

                    tracing::info!(
                        task_id = %task.task_id,
                        "StdbTaskPoller: claimed task via SpacetimeDB"
                    );
                    return Some(ClaimedTask {
                        task_id: task.task_id,
                        description,
                        via_stdb: true,
                    });
                }
                Err(e) => {
                    // Another agent claimed it first — evict from local cache
                    // and try the next task in this same poll cycle.
                    self.stdb.evict(&task.task_id);
                    tracing::warn!(
                        task_id = %task.task_id,
                        error = %e,
                        "StdbTaskPoller: assign_task failed (race?), trying next cached task"
                    );
                }
            }
        }

        // All cached tasks were already claimed; caller will re-fetch from server.
        None
    }

    async fn poll_via_rest(&self) -> Option<ClaimedTask> {
        let task: HexFloTask = self.rest.poll_task().await?;
        Some(ClaimedTask {
            task_id: task.id.clone(),
            description: task.title.clone(),
            via_stdb: false,
        })
    }

    /// Report task completion (or failure).
    ///
    /// StDB path: calls `complete_task` reducer with JSON-encoded result.
    /// REST path: calls `TaskExecutor::report_done`.
    pub async fn report_done(
        &self,
        claimed: &ClaimedTask,
        result: &str,
        success: bool,
    ) -> Result<(), String> {
        if claimed.via_stdb && self.stdb.is_connected() {
            let completion = TaskCompletionBody {
                task_id: claimed.task_id.clone(),
                status: if success { TaskStatus::Completed } else { TaskStatus::Failed },
                result: Some(result.to_string()),
                error: None,
                agent_id: None,
            };
            let result_json = serde_json::to_string(&completion).unwrap_or_default();
            self.stdb
                .complete_task(&claimed.task_id, &result_json)
                .await
        } else {
            self.rest
                .report_done(&claimed.task_id, result, success)
                .await
        }
    }

    /// Also expose the project init helper from `TaskExecutor` for daemon startup.
    pub async fn init_project(&self, project_path: &str) {
        self.rest.init_project(project_path).await;
    }
}

// ── P5.1: TaskPayload contract tests ─────────────────────────────────────────
//
// Verify the JSON schema that the supervisor encodes (ADR-2603300100 P4.1)
// can be decoded by the daemon — no external dependencies required.

#[cfg(test)]
mod tests {
    use super::TaskPayload;

    /// Full payload roundtrip — all fields present.
    #[test]
    fn task_payload_full_roundtrip() {
        let json = serde_json::json!({
            "step_id": "P2.1",
            "description": "Implement StdbInferenceAdapter",
            "model_hint": "openai/gpt-4o-mini",
            "output_dir": "/workspace/hex-agent"
        });
        let p: TaskPayload = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(p.step_id, "P2.1");
        assert_eq!(p.description, "Implement StdbInferenceAdapter");
        assert_eq!(p.model_hint.as_deref(), Some("openai/gpt-4o-mini"));
        assert_eq!(p.output_dir.as_deref(), Some("/workspace/hex-agent"));
    }

    /// Optional fields absent → `None` (not deserialization error).
    #[test]
    fn task_payload_optional_fields_default_to_none() {
        let json = serde_json::json!({
            "step_id": "P0.1",
            "description": "Fix SpacetimeDB module deployment"
        });
        let p: TaskPayload = serde_json::from_value(json).unwrap();
        assert!(p.model_hint.is_none());
        assert!(p.output_dir.is_none());
    }

    /// Supervisor also encodes `role` and `output_dir` as `outputDir`? — verify the
    /// actual field name coming from supervisor JSON matches our serde field names.
    #[test]
    fn supervisor_task_encoding_matches_payload_schema() {
        // Supervisor encodes snake_case keys — must match TaskPayload field names.
        let supervisor_json = serde_json::json!({
            "role": "hex-coder",
            "step_id": "P1.2",
            "description": "StdbTaskPoller subscribe to swarm_task table",
            "output_dir": "/workspace/examples/foo"
        });
        let p: TaskPayload = serde_json::from_value(supervisor_json).unwrap();
        assert_eq!(p.step_id, "P1.2");
        assert_eq!(p.output_dir.as_deref(), Some("/workspace/examples/foo"));
        // `role` is extra and harmlessly ignored by serde
    }

    /// Daemon fallback: when description is not valid JSON, build TaskPayload from raw string.
    #[test]
    fn daemon_fallback_from_raw_description() {
        let raw = "hex-coder: implement foo [iteration 1]";
        let task_id = "abc-123";
        let p = serde_json::from_str::<TaskPayload>(raw).unwrap_or_else(|_| TaskPayload {
            step_id: task_id.to_string(),
            description: raw.to_string(),
            model_hint: None,
            output_dir: None,
            role: None,
        });
        assert_eq!(p.step_id, task_id);
        assert_eq!(p.description, raw);
        assert!(p.model_hint.is_none());
    }
}
