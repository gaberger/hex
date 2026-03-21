//! Commit-to-task correlation (ADR-044 Phase 3).
//!
//! Scans commit messages for task IDs, agent names, and conventional commit
//! patterns to link git history with HexFlo swarm tasks.

use std::path::Path;

use serde::Serialize;

/// Patterns we look for in commit messages to extract task references:
/// - `task: <uuid>` or `task-id: <uuid>` (git trailer style)
/// - `feat(<task-id>): ...` (conventional commit scope)
/// - `commit <sha>` in HexFlo task result strings
/// - `Co-Authored-By:` for agent attribution
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitTaskLink {
    pub commit_sha: String,
    pub commit_short: String,
    pub commit_message: String,
    pub commit_timestamp: i64,
    pub author: String,
    pub task_ids: Vec<String>,
    pub agent_name: Option<String>,
    pub conventional_scope: Option<String>,
}

/// Scan recent commits for task references.
pub fn find_task_commits(
    root_path: &Path,
    limit: usize,
) -> Result<Vec<CommitTaskLink>, String> {
    let repo = git2::Repository::open(root_path)
        .map_err(|e| format!("Failed to open repo: {}", e))?;

    let mut revwalk = repo.revwalk()
        .map_err(|e| format!("Failed to create revwalk: {}", e))?;
    revwalk.push_head().map_err(|e| format!("Failed to push HEAD: {}", e))?;
    revwalk.set_sorting(git2::Sort::TIME)
        .map_err(|e| format!("Failed to set sorting: {}", e))?;

    let mut results = Vec::new();

    for oid_result in revwalk {
        if results.len() >= limit {
            break;
        }
        let oid = oid_result.map_err(|e| format!("Revwalk error: {}", e))?;
        let commit = repo.find_commit(oid)
            .map_err(|e| format!("Commit not found: {}", e))?;

        let message = commit.message().unwrap_or("").to_string();
        let author = commit.author().name().unwrap_or("").to_string();

        let task_ids = extract_task_ids(&message);
        let agent_name = extract_agent_name(&message);
        let conventional_scope = extract_conventional_scope(&message);

        // Only include commits that have at least one correlation
        if !task_ids.is_empty() || agent_name.is_some() {
            let sha = format!("{}", oid);
            results.push(CommitTaskLink {
                commit_short: sha[..7.min(sha.len())].to_string(),
                commit_sha: sha,
                commit_message: message.lines().next().unwrap_or("").to_string(),
                commit_timestamp: commit.time().seconds(),
                author,
                task_ids,
                agent_name,
                conventional_scope,
            });
        }
    }

    Ok(results)
}

/// Extract task UUIDs from commit message.
/// Looks for patterns like `task: <uuid>`, `task-id: <uuid>`, or UUIDs in general.
fn extract_task_ids(message: &str) -> Vec<String> {
    let mut ids = Vec::new();

    // Pattern 1: `task: <uuid>` or `task-id: <uuid>` trailer
    for line in message.lines() {
        let lower = line.trim().to_lowercase();
        if lower.starts_with("task:") || lower.starts_with("task-id:") {
            let value = line.split(':').skip(1).collect::<Vec<_>>().join(":").trim().to_string();
            if !value.is_empty() {
                ids.push(value);
            }
        }
    }

    // Pattern 2: UUID-like patterns (8-4-4-4-12 hex) — manual scan
    let chars: Vec<char> = message.chars().collect();
    let len = chars.len();
    // UUID is 36 chars: 8-4-4-4-12
    if len >= 36 {
        let mut i = 0;
        while i + 36 <= len {
            let candidate: String = chars[i..i + 36].iter().collect();
            if is_uuid_like(&candidate) && !ids.contains(&candidate) {
                ids.push(candidate);
                i += 36;
            } else {
                i += 1;
            }
        }
    }

    ids
}

/// Check if a 36-char string matches UUID format: 8-4-4-4-12 hex.
fn is_uuid_like(s: &str) -> bool {
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 5 {
        return false;
    }
    let expected_lens = [8, 4, 4, 4, 12];
    for (part, &expected) in parts.iter().zip(expected_lens.iter()) {
        if part.len() != expected || !part.chars().all(|c| c.is_ascii_hexdigit()) {
            return false;
        }
    }
    true
}

/// Extract agent name from `Co-Authored-By:` trailer.
pub fn extract_agent_name(message: &str) -> Option<String> {
    for line in message.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("Co-Authored-By:") || trimmed.starts_with("Co-authored-by:") {
            let value = trimmed.split(':').skip(1).collect::<Vec<_>>().join(":").trim().to_string();
            // Extract name before email: "agent-name <email>"
            if let Some(name) = value.split('<').next() {
                let name = name.trim().to_string();
                if !name.is_empty() {
                    return Some(name);
                }
            }
            return Some(value);
        }
    }
    None
}

/// Extract conventional commit scope: `type(scope): message` → scope
fn extract_conventional_scope(message: &str) -> Option<String> {
    let first_line = message.lines().next().unwrap_or("");
    if let Some(paren_start) = first_line.find('(') {
        if let Some(paren_end) = first_line[paren_start..].find(')') {
            let scope = &first_line[paren_start + 1..paren_start + paren_end];
            if !scope.is_empty() && first_line[paren_start + paren_end..].starts_with("):") {
                return Some(scope.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_task_id_from_trailer() {
        let msg = "feat: add auth\n\ntask: abc-123-def";
        let ids = extract_task_ids(msg);
        assert_eq!(ids, vec!["abc-123-def"]);
    }

    #[test]
    fn extract_uuid_from_message() {
        let msg = "fix(12345678-1234-1234-1234-123456789abc): resolve bug";
        let ids = extract_task_ids(msg);
        assert!(ids.contains(&"12345678-1234-1234-1234-123456789abc".to_string()));
    }

    #[test]
    fn extract_agent_from_coauthor() {
        let msg = "feat: thing\n\nCo-Authored-By: hex-coder <agent@hex>";
        assert_eq!(extract_agent_name(msg), Some("hex-coder".to_string()));
    }

    #[test]
    fn extract_scope() {
        assert_eq!(
            extract_conventional_scope("feat(hex-nexus): add git"),
            Some("hex-nexus".to_string())
        );
        assert_eq!(extract_conventional_scope("fix: no scope"), None);
    }
}
