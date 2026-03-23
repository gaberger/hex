//! `hex dev` — interactive TUI-driven development pipeline (ADR-2603232005).
//!
//! Entry point that creates or resumes a DevSession, then launches the
//! ratatui TUI for the full ADR → Plan → Code → Validate → Commit pipeline.

use anyhow::{bail, Result};
use clap::Subcommand;
use colored::Colorize;

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
        } => {
            start_session(description, quick, auto, dry_run, model, provider, budget).await
        }
    }
}

fn list_sessions() -> Result<()> {
    let sessions = DevSession::list_all()?;
    if sessions.is_empty() {
        println!("{}", "No dev sessions found.".dimmed());
        return Ok(());
    }
    println!(
        "{:<36}  {:<12}  {:<10}  {:<8}  {}",
        "ID".bold(),
        "Status".bold(),
        "Phase".bold(),
        "Cost".bold(),
        "Feature".bold(),
    );
    for s in &sessions {
        let status_colored = match s.status {
            SessionStatus::InProgress => s.status.to_string().yellow().to_string(),
            SessionStatus::Paused => s.status.to_string().blue().to_string(),
            SessionStatus::Completed => s.status.to_string().green().to_string(),
            SessionStatus::Failed => s.status.to_string().red().to_string(),
        };
        println!(
            "{:<36}  {:<12}  {:<10}  ${:<7.2}  {}",
            s.id.dimmed(),
            status_colored,
            s.current_phase,
            s.total_cost_usd,
            s.feature_description,
        );
    }
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
    let session = DevSession::load_latest()?;
    match session {
        Some(s) => {
            let config = DevConfig::from_args(
                s.feature_description.clone(),
                false, false, false,
                "".into(),
                "openrouter".into(),
                0.0,
            );
            launch_tui(s, config).await
        }
        None => {
            bail!("No in-progress or paused session found. Use `hex dev start <description>` to begin.");
        }
    }
}

async fn resume_by_id(id: &str) -> Result<()> {
    let session = DevSession::load(id)?;
    let config = DevConfig::from_args(
        session.feature_description.clone(),
        false, false, false,
        "".into(),
        "openrouter".into(),
        0.0,
    );
    launch_tui(session, config).await
}

async fn start_session(
    description: String,
    quick: bool,
    auto: bool,
    dry_run: bool,
    model: String,
    provider: String,
    budget: f64,
) -> Result<()> {
    let config = DevConfig::from_args(
        description.clone(),
        quick,
        auto,
        dry_run,
        model.clone(),
        provider,
        budget,
    );

    let mut session = DevSession::new(&description);
    if !model.is_empty() {
        session.model_selections.insert("default".into(), model);
    }
    session.save()?;

    println!(
        "{} Created session {} for: {} [mode: {}]",
        "✓".green(),
        session.id.dimmed(),
        description.bold(),
        config.mode,
    );

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

    let app = TuiApp::with_config(session, config);
    app.run()?;
    Ok(())
}
