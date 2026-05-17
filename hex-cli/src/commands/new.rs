//! Structured project intake (ADR-2604131500 §2).
//!
//! `hex new <path>` — create or adopt a directory, run `hex init`, register the
//! project, seed trust at suggest level, and optionally copy taste preferences.
//!
//! Non-interactive mode: `hex new ./myapp --name myapp --description "My app"`
//! Taste import:         `hex new ./myapp --taste-from other-project`

use colored::Colorize;
use serde_json::json;

use super::init::InitArgs;
use crate::nexus_client::NexusClient;

pub async fn run(
    path: &str,
    name: Option<String>,
    description: Option<String>,
    taste_from: Option<String>,
) -> anyhow::Result<()> {
    // ── 1. Ensure target directory exists ─────────────────────────────
    let target = std::path::Path::new(path);
    if !target.exists() {
        std::fs::create_dir_all(target)?;
    }
    let abs_path = target
        .canonicalize()
        .unwrap_or_else(|_| std::path::PathBuf::from(path));

    let dir_name = abs_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "unnamed".to_string());

    let proj_name = name.clone().unwrap_or_else(|| dir_name.clone());

    println!(
        "\n{} Creating project {} at {}",
        "\u{2b21}".cyan(),
        proj_name.bold(),
        abs_path.display().to_string().dimmed()
    );

    // ── 2. Run hex init (reuse existing init logic) ──────────────────
    let init_args = InitArgs {
        path: abs_path.display().to_string(),
        name: Some(proj_name.clone()),
        scaffold: false,
        no_claude_md: false,
        skip_interview: true, // non-interactive for `hex new`
        force: false,
    };

    // init::run prints its own progress; if the project is already initialized
    // it will bail with a helpful message (use --force to reinit).
    super::init::run(init_args).await?;

    // ── 3. Register project via nexus (if not already done by init) ──
    // init::run already calls register_project_in_nexus internally.
    // We do a best-effort description update if --description was provided,
    // since init doesn't support description.
    if let Some(desc) = &description {
        let client = NexusClient::from_env();
        if client.ensure_running().await.is_ok() {
            let body = json!({
                "rootPath": abs_path.display().to_string(),
                "name": &proj_name,
                "description": desc,
            });
            // Re-register with description — the endpoint is idempotent
            let _ = client.post("/api/projects/register", &body).await;
        }
    }

    // ── 4. Trust seeding (suggest level) ─────────────────────────────
    // Trust is seeded at "suggest" level automatically by the nexus on
    // project registration (P4.2). Print confirmation for visibility.

    // ── 5. Taste preferences ─────────────────────────────────────────
    if let Some(source_project) = &taste_from {
        copy_taste_from(&proj_name, source_project).await?;
    }

    // ── 6. Summary ───────────────────────────────────────────────────
    println!();
    let separator = "\u{2500}".repeat(50);
    println!("  {}", separator.dimmed());
    println!(
        "  {} Project {} registered. Trust: all {}. Run {} to check status.",
        "\u{2713}".green(),
        proj_name.bold(),
        "suggest".yellow(),
        "hex brief".cyan(),
    );
    if taste_from.is_some() {
        println!(
            "  {} Taste preferences copied from {}.",
            "\u{2713}".green(),
            taste_from.as_deref().unwrap_or("").bold(),
        );
    }
    println!("  {}", separator.dimmed());
    println!();

    Ok(())
}

/// Copy taste preferences from one project to the newly created one.
///
/// Fetches all taste entries for `source_project` via GET /api/taste,
/// then POSTs each entry targeting `target_project`.
async fn copy_taste_from(target_project: &str, source_project: &str) -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    if nexus.ensure_running().await.is_err() {
        eprintln!(
            "  {} Taste copy skipped — hex-nexus not reachable.",
            "\u{2022}".dimmed()
        );
        return Ok(());
    }

    let source_path = format!("/api/taste?project={}", source_project);
    let entries = match nexus.get(&source_path).await {
        Ok(val) if val.is_array() => val,
        Ok(val) => json!([val]),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("404") || msg.contains("not found") {
                eprintln!(
                    "  {} No taste preferences found for source project '{}'.",
                    "!".yellow().bold(),
                    source_project,
                );
            } else {
                eprintln!(
                    "  {} Could not fetch taste from '{}': {}",
                    "!".yellow().bold(),
                    source_project,
                    msg,
                );
            }
            return Ok(());
        }
    };

    let arr = entries.as_array().cloned().unwrap_or_default();
    if arr.is_empty() {
        println!(
            "  {} No taste preferences in '{}' to copy.",
            "\u{2022}".dimmed(),
            source_project,
        );
        return Ok(());
    }

    let mut copied = 0usize;
    for entry in &arr {
        let body = json!({
            "project": target_project,
            "scope": entry["scope"].as_str().unwrap_or("universal"),
            "category": entry["category"].as_str().unwrap_or("general"),
            "name": entry["name"].as_str().unwrap_or(""),
            "value": entry["value"].as_str().unwrap_or(""),
        });
        if nexus.post("/api/taste", &body).await.is_ok() {
            copied += 1;
        }
    }

    println!(
        "  {} Copied {} taste preference{} from '{}'.",
        "\u{2713}".green(),
        copied,
        if copied == 1 { "" } else { "s" },
        source_project,
    );

    Ok(())
}
