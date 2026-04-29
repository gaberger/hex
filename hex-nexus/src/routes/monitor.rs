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
    let output = Command::new("hex")
        .args(["sched", "queue", "history"])
        .output();

    let Ok(output) = output else {
        return Vec::new();
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut tasks = Vec::new();

    for line in stdout.lines().skip(3).take(5) {
        if line.contains('│') && !line.contains("──") {
            let parts: Vec<&str> = line.split('│').collect();
            if parts.len() >= 5 {
                tasks.push(TaskSummary {
                    id: parts[1].trim().to_string(),
                    kind: parts[2].trim().to_string(),
                    status: parts[3].trim().to_string(),
                    payload: parts[4].trim().chars().take(40).collect(),
                });
            }
        }
    }

    tasks
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
