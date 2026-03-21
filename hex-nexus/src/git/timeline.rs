//! Unified activity timeline (ADR-044 Phase 3).
//!
//! Merges git commit history with HexFlo task events into a single
//! chronologically-sorted timeline for the project detail view.

use std::path::Path;

use serde::Serialize;

use crate::ports::state::SwarmTaskInfo;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineEntry {
    pub timestamp: i64,
    pub kind: TimelineKind,
    pub title: String,
    pub detail: String,
    pub sha: Option<String>,
    pub task_id: Option<String>,
    pub agent: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TimelineKind {
    Commit,
    TaskCreated,
    TaskCompleted,
    WorktreeCreated,
    WorktreeRemoved,
}

/// Build a unified timeline from git log + HexFlo tasks.
pub fn build_timeline(
    root_path: &Path,
    tasks: &[SwarmTaskInfo],
    limit: usize,
) -> Result<Vec<TimelineEntry>, String> {
    let mut entries = Vec::new();

    // 1. Git commits
    let log = super::log::get_log(root_path, None, None, limit)?;
    for commit in &log.commits {
        let agent = super::correlation::extract_agent_name(&commit.message);
        entries.push(TimelineEntry {
            timestamp: commit.timestamp,
            kind: TimelineKind::Commit,
            title: commit.message.lines().next().unwrap_or("").to_string(),
            detail: format!("{} by {}", commit.short_sha, commit.author_name),
            sha: Some(commit.short_sha.clone()),
            task_id: None,
            agent,
        });
    }

    // 2. HexFlo task events
    for task in tasks {
        // Task creation
        if let Ok(created_at) = chrono::DateTime::parse_from_rfc3339(&task.created_at) {
            entries.push(TimelineEntry {
                timestamp: created_at.timestamp(),
                kind: TimelineKind::TaskCreated,
                title: format!("Task created: {}", task.title),
                detail: format!("Status: {} | Agent: {}", task.status, if task.agent_id.is_empty() { "unassigned" } else { &task.agent_id }),
                sha: None,
                task_id: Some(task.id.clone()),
                agent: if task.agent_id.is_empty() { None } else { Some(task.agent_id.clone()) },
            });
        }

        // Task completion
        if task.status == "completed" && !task.completed_at.is_empty() {
            if let Ok(completed_at) = chrono::DateTime::parse_from_rfc3339(&task.completed_at) {
                // Extract commit hash from result if present
                let sha = if task.result.contains("commit ") {
                    task.result
                        .split("commit ")
                        .nth(1)
                        .map(|s| s.split_whitespace().next().unwrap_or("").to_string())
                } else {
                    None
                };

                entries.push(TimelineEntry {
                    timestamp: completed_at.timestamp(),
                    kind: TimelineKind::TaskCompleted,
                    title: format!("Task completed: {}", task.title),
                    detail: task.result.clone(),
                    sha,
                    task_id: Some(task.id.clone()),
                    agent: if task.agent_id.is_empty() { None } else { Some(task.agent_id.clone()) },
                });
            }
        }
    }

    // Sort by timestamp descending (most recent first)
    entries.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

    // Truncate to limit
    entries.truncate(limit);

    Ok(entries)
}
