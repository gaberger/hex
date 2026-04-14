//! Brain API routes (ADR-2604102200).
//!
//! GET  /api/brain/status - Service status
//! POST /api/brain/test  - Run a test
//! GET  /api/brain/scores - Get method scores

use axum::{
    extract::{Query, State},
    Json,
};
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Default)]
pub struct BrainStatusQuery {
    /// Optional project ID — filters queue_pending/queue_running to tasks
    /// whose `project_id` field matches. When omitted, all tasks count
    /// (useful for hex-intf's own operator view; harmful as a default for
    /// installed targets which should always scope).
    pub project: Option<String>,
}

use crate::brain_service;
use crate::state::SharedState;

/// Kinds of task the brain queue can carry. Serialized as kebab-case so
/// `RemoteShell` becomes `"remote-shell"` on the wire (ADR-2604141200).
///
/// Payload shape varies by kind:
/// - `HexCommand` — raw `hex <subcommand>` string
/// - `Workplan`   — path to a workplan JSON
/// - `Shell`      — local shell command (sandboxed, rejects `echo FIXME` stubs)
/// - `RemoteShell` — JSON-encoded [`RemoteShellPayload`] `{host, command}`;
///   the agent on `host` polls `/api/brain/queue?kind=remote-shell&host=<host>`
///   and executes against its local whitelist (ADR-2604141200 P3).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum TaskKind {
    HexCommand,
    Workplan,
    Shell,
    RemoteShell,
}

impl TaskKind {
    /// Wire-form identifier as persisted in the `kind` field of brain-task
    /// records. Matches the `serde(rename_all = "kebab-case")` output.
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskKind::HexCommand => "hex-command",
            TaskKind::Workplan => "workplan",
            TaskKind::Shell => "shell",
            TaskKind::RemoteShell => "remote-shell",
        }
    }

    /// Parse a wire-form string back into a variant. Used by queue-list and
    /// the agent poll loop to route by kind.
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "hex-command" => Some(TaskKind::HexCommand),
            "workplan" => Some(TaskKind::Workplan),
            "shell" => Some(TaskKind::Shell),
            "remote-shell" => Some(TaskKind::RemoteShell),
            _ => None,
        }
    }
}

/// Structured payload for a [`TaskKind::RemoteShell`] task. Serialized to
/// JSON and stored in the brain-task's `payload` field so host+command
/// travel together through the queue. The receiving hex-agent matches
/// `host` against its own hostname before executing `command`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteShellPayload {
    pub host: String,
    pub command: String,
}

impl RemoteShellPayload {
    /// Encode as the JSON string used in brain-task `payload`.
    pub fn to_payload_string(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_string())
    }

    /// Best-effort parse of a payload string. Returns `None` when the
    /// payload isn't a remote-shell shape (legacy string payloads, or a
    /// malformed task record).
    pub fn parse(payload: &str) -> Option<Self> {
        serde_json::from_str(payload).ok()
    }
}

#[derive(Serialize)]
pub struct BrainStatus {
    pub service_enabled: bool,
    pub test_model: String,
    pub interval_secs: u64,
    pub last_test: String,
    /// Pending brain tasks waiting for the next daemon tick.
    pub queue_pending: u32,
    /// Tasks the daemon is currently executing (status=in_progress).
    /// Non-zero = "brain is actively working right now" — the signal
    /// operators need to distinguish "stalled with queue" from "draining".
    pub queue_running: u32,
    /// Seconds since last brain_tick event (null if never). Operators watching
    /// the statusline use this to verify brain is actually iterating.
    pub last_tick_secs_ago: Option<u64>,
    /// Most recently started in-progress task (id + kind + payload preview).
    /// `None` when nothing is currently running.
    pub current_task: Option<BrainCurrentTask>,
}

#[derive(Serialize)]
pub struct BrainCurrentTask {
    pub id: String,
    pub kind: String,
    pub payload: String,
}

#[derive(Deserialize)]
pub struct BrainTestRequest {
    pub model: String,
}

#[derive(Serialize)]
pub struct BrainTestResponse {
    pub outcome: String,
    pub reward: f64,
    pub response: String,
}

pub async fn status(
    State(state): State<SharedState>,
    Query(query): Query<BrainStatusQuery>,
) -> Json<BrainStatus> {
    let test_model = std::env::var("HEX_BRAIN_TEST_MODEL")
        .unwrap_or_else(|_| "nemotron-mini".to_string());

    let last_test = state
        .brain_last_test
        .read()
        .await
        .clone()
        .unwrap_or_else(|| "never".to_string());

    // Count brain tasks by status (pending / in_progress). One search, two
    // buckets. Best-effort: if the state port isn't configured, counts = 0.
    let (queue_pending, queue_running, current_task) =
        if let Some(sp) = state.state_port.as_ref() {
            match sp.hexflo_memory_search("brain-task:").await {
                Ok(entries) => {
                    let mut pending = 0u32;
                    let mut running = 0u32;
                    let mut current: Option<BrainCurrentTask> = None;
                    for (_key, value) in &entries {
                        let Ok(task) = serde_json::from_str::<serde_json::Value>(value) else {
                            continue;
                        };
                        // Project scoping: when a project filter is set, skip tasks
                        // whose project_id doesn't match. Tasks enqueued before the
                        // project_id field existed have "" — those are excluded from
                        // filtered views (visible only in unscoped queries).
                        if let Some(want) = &query.project {
                            let got = task.get("project_id").and_then(|v| v.as_str()).unwrap_or("");
                            if got != want {
                                continue;
                            }
                        }
                        match task.get("status").and_then(|s| s.as_str()) {
                            Some("pending") => pending += 1,
                            Some("in_progress") => {
                                running += 1;
                                if current.is_none() {
                                    current = Some(BrainCurrentTask {
                                        id: task.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                                        kind: task.get("kind").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                                        payload: task.get("payload").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                                    });
                                }
                            }
                            _ => {}
                        }
                    }
                    (pending, running, current)
                }
                Err(_) => (0, 0, None),
            }
        } else {
            (0, 0, None)
        };

    Json(BrainStatus {
        service_enabled: true,
        test_model,
        interval_secs: 10,
        last_test,
        queue_pending,
        queue_running,
        last_tick_secs_ago: None, // TODO: read from event_adapter once a brain_tick filter exists
        current_task,
    })
}

pub async fn test(
    State(state): State<SharedState>,
    Json(_req): Json<BrainTestRequest>,
) -> Json<BrainTestResponse> {
    // Run a test cycle synchronously
    let result = match brain_service::run_improvement_cycle(&state).await {
        Ok(outcome) => BrainTestResponse {
            outcome: outcome.outcome,
            reward: outcome.reward,
            response: "test completed".to_string(),
        },
        Err(e) => BrainTestResponse {
            outcome: "error".to_string(),
            reward: -0.5,
            response: e,
        },
    };

    // Record the timestamp regardless of outcome — a failed test is still a
    // test. Operators care "when did we last probe?" not "when did we last
    // get a green result." (errors are visible in the response body itself.)
    *state.brain_last_test.write().await = Some(chrono::Utc::now().to_rfc3339());

    Json(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_kind_serializes_as_kebab_case() {
        // Wire format is the contract — downstream agents (and the
        // `hex brain queue` CLI) grep on these exact strings.
        assert_eq!(TaskKind::HexCommand.as_str(), "hex-command");
        assert_eq!(TaskKind::Workplan.as_str(), "workplan");
        assert_eq!(TaskKind::Shell.as_str(), "shell");
        assert_eq!(TaskKind::RemoteShell.as_str(), "remote-shell");
    }

    #[test]
    fn task_kind_round_trips_through_from_str() {
        for kind in [
            TaskKind::HexCommand,
            TaskKind::Workplan,
            TaskKind::Shell,
            TaskKind::RemoteShell,
        ] {
            assert_eq!(TaskKind::from_str(kind.as_str()), Some(kind));
        }
        assert_eq!(TaskKind::from_str("unknown"), None);
    }

    #[test]
    fn remote_shell_payload_round_trips() {
        let p = RemoteShellPayload {
            host: "bazzite".to_string(),
            command: "nvidia-smi".to_string(),
        };
        let s = p.to_payload_string();
        let back = RemoteShellPayload::parse(&s).expect("parses");
        assert_eq!(back, p);
    }

    #[test]
    fn remote_shell_payload_rejects_non_remote_shell_strings() {
        // Legacy tasks with a plain-string payload (e.g. "hex analyze .")
        // must not accidentally parse as remote-shell.
        assert!(RemoteShellPayload::parse("hex analyze .").is_none());
    }
}