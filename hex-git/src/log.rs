//! Git log: paginated commit history.

use std::path::Path;

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitInfo {
    pub sha: String,
    pub short_sha: String,
    pub message: String,
    pub author_name: String,
    pub author_email: String,
    pub timestamp: i64,
    pub parent_count: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogResult {
    pub commits: Vec<CommitInfo>,
    pub has_more: bool,
    pub next_cursor: Option<String>,
}

pub fn get_log(
    root_path: &Path,
    branch: Option<&str>,
    cursor: Option<&str>,
    limit: usize,
) -> Result<LogResult, String> {
    let repo = git2::Repository::open(root_path)
        .map_err(|e| format!("Failed to open repo: {}", e))?;

    let mut revwalk = repo
        .revwalk()
        .map_err(|e| format!("Failed to create revwalk: {}", e))?;

    // Start from cursor SHA or branch head
    if let Some(sha) = cursor {
        let oid = git2::Oid::from_str(sha)
            .map_err(|e| format!("Invalid cursor SHA: {}", e))?;
        revwalk.push(oid).map_err(|e| format!("Failed to push cursor: {}", e))?;
    } else if let Some(b) = branch {
        let refname = format!("refs/heads/{}", b);
        let reference = repo
            .find_reference(&refname)
            .map_err(|e| format!("Branch '{}' not found: {}", b, e))?;
        let oid = reference
            .target()
            .ok_or_else(|| format!("Branch '{}' has no target", b))?;
        revwalk.push(oid).map_err(|e| format!("Failed to push ref: {}", e))?;
    } else {
        // Default: HEAD
        revwalk.push_head().map_err(|e| format!("Failed to push HEAD: {}", e))?;
    }

    revwalk.set_sorting(git2::Sort::TIME)
        .map_err(|e| format!("Failed to set sorting: {}", e))?;

    // Fetch limit+1 to know if there are more
    let fetch_count = limit + 1;
    let mut commits = Vec::with_capacity(limit);
    let mut count = 0usize;

    for oid_result in revwalk {
        if count >= fetch_count {
            break;
        }
        let oid = oid_result.map_err(|e| format!("Revwalk error: {}", e))?;
        let commit = repo
            .find_commit(oid)
            .map_err(|e| format!("Failed to find commit {}: {}", oid, e))?;

        let sha = format!("{}", oid);
        commits.push(CommitInfo {
            short_sha: sha[..7.min(sha.len())].to_string(),
            sha,
            message: commit.message().unwrap_or("").to_string(),
            author_name: commit.author().name().unwrap_or("").to_string(),
            author_email: commit.author().email().unwrap_or("").to_string(),
            timestamp: commit.time().seconds(),
            parent_count: commit.parent_count(),
        });
        count += 1;
    }

    let has_more = commits.len() > limit;
    if has_more {
        commits.truncate(limit);
    }

    let next_cursor = if has_more {
        commits.last().map(|c| c.sha.clone())
    } else {
        None
    };

    Ok(LogResult {
        commits,
        has_more,
        next_cursor,
    })
}
