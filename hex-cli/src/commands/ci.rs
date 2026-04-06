//! `hex ci` — Run all hex enforcement gates.
//!
//! ADR-2604061100: single entry point for CI systems.
//! Gates: architecture boundaries, ADR rules, workplan done_commands, spec coverage.

use colored::Colorize;

pub async fn run() -> anyhow::Result<()> {
    println!("{} hex ci", "\u{2b21}".cyan());
    println!();

    let mut all_passed = true;

    // Gate 1: Architecture boundaries
    all_passed &= gate_analyze().await;

    // Gate 2: ADR rule compliance
    all_passed &= gate_enforce().await;

    // Gate 3: Workplan done_command sweep
    all_passed &= gate_workplan_done_commands().await;

    // Gate 4: Spec coverage — every step must reference >=1 spec ID
    all_passed &= gate_spec_coverage().await;

    println!();
    if all_passed {
        println!("{} All gates passed", "\u{2713}".green().bold());
        Ok(())
    } else {
        println!("{} One or more gates failed", "\u{2717}".red().bold());
        std::process::exit(1);
    }
}

async fn gate_analyze() -> bool {
    print!("  {} Architecture boundaries ... ", "\u{25cb}".dimmed());
    let nexus = crate::nexus_client::NexusClient::from_env();
    match nexus.get("/api/analyze?path=.").await {
        Ok(resp) => {
            let violations = resp["violations"]
                .as_array()
                .map(|v| v.len())
                .unwrap_or(0);
            if violations == 0 {
                println!("{}", "pass".green());
                true
            } else {
                println!("{} ({} violation{})", "fail".red(), violations, if violations == 1 { "" } else { "s" });
                if let Some(arr) = resp["violations"].as_array() {
                    for v in arr.iter().take(5) {
                        let msg = v["message"].as_str().unwrap_or("");
                        println!("      {}", msg.dimmed());
                    }
                    if arr.len() > 5 {
                        println!("      ... and {} more", arr.len() - 5);
                    }
                }
                false
            }
        }
        Err(_) => {
            // Nexus not running — fall back to local cargo check as a proxy
            println!("{} (nexus unavailable, running cargo check)", "?".yellow());
            let out = tokio::process::Command::new("cargo")
                .args(["check", "--workspace", "--quiet"])
                .output()
                .await;
            match out {
                Ok(o) if o.status.success() => { println!("      {} cargo check", "pass".green()); true }
                Ok(o) => {
                    let stderr = String::from_utf8_lossy(&o.stderr);
                    println!("      {} cargo check failed:", "fail".red());
                    for line in stderr.lines().take(10) {
                        println!("        {}", line.dimmed());
                    }
                    false
                }
                Err(e) => { println!("      {} could not run cargo check: {}", "fail".red(), e); false }
            }
        }
    }
}

async fn gate_enforce() -> bool {
    print!("  {} ADR rule compliance ....... ", "\u{25cb}".dimmed());

    let rules_file = std::path::Path::new(".hex/adr-rules.toml");
    if !rules_file.exists() {
        println!("{} (no .hex/adr-rules.toml)", "skip".yellow());
        return true;
    }

    let content = match std::fs::read_to_string(rules_file) {
        Ok(c) => c,
        Err(e) => {
            println!("{} (cannot read rules: {})", "skip".yellow(), e);
            return true;
        }
    };

    // Parse [[adr_rules]] directly — same schema as analyze.rs
    #[derive(serde::Deserialize)]
    struct RulesFile {
        #[serde(default)]
        adr_rules: Vec<AdrRule>,
    }
    #[derive(serde::Deserialize)]
    struct AdrRule {
        adr: String,
        message: String,
        #[serde(default)]
        file_patterns: Vec<String>,
        #[serde(default)]
        violation_patterns: Vec<String>,
    }

    let parsed: RulesFile = match toml::from_str(&content) {
        Ok(p) => p,
        Err(e) => {
            println!("{} (parse error: {})", "fail".red(), e);
            return false;
        }
    };

    let rules: Vec<&AdrRule> = parsed.adr_rules.iter()
        .filter(|r| !r.violation_patterns.is_empty())
        .collect();

    if rules.is_empty() {
        println!("{} (0 rules)", "pass".green());
        return true;
    }

    // Scan source files for violations
    let src = std::path::Path::new("src");
    if !src.is_dir() {
        println!("{} ({} rules, no src/)", "pass".green(), rules.len());
        return true;
    }

    let files = collect_source_files(src);
    let mut violations: Vec<String> = Vec::new();

    for path in &files {
        let rel = path.to_string_lossy().to_string();
        let file_content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        for rule in &rules {
            // Match file_patterns: "src/core/domain/**" → check if path starts with prefix
            if !rule.file_patterns.is_empty() {
                let matches = rule.file_patterns.iter().any(|p| {
                    let prefix = p.trim_end_matches("/**").trim_end_matches("**");
                    rel.starts_with(prefix)
                });
                if !matches { continue; }
            }

            for (line_num, line) in file_content.lines().enumerate() {
                let trimmed = line.trim();
                if trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with('*') {
                    continue;
                }
                for pattern in &rule.violation_patterns {
                    if line.contains(pattern.as_str()) {
                        violations.push(format!(
                            "{} [{}] {}:{}",
                            rule.adr, rule.message, rel, line_num + 1
                        ));
                        break;
                    }
                }
            }
        }
    }

    if violations.is_empty() {
        println!("{} ({} rule{})", "pass".green(), rules.len(), if rules.len() == 1 { "" } else { "s" });
        true
    } else {
        println!("{} ({} violation{})", "fail".red(), violations.len(), if violations.len() == 1 { "" } else { "s" });
        for v in violations.iter().take(5) {
            println!("      {}", v.dimmed());
        }
        if violations.len() > 5 {
            println!("      ... and {} more", violations.len() - 5);
        }
        false
    }
}

fn collect_source_files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.is_dir() {
                files.extend(collect_source_files(&path));
            } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if matches!(ext, "ts" | "tsx" | "js" | "jsx" | "rs" | "go" | "py") {
                    files.push(path);
                }
            }
        }
    }
    files
}

async fn gate_workplan_done_commands() -> bool {
    print!("  {} Workplan done_commands .... ", "\u{25cb}".dimmed());

    let pattern = std::path::Path::new("docs/workplans");
    if !pattern.is_dir() {
        println!("{} (no docs/workplans/ directory)", "skip".yellow());
        return true;
    }

    let entries: Vec<_> = std::fs::read_dir(pattern)
        .unwrap_or_else(|_| panic!("cannot read docs/workplans"))
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("json"))
        .collect();

    let mut failures: Vec<String> = Vec::new();

    for entry in &entries {
        let path = entry.path();
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let workplan: serde_json::Value = match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(_) => continue,
        };

        // Support both "steps" (new format) and "phases[].tasks" (old format)
        let steps = collect_steps(&workplan);

        for step in steps {
            let done_cmd = match step["done_command"].as_str() {
                Some(c) if !c.is_empty() => c,
                _ => continue,
            };
            let step_id = step["id"].as_str().unwrap_or("?");
            let workplan_id = workplan["id"].as_str().unwrap_or("?");
            let condition = step["done_condition"].as_str().unwrap_or("(no condition text)");

            let out = tokio::process::Command::new("sh")
                .args(["-c", done_cmd])
                .output()
                .await;

            let passed = matches!(out, Ok(ref o) if o.status.success());
            if !passed {
                failures.push(format!(
                    "{} / {}: {}\n        command: {}",
                    workplan_id, step_id, condition, done_cmd
                ));
            }
        }
    }

    if failures.is_empty() {
        println!("{}", "pass".green());
        true
    } else {
        println!("{} ({} failed)", "fail".red(), failures.len());
        for f in &failures {
            println!("      {}", f.dimmed());
        }
        false
    }
}

fn collect_steps(workplan: &serde_json::Value) -> Vec<serde_json::Value> {
    // New format: top-level "steps" array
    if let Some(steps) = workplan["steps"].as_array() {
        return steps.clone();
    }
    // Old format: phases[].tasks
    let mut tasks = Vec::new();
    if let Some(phases) = workplan["phases"].as_array() {
        for phase in phases {
            if let Some(phase_tasks) = phase["tasks"].as_array() {
                tasks.extend(phase_tasks.iter().cloned());
            }
        }
    }
    tasks
}

async fn gate_spec_coverage() -> bool {
    print!("  {} Spec coverage ............. ", "\u{25cb}".dimmed());

    let pattern = std::path::Path::new("docs/workplans");
    if !pattern.is_dir() {
        println!("{} (no docs/workplans/ directory)", "skip".yellow());
        return true;
    }

    let mut missing: Vec<String> = Vec::new();
    let mut checked = 0u32;

    for entry in std::fs::read_dir(pattern)
        .unwrap_or_else(|_| panic!("cannot read docs/workplans"))
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("json"))
    {
        let path = entry.path();
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let workplan: serde_json::Value = match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(_) => continue,
        };

        // Only enforce spec coverage when:
        // 1. The workplan references a spec file (top-level "specs" field, non-empty), AND
        // 2. At least one step already has spec references — proving the author
        //    intentionally started spec tracing. This prevents retroactive failures
        //    on historical workplans that predate step-level spec requirements.
        let specs_path = workplan["specs"].as_str().unwrap_or("").trim();
        if specs_path.is_empty() || !std::path::Path::new(specs_path).exists() {
            continue;
        }
        let steps_preview = collect_steps(&workplan);
        let any_step_has_specs = steps_preview.iter().any(|s| {
            s["specs"].as_array().map(|a| !a.is_empty()).unwrap_or(false)
        });
        if !any_step_has_specs {
            continue;
        }

        let workplan_id = workplan["id"].as_str().unwrap_or("?").to_string();
        let steps = collect_steps(&workplan);

        for step in &steps {
            checked += 1;
            let step_id = step["id"].as_str().unwrap_or("?");
            let specs = step["specs"].as_array();
            let has_specs = specs.map(|s| !s.is_empty()).unwrap_or(false);
            if !has_specs {
                missing.push(format!("{} / {}", workplan_id, step_id));
            }
        }
    }

    if missing.is_empty() {
        println!("{} ({} step{} checked)", "pass".green(), checked, if checked == 1 { "" } else { "s" });
        true
    } else {
        println!("{} ({} step{} without spec refs)", "fail".red(), missing.len(), if missing.len() == 1 { "" } else { "s" });
        for m in &missing {
            println!("      {}", m.dimmed());
        }
        false
    }
}
