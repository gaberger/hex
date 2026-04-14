//! Evidence-based verification for workplan task reconciliation.
//!
//! ADR-2604142200: reconcile must require **positive file evidence** before
//! promoting any task to `done`.  Two pure-ish functions:
//!
//! - `collect_evidence` — gathers filesystem, symbol, and git signals
//! - `verify`           — applies the AND rule to decide Promote vs KeepPending

use std::path::{Path, PathBuf};

/// Minimal view of a workplan task — just the fields evidence collection needs.
#[derive(Debug, Clone)]
pub struct WorkplanTask {
    pub id: String,
    pub description: String,
    pub files: Vec<String>,
    pub done_command: String,
    pub created_at: String,
    pub adr_scope: String,
}

/// Collected evidence for a single workplan task.
#[derive(Debug, Clone)]
pub struct TaskEvidence {
    pub files_exist: Vec<(PathBuf, bool)>,
    pub declared_symbols: Vec<String>,
    pub symbol_hits: Vec<(String, PathBuf)>,
    pub matching_commits: Vec<String>,
}

/// Verdict after evaluating evidence.
#[derive(Debug)]
pub enum VerifyResult {
    Promote,
    KeepPending { reason: String },
}

/// Collect all evidence signals for a task against a repo checkout.
///
/// `repo_root` — absolute path to the repository root.
/// `branch`    — the branch to search git log on (pass `""` for current HEAD).
pub fn collect_evidence(task: &WorkplanTask, repo_root: &Path, branch: &str) -> TaskEvidence {
    let files_exist = check_files_exist(&task.files, repo_root);
    let declared_symbols = extract_declared_symbols(&task.description);
    let existing_files: Vec<&Path> = files_exist
        .iter()
        .filter(|(_, exists)| *exists)
        .map(|(p, _)| p.as_path())
        .collect();
    let symbol_hits = find_symbol_hits(&declared_symbols, &existing_files, repo_root);
    let matching_commits = find_matching_commits(&task.files, repo_root, branch);

    TaskEvidence {
        files_exist,
        declared_symbols,
        symbol_hits,
        matching_commits,
    }
}

/// Apply strict AND logic to decide whether a task can be promoted.
///
/// All three conditions must hold:
/// 1. Every declared file exists on disk
/// 2. At least one declared symbol found in the declared files (when symbols exist)
/// 3. At least one commit matches the phase/task-id pattern
pub fn verify(ev: &TaskEvidence) -> VerifyResult {
    // Rule 1: every declared file must exist
    if ev.files_exist.is_empty() {
        return VerifyResult::KeepPending {
            reason: "no files declared for task".into(),
        };
    }
    let missing: Vec<_> = ev
        .files_exist
        .iter()
        .filter(|(_, exists)| !exists)
        .map(|(p, _)| p.display().to_string())
        .collect();
    if !missing.is_empty() {
        return VerifyResult::KeepPending {
            reason: format!("files missing on disk: {}", missing.join(", ")),
        };
    }

    // Rule 2: symbol coverage (only enforced when symbols were extracted)
    if !ev.declared_symbols.is_empty() && ev.symbol_hits.is_empty() {
        return VerifyResult::KeepPending {
            reason: format!(
                "no symbols found in declared files (looked for: {})",
                ev.declared_symbols.join(", ")
            ),
        };
    }

    // Rule 3: git commit evidence
    if ev.matching_commits.is_empty() {
        return VerifyResult::KeepPending {
            reason: "no git commit found matching phase/task-id pattern".into(),
        };
    }

    VerifyResult::Promote
}

// ── Internals ────────────────────────────────────────────────────────

fn check_files_exist(files: &[String], repo_root: &Path) -> Vec<(PathBuf, bool)> {
    files
        .iter()
        .map(|f| {
            let p = repo_root.join(f);
            let exists = p.exists();
            (PathBuf::from(f), exists)
        })
        .collect()
}

/// Extract declared symbols from a task description using the regex:
///   (struct|enum|trait|fn|impl)\s+([A-Z][A-Za-z0-9_]*)
///
/// This captures type-level identifiers that are meaningful evidence
/// of implementation — not prose words or common programming terms.
fn extract_declared_symbols(description: &str) -> Vec<String> {
    let re = regex::Regex::new(r"(?:struct|enum|trait|fn|impl)\s+([A-Z][A-Za-z0-9_]*)").unwrap();
    let mut symbols: Vec<String> = re
        .captures_iter(description)
        .map(|cap| cap[1].to_string())
        .collect();
    symbols.sort();
    symbols.dedup();
    symbols
}

/// Grep for each declared symbol in the existing task files.
/// Returns `(symbol, file_path)` pairs for every hit.
fn find_symbol_hits(
    symbols: &[String],
    existing_files: &[&Path],
    repo_root: &Path,
) -> Vec<(String, PathBuf)> {
    let mut hits = Vec::new();
    if symbols.is_empty() || existing_files.is_empty() {
        return hits;
    }
    for sym in symbols {
        for file in existing_files {
            let abs = repo_root.join(file);
            let output = std::process::Command::new("grep")
                .args(["-l", sym.as_str()])
                .arg(&abs)
                .output();
            if let Ok(out) = output {
                if !out.stdout.is_empty() {
                    hits.push((sym.clone(), file.to_path_buf()));
                }
            }
        }
    }
    hits
}

/// Find commits touching the declared files whose message matches the
/// phase/task-id convention:
///   \(p[0-9]+(\.[0-9]+)*\)  — e.g. (p1.2)
///   P[0-9]+(\.[0-9]+)*      — e.g. P1.2
///   Task-Id:\s*P[0-9]+      — e.g. Task-Id: P1.2
fn find_matching_commits(
    files: &[String],
    repo_root: &Path,
    branch: &str,
) -> Vec<String> {
    if files.is_empty() {
        return Vec::new();
    }

    let mut git_args = vec![
        "log".to_string(),
        "--oneline".to_string(),
    ];
    if !branch.is_empty() {
        git_args.push(branch.to_string());
    }
    git_args.push("--".to_string());
    for f in files {
        git_args.push(f.clone());
    }

    let git_output = std::process::Command::new("git")
        .args(&git_args)
        .current_dir(repo_root)
        .output();

    let log_text = match git_output {
        Ok(out) => String::from_utf8_lossy(&out.stdout).to_string(),
        Err(_) => return Vec::new(),
    };

    let pattern = r"\(p[0-9]+(\.[0-9]+)*\)|P[0-9]+(\.[0-9]+)*|Task-Id:\s*P[0-9]+(\.[0-9]+)*";
    let re = regex::Regex::new(pattern).unwrap();

    log_text
        .lines()
        .filter(|line| re.is_match(line))
        .map(|line| line.to_string())
        .collect()
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_symbols_captures_struct() {
        let syms = extract_declared_symbols("pub struct TaskEvidence with fields");
        assert!(syms.contains(&"TaskEvidence".to_string()));
    }

    #[test]
    fn extract_symbols_captures_enum() {
        let syms = extract_declared_symbols("enum VerifyResult { Promote, KeepPending }");
        assert!(syms.contains(&"VerifyResult".to_string()));
    }

    #[test]
    fn extract_symbols_captures_trait() {
        let syms = extract_declared_symbols("impl trait EvidenceCollector for Foo");
        assert!(syms.contains(&"EvidenceCollector".to_string()));
    }

    #[test]
    fn extract_symbols_ignores_lowercase() {
        let syms = extract_declared_symbols("fn collect_evidence should work");
        assert!(syms.is_empty());
    }

    #[test]
    fn extract_symbols_deduplicates() {
        let syms = extract_declared_symbols("struct TaskEvidence and struct TaskEvidence again");
        assert_eq!(syms.len(), 1);
    }

    #[test]
    fn verify_keeps_pending_when_no_files() {
        let ev = TaskEvidence {
            files_exist: vec![],
            declared_symbols: vec![],
            symbol_hits: vec![],
            matching_commits: vec![],
        };
        assert!(matches!(verify(&ev), VerifyResult::KeepPending { .. }));
    }

    #[test]
    fn verify_keeps_pending_when_files_missing() {
        let ev = TaskEvidence {
            files_exist: vec![(PathBuf::from("missing.rs"), false)],
            declared_symbols: vec![],
            symbol_hits: vec![],
            matching_commits: vec!["abc1234 (p1.1) stuff".into()],
        };
        assert!(matches!(verify(&ev), VerifyResult::KeepPending { .. }));
    }

    #[test]
    fn verify_keeps_pending_when_no_commits() {
        let ev = TaskEvidence {
            files_exist: vec![(PathBuf::from("src/lib.rs"), true)],
            declared_symbols: vec![],
            symbol_hits: vec![],
            matching_commits: vec![],
        };
        assert!(matches!(verify(&ev), VerifyResult::KeepPending { .. }));
    }

    #[test]
    fn verify_keeps_pending_when_symbols_unmatched() {
        let ev = TaskEvidence {
            files_exist: vec![(PathBuf::from("src/lib.rs"), true)],
            declared_symbols: vec!["TaskEvidence".into()],
            symbol_hits: vec![],
            matching_commits: vec!["abc1234 P1.1 stuff".into()],
        };
        assert!(matches!(verify(&ev), VerifyResult::KeepPending { .. }));
    }

    #[test]
    fn verify_promotes_with_full_evidence() {
        let ev = TaskEvidence {
            files_exist: vec![(PathBuf::from("src/lib.rs"), true)],
            declared_symbols: vec!["TaskEvidence".into()],
            symbol_hits: vec![("TaskEvidence".into(), PathBuf::from("src/lib.rs"))],
            matching_commits: vec!["abc1234 P1.1 evidence collector".into()],
        };
        assert!(matches!(verify(&ev), VerifyResult::Promote));
    }

    #[test]
    fn verify_promotes_when_no_symbols_but_files_and_commits_ok() {
        let ev = TaskEvidence {
            files_exist: vec![(PathBuf::from("src/lib.rs"), true)],
            declared_symbols: vec![],
            symbol_hits: vec![],
            matching_commits: vec!["abc1234 (p2.1) refactor".into()],
        };
        assert!(matches!(verify(&ev), VerifyResult::Promote));
    }
}
