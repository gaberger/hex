use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use std::process::Command;
use crate::state::SharedState;

#[derive(Debug, Serialize, Deserialize)]
pub struct MonitorSnapshot {
    pub timestamp: String,
    pub daemon: DaemonStatus,
    pub queue: QueueStatus,
    pub recent_activity: Vec<TaskSummary>,
    pub recent_commits: Vec<CommitSummary>,
    pub focus: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DaemonStatus {
    pub running: bool,
    pub pid: Option<u32>,
    pub interval_secs: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct QueueStatus {
    pub pending: usize,
    pub running: usize,
    pub current_task: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TaskSummary {
    pub id: String,
    pub kind: String,
    pub status: String,
    pub payload: String,
    pub failure_reason: Option<String>,
    pub duration_s: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CommitSummary {
    pub hash: String,
    pub message: String,
}

pub async fn get_monitor_snapshot(
    State(_state): State<SharedState>,
) -> Json<MonitorSnapshot> {
    let snapshot = MonitorSnapshot {
        timestamp: chrono::Utc::now().to_rfc3339(),
        daemon: get_daemon_status(),
        queue: get_queue_status(),
        recent_activity: get_recent_activity(),
        recent_commits: get_recent_commits(),
        focus: get_focus(),
    };

    Json(snapshot)
}

fn get_daemon_status() -> DaemonStatus {
    let output = Command::new("hex")
        .args(["sched", "daemon-status"])
        .output();

    let Ok(output) = output else {
        return DaemonStatus {
            running: false,
            pid: None,
            interval_secs: None,
        };
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let running = output.status.success() && stdout.contains("running");

    let pid = if running {
        stdout
            .lines()
            .find(|l| l.contains("pid="))
            .and_then(|l| {
                l.split("pid=")
                    .nth(1)
                    .and_then(|s| s.split_whitespace().next())
                    .and_then(|s| s.parse::<u32>().ok())
            })
    } else {
        None
    };

    DaemonStatus {
        running,
        pid,
        interval_secs: None,
    }
}

fn get_queue_status() -> QueueStatus {
    let output = Command::new("hex")
        .args(["sched", "status"])
        .output();

    let Ok(output) = output else {
        return QueueStatus {
            pending: 0,
            running: 0,
            current_task: None,
        };
    };

    let stdout = String::from_utf8_lossy(&output.stdout);

    let pending = stdout
        .lines()
        .find(|l| l.contains("Queue:"))
        .and_then(|l| {
            if l.contains("idle") {
                Some(0)
            } else {
                l.split_whitespace()
                    .find(|s| s.chars().all(|c| c.is_numeric()))
                    .and_then(|s| s.parse().ok())
            }
        })
        .unwrap_or(0);

    let running = if stdout.contains("running") { 1 } else { 0 };

    let current_task = stdout
        .lines()
        .find(|l| l.contains("Current:"))
        .map(|l| l.trim().to_string());

    QueueStatus {
        pending,
        running,
        current_task,
    }
}

fn get_recent_activity() -> Vec<TaskSummary> {
    // Query memory directly for brain-tasks, extract failure details
    let output = Command::new("curl")
        .args([
            "-s",
            "http://localhost:5555/api/hexflo/memory/search?q=brain-task:",
        ])
        .output();

    let Ok(output) = output else {
        return Vec::new();
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = match serde_json::from_str(&stdout) {
        Ok(j) => j,
        Err(_) => return Vec::new(),
    };

    let results = match json.get("results").and_then(|r| r.as_array()) {
        Some(r) => r,
        None => return Vec::new(),
    };

    // Parse tasks, sort by completed_at, take last 5
    let mut tasks: Vec<(String, TaskSummary)> = results
        .iter()
        .filter_map(|entry| {
            let value_str = entry.get("value")?.as_str()?;
            let task_json: serde_json::Value = serde_json::from_str(value_str).ok()?;

            let id = task_json.get("id")?.as_str()?.to_string();
            let kind = task_json.get("kind")?.as_str()?.to_string();
            let status = task_json.get("status")?.as_str()?.to_string();
            let payload = task_json.get("payload")?.as_str()?.to_string();
            let completed_at = task_json.get("completed_at")?.as_str()?.to_string();

            // Extract failure reason for failed/skipped tasks
            let failure_reason = task_json.get("result").and_then(|r| r.as_str()).map(|result_str| {
                // Try to parse result JSON to extract structured data
                if let Ok(result_json) = serde_json::from_str::<serde_json::Value>(result_str) {
                    // First try failures array
                    if let Some(failures) = result_json
                        .get("failures")
                        .and_then(|f| f.as_array())
                        .and_then(|arr| arr.first())
                        .and_then(|f| f.as_str())
                    {
                        return failures.chars().take(150).collect::<String>();
                    }

                    // If no failures, show summary for failed/skipped tasks
                    if status == "failed" {
                        let completed = result_json.get("completed").and_then(|c| c.as_u64()).unwrap_or(0);
                        let failed = result_json.get("failed").and_then(|f| f.as_u64()).unwrap_or(0);
                        let skipped = result_json.get("skipped").and_then(|s| s.as_u64()).unwrap_or(0);
                        return format!("✓{} ✗{} ○{}", completed, failed, skipped);
                    }
                }

                // Fallback: show first line of result
                result_str.lines().next().unwrap_or("").chars().take(80).collect()
            });

            // Extract duration
            let duration_s = task_json.get("result").and_then(|r| r.as_str()).and_then(|result_str| {
                serde_json::from_str::<serde_json::Value>(result_str)
                    .ok()
                    .and_then(|result_json| result_json.get("duration_s").and_then(|d| d.as_u64()))
            });

            Some((
                completed_at,
                TaskSummary {
                    id: id.chars().take(8).collect(),
                    kind,
                    status,
                    payload: payload.chars().take(40).collect(),
                    failure_reason,
                    duration_s,
                },
            ))
        })
        .collect();

    tasks.sort_by(|a, b| b.0.cmp(&a.0)); // Sort descending by completed_at
    tasks.into_iter().take(5).map(|(_, t)| t).collect()
}

fn get_recent_commits() -> Vec<CommitSummary> {
    let output = Command::new("git")
        .args(["log", "--oneline", "--since=1 hour ago", "-5"])
        .output();

    let Ok(output) = output else {
        return Vec::new();
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(2, ' ');
            let hash = parts.next()?.to_string();
            let message = parts.next()?.to_string();
            Some(CommitSummary { hash, message })
        })
        .collect()
}

fn get_focus() -> Option<String> {
    let output = Command::new("hex")
        .args(["focus"])
        .output();

    let Ok(output) = output else {
        return None;
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .find(|l| l.contains("Focus:"))
        .map(|l| l.trim().to_string())
}
