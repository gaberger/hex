//! Sched API routes (ADR-2604102200).
//!
//! GET  /api/sched/status - Service status
//! POST /api/sched/test  - Run a test
//! GET  /api/sched/scores - Get method scores

use axum::{
    extract::{Query, State},
    Json,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Deserialize, Default)]
pub struct SchedStatusQuery {
    /// Optional project ID — filters queue_pending/queue_running to tasks
    /// whose `project_id` field matches. When omitted, all tasks count
    /// (useful for hex-intf's own operator view; harmful as a default for
    /// installed targets which should always scope).
    pub project: Option<String>,
}

use crate::sched_service;
use crate::state::SharedState;

/// Kinds of task the sched queue can carry. Serialized as kebab-case so
/// `RemoteShell` becomes `"remote-shell"` on the wire (ADR-2604141200).
///
/// Payload shape varies by kind:
/// - `HexCommand` — raw `hex <subcommand>` string
/// - `Workplan`   — path to a workplan JSON
/// - `Shell`      — local shell command (sandboxed, rejects `echo FIXME` stubs)
/// - `RemoteShell` — JSON-encoded [`RemoteShellPayload`] `{host, command}`;
///   the agent on `host` polls `/api/sched/queue?kind=remote-shell&host=<host>`
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
    /// Wire-form identifier as persisted in the `kind` field of sched-task
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
/// JSON and stored in the sched-task's `payload` field so host+command
/// travel together through the queue. The receiving hex-agent matches
/// `host` against its own hostname before executing `command`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteShellPayload {
    pub host: String,
    pub command: String,
}

impl RemoteShellPayload {
    /// Encode as the JSON string used in sched-task `payload`.
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
pub struct SchedStatus {
    pub service_enabled: bool,
    pub test_model: String,
    pub interval_secs: u64,
    pub last_test: String,
    /// Pending sched tasks waiting for the next daemon tick.
    pub queue_pending: u32,
    /// Tasks the daemon is currently executing (status=in_progress).
    /// Non-zero = "sched is actively working right now" — the signal
    /// operators need to distinguish "stalled with queue" from "draining".
    pub queue_running: u32,
    /// Seconds since last sched_tick event (null if never). Operators watching
    /// the statusline use this to verify sched is actually iterating.
    pub last_tick_secs_ago: Option<u64>,
    /// Most recently started in-progress task (id + kind + payload preview).
    /// `None` when nothing is currently running.
    pub current_task: Option<SchedCurrentTask>,
}

#[derive(Serialize)]
pub struct SchedCurrentTask {
    pub id: String,
    pub kind: String,
    pub payload: String,
}

#[derive(Deserialize)]
pub struct SchedTestRequest {
    pub model: String,
}

#[derive(Serialize)]
pub struct SchedTestResponse {
    pub outcome: String,
    pub reward: f64,
    pub response: String,
}

pub async fn status(
    State(state): State<SharedState>,
    Query(query): Query<SchedStatusQuery>,
) -> Json<SchedStatus> {
    let test_model = std::env::var("HEX_SCHED_TEST_MODEL")
        .unwrap_or_else(|_| "nemotron-mini".to_string());

    let last_test = state
        .sched_last_test
        .read()
        .await
        .clone()
        .unwrap_or_else(|| "never".to_string());

    // Count sched tasks by status (pending / in_progress). One search, two
    // buckets. Best-effort: if the state port isn't configured, counts = 0.
    let (queue_pending, queue_running, current_task) =
        if let Some(sp) = state.state_port.as_ref() {
            match sp.hexflo_memory_search("brain-task:").await {
                Ok(entries) => {
                    let mut pending = 0u32;
                    let mut running = 0u32;
                    let mut current: Option<SchedCurrentTask> = None;
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
                                    current = Some(SchedCurrentTask {
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

    Json(SchedStatus {
        service_enabled: true,
        test_model,
        interval_secs: 10,
        last_test,
        queue_pending,
        queue_running,
        last_tick_secs_ago: None, // TODO: read from event_adapter once a sched_tick filter exists
        current_task,
    })
}

pub async fn test(
    State(state): State<SharedState>,
    Json(_req): Json<SchedTestRequest>,
) -> Json<SchedTestResponse> {
    // Run a test cycle synchronously
    let result = match sched_service::run_improvement_cycle(&state).await {
        Ok(outcome) => SchedTestResponse {
            outcome: outcome.outcome,
            reward: outcome.reward,
            response: "test completed".to_string(),
        },
        Err(e) => SchedTestResponse {
            outcome: "error".to_string(),
            reward: -0.5,
            response: e,
        },
    };

    // Record the timestamp regardless of outcome — a failed test is still a
    // test. Operators care "when did we last probe?" not "when did we last
    // get a green result." (errors are visible in the response body itself.)
    *state.sched_last_test.write().await = Some(chrono::Utc::now().to_rfc3339());

    Json(result)
}

/// Truncated summary of a single sched-queue task, returned by
/// `GET /api/sched/queue/history` (wp-sched-queue-history P1.2).
///
/// Source of truth is `hexflo_memory` with key prefix `brain-task:` — the same
/// store the existing `status` endpoint and `hex sched` CLI read. Payload and
/// result fields are truncated to keep response bodies bounded and shell-
/// table-friendly. Operators who need the full record can still query
/// `/api/hexflo/memory/brain-task:<id>` directly.
#[derive(Serialize, Clone, Debug)]
pub struct SchedTaskSummary {
    pub id: String,
    pub kind: String,
    pub status: String,
    /// First 80 chars of the task payload (command, workplan path, etc.).
    pub payload_truncated: String,
    /// First 300 chars of the recorded result. Contains the
    /// `no git evidence` marker when the evidence-guard (ADR-2604141400 §1 P1)
    /// flipped a vacuous exit-0 drain to failed — this is the primary signal
    /// operators hunt for in history output.
    pub result_truncated: String,
    /// Task creation timestamp in microseconds since Unix epoch. 0 when the
    /// stored record has an unparseable `created_at` string.
    pub created_at_us: i64,
    /// Completion timestamp in microseconds since Unix epoch. 0 when the task
    /// has not yet completed or the timestamp is unparseable.
    pub completed_at_us: i64,
    /// Workplan timeout in seconds plumbed through the enqueue payload
    /// (ADR-2604142155 P2.1). Used by the daemon's `sweep_stuck_tasks()` to
    /// auto-fail tasks that exceed `timeout_s + 30s` grace. `None` when the
    /// stored record predates P2.1 — sweep falls back to the kind-default
    /// lease window.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_s: Option<u64>,
}

/// Parse an RFC3339 timestamp into microseconds since Unix epoch. Returns 0 on
/// parse failure so the row is still renderable — losing a timestamp is a
/// less-bad failure than hiding an entire failed task from the operator.
fn rfc3339_to_us(s: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.timestamp_micros())
        .unwrap_or(0)
}

/// Project a raw sched-task record (serde_json::Value as stored in
/// hexflo_memory) into the wire-stable `SchedTaskSummary` shape.
fn summarize_task(task: &serde_json::Value) -> SchedTaskSummary {
    let payload = task
        .get("payload")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let result = task
        .get("result")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let created_at = task
        .get("created_at")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let completed_at = task
        .get("completed_at")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    SchedTaskSummary {
        id: task.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        kind: task.get("kind").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        status: task.get("status").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        payload_truncated: payload.chars().take(80).collect(),
        result_truncated: result.chars().take(300).collect(),
        created_at_us: rfc3339_to_us(created_at),
        completed_at_us: if completed_at.is_empty() { 0 } else { rfc3339_to_us(completed_at) },
        timeout_s: task.get("timeout_s").and_then(|v| v.as_u64()),
    }
}

/// GET /api/sched/queue/history[?status=failed][&limit=20]
///
/// Returns a paginated, reverse-chronological list of sched-queue tasks. Primary
/// consumer is `hex sched queue history`, which operators use to verify the
/// evidence-guard (ADR-2604141400 §1 P1) correctly flips silent-drain workplans
/// to `failed`. Without this surface, the guard shipped but was invisible.
///
/// Parameters:
/// - `status` — exact match filter ("pending", "in_progress", "completed",
///   "failed"). Omit to include all statuses.
/// - `limit`  — max rows to return. Clamped to [1, 200]; default 20.
///
/// Sort: newest first by `created_at_us`. Reads from `hexflo_memory` via
/// `hexflo_memory_search("brain-task:")`, so both SQLite and SpacetimeDB-backed
/// state adapters are transparently supported.
pub async fn queue_history(
    State(state): State<SharedState>,
    Query(params): Query<HashMap<String, String>>,
) -> Json<Vec<SchedTaskSummary>> {
    let status_filter = params.get("status").cloned();
    let limit: usize = params
        .get("limit")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(20)
        .clamp(1, 200);

    let Some(sp) = state.state_port.as_ref() else {
        // No state adapter configured — return empty rather than 500. The CLI
        // renders "no history" which is accurate in this case.
        return Json(Vec::new());
    };

    let entries = match sp.hexflo_memory_search("brain-task:").await {
        Ok(e) => e,
        Err(_) => return Json(Vec::new()),
    };

    let mut summaries: Vec<SchedTaskSummary> = entries
        .into_iter()
        .filter_map(|(_key, value)| serde_json::from_str::<serde_json::Value>(&value).ok())
        .filter(|task| {
            status_filter.as_deref().is_none_or(|want| {
                task.get("status").and_then(|s| s.as_str()) == Some(want)
            })
        })
        .map(|task| summarize_task(&task))
        .collect();

    // Newest first — operators debugging a recent drain want the latest rows
    // at the top. Tasks with missing/unparseable `created_at` (us=0) sink to
    // the bottom, which is the correct place for "corrupted record" noise.
    summaries.sort_by(|a, b| b.created_at_us.cmp(&a.created_at_us));
    summaries.truncate(limit);
    Json(summaries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_kind_serializes_as_kebab_case() {
        // Wire format is the contract — downstream agents (and the
        // `hex sched queue` CLI) grep on these exact strings.
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

    // ── wp-sched-queue-history P1.2: projection semantics ─────────────────
    // summarize_task is the contract shared with the CLI renderer. It must:
    //   - truncate payload to 80 and result to 300 (bounded response)
    //   - parse RFC3339 timestamps into microseconds
    //   - treat a null/missing completed_at as 0 (not an error)
    // If these break, the `hex sched queue history` table columns go wrong.

    #[test]
    fn summarize_task_truncates_payload_and_result() {
        let long_payload = "x".repeat(500);
        let long_result = "y".repeat(500);
        let task = serde_json::json!({
            "id": "abc",
            "kind": "shell",
            "status": "failed",
            "payload": long_payload,
            "result": long_result,
            "created_at": "2026-04-14T10:00:00Z",
            "completed_at": "2026-04-14T10:00:05Z",
        });
        let s = summarize_task(&task);
        assert_eq!(s.payload_truncated.chars().count(), 80);
        assert_eq!(s.result_truncated.chars().count(), 300);
    }

    #[test]
    fn summarize_task_handles_missing_completed_at() {
        let task = serde_json::json!({
            "id": "abc",
            "kind": "workplan",
            "status": "pending",
            "payload": "docs/workplans/wp-foo.json",
            "created_at": "2026-04-14T10:00:00Z",
        });
        let s = summarize_task(&task);
        assert_eq!(s.completed_at_us, 0);
        assert!(s.created_at_us > 0, "valid RFC3339 must parse to nonzero us");
    }

    #[test]
    fn summarize_task_surfaces_no_git_evidence_marker() {
        // The guard (ADR-2604141400 §1 P1) writes "no git evidence" into
        // `result` on silent-drain failures. That marker MUST survive the
        // 300-char truncation for short result strings — this is the primary
        // operator signal the history endpoint exists to expose.
        let task = serde_json::json!({
            "id": "abc",
            "kind": "workplan",
            "status": "failed",
            "payload": "docs/workplans/wp-foo.json",
            "result": "exit=0 but no git evidence of work (HEAD unchanged)",
            "created_at": "2026-04-14T10:00:00Z",
            "completed_at": "2026-04-14T10:00:05Z",
        });
        let s = summarize_task(&task);
        assert!(s.result_truncated.contains("no git evidence"));
    }

    #[test]
    fn rfc3339_to_us_returns_zero_on_garbage() {
        assert_eq!(rfc3339_to_us("not a timestamp"), 0);
        assert_eq!(rfc3339_to_us(""), 0);
    }

    // ── ADR-2604142155 P2.1: timeout_s surfaced through history ─────────
    // Operators rely on `hex sched queue history` to verify the daemon's
    // sweeper armed itself with the workplan's declared timeout. If the
    // field is dropped at the projection layer the sweep diagnostics are
    // unverifiable from the operator surface.

    #[test]
    fn summarize_task_surfaces_timeout_s() {
        let task = serde_json::json!({
            "id": "abc",
            "kind": "workplan",
            "status": "in_progress",
            "payload": "docs/workplans/wp-foo.json",
            "created_at": "2026-04-14T10:00:00Z",
            "timeout_s": 1800u64,
        });
        let s = summarize_task(&task);
        assert_eq!(s.timeout_s, Some(1800));
    }

    #[test]
    fn summarize_task_timeout_s_absent_is_none() {
        let task = serde_json::json!({
            "id": "abc",
            "kind": "shell",
            "status": "pending",
            "payload": "echo hi",
            "created_at": "2026-04-14T10:00:00Z",
        });
        let s = summarize_task(&task);
        assert_eq!(s.timeout_s, None);
    }
}