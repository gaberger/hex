//! `hex dev` — interactive TUI-driven development pipeline (ADR-2603232005).
//!
//! Entry point that creates or resumes a DevSession, then launches the
//! ratatui TUI for the full ADR → Plan → Code → Validate → Commit pipeline.

use std::io::IsTerminal;
use std::time::{Duration, SystemTime};

use anyhow::{bail, Result};
use clap::Subcommand;
use colored::Colorize;

use crate::fmt::{pretty_table, status_badge, truncate};
use crate::pipeline::DevConfig;
use crate::session::{DevSession, SessionStatus};
use crate::tui::TuiApp;

#[derive(Subcommand)]
pub enum DevAction {
    /// Start a new dev session for a feature
    Start {
        /// Feature description
        description: String,

        /// Skip gates — auto-approve all checkpoints
        #[arg(long)]
        quick: bool,

        /// Fully autonomous — no gates, no pauses
        #[arg(long, short)]
        auto: bool,

        /// Dry-run — show what would happen without calling inference
        #[arg(long)]
        dry_run: bool,

        /// Inference model override (OpenRouter model ID, e.g. deepseek/deepseek-r1).
        /// If omitted, each phase auto-selects the best model for its task type.
        #[arg(long, default_value = "")]
        model: String,

        /// Inference provider
        #[arg(long, default_value = "openrouter")]
        provider: String,

        /// Cost budget ceiling in USD (0 = unlimited)
        #[arg(long, default_value = "0.0")]
        budget: f64,

        /// Output directory for generated files.
        /// Defaults to examples/<slug>/ based on the feature description.
        /// Use --dir . to write to the current directory.
        #[arg(long, default_value = "")]
        dir: String,
    },

    /// Resume the most recent in-progress session
    Resume,

    /// Resume a specific session by ID
    Load {
        /// Session ID to resume
        id: String,
    },

    /// List all dev sessions
    List,

    /// Clean up completed sessions
    Clean,
}

pub async fn run(action: DevAction) -> Result<()> {
    match action {
        DevAction::List => list_sessions(),
        DevAction::Clean => clean_sessions(),
        DevAction::Resume => resume_latest().await,
        DevAction::Load { id } => resume_by_id(&id).await,
        DevAction::Start {
            description,
            quick,
            auto,
            dry_run,
            model,
            provider,
            budget,
            dir,
        } => {
            start_session(description, quick, auto, dry_run, model, provider, budget, dir).await
        }
    }
}

fn list_sessions() -> Result<()> {
    let sessions = DevSession::list_all()?;
    if sessions.is_empty() {
        println!("{}", "No dev sessions found.".dimmed());
        return Ok(());
    }
    let rows: Vec<Vec<String>> = sessions.iter().map(|s| {
        vec![
            s.id.clone(),
            status_badge(&s.status.to_string()),
            s.current_phase.to_string(),
            format!("${:.4}", s.total_cost_usd),
            truncate(&s.feature_description, 50),
        ]
    }).collect();
    println!("{}", pretty_table(&["ID", "Status", "Phase", "Cost", "Feature"], &rows));
    Ok(())
}

fn clean_sessions() -> Result<()> {
    let count = DevSession::clean_completed()?;
    println!(
        "Cleaned {} completed session{}.",
        count,
        if count == 1 { "" } else { "s" }
    );
    Ok(())
}

async fn resume_latest() -> Result<()> {
    crate::commands::nexus::ensure_nexus_running().await?;
    let session = DevSession::load_latest()?;
    match session {
        Some(s) => {
            let config = config_from_session(&s);
            launch_tui(s, config).await
        }
        None => {
            bail!("No in-progress or paused session found. Use `hex dev start <description>` to begin.");
        }
    }
}

async fn resume_by_id(id: &str) -> Result<()> {
    crate::commands::nexus::ensure_nexus_running().await?;
    let session = DevSession::load(id)?;

    // Completed/failed sessions can't be resumed — show summary instead
    if matches!(session.status, SessionStatus::Completed | SessionStatus::Failed) {
        print_session_summary(&session);
        return Ok(());
    }

    let config = config_from_session(&session);
    launch_tui(session, config).await
}

/// Reconstruct a `DevConfig` from a persisted session, preserving the
/// original output_dir, provider, and model selections.
fn config_from_session(session: &DevSession) -> DevConfig {
    let model = session
        .model_selections
        .get("default")
        .cloned()
        .unwrap_or_default();
    let provider = session
        .provider
        .clone()
        .unwrap_or_else(|| "openrouter".into());
    let output_dir = session
        .output_dir
        .clone()
        .unwrap_or_else(|| ".".into());

    DevConfig::from_args(
        session.feature_description.clone(),
        false,  // interactive mode on resume
        false,
        false,
        model,
        provider,
        0.0,
        output_dir,
    )
}

async fn start_session(
    description: String,
    quick: bool,
    auto: bool,
    dry_run: bool,
    model: String,
    provider: String,
    budget: f64,
    dir: String,
) -> Result<()> {
    // Ensure hex-nexus is running (with agent) — required for the dev pipeline
    crate::commands::nexus::ensure_nexus_running().await?;

    // ── Determine output directory ────────────────────────────────────
    let output_dir = if dir.is_empty() {
        // Auto-generate: examples/<slug>/
        let slug = description
            .to_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() || c == '-' { c } else { '-' })
            .collect::<String>();
        let slug = slug
            .split('-')
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("-");
        let slug = if slug.len() > 50 { &slug[..50] } else { &slug };
        let slug = slug.trim_end_matches('-');
        format!("examples/{}", slug)
    } else {
        dir
    };

    // ── Clean up stale runs before creating the output directory ─────
    // If the target directory (or sibling directories with the same slug prefix)
    // exists, has no passing binary, and is older than 5 minutes, remove it.
    cleanup_stale_runs(&output_dir);

    // ── Ensure project is initialized in the output directory ─────────
    // project_id is the root of all traceability: session → swarm → tasks → agents
    std::fs::create_dir_all(&output_dir)?;
    let project_id = match read_project_id_in(&output_dir) {
        Some(id) => id,
        None => {
            println!(
                "{} Initializing hex project in {}...",
                "⬡".yellow(),
                output_dir,
            );
            crate::commands::init::run_init_in(&output_dir, &description).await?;
            read_project_id_in(&output_dir).ok_or_else(|| anyhow::anyhow!(
                "hex init completed but .hex/project.json still missing in {}",
                output_dir,
            ))?
        }
    };

    // ── Ensure agent is registered ──────────────────────────────────
    // If no agent session file exists, register with nexus now.
    // This writes ~/.hex/sessions/agent-{CLAUDE_SESSION_ID}.json.
    let project_path = std::path::PathBuf::from(&output_dir)
        .canonicalize()
        .unwrap_or_else(|_| std::path::PathBuf::from(&output_dir));
    let project_name = project_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("hex-project");
    if DevSession::new("probe").agent_id.is_none() {
        let _ = crate::commands::hook::register_session_agent(
            &project_path,
            project_name,
        ).await;
    }

    let config = DevConfig::from_args(
        description.clone(),
        quick,
        auto,
        dry_run,
        model.clone(),
        provider,
        budget,
        output_dir.clone(),
    );

    // Create session AFTER agent registration so agent_id is resolved
    let mut session = DevSession::new(&description);
    if !model.is_empty() {
        session.model_selections.insert("default".into(), model);
    }
    session.output_dir = Some(output_dir.clone());
    session.provider = Some(config.provider.clone());
    session.project_id = Some(project_id.clone());
    session.save()?;

    println!(
        "{} Created session {} for: {} [mode: {}]",
        "✓".green(),
        session.id.dimmed(),
        description.bold(),
        config.mode,
    );

    // Detect TTY — fall back to headless if no terminal available
    if config.mode.needs_tty() && !std::io::stdout().is_terminal() {
        println!(
            "{} No TTY detected — running in headless (auto) mode",
            "⚠".yellow(),
        );
        let mut config = config;
        config.mode = crate::pipeline::DevMode::Auto;
        let app = TuiApp::with_config(session, config);
        return app.run();
    }

    let app = TuiApp::with_config(session, config);
    app.run()?;
    Ok(())
}

async fn launch_tui(session: DevSession, config: DevConfig) -> Result<()> {
    println!(
        "{} Resuming session {} — phase: {} [mode: {}]",
        "▶".yellow(),
        session.id.dimmed(),
        session.current_phase,
        config.mode,
    );

    // Detect TTY — fall back to headless if no terminal available
    if !std::io::stdout().is_terminal() {
        println!(
            "{} No TTY detected — running in headless mode",
            "⚠".yellow(),
        );
        let mut app = TuiApp::with_config(session, config);
        app.config.mode = crate::pipeline::DevMode::Auto;
        return app.run();
    }

    let app = TuiApp::with_config(session, config);
    app.run()?;
    Ok(())
}

/// Remove stale output directories before starting a new run.
///
/// Strategy:
/// 1. If `output_dir` itself exists, has no passing binary, and was created
///    more than 5 minutes ago — delete it and log the cleanup.
/// 2. Also scan the parent directory for sibling directories that share the
///    same slug prefix (handles the "12 temperature converter" duplicate situation).
///
/// A directory is considered "successful" if `target/debug/<name>` or
/// `node_modules/` exists inside it (indicating a completed build).
fn cleanup_stale_runs(output_dir: &str) {
    let stale_threshold = Duration::from_secs(5 * 60);
    let path = std::path::Path::new(output_dir);

    // Clean the exact target directory if stale
    cleanup_if_stale(path, stale_threshold);

    // Clean sibling directories with the same slug prefix
    let slug = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    if slug.is_empty() {
        return;
    }

    let parent = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    let entries = match std::fs::read_dir(parent) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let sibling = entry.path();
        // Skip the exact target dir (already handled above)
        if sibling == path {
            continue;
        }
        let sibling_name = sibling
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        // Match directories that share the slug as a prefix
        if sibling_name.starts_with(slug) && sibling.is_dir() {
            cleanup_if_stale(&sibling, stale_threshold);
        }
    }
}

/// Delete `dir` if it exists, has no successful build artifact, and its
/// metadata mtime is older than `threshold`. Logs the outcome.
fn cleanup_if_stale(dir: &std::path::Path, threshold: Duration) {
    if !dir.exists() {
        return;
    }

    // A successful run leaves either a compiled binary or node_modules
    let has_binary = dir
        .file_name()
        .and_then(|n| n.to_str())
        .map(|name| dir.join("target").join("debug").join(name).exists())
        .unwrap_or(false);
    let has_node_modules = dir.join("node_modules").exists();
    if has_binary || has_node_modules {
        return;
    }

    // Check age via mtime
    let is_old = dir
        .metadata()
        .and_then(|m| m.modified())
        .map(|mtime| {
            SystemTime::now()
                .duration_since(mtime)
                .unwrap_or(Duration::ZERO)
                > threshold
        })
        .unwrap_or(false);

    if !is_old {
        return;
    }

    match std::fs::remove_dir_all(dir) {
        Ok(()) => println!(
            "{} Cleaned up stale run: {}",
            "⬡".yellow(),
            dir.display()
        ),
        Err(e) => eprintln!(
            "warn: could not remove stale run {}: {}",
            dir.display(),
            e
        ),
    }
}

/// Read project_id from `.hex/project.json` in the given directory.
fn read_project_id_in(dir: &str) -> Option<String> {
    let project_json = std::path::Path::new(dir).join(".hex/project.json");
    let content = std::fs::read_to_string(project_json).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&content).ok()?;
    parsed["id"].as_str().filter(|s| !s.is_empty()).map(|s| s.to_string())
}

/// Print a read-only summary for completed/failed sessions.
fn print_session_summary(session: &DevSession) {
    println!(
        "{} Session {} — {} ({})",
        if session.status == SessionStatus::Completed { "✓".green() } else { "✗".red() },
        session.id.dimmed(),
        session.status,
        session.current_phase,
    );
    println!("  Feature: {}", session.feature_description.bold());
    println!("  Cost:    ${:.4}", session.total_cost_usd);
    println!("  Tokens:  {}", session.total_tokens);
    if let Some(ref adr) = session.adr_path {
        println!("  ADR:     {}", adr);
    }
    if let Some(ref wp) = session.workplan_path {
        println!("  Plan:    {}", wp);
    }
    if let Some(ref dir) = session.output_dir {
        println!("  Dir:     {}", dir);
    }
    if !session.completed_steps.is_empty() {
        println!("  Steps:   {} completed", session.completed_steps.len());
    }
    if !session.tool_calls.is_empty() {
        println!("  Calls:   {} tool calls logged", session.tool_calls.len());
    }
    if let Some(ref qr) = session.quality_result {
        println!("  Quality: Grade {} (score {})", qr.grade, qr.score);
    }
    println!(
        "\n  {} This session is {}. Use `hex dev start` to create a new one.",
        "ℹ".blue(),
        session.status,
    );
}
