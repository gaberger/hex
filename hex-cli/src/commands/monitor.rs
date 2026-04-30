use anyhow::Result;
use chrono::{DateTime, Utc};
use clap::Parser;
use serde::{Deserialize, Serialize};
use std::process::Command;

#[derive(Parser, Debug)]
pub struct MonitorArgs {
    /// Output as JSON
    #[arg(long)]
    json: bool,

    /// Watch mode - refresh every N seconds
    #[arg(long, short = 'w')]
    watch: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MonitorSnapshot {
    pub timestamp: DateTime<Utc>,
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
    #[serde(default)]
    pub failure_reason: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CommitSummary {
    pub hash: String,
    pub message: String,
}

pub async fn run(args: MonitorArgs) -> Result<()> {
    loop {
        let snapshot = collect_snapshot().await?;

        if args.json {
            println!("{}", serde_json::to_string_pretty(&snapshot)?);
        } else {
            print_snapshot(&snapshot);
        }

        if let Some(interval) = args.watch {
            tokio::time::sleep(tokio::time::Duration::from_secs(interval)).await;
            // Clear screen
            print!("\x1B[2J\x1B[1;1H");
        } else {
            break;
        }
    }

    Ok(())
}

async fn collect_snapshot() -> Result<MonitorSnapshot> {
    let daemon = get_daemon_status()?;
    let queue = get_queue_status()?;
    let recent_activity = get_recent_activity()?;
    let recent_commits = get_recent_commits()?;
    let focus = get_focus()?;

    Ok(MonitorSnapshot {
        timestamp: Utc::now(),
        daemon,
        queue,
        recent_activity,
        recent_commits,
        focus,
    })
}

fn get_daemon_status() -> Result<DaemonStatus> {
    let output = Command::new("hex")
        .args(["sched", "daemon-status"])
        .output()?;

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

    Ok(DaemonStatus {
        running,
        pid,
        interval_secs: None, // TODO: parse from status
    })
}

fn get_queue_status() -> Result<QueueStatus> {
    let output = Command::new("hex")
        .args(["sched", "status"])
        .output()?;

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

    Ok(QueueStatus {
        pending,
        running,
        current_task,
    })
}

fn get_recent_activity() -> Result<Vec<TaskSummary>> {
    // Use curl to query sched API directly for recent tasks
    let output = Command::new("curl")
        .args([
            "-s",
            "http://localhost:5555/api/sched/tasks?limit=5",
        ])
        .output()?;

    if !output.status.success() {
        return Ok(Vec::new());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let tasks_json: Vec<serde_json::Value> = match serde_json::from_str(&stdout) {
        Ok(j) => j,
        Err(_) => return Ok(Vec::new()),
    };

    // Parse tasks directly from API response
    let tasks: Vec<TaskSummary> = tasks_json
        .iter()
        .filter_map(|task_json| {

            let id = task_json.get("id")?.as_str()?.to_string();
            let kind = task_json.get("kind")?.as_str()?.to_string();
            let status = task_json.get("status")?.as_str()?.to_string();
            let payload = task_json.get("payload")?.as_str()?.to_string();

            // Extract failure reason from result field for failed tasks
            let failure_reason = if status == "failed" {
                task_json.get("result")
                    .and_then(|r| r.as_str())
                    .and_then(|r_str| {
                        serde_json::from_str::<serde_json::Value>(r_str).ok()
                    })
                    .and_then(|r_json| {
                        r_json.get("failures")
                            .and_then(|f| f.as_array())
                            .and_then(|arr| arr.first())
                            .and_then(|f| f.as_str())
                            .map(|s| s.to_string())
                    })
            } else {
                None
            };

            Some(TaskSummary {
                id,
                kind,
                status,
                payload: payload.chars().take(40).collect(),
                failure_reason,
            })
        })
        .collect();

    Ok(tasks)
}

fn get_recent_commits() -> Result<Vec<CommitSummary>> {
    let output = Command::new("git")
        .args(["log", "--oneline", "--since=1 hour ago", "-5"])
        .output()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let commits: Vec<CommitSummary> = stdout
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(2, ' ');
            let hash = parts.next()?.to_string();
            let message = parts.next()?.to_string();
            Some(CommitSummary { hash, message })
        })
        .collect();

    Ok(commits)
}

fn get_focus() -> Result<Option<String>> {
    let output = Command::new("hex")
        .args(["focus"])
        .output()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let focus = stdout
        .lines()
        .find(|l| l.contains("Focus:"))
        .map(|l| l.trim().to_string());

    Ok(focus)
}

fn print_snapshot(snapshot: &MonitorSnapshot) {
    println!("⬡ hex Monitor — {}", snapshot.timestamp.format("%Y-%m-%d %H:%M:%S"));
    println!();

    // Daemon
    println!("━━━ Daemon ━━━");
    if snapshot.daemon.running {
        print!("  ✓ running");
        if let Some(pid) = snapshot.daemon.pid {
            print!(" (PID {})", pid);
        }
        println!();
    } else {
        println!("  ✗ not running");
    }
    println!();

    // Queue
    println!("━━━ Queue ━━━");
    if snapshot.queue.running > 0 {
        println!("  ▶ {} running", snapshot.queue.running);
        if let Some(ref task) = snapshot.queue.current_task {
            println!("    {}", task);
        }
    }
    if snapshot.queue.pending > 0 {
        println!("  ⤵ {} pending", snapshot.queue.pending);
    }
    if snapshot.queue.running == 0 && snapshot.queue.pending == 0 {
        println!("  ○ idle");
    }
    println!();

    // Recent activity
    println!("━━━ Recent Activity ━━━");
    if snapshot.recent_activity.is_empty() {
        println!("  (none)");
    } else {
        for task in &snapshot.recent_activity {
            let id_short = if task.id.len() >= 8 {
                &task.id[..8]
            } else {
                &task.id
            };

            let payload_display = if task.status == "failed" {
                if let Some(reason) = &task.failure_reason {
                    let truncated = if reason.len() > 60 {
                        format!("{}...", &reason[..60])
                    } else {
                        reason.clone()
                    };
                    format!("❌ {}", truncated)
                } else {
                    task.payload.clone()
                }
            } else {
                task.payload.clone()
            };

            println!("  {} │ {:10} │ {:10} │ {}",
                id_short,
                task.kind,
                task.status,
                payload_display
            );
        }
    }
    println!();

    // Recent commits
    println!("━━━ Recent Commits ━━━");
    if snapshot.recent_commits.is_empty() {
        println!("  (none in last hour)");
    } else {
        for commit in &snapshot.recent_commits {
            println!("  {} {}", commit.hash, commit.message);
        }
    }
    println!();

    // Focus
    if let Some(ref focus) = snapshot.focus {
        println!("━━━ Focus ━━━");
        println!("  {}", focus);
        println!();
    }

    println!("Last updated: {}", snapshot.timestamp.format("%H:%M:%S"));
}
