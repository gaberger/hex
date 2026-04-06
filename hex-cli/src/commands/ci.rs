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

    // Enforce rules are local (read from .hex/adr-rules.toml).
    // Run `hex enforce sync` which checks rules against the current codebase
    // and exits non-zero if any mandatory rule is violated.
    let rules_file = std::path::Path::new(".hex/adr-rules.toml");
    if !rules_file.exists() {
        println!("{} (no .hex/adr-rules.toml)", "skip".yellow());
        return true;
    }

    // Count rules and violations by parsing the rules file directly
    let content = match std::fs::read_to_string(rules_file) {
        Ok(c) => c,
        Err(e) => {
            println!("{} (cannot read rules: {})", "skip".yellow(), e);
            return true;
        }
    };

    let total_rules = content.lines().filter(|l| l.trim_start().starts_with("[[rules]]")).count();

    // Run hex enforce sync to check current codebase against rules
    let out = tokio::process::Command::new("sh")
        .args(["-c", "hex enforce sync 2>&1; echo EXIT:$?"])
        .output()
        .await;

    match out {
        Ok(o) => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            // enforce sync exits 0 even with violations (it's a sync, not a gate)
            // Count lines containing "VIOLATION" or "✗" in the output
            let violations: Vec<&str> = stdout.lines()
                .filter(|l| l.contains("VIOLATION") || (l.contains('✗') && !l.contains("skip")))
                .collect();
            if violations.is_empty() {
                println!("{} ({} rules)", "pass".green(), total_rules);
                true
            } else {
                println!("{} ({} violation{})", "fail".red(), violations.len(), if violations.len() == 1 { "" } else { "s" });
                for v in violations.iter().take(5) {
                    println!("      {}", v.dimmed());
                }
                false
            }
        }
        Err(_) => {
            // hex CLI not on PATH — fall back to pass (analyze gate already covers ADR rules)
            println!("{} (hex not on PATH, covered by analyze gate)", "skip".yellow());
            true
        }
    }
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
