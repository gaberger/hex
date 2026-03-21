//! Violation-to-commit blame (ADR-044 Phase 3).
//!
//! For each architecture violation from `hex analyze`, finds which commit
//! introduced the offending import line using `git blame`.

use std::path::Path;

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViolationBlame {
    pub file: String,
    pub line: usize,
    pub violation_message: String,
    pub blame_sha: String,
    pub blame_short: String,
    pub blame_author: String,
    pub blame_timestamp: i64,
    pub blame_summary: String,
}

/// Input: a violation with file path and line number.
#[derive(Debug, Clone)]
pub struct ViolationInput {
    pub file: String,
    pub line: usize,
    pub message: String,
}

/// Blame a single file:line to find the commit that introduced it.
pub fn blame_line(
    root_path: &Path,
    file_path: &str,
    line: usize,
) -> Result<Option<BlameResult>, String> {
    let repo = git2::Repository::open(root_path)
        .map_err(|e| format!("Failed to open repo: {}", e))?;

    let mut blame_opts = git2::BlameOptions::new();
    let blame = repo
        .blame_file(Path::new(file_path), Some(&mut blame_opts))
        .map_err(|e| format!("Blame failed for {}: {}", file_path, e))?;

    // git blame lines are 1-indexed
    if line == 0 {
        return Ok(None);
    }

    let hunk = match blame.get_line(line) {
        Some(h) => h,
        None => return Ok(None),
    };

    let oid = hunk.final_commit_id();
    let sha = format!("{}", oid);

    // Look up commit details
    let (author, timestamp, summary) = match repo.find_commit(oid) {
        Ok(commit) => (
            commit.author().name().unwrap_or("").to_string(),
            commit.time().seconds(),
            commit.summary().unwrap_or("").to_string(),
        ),
        Err(_) => (String::new(), 0, String::new()),
    };

    Ok(Some(BlameResult {
        sha: sha.clone(),
        short_sha: sha[..7.min(sha.len())].to_string(),
        author,
        timestamp,
        summary,
    }))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlameResult {
    pub sha: String,
    pub short_sha: String,
    pub author: String,
    pub timestamp: i64,
    pub summary: String,
}

/// Blame multiple violations in batch. Returns results for violations
/// where blame could be resolved.
pub fn blame_violations(
    root_path: &Path,
    violations: &[ViolationInput],
) -> Vec<ViolationBlame> {
    let mut results = Vec::new();

    for v in violations {
        match blame_line(root_path, &v.file, v.line) {
            Ok(Some(blame)) => {
                results.push(ViolationBlame {
                    file: v.file.clone(),
                    line: v.line,
                    violation_message: v.message.clone(),
                    blame_sha: blame.sha,
                    blame_short: blame.short_sha,
                    blame_author: blame.author,
                    blame_timestamp: blame.timestamp,
                    blame_summary: blame.summary,
                });
            }
            _ => {
                // Skip violations where blame fails (e.g., uncommitted code)
            }
        }
    }

    results
}
