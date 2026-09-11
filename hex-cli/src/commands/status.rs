//! Project status command.
//!
//! `hex status` — shows project info, git state, and service health.

use colored::Colorize;


pub async fn run() -> anyhow::Result<()> {
    println!("{} hex project status", "\u{2b21}".cyan());
    println!();

    // Detect project root
    let cwd = std::env::current_dir()?;
    println!("  Project: {}", cwd.display());

    // Check for hex configuration
    let hex_dir = cwd.join(".hex");
    if hex_dir.is_dir() {
        println!("  Config:  {}", ".hex/ found".green());
    } else {
        println!("  Config:  {}", "no .hex/ directory".yellow());
    }

    // Check git status
    let git_output = tokio::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .current_dir(&cwd)
        .output()
        .await;

    match git_output {
        Ok(output) if output.status.success() => {
            let hash = String::from_utf8_lossy(&output.stdout).trim().to_string();
            println!("  Git:     {}", hash);
        }
        _ => {
            println!("  Git:     {}", "not a git repository".dimmed());
        }
    }

    // Check branch
    let branch_output = tokio::process::Command::new("git")
        .args(["branch", "--show-current"])
        .current_dir(&cwd)
        .output()
        .await;

    if let Ok(output) = branch_output {
        if output.status.success() {
            let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
            println!("  Branch:  {}", branch);
        }
    }

    Ok(())
}
