use clap::Args;
use colored::Colorize;
use serde::{Deserialize, Serialize};

mod prereq;
mod services;
mod models;
mod config;
mod validate;

use prereq::PrereqChecker;
use services::ServiceStarter;
use models::ModelLoader;
use config::ConfigSetup;
use validate::BootstrapValidator;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BootstrapProfile {
    Dev,
    Ci,
    Prod,
}

impl BootstrapProfile {
    fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "dev" => Some(BootstrapProfile::Dev),
            "ci" => Some(BootstrapProfile::Ci),
            "prod" => Some(BootstrapProfile::Prod),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapConfig {
    pub profile: String,
    pub skip_models: bool,
    pub skip_prereq: bool,
    pub force: bool,
    pub dry_run: bool,
}

impl Default for BootstrapConfig {
    fn default() -> Self {
        Self {
            profile: "dev".to_string(),
            skip_models: false,
            skip_prereq: false,
            force: false,
            dry_run: false,
        }
    }
}

#[derive(Debug, Args)]
pub struct BootstrapArgs {
    /// Bootstrap profile: dev (local Ollama), ci (Claude API), prod (remote)
    #[arg(long, default_value = "dev")]
    pub profile: String,

    /// Skip model downloading
    #[arg(long)]
    pub skip_models: bool,

    /// Skip OS prerequisite checks (for CI)
    #[arg(long)]
    pub skip_prereq: bool,

    /// Force restart services even if running
    #[arg(long)]
    pub force: bool,

    /// Show what would be done without side effects
    #[arg(long)]
    pub dry_run: bool,
}

pub async fn run(args: BootstrapArgs) -> anyhow::Result<()> {
    let config = BootstrapConfig {
        profile: args.profile.clone(),
        skip_models: args.skip_models,
        skip_prereq: args.skip_prereq,
        force: args.force,
        dry_run: args.dry_run,
    };

    if args.dry_run {
        println!("{}", "🔍 DRY RUN MODE — No changes will be made".yellow());
        println!();
    }

    // Phase 1: Check prerequisites
    if !config.skip_prereq {
        println!("{}", "⬡ Checking prerequisites...".cyan());
        let checker = PrereqChecker::new();
        let report = checker.check_all().await?;

        if !report.all_ok() {
            println!("{}", report.format());
            if !config.dry_run {
                return Err(anyhow::anyhow!("Prerequisites check failed"));
            }
        } else {
            println!("{}", "✓ All prerequisites met".green());
        }
        println!();
    }

    // Phase 2: Start services in parallel
    println!("{}", "⬡ Starting services...".cyan());
    let starter = ServiceStarter::new(config.force, config.dry_run);
    let service_status = starter.start_all().await?;

    for status in &service_status {
        let icon = if status.running { "✓".green() } else { "✗".red() };
        println!(
            "  {} {} {}",
            icon,
            status.name,
            if let Some(pid) = status.pid {
                format!("(PID {})", pid)
            } else {
                String::new()
            }
        );
    }
    println!();

    // Phase 3: Load models
    if !config.skip_models {
        println!("{}", "⬡ Loading inference models...".cyan());
        let loader = ModelLoader::new(config.dry_run);
        let model_status = loader.load_default_models().await?;

        for status in &model_status {
            let icon = if status.loaded { "✓".green() } else { "✗".red() };
            println!(
                "  {} {} ({:.1} GB)",
                icon, status.name, status.size_mb as f64 / 1024.0
            );
        }
        println!();
    }

    // Phase 4: Configure project
    println!("{}", "⬡ Setting up configuration...".cyan());
    let configurator = ConfigSetup::new(config.clone());
    configurator.setup().await?;
    println!("{}", "✓ Configuration created".green());
    println!();

    // Phase 5: Validate
    println!("{}", "⬡ Validating bootstrap...".cyan());
    let validator = BootstrapValidator::new();
    let report = validator.validate_all().await?;

    if report.ready {
        println!("{}", report.format_success());
    } else {
        println!("{}", report.format_warning());
        if !config.dry_run {
            return Err(anyhow::anyhow!("Bootstrap validation failed"));
        }
    }

    Ok(())
}
