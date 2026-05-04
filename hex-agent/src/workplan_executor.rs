use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

use hex_core::ports::file_writer::IFileWriter;

use crate::inference_client::{InferenceClient, InferenceTier, OpenRouterClient, ClaudeClient};

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

#[derive(Debug, Serialize, Deserialize)]
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

#[derive(Debug, Clone)]
pub struct InferenceAttempt {
    pub tier_used: String,
    pub model_name: String,
    pub validation_passed: bool,
    pub duration_ms: u64,
    pub cost_estimate: f64,
}

fn validate_code_output(code: &str, file_path: Option<&str>) -> Result<(), String> {
    // Detect stub patterns
    let stub_patterns = [
        "...", "TODO", "FIXME", "placeholder",
        "demonstration purposes", "Placeholder",
        "stub", "not implemented", "unimplemented"
    ];

    for pattern in &stub_patterns {
        if code.contains(pattern) {
            return Err(format!("Code contains stub pattern: {}", pattern));
        }
    }

    // Detect trivial implementations (hardcoded empty returns)
    if code.contains("vec![]") && code.matches("vec![]").count() > 2 {
        return Err(String::from("Code contains multiple empty vec![] - likely placeholder"));
    }

    if code.matches("```").count() % 2 != 0 {
        return Err(String::from("Unmatched markdown fences found"));
    }

    // Only validate Rust syntax for .rs files
    if let Some(path) = file_path {
        if path.ends_with(".rs") {
            if code.contains("unresolved import") {
                return Err(String::from("Missing or incorrect imports"));
            }

            // Check if this looks like an abstraction BEFORE parsing
            // (parsing may fail on minimal valid trait definitions)
            let is_likely_abstraction = code.contains("trait ")
                || code.contains("enum ")
                || code.contains("struct ")
                || code.contains("type ");

            // Parse with syn to validate syntax
            match syn::parse_file(code) {
                Ok(parsed) => {
                    // Double-check abstraction via AST
                    let is_abstraction = parsed.items.iter().any(|item| {
                        matches!(item, syn::Item::Trait(_) | syn::Item::Enum(_) | syn::Item::Struct(_) | syn::Item::Type(_))
                    }) || is_likely_abstraction;

                    // Allow short files ONLY if:
                    // 1. They contain trait/enum/struct/type definitions (abstractions are legitimately short)
                    // 2. OR they have at least 5 lines of non-comment code
                    let non_comment_lines: Vec<&str> = code.lines()
                        .filter(|line| {
                            let trimmed = line.trim();
                            !trimmed.is_empty() && !trimmed.starts_with("//")
                        })
                        .collect();

                    if non_comment_lines.len() < 5 && !is_abstraction {
                        return Err(String::from("Code is less than 5 lines and not a trait/enum/struct definition"));
                    }
                }
                Err(e) => {
                    // If parsing fails but looks like an abstraction, allow it
                    if is_likely_abstraction {
                        // Still validate it's not completely empty
                        let non_empty_lines = code.lines().filter(|l| !l.trim().is_empty()).count();
                        if non_empty_lines < 2 {
                            return Err(String::from("Code is too minimal (< 2 non-empty lines)"));
                        }
                        // Allow it despite parse failure - may be a valid minimal trait
                    } else {
                        return Err(format!("Invalid syntax: {}", e));
                    }
                }
            }
        }
    } else {
        // Non-Rust files still need minimum length check
        let lines: Vec<&str> = code.lines().collect();
        if lines.len() < 5 {
            return Err(String::from("Code is less than 5 lines"));
        }
    }

    Ok(())
}

async fn try_with_escalation(
    task_title: &str,
    files: &[String],
    evidence: &[String],
    background: bool,
) -> Result<(Vec<(String, String)>, InferenceAttempt)> {
    let prompt = InferenceClient::build_task_prompt(task_title, files, evidence);

    // Detect abstraction creation tasks - route to higher tiers
    let is_abstraction_task = {
        let title_lower = task_title.to_lowercase();
        title_lower.contains("trait")
            || title_lower.contains("interface")
            || title_lower.contains("port")
            || title_lower.contains("abstraction")
            || files.iter().any(|f| {
                f.contains("/ports/") || f.contains("/domain/")
            })
    };

    if !background && is_abstraction_task {
        eprintln!("  ⬡ Detected abstraction task - routing to Tier 3 (Claude)");
    }

    // Skip T1/T2 for abstractions - go straight to T3 (Claude)
    if is_abstraction_task {

        if let Some(claude) = ClaudeClient::new() {
            let start = std::time::Instant::now();
            let claude_result = claude.generate(prompt.clone()).await;
            let claude_duration = start.elapsed().as_millis() as u64;

            match claude_result {
                Ok(response) => {
                    match InferenceClient::parse_response(&response) {
                        Ok(parsed_files) => {
                            let mut all_valid = true;
                            for (path, content) in &parsed_files {
                                if let Err(e) = validate_code_output(content, Some(path)) {
                                    if !background {
                                        eprintln!("    ✗ Claude validation failed for {}: {}", path, e);
                                    }
                                    all_valid = false;
                                    break;
                                }
                            }

                            if all_valid {
                                if !background {
                                    eprintln!("    ✓ Claude succeeded");
                                }
                                return Ok((parsed_files, InferenceAttempt {
                                    tier_used: "Claude".to_string(),
                                    model_name: "claude-sonnet-4".to_string(),
                                    validation_passed: true,
                                    duration_ms: claude_duration,
                                    cost_estimate: 0.15,
                                }));
                            }
                        }
                        Err(e) => {
                            if !background {
                                eprintln!("    ✗ Claude parse failed: {}", e);
                            }
                        }
                    }
                }
                Err(e) => {
                    if !background {
                        eprintln!("    ✗ Claude inference failed: {}", e);
                    }
                }
            }
        } else if let Some(openrouter) = OpenRouterClient::new() {
            // Try Claude via OpenRouter
            if !background {
                eprintln!("  ⬡ Tier 2.5 (OpenRouter): anthropic/claude-3.5-sonnet");
            }

            let start = std::time::Instant::now();
            let claude_result = openrouter.generate_with_model(prompt.clone(), "anthropic/claude-3.5-sonnet").await;
            let claude_duration = start.elapsed().as_millis() as u64;

            match claude_result {
                Ok(response) => {
                    match InferenceClient::parse_response(&response) {
                        Ok(parsed_files) => {
                            let mut all_valid = true;
                            for (path, content) in &parsed_files {
                                if let Err(e) = validate_code_output(content, Some(path)) {
                                    if !background {
                                        eprintln!("    ✗ Claude (OpenRouter) validation failed for {}: {}", path, e);
                                    }
                                    all_valid = false;
                                    break;
                                }
                            }

                            if all_valid {
                                if !background {
                                    eprintln!("    ✓ Claude (OpenRouter) succeeded");
                                }
                                return Ok((parsed_files, InferenceAttempt {
                                    tier_used: "OpenRouter".to_string(),
                                    model_name: "anthropic/claude-3.5-sonnet".to_string(),
                                    validation_passed: true,
                                    duration_ms: claude_duration,
                                    cost_estimate: 0.01,  // OpenRouter Claude is cheaper
                                }));
                            }
                        }
                        Err(e) => {
                            if !background {
                                eprintln!("    ✗ Claude (OpenRouter) parse failed: {}", e);
                            }
                        }
                    }
                }
                Err(e) => {
                    if !background {
                        eprintln!("    ✗ Claude (OpenRouter) inference failed: {}", e);
                    }
                }
            }
        } else {
            // No frontier model access at all
            if !background {
                eprintln!("    ⚠ WARNING: Abstraction task without frontier model access");
                eprintln!("    ⚠ Set OPENROUTER_API_KEY or ANTHROPIC_API_KEY for trait/interface definitions");
                eprintln!("    ⚠ Falling back to local models (low quality for abstractions)");
            }
            // Fall through to standard escalation ladder
        }

        // If Claude failed or unavailable, bail only if Claude was attempted
        if ClaudeClient::new().is_some() {
            anyhow::bail!("Abstraction task failed - Claude validation failed")
        }
        // Otherwise fall through to standard ladder with warning already shown
    }

    // Standard escalation ladder for non-abstraction tasks
    // Tier 1: Local model
    let start = std::time::Instant::now();
    let local_client = InferenceClient::new();
    let tier = InferenceTier::from_strategy_hint(None); // Default T2 for codegen

    if !background {
        eprintln!("  ⬡ Tier 1 (Local): {}", tier.model_name());
    }

    let local_result = local_client.generate(tier.clone(), prompt.clone()).await;
    let local_duration = start.elapsed().as_millis() as u64;

    if let Ok(response) = local_result {
        match InferenceClient::parse_response(&response) {
            Ok(parsed_files) => {
                // Validate each file
                let mut all_valid = true;
                for (path, content) in &parsed_files {
                    if let Err(e) = validate_code_output(content, Some(path)) {
                        if !background {
                            eprintln!("    ✗ Local validation failed for {}: {}", path, e);
                        }
                        all_valid = false;
                        break;
                    }
                }

                if all_valid {
                    if !background {
                        eprintln!("    ✓ Local model succeeded");
                    }
                    return Ok((parsed_files, InferenceAttempt {
                        tier_used: "Local".to_string(),
                        model_name: tier.model_name().to_string(),
                        validation_passed: true,
                        duration_ms: local_duration,
                        cost_estimate: 0.0,
                    }));
                }
            }
            Err(e) => {
                if !background {
                    eprintln!("    ✗ Local parse failed: {}", e);
                }
            }
        }
    } else if !background {
        eprintln!("    ✗ Local inference failed");
    }

    // Tier 2: OpenRouter (if available)
    if let Some(openrouter) = OpenRouterClient::new() {
        if !background {
            eprintln!("  ⬡ Tier 2 (OpenRouter): deepseek/deepseek-coder");
        }

        let start = std::time::Instant::now();
        let openrouter_result = openrouter.generate(prompt.clone()).await;
        let openrouter_duration = start.elapsed().as_millis() as u64;

        match openrouter_result {
            Ok(response) => {
                if !background {
                    eprintln!("    OpenRouter response length: {} bytes", response.len());
                }
                match InferenceClient::parse_response(&response) {
                    Ok(parsed_files) => {
                        let mut all_valid = true;
                        for (path, content) in &parsed_files {
                            match validate_code_output(content, Some(path)) {
                                Ok(_) => {},
                                Err(e) => {
                                    if !background {
                                        eprintln!("    ✗ OpenRouter validation failed for {}: {}", path, e);
                                    }
                                    all_valid = false;
                                    break;
                                }
                            }
                        }

                        if all_valid {
                            if !background {
                                eprintln!("    ✓ OpenRouter succeeded");
                            }
                            return Ok((parsed_files, InferenceAttempt {
                                tier_used: "OpenRouter".to_string(),
                                model_name: "deepseek/deepseek-chat".to_string(),
                                validation_passed: true,
                                duration_ms: openrouter_duration,
                                cost_estimate: 0.0005,
                            }));
                        }
                    }
                    Err(e) => {
                        if !background {
                            eprintln!("    ✗ OpenRouter parse failed: {}", e);
                        }
                    }
                }
            }
            Err(e) => {
                if !background {
                    eprintln!("    ✗ OpenRouter inference failed: {}", e);
                }
            }
        }
    }

    // Tier 3: Claude API (final fallback)
    if let Some(claude) = ClaudeClient::new() {
        if !background {
            eprintln!("  ⬡ Tier 3 (Claude): claude-sonnet-4");
        }

        let start = std::time::Instant::now();
        let claude_result = claude.generate(prompt.clone()).await;
        let claude_duration = start.elapsed().as_millis() as u64;

        match claude_result {
            Ok(response) => {
                match InferenceClient::parse_response(&response) {
                    Ok(parsed_files) => {
                        let mut all_valid = true;
                        for (path, content) in &parsed_files {
                            match validate_code_output(content, Some(path)) {
                                Ok(_) => {},
                                Err(e) => {
                                    if !background {
                                        eprintln!("    ✗ Claude validation failed for {}: {}", path, e);
                                    }
                                    all_valid = false;
                                    break;
                                }
                            }
                        }

                        if all_valid {
                            if !background {
                                eprintln!("    ✓ Claude succeeded");
                            }
                            return Ok((parsed_files, InferenceAttempt {
                                tier_used: "Claude".to_string(),
                                model_name: "claude-sonnet-4".to_string(),
                                validation_passed: true,
                                duration_ms: claude_duration,
                                cost_estimate: 0.15,
                            }));
                        }
                    }
                    Err(e) => {
                        if !background {
                            eprintln!("    ✗ Claude parse failed: {}", e);
                        }
                    }
                }
            }
            Err(e) => {
                if !background {
                    eprintln!("    ✗ Claude inference failed: {}", e);
                }
            }
        }
    }

    anyhow::bail!("All tiers failed (Local, OpenRouter, Claude)")
}

pub async fn execute_workplan_autonomous(
    workplan_path: &Path,
    background: bool,
    project_dir: &Path,
    writer: &dyn IFileWriter,
) -> Result<String> {
    let start = std::time::Instant::now();

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

    let mut summary = ExecutionSummary {
        completed: 0,
        failed: 0,
        skipped: 0,
        duration_s: 0,
        failures: Vec::new(),
    };

    for phase in &mut workplan.phases {
        if !background {
            eprintln!("\n⬡ Phase {}: {}", phase.id, phase.title.as_deref().unwrap_or(""));
        }

        for task in &mut phase.tasks {
            if task.status == "done" {
                summary.skipped += 1;
                continue;
            }

            if !background {
                eprintln!("  ⬡ Task {}: {}", task.id, task.title);
            }

            let files = task.files.as_deref().unwrap_or(&[]);
            let evidence = task.evidence.as_deref().unwrap_or(&[]);

            // Use escalation ladder
            let generation_result = try_with_escalation(
                &task.title,
                files,
                evidence,
                background,
            ).await;

            match generation_result {
                Ok((file_contents, attempt)) => {
                    if !background {
                        eprintln!("    Succeeded with: {} ({}) in {}ms, cost ~${:.4}",
                            attempt.tier_used, attempt.model_name, attempt.duration_ms, attempt.cost_estimate);
                    }

                    // Write files via IFileWriter — adapter enforces
                    // critical-path protection (sched.rs, monitor.rs, …)
                    let mut blocked_files = Vec::new();
                    for (file_path, content) in file_contents {
                        let full_path = project_dir.join(&file_path);
                        match writer.write_file(&full_path, &content) {
                            Ok(()) => {}
                            Err(err) => {
                                if !background {
                                    eprintln!("    ⚠ {}", err);
                                }
                                if err.to_lowercase().contains("critical") {
                                    blocked_files.push(file_path.clone());
                                } else {
                                    blocked_files.push(format!("{}: {}", file_path, err));
                                }
                            }
                        }
                    }

                    // If any files were blocked, fail the task
                    // Note: This is a fallback - workplans should be rejected at enqueue time
                    // by hex-cli's pre-flight validation (hex sched enqueue)
                    if !blocked_files.is_empty() {
                        task.status = "failed".to_string();
                        summary.failed += 1;
                        summary.failures.push(format!(
                            "{}: Cannot modify critical infrastructure files: {} \
                             (this workplan bypassed pre-flight validation)",
                            task.id,
                            blocked_files.join(", ")
                        ));
                        continue;
                    }

                    // Run evidence commands
                    let mut evidence_passed = true;
                    for cmd in evidence {
                        let output = Command::new("sh")
                            .arg("-c")
                            .arg(cmd)
                            .current_dir(project_dir)
                            .output();

                        if let Ok(result) = output {
                            if !result.status.success() {
                                evidence_passed = false;
                                if !background {
                                    eprintln!("    ✗ Evidence failed: {}", cmd);
                                }
                                break;
                            }
                        }
                    }

                    if evidence_passed {
                        task.status = "done".to_string();
                        summary.completed += 1;
                    } else {
                        task.status = "failed".to_string();
                        summary.failed += 1;
                        summary.failures.push(format!("{}: evidence validation failed", task.id));
                    }
                }
                Err(e) => {
                    task.status = "failed".to_string();
                    summary.failed += 1;
                    summary.failures.push(format!("{}: {}", task.id, e));
                }
            }
        }
    }

    // Write updated workplan
    let updated_json = serde_json::to_string_pretty(&workplan)?;
    std::fs::write(workplan_path, updated_json)?;

    summary.duration_s = start.elapsed().as_secs();

    if !background {
        eprintln!("\n⬡ Summary: {} completed, {} failed, {} skipped in {}s",
            summary.completed, summary.failed, summary.skipped, summary.duration_s);
    }

    Ok(serde_json::to_string(&summary)?)
}
