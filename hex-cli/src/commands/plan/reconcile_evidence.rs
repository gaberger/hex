//! Evidence-based verification for workplan task reconciliation.
//!
//! ADR-2026-04-14-2201: reconcile must require **positive file evidence** before
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
    collect_evidence_strict(task, repo_root, branch, None)
}

/// ADR-2026-04-27-0800 P0.2: workplan-aware variant.
///
/// When `require_workplan_id` is `Some(id)`, a commit only counts as evidence
/// if its message references that workplan id in addition to a task pattern.
/// This closes the cross-workplan false-match (`(p0.2)` from a different
/// workplan satisfying this workplan's P0.2).
pub fn collect_evidence_strict(
    task: &WorkplanTask,
    repo_root: &Path,
    branch: &str,
    require_workplan_id: Option<&str>,
) -> TaskEvidence {
    let files_exist = check_files_exist(&task.files, repo_root);
    let declared_symbols = extract_declared_symbols(&task.description);
    let existing_files: Vec<&Path> = files_exist
        .iter()
        .filter(|(_, exists)| *exists)
        .map(|(p, _)| p.as_path())
        .collect();
    let symbol_hits = find_symbol_hits(&declared_symbols, &existing_files, repo_root);
    let matching_commits = find_matching_commits_scoped(
        &task.id,
        &task.files,
        repo_root,
        branch,
        require_workplan_id,
    );

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
    find_matching_commits_scoped("", files, repo_root, branch, None)
}

/// Workplan-scoped variant. When `require_workplan_id` is `Some(id)`, the
/// commit body (we use `--pretty=format:%H %s%n%b`) must reference that id;
/// otherwise we accept any commit matching the task-id convention. When
/// `task_id` is non-empty, commits are required to match THIS task's id —
/// no more "any P*.* in any commit" false matches across tasks.
fn find_matching_commits_scoped(
    task_id: &str,
    files: &[String],
    repo_root: &Path,
    branch: &str,
    require_workplan_id: Option<&str>,
) -> Vec<String> {
    if files.is_empty() {
        return Vec::new();
    }

    // Use full subject+body so we can match Task-Id: footers and workplan refs.
    let mut git_args = vec![
        "log".to_string(),
        "--pretty=format:%H %s%n%b%n--END--".to_string(),
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

    let task_pattern = if !task_id.is_empty() {
        // Match THIS task only: (p1.2), P1.2, or Task-Id: P1.2 — exact id.
        let escaped = regex::escape(task_id);
        let escaped_lower = regex::escape(&task_id.to_lowercase());
        format!(
            r"\({}\)|(?:^|[\s,;:.\-]){}(?:[\s,;:.\-]|$)|Task-Id:\s*{}",
            escaped_lower, escaped, escaped
        )
    } else {
        r"\(p[0-9]+(\.[0-9]+)*\)|P[0-9]+(\.[0-9]+)*|Task-Id:\s*P[0-9]+(\.[0-9]+)*".to_string()
    };
    let task_re = match regex::Regex::new(&task_pattern) {
        Ok(re) => re,
        Err(_) => return Vec::new(),
    };
    let workplan_re = require_workplan_id.and_then(|id| {
        let escaped = regex::escape(id);
        regex::Regex::new(&escaped).ok()
    });

    let mut hits = Vec::new();
    for entry in log_text.split("--END--") {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        if !task_re.is_match(entry) {
            continue;
        }
        if let Some(ref wp_re) = workplan_re {
            if !wp_re.is_match(entry) {
                continue;
            }
        }
        // Keep only the first line (subject) for the report column.
        let first = entry.lines().next().unwrap_or("").to_string();
        hits.push(first);
    }
    hits
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

    /// ADR-2026-04-27-0800 P0.2 regression: a fresh git repo with one commit whose
    /// subject contains `(p0.2)` from an UNRELATED workplan must not satisfy
    /// this workplan's P0.2. Reproduces the 2026-04-27 false-match.
    #[test]
    fn strict_mode_rejects_cross_workplan_task_id_match() {
        let tmp = std::env::temp_dir().join(format!("hex-recon-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let run = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(&tmp)
                .output()
                .expect("git failed");
        };
        run(&["init", "-q"]);
        run(&["config", "user.email", "t@t"]);
        run(&["config", "user.name", "t"]);
        std::fs::write(tmp.join("ci.rs"), "// step 1\n").unwrap();
        run(&["add", "ci.rs"]);
        // Commit subject mentions (p0.2) but is from an unrelated workplan.
        run(&["commit", "-q", "-m", "feat(p0.2): unrelated TLC daemon work"]);
        std::fs::write(tmp.join("ci.rs"), "// step 2\n").unwrap();
        run(&["add", "ci.rs"]);
        run(&["commit", "-q", "-m", "another change"]);

        let task = WorkplanTask {
            id: "P0.2".into(),
            description: "tighten reconcile evidence rule".into(),
            files: vec!["ci.rs".into()],
            done_command: String::new(),
            created_at: "2026-04-27T07:55:00Z".into(),
            adr_scope: "ADR-2026-04-27-0800".into(),
        };

        // Loose mode: cross-workplan match incorrectly counts as evidence.
        let loose = collect_evidence_strict(&task, &tmp, "", None);
        assert!(
            !loose.matching_commits.is_empty(),
            "loose mode is expected to false-match (the bug we're closing)"
        );

        // Strict mode: same task with workplan_id requirement → no match.
        let strict = collect_evidence_strict(&task, &tmp, "", Some("wp-ADR-doctor-self-fix"));
        assert!(
            strict.matching_commits.is_empty(),
            "strict mode must reject commits that don't reference the workplan id; got {:?}",
            strict.matching_commits
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn strict_mode_accepts_commit_referencing_workplan_id() {
        let tmp = std::env::temp_dir().join(format!("hex-recon-ok-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let run = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(&tmp)
                .output()
                .expect("git failed");
        };
        run(&["init", "-q"]);
        run(&["config", "user.email", "t@t"]);
        run(&["config", "user.name", "t"]);
        std::fs::write(tmp.join("ci.rs"), "// real impl\n").unwrap();
        run(&["add", "ci.rs"]);
        // Subject names BOTH the task id and the workplan id.
        run(&[
            "commit",
            "-q",
            "-m",
            "feat(p0.2): wp-ADR-doctor-self-fix — reconcile evidence rule",
        ]);

        let task = WorkplanTask {
            id: "P0.2".into(),
            description: "tighten reconcile evidence rule".into(),
            files: vec!["ci.rs".into()],
            done_command: String::new(),
            created_at: "2026-04-27T07:55:00Z".into(),
            adr_scope: "ADR-2026-04-27-0800".into(),
        };
        let strict = collect_evidence_strict(&task, &tmp, "", Some("wp-ADR-doctor-self-fix"));
        assert!(
            !strict.matching_commits.is_empty(),
            "strict mode should accept commits that reference the workplan id"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
