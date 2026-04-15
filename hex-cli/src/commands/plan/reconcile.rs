//! Workplan reconciliation with evidence-gated mutation.
//!
//! ADR-2604142200 requires **positive file evidence** before marking tasks done.
//! The `mutate` parameter (default: `false`) controls whether reconcile writes
//! changes back to workplan JSON. Callers must opt in to mutation explicitly.
//!
//! ## Call-site audit
//!
//! | Caller | Location | Intent | mutate |
//! |--------|----------|--------|--------|
//! | `hex plan reconcile --update` | plan/mod.rs CLI dispatch | Explicit operator | `true` (via `--update`) |
//! | `hex hey "reconcile"` | hey.rs NL classifier | Explicit operator | `true` (via `--update`) |
//! | `autofix_workplan()` | sched.rs (validate) | Passive daemon | `false` — report only |
//! | `autofix_workplan()` | hook.rs (SessionStart) | Passive hook | `false` — report only |
//!
//! Passive callers (sched, hook) use `autofix_workplan()` which is a separate
//! code path that predates evidence verification. Those callers should migrate
//! to `reconcile::run(mutate=false)` to get evidence checks without mutation.

use colored::Colorize;
use tabled::Tabled;

use crate::fmt::{HexTable, truncate};
use super::{reconcile_evidence, schema_validate, Workplan};

#[derive(Tabled)]
struct ReconcileRow {
    #[tabled(rename = "Step")]
    id: String,
    #[tabled(rename = "")]
    icon: String,
    #[tabled(rename = "Result")]
    result: String,
    #[tabled(rename = "Git Evidence")]
    git_evidence: String,
    #[tabled(rename = "Done condition (excerpt)")]
    condition: String,
}

struct TaskInfo {
    id: String,
    condition: String,
    files: Vec<String>,
    done_command: String,
    current_status: String,
}

/// Run reconciliation on a single workplan.
///
/// * `mutate` — when `false` (default), only reports; when `true`, writes
///   confirmed-done / demoted statuses back to the workplan JSON.
/// * `audit` — re-verify tasks already marked done; demotes false positives.
/// * `dry_run` — explicit read-only mode; suppresses the "re-run with --update" hint.
/// * `why_task` — if set, show full evidence detail for this single task only.
/// * `force_task` — if set, force-promote this task regardless of evidence.
pub(crate) async fn run(
    file: &str,
    mutate: bool,
    audit: bool,
    dry_run: bool,
    why_task: Option<&str>,
    force_task: Option<&str>,
) -> anyhow::Result<()> {
    if dry_run && mutate {
        anyhow::bail!("--dry-run and --update are mutually exclusive");
    }
    if let Some(id) = force_task {
        if !mutate {
            anyhow::bail!("--force requires --update (mutation must be explicit)");
        }
        return run_force(file, id).await;
    }

    let path = resolve_path(file);
    let content = std::fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("Cannot read {}: {}", path.display(), e))?;

    let mut raw: serde_json::Value = serde_json::from_str(&content)?;
    let workplan: Workplan = serde_json::from_str(&content)?;

    if let Err(violations) = schema_validate::validate_workplan_evidence(&workplan) {
        eprintln!("{}", schema_validate::format_violations(&violations));
        anyhow::bail!(
            "refusing to reconcile {}: schema validation failed ({} violations)",
            path.display(),
            violations.len()
        );
    }

    let tasks_to_check = extract_tasks(&workplan);

    if let Some(task_id) = why_task {
        return run_why(&workplan, &tasks_to_check, task_id);
    }

    let effective_mutate = mutate && !dry_run;

    println!("{} Reconciling: {}", "\u{2b21}".cyan(), workplan.display_title());
    if dry_run {
        println!("  {} --dry-run: no changes will be written", "\u{2139}".dimmed());
    } else if !effective_mutate {
        println!("  {} dry-run (pass --update to write changes)", "\u{2139}".dimmed());
    }
    if !workplan.created_at.is_empty() {
        println!("  Created: {} (git evidence baseline)", workplan.created_at);
    }
    println!();

    let mut rows: Vec<ReconcileRow> = Vec::new();
    let mut step_results: Vec<(String, bool)> = Vec::new();
    let mut evidence_map: std::collections::HashMap<String, reconcile_evidence::TaskEvidence> =
        std::collections::HashMap::new();
    let repo_root = std::env::current_dir().unwrap_or_default();
    let mut demoted_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

    for task in &tasks_to_check {
        let was_done = task.current_status == "done" || task.current_status == "completed";
        if was_done && !audit {
            rows.push(ReconcileRow {
                id: task.id.clone(),
                icon: "\u{2705}".to_string(),
                result: "already done".green().to_string(),
                git_evidence: String::new(),
                condition: truncate(&task.condition, 50),
            });
            step_results.push((task.id.clone(), true));
            continue;
        }

        let git_label = super::git_evidence_label(&task.files, &workplan.created_at);

        let wp_task = reconcile_evidence::WorkplanTask {
            id: task.id.clone(),
            description: task.condition.clone(),
            files: task.files.clone(),
            done_command: task.done_command.clone(),
            created_at: workplan.created_at.clone(),
            adr_scope: workplan.adr.clone(),
        };
        let evidence = reconcile_evidence::collect_evidence(&wp_task, &repo_root, "");
        let verdict = reconcile_evidence::verify(&evidence);

        let (is_done, icon, result_text) = match &verdict {
            reconcile_evidence::VerifyResult::Promote => {
                evidence_map.insert(task.id.clone(), evidence);
                (
                    true,
                    "\u{2705}".to_string(),
                    "evidence-confirmed".green().to_string(),
                )
            }
            reconcile_evidence::VerifyResult::KeepPending { reason } => {
                if was_done {
                    eprintln!(
                        "  {} {}: demoting (was done) — {}",
                        "\u{26a0}".red(),
                        task.id,
                        reason
                    );
                    demoted_ids.insert(task.id.clone());
                    (
                        false,
                        "\u{26a0}".to_string(),
                        "demoted (no evidence)".red().to_string(),
                    )
                } else {
                    eprintln!(
                        "  {} {}: kept pending — {}",
                        "\u{25cb}".dimmed(),
                        task.id,
                        reason
                    );
                    (
                        false,
                        "\u{274c}".to_string(),
                        "needs work".yellow().to_string(),
                    )
                }
            }
        };

        step_results.push((task.id.clone(), is_done));

        rows.push(ReconcileRow {
            id: task.id.clone(),
            icon,
            result: result_text,
            git_evidence: git_label,
            condition: truncate(&task.condition, 50),
        });
    }

    println!("{}", HexTable::render(&rows));
    println!();

    let done_count = step_results.iter().filter(|(_, d)| *d).count();
    println!("  {}/{} steps confirmed done", done_count, step_results.len());

    let reconciled = step_results
        .iter()
        .filter(|(_, d)| *d)
        .count()
        .saturating_sub(
            tasks_to_check
                .iter()
                .filter(|t| t.current_status == "done" || t.current_status == "completed")
                .count(),
        );

    if reconciled > 0 {
        println!(
            "  Reconciled {} task(s) from todo {} done based on git evidence",
            reconciled,
            "\u{2192}".cyan()
        );
    }

    if dry_run {
        if reconciled > 0 || !demoted_ids.is_empty() {
            println!(
                "  {} --dry-run: {} promotion(s), {} demotion(s) would apply",
                "\u{2139}".dimmed(),
                reconciled,
                demoted_ids.len()
            );
        }
        return Ok(());
    }

    if effective_mutate && (done_count > 0 || !demoted_ids.is_empty()) {
        mutate_workplan(&mut raw, &step_results, &evidence_map, &demoted_ids, &path)?;
    } else if effective_mutate {
        println!(
            "  {} No changes — no steps confirmed done",
            "\u{26a0}".yellow()
        );
    } else if reconciled > 0 || !demoted_ids.is_empty() {
        println!(
            "  {} Re-run with --update to persist changes",
            "\u{2139}".dimmed()
        );
    }

    Ok(())
}

/// `--why <task-id>`: Print full evidence breakdown for a single task.
fn run_why(workplan: &Workplan, tasks: &[TaskInfo], task_id: &str) -> anyhow::Result<()> {
    let task = tasks
        .iter()
        .find(|t| t.id == task_id)
        .ok_or_else(|| anyhow::anyhow!("task '{}' not found in workplan", task_id))?;

    let repo_root = std::env::current_dir().unwrap_or_default();
    let wp_task = reconcile_evidence::WorkplanTask {
        id: task.id.clone(),
        description: task.condition.clone(),
        files: task.files.clone(),
        done_command: task.done_command.clone(),
        created_at: workplan.created_at.clone(),
        adr_scope: workplan.adr.clone(),
    };

    let evidence = reconcile_evidence::collect_evidence(&wp_task, &repo_root, "");
    let verdict = reconcile_evidence::verify(&evidence);

    println!(
        "{} Evidence report for task: {}",
        "\u{2b21}".cyan(),
        task_id.bold()
    );
    println!("  Status: {}", task.current_status);
    println!("  Condition: {}", task.condition);
    println!();

    // Files
    println!("  {} Files declared: {}", "\u{1f4c1}".dimmed(), task.files.len());
    for (path, exists) in &evidence.files_exist {
        let (icon, label) = if *exists {
            ("\u{2705}", "found".green())
        } else {
            ("\u{274c}", "MISSING".red())
        };
        println!("    {} {} — {}", icon, path.display(), label);
    }
    println!();

    // Symbols
    println!(
        "  {} Symbols extracted: {}",
        "\u{1f50d}".dimmed(),
        if evidence.declared_symbols.is_empty() {
            "(none)".dimmed().to_string()
        } else {
            evidence.declared_symbols.join(", ")
        }
    );
    if !evidence.declared_symbols.is_empty() {
        for sym in &evidence.declared_symbols {
            let hit = evidence
                .symbol_hits
                .iter()
                .find(|(s, _)| s == sym);
            match hit {
                Some((_, file)) => println!(
                    "    {} {} — found in {}",
                    "\u{2705}",
                    sym,
                    file.display()
                ),
                None => println!("    {} {} — {}", "\u{274c}", sym, "not found".red()),
            }
        }
    }
    println!();

    // Commits
    println!(
        "  {} Matching commits: {}",
        "\u{1f4dd}".dimmed(),
        evidence.matching_commits.len()
    );
    if evidence.matching_commits.is_empty() {
        println!("    {}", "(no commits matching phase/task-id pattern)".dimmed());
    } else {
        for commit in &evidence.matching_commits {
            println!("    {}", commit);
        }
    }
    println!();

    // Verdict
    match &verdict {
        reconcile_evidence::VerifyResult::Promote => {
            println!(
                "  {} Verdict: {} — all evidence rules satisfied",
                "\u{2705}",
                "PROMOTE".green().bold()
            );
        }
        reconcile_evidence::VerifyResult::KeepPending { reason } => {
            println!(
                "  {} Verdict: {} — {}",
                "\u{274c}",
                "KEEP PENDING".red().bold(),
                reason
            );
        }
    }

    Ok(())
}

/// `--force <task-id>`: Promote a task regardless of evidence, with audit trail.
async fn run_force(file: &str, task_id: &str) -> anyhow::Result<()> {
    let path = resolve_path(file);
    let content = std::fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("Cannot read {}: {}", path.display(), e))?;

    let mut raw: serde_json::Value = serde_json::from_str(&content)?;
    let workplan: Workplan = serde_json::from_str(&content)?;

    let tasks = extract_tasks(&workplan);
    if !tasks.iter().any(|t| t.id == task_id) {
        anyhow::bail!("task '{}' not found in workplan", task_id);
    }

    let forced_by = whoami();
    let timestamp = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

    let force_evidence = serde_json::json!({
        "forced": true,
        "forced_by": forced_by,
        "forced_at": timestamp,
        "reason": "operator override via --force",
    });

    let mut updated = false;

    if let Some(steps) = raw.get_mut("steps").and_then(|v| v.as_array_mut()) {
        for step in steps.iter_mut() {
            if step.get("id").and_then(|v| v.as_str()) == Some(task_id) {
                step["status"] = serde_json::json!("done");
                step["evidence"] = force_evidence.clone();
                updated = true;
            }
        }
    }

    if let Some(phases) = raw.get_mut("phases").and_then(|v| v.as_array_mut()) {
        for phase in phases.iter_mut() {
            if let Some(tasks) = phase.get_mut("tasks").and_then(|v| v.as_array_mut()) {
                for task in tasks.iter_mut() {
                    if task.get("id").and_then(|v| v.as_str()) == Some(task_id) {
                        task["status"] = serde_json::json!("done");
                        task["evidence"] = force_evidence.clone();
                        updated = true;
                    }
                }
            }
        }
    }

    if !updated {
        anyhow::bail!("task '{}' found in parsed workplan but not in raw JSON", task_id);
    }

    if all_tasks_marked_done(&raw) {
        let prev = raw.get("status").and_then(|v| v.as_str()).unwrap_or("").to_string();
        if prev != "done" {
            raw["status"] = serde_json::json!("done");
            println!(
                "  {} Workplan status flipped: {} {} done",
                "\u{2713}".green(),
                if prev.is_empty() { "<unset>" } else { &prev },
                "\u{2192}".cyan()
            );
        }
    }

    std::fs::write(&path, serde_json::to_string_pretty(&raw)?)?;

    println!(
        "  {} Force-promoted {} (forced_by={}, at={})",
        "\u{26a0}".yellow(),
        task_id.bold(),
        forced_by,
        timestamp
    );
    println!("  {} Updated {}", "\u{2713}".green(), path.display());

    Ok(())
}

// ── Helpers ────────────────────────────────────────────────────────

fn resolve_path(file: &str) -> std::path::PathBuf {
    if file.contains('/') {
        std::path::PathBuf::from(file)
    } else {
        std::path::PathBuf::from("docs/workplans").join(file)
    }
}

fn extract_tasks(workplan: &Workplan) -> Vec<TaskInfo> {
    if !workplan.steps.is_empty() {
        workplan
            .steps
            .iter()
            .map(|s| {
                let condition = if !s.done_condition.is_empty() {
                    s.done_condition.clone()
                } else {
                    s.verify.clone()
                };
                TaskInfo {
                    id: s.id.clone(),
                    condition,
                    files: s.files.clone(),
                    done_command: s.done_command.clone(),
                    current_status: s.status.clone(),
                }
            })
            .collect()
    } else {
        workplan
            .phases
            .iter()
            .flat_map(|p| {
                p.tasks.iter().map(|t| TaskInfo {
                    id: t.id.clone(),
                    condition: t.name.clone(),
                    files: t.files.clone(),
                    done_command: t.done_command.clone(),
                    current_status: t.status.clone(),
                })
            })
            .collect()
    }
}

fn whoami() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_else(|_| "unknown".to_string())
}

fn mutate_workplan(
    raw: &mut serde_json::Value,
    step_results: &[(String, bool)],
    evidence_map: &std::collections::HashMap<String, reconcile_evidence::TaskEvidence>,
    demoted_ids: &std::collections::HashSet<String>,
    path: &std::path::Path,
) -> anyhow::Result<()> {
    let mut updated = false;
    let done_ids: std::collections::HashSet<&str> = step_results
        .iter()
        .filter(|(_, d)| *d)
        .map(|(id, _)| id.as_str())
        .collect();

    if let Some(steps) = raw.get_mut("steps").and_then(|v| v.as_array_mut()) {
        for step in steps.iter_mut() {
            let id = step
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if done_ids.contains(id.as_str())
                && step.get("status").and_then(|v| v.as_str()) != Some("done")
            {
                step["status"] = serde_json::json!("done");
                if let Some(ev) = evidence_map.get(id.as_str()) {
                    step["evidence"] = evidence_snapshot_json(ev);
                }
                updated = true;
            } else if demoted_ids.contains(&id) {
                step["status"] = serde_json::json!("pending");
                if let Some(obj) = step.as_object_mut() {
                    obj.remove("evidence");
                }
                updated = true;
            }
        }
    }

    if let Some(phases) = raw.get_mut("phases").and_then(|v| v.as_array_mut()) {
        for phase in phases.iter_mut() {
            if let Some(tasks) = phase.get_mut("tasks").and_then(|v| v.as_array_mut()) {
                for task in tasks.iter_mut() {
                    let id = task
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    if done_ids.contains(id.as_str())
                        && task.get("status").and_then(|v| v.as_str()) != Some("done")
                    {
                        task["status"] = serde_json::json!("done");
                        if let Some(ev) = evidence_map.get(id.as_str()) {
                            task["evidence"] = evidence_snapshot_json(ev);
                        }
                        updated = true;
                    } else if demoted_ids.contains(&id) {
                        task["status"] = serde_json::json!("pending");
                        if let Some(obj) = task.as_object_mut() {
                            obj.remove("evidence");
                        }
                        updated = true;
                    }
                }
            }
        }
    }

    let current = raw
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if all_tasks_marked_done(raw) {
        if current != "done" {
            raw["status"] = serde_json::json!("done");
            updated = true;
            let from = if current.is_empty() {
                "<unset>".to_string()
            } else {
                current
            };
            println!(
                "  {} Workplan status flipped: {} {} done",
                "\u{2713}".green(),
                from,
                "\u{2192}".cyan()
            );
        }
    } else if !demoted_ids.is_empty() && current == "done" {
        raw["status"] = serde_json::json!("in_progress");
        updated = true;
        println!(
            "  {} Workplan status rolled back: done {} in_progress (demotions)",
            "\u{26a0}".red(),
            "\u{2192}".cyan()
        );
    }

    if updated {
        std::fs::write(path, serde_json::to_string_pretty(raw)?)?;
        println!("  {} Updated {}", "\u{2713}".green(), path.display());
    }

    Ok(())
}

fn all_tasks_marked_done(raw: &serde_json::Value) -> bool {
    let mut saw_task = false;

    if let Some(steps) = raw.get("steps").and_then(|v| v.as_array()) {
        for step in steps {
            saw_task = true;
            let status = step.get("status").and_then(|v| v.as_str()).unwrap_or("");
            if status != "done" && status != "completed" {
                return false;
            }
        }
    }

    if let Some(phases) = raw.get("phases").and_then(|v| v.as_array()) {
        for phase in phases {
            if let Some(tasks) = phase.get("tasks").and_then(|v| v.as_array()) {
                for task in tasks {
                    saw_task = true;
                    let status = task.get("status").and_then(|v| v.as_str()).unwrap_or("");
                    if status != "done" && status != "completed" {
                        return false;
                    }
                }
            }
        }
    }

    saw_task
}

fn evidence_snapshot_json(ev: &reconcile_evidence::TaskEvidence) -> serde_json::Value {
    let files_found: Vec<String> = ev
        .files_exist
        .iter()
        .filter(|(_, exists)| *exists)
        .map(|(p, _)| p.display().to_string())
        .collect();
    let symbols_matched: Vec<String> = ev.symbol_hits.iter().map(|(sym, _)| sym.clone()).collect();
    serde_json::json!({
        "files_found": files_found,
        "symbols_matched": symbols_matched,
        "matching_commits": ev.matching_commits,
    })
}
