use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;
use crate::inference_client::{InferenceClient, InferenceTier};

#[derive(Debug, Serialize, Deserialize)]
struct Workplan {
    id: String,
    title: Option<String>,
    phases: Vec<Phase>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Phase {
    id: String,
    title: Option<String>,
    tasks: Vec<Task>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Task {
    id: String,
    title: String,
    status: String,
    strategy_hint: Option<String>,
    files: Option<Vec<String>>,
    evidence: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct ExecutionSummary {
    pub completed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub duration_s: u64,
    pub failures: Vec<String>,
}

pub async fn execute_workplan_autonomous(
    workplan_path: &Path,
    background: bool,
    project_dir: &Path,
) -> Result<String> {
    let start = std::time::Instant::now();

    // Read workplan JSON
    let content = std::fs::read_to_string(workplan_path)
        .with_context(|| format!("Failed to read workplan: {}", workplan_path.display()))?;

    let mut workplan: Workplan = serde_json::from_str(&content)
        .context("Failed to parse workplan JSON")?;

    if !background {
        eprintln!("⬡ Executing workplan: {}", workplan.title.as_deref().unwrap_or(&workplan.id));
        eprintln!("  Phases: {}  Tasks: {}",
            workplan.phases.len(),
            workplan.phases.iter().map(|p| p.tasks.len()).sum::<usize>()
        );
    }

    let mut completed = 0;
    let mut failed = 0;
    let mut skipped = 0;
    let mut failures = Vec::new();

    // Change to project directory
    std::env::set_current_dir(project_dir)
        .context("Failed to change to project directory")?;

    // Iterate phases and tasks
    let total_phases = workplan.phases.len();
    for phase_idx in 0..total_phases {
        let task_count = workplan.phases[phase_idx].tasks.len();

        for task_idx in 0..task_count {
            // Clone task data to avoid borrow conflicts
            let task = workplan.phases[phase_idx].tasks[task_idx].clone();

            // Skip non-pending tasks
            if task.status != "pending" {
                skipped += 1;
                continue;
            }

            if !background {
                eprintln!("\n⬡ Task {}: {}", task.id, task.title);
            }

            // Execute task
            let task_result = execute_task(&task, project_dir, background).await;

            // Now we can mutate the workplan
            match task_result {
                Ok(_) => {
                    workplan.phases[phase_idx].tasks[task_idx].status = "done".to_string();
                    completed += 1;
                    if !background {
                        eprintln!("  ✓ completed");
                    }
                }
                Err(e) => {
                    workplan.phases[phase_idx].tasks[task_idx].status = "failed".to_string();
                    failed += 1;
                    let error_msg = format!("{}: {}", task.id, e);
                    failures.push(error_msg.clone());
                    if !background {
                        eprintln!("  ✗ failed: {}", e);
                    }
                    // Save before stopping
                    save_workplan(&workplan, workplan_path)?;
                    // Stop on first failure
                    break;
                }
            }

            // Save progress after each successful task
            save_workplan(&workplan, workplan_path)?;
        }

        // Stop executing phases if any task failed
        if failed > 0 {
            break;
        }
    }

    let duration_s = start.elapsed().as_secs();

    let summary = ExecutionSummary {
        completed,
        failed,
        skipped,
        duration_s,
        failures,
    };

    Ok(serde_json::to_string_pretty(&summary)?)
}

async fn execute_task(
    task: &Task,
    project_dir: &Path,
    background: bool,
) -> Result<()> {
    // P3: Inference tier routing
    let tier = InferenceTier::from_strategy_hint(task.strategy_hint.as_deref());

    if !background {
        eprintln!("    tier: {:?} ({})", tier, tier.model_name());
    }

    let files_list = task.files.as_ref().map(|f| f.as_slice()).unwrap_or(&[]);
    let evidence_list = task.evidence.as_ref().map(|e| e.as_slice()).unwrap_or(&[]);

    // Build prompt
    let prompt = InferenceClient::build_task_prompt(
        &task.title,
        files_list,
        evidence_list,
    );

    if !background {
        eprintln!("    calling inference...");
    }

    // Call inference
    let client = InferenceClient::new();
    let response = client.generate(tier, prompt).await
        .context("Inference generation failed")?;

    // Parse response to extract files
    let generated_files = InferenceClient::parse_response(&response)
        .context("Failed to parse LLM response")?;

    if !background {
        eprintln!("    generated {} files", generated_files.len());
    }

    // Write generated files
    for (path, content) in &generated_files {
        let file_path = project_dir.join(path);
        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
        }
        std::fs::write(&file_path, content)
            .with_context(|| format!("Failed to write file: {}", path))?;

        if !background {
            eprintln!("    wrote: {}", path);
        }
    }

    // P4: Compile gate (simplified - just check cargo/tsc existence)
    let has_cargo = project_dir.join("Cargo.toml").exists();
    let has_ts = project_dir.join("tsconfig.json").exists();

    if has_cargo {
        if !background {
            eprintln!("    running cargo check...");
        }
        let check_output = Command::new("cargo")
            .arg("check")
            .arg("--workspace")
            .current_dir(project_dir)
            .output()
            .context("Failed to run cargo check")?;

        if !check_output.status.success() {
            let stderr = String::from_utf8_lossy(&check_output.stderr);
            // Rollback: git reset --hard HEAD
            Command::new("git")
                .args(["reset", "--hard", "HEAD"])
                .current_dir(project_dir)
                .output()
                .context("Failed to rollback after compile error")?;
            anyhow::bail!("Cargo check failed:\n{}", stderr);
        }
    } else if has_ts {
        if !background {
            eprintln!("    running tsc --noEmit...");
        }
        let check_output = Command::new("tsc")
            .arg("--noEmit")
            .current_dir(project_dir)
            .output()
            .context("Failed to run tsc")?;

        if !check_output.status.success() {
            let stderr = String::from_utf8_lossy(&check_output.stderr);
            Command::new("git")
                .args(["reset", "--hard", "HEAD"])
                .current_dir(project_dir)
                .output()
                .context("Failed to rollback after compile error")?;
            anyhow::bail!("TypeScript check failed:\n{}", stderr);
        }
    }

    // Git add + commit
    let commit_msg = format!("feat({}): {}", task.id, task.title);

    Command::new("git")
        .args(["add", "-A"])
        .current_dir(project_dir)
        .output()
        .context("Failed to git add")?;

    let commit_output = Command::new("git")
        .args(["commit", "-m", &commit_msg, "--allow-empty"])
        .current_dir(project_dir)
        .output()
        .context("Failed to git commit")?;

    if !commit_output.status.success() {
        let stderr = String::from_utf8_lossy(&commit_output.stderr);
        anyhow::bail!("Git commit failed: {}", stderr);
    }

    if !background {
        eprintln!("    ✓ committed");
    }

    Ok(())
}

fn save_workplan(workplan: &Workplan, path: &Path) -> Result<()> {
    let content = serde_json::to_string_pretty(workplan)?;
    let tmp_path = path.with_extension("tmp");
    std::fs::write(&tmp_path, content)?;
    std::fs::rename(&tmp_path, path)?;
    Ok(())
}
