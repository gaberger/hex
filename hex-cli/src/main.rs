// Pre-existing clippy lints — tracked for cleanup in ADR-2026-03-22-2050
#![allow(
    clippy::manual_strip,
    clippy::ptr_arg,
    clippy::unnecessary_sort_by,
    clippy::literal_string_with_formatting_args
)]
use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

pub mod assets;
mod commands;
pub mod fmt;

use commands::{
    adr::AdrAction,
    bootstrap::BootstrapArgs,
    spec::SpecAction,
    analyze,
    doctor,
    hook::HookEvent,
    insight::InsightAction,
    init::InitArgs,
    refresh::RefreshArgs,
    memory::MemoryAction,
    plan::PlanAction,
    skill::SkillAction,
    status,
    worktree::WorktreeAction,
    hey::HeyArgs,
};

#[derive(Parser)]
#[command(
    name = "hex",
    version,
    about = "Hexagonal architecture for LLM-driven development"
)]
struct Cli {
    /// Enable verbose logging
    #[arg(short, long, global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

// ── P2: hex config — groups trust, taste, inference, enforce, secrets ──
#[derive(Subcommand)]
enum ConfigAction {    /// Manage inference providers (Ollama, vLLM, self-hosted)
    Inference {
        #[command(subcommand)]
        action: commands::inference::InferenceAction,
    },
}

// ── P3: hex dev — groups analyze, validate, test, ci, worktree, init, new, report + session ──
#[derive(Subcommand)]
enum DevGroupAction {
    /// Architecture health check
    Analyze {
        /// Project root path
        #[arg(default_value = ".")]
        path: String,
        #[arg(long)]
        strict: bool,
        #[arg(long)]
        adr_compliance: bool,
        #[arg(long)]
        json: bool,
        #[arg(long, value_name = "PATH")]
        file: Option<String>,
        #[arg(long)]
        quiet: bool,
        #[arg(long)]
        violations_only: bool,
        #[arg(long)]
        exit_code: bool,
    },
    /// Run full build pipeline (build → test → analyze → validate)
    Validate {
        #[arg(long)]
        skip_test: bool,
        #[arg(long)]
        strict: bool,
        #[arg(long)]
        parallel: bool,
    },
    /// Run integration tests (unit, lint, arch, inference)
    Test {
        #[command(subcommand)]
        action: commands::test::TestAction,
    },
    /// Run all hex enforcement gates
    Ci {
        #[arg(long)]
        standalone_gate: bool,
    },
    /// Git worktree management (list, merge, cleanup)
    Worktree {
        #[command(subcommand)]
        action: WorktreeAction,
    },
    /// Initialize hex in a project directory
    Init(InitArgs),
    /// Refresh hex-managed sections of CLAUDE.md in place (no interview, no reset)
    Refresh(RefreshArgs),
    /// Structured project intake — create, init, register, seed trust
    New {
        /// Target directory path
        path: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        description: Option<String>,
        /// Scaffold language: rust | go | ts
        #[arg(long, default_value = "rust")]
        lang: String,
    },
}

#[derive(Subcommand)]
enum Commands {
    // ════════════════════════════════════════════════════════════════════
    // Grouped parent commands (P2/P3/P4)
    // ════════════════════════════════════════════════════════════════════

    /// Project configuration (inference providers and model tiers)
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// Development tools (analyze, validate, test, ci, worktree, init, new, refresh)
    Dev {
        #[command(subcommand)]
        action: DevGroupAction,
    },

    // ════════════════════════════════════════════════════════════════════
    // Standalone commands (not grouped)
    // ════════════════════════════════════════════════════════════════════

    /// Bootstrap hex environment (prerequisites, services, models, config)
    Bootstrap(BootstrapArgs),
    /// Refresh hex-managed sections of CLAUDE.md in place (no interview, no reset)
    Refresh(RefreshArgs),
    /// Do the next right thing — check project health and suggest/execute actions
    Go,
    /// Knowledge graph — build/query/path/explain a project's code+docs graph
    Graph {
        #[command(subcommand)]
        action: commands::graph::GraphAction,
    },
    /// Hey Hex — natural language task classifier (ADR-2026-04-14-0000)
    Hey(HeyArgs),
    /// Adversarially verify a claim about the repo — returns CONFIRMED / REFUTED / INCONCLUSIVE
    Verify(commands::verify::VerifyArgs),
    /// Direct executor — task → one agent → evidence → commit (ADR-2026-06-04-1740 Path A)
    #[command(name = "do")]
    Do {
        #[command(subcommand)]
        action: commands::direct::DoAction,
    },
    /// Agentic inference benchmarks — run the corpus through the loop in isolated worktrees (ADR-2606071734)
    Bench {
        #[command(subcommand)]
        action: commands::bench::BenchAction,
    },
    /// Cooperative build — diverge, red-team, synthesize, then build to a gate
    Build(commands::build::BuildArgs),
    /// Adversarial pass — hunt a target for bugs, verify each, fix under a gate
    Harden(commands::build::HardenArgs),
    /// Insight extraction surfaces (punch-list, gap detection)
    Insight {
        #[command(subcommand)]
        action: InsightAction,
    },
    /// Persistent memory
    Memory {
        #[command(subcommand)]
        action: MemoryAction,
    },
    /// Architecture Decision Records
    Adr {
        #[command(subcommand)]
        action: AdrAction,
    },
    /// Behavioral specs (docs/specs/)
    Spec {
        #[command(subcommand)]
        action: SpecAction,
    },
    /// Workplan management (create, list, status)
    Plan {
        #[command(subcommand)]
        action: PlanAction,
    },
    /// Claude Code hook handler (called by .claude/settings.json hooks)
    Hook {
        #[command(subcommand)]
        event: HookEvent,
    },
    /// Manage skills (list, sync, show)
    Skill {
        #[command(subcommand)]
        action: SkillAction,
    },
    /// Inspect and sync embedded assets baked into the binary (ADR-2026-03-22-1522)
    Assets {
        #[command(subcommand)]
        action: commands::assets_cmd::AssetsAction,
    },
    /// Project status
    Status,
    /// Internal documentation health (ADR-047) — terminology, freshness, module READMEs
    Docs {
        #[command(subcommand)]
        action: commands::docs::DocsAction,
    },
    /// Installation verification and pipeline validation (ADR-067)
    Doctor {
        /// Show detailed output
        #[arg(long, short)]
        verbose: bool,
        /// Attempt to fix issues automatically
        #[arg(long, short)]
        fix: bool,
        /// Run a specific check only (e.g. "composition")
        #[arg(value_name = "CHECK")]
        check: Option<String>,
    },
    /// Update hex to the latest release (ADR-2026-04-08-0929)
    #[command(name = "self-update")]
    SelfUpdate {
        /// Only check for updates, do not install
        #[arg(long)]
        check: bool,
        /// Install a specific version tag (e.g. v26.4.30)
        #[arg(long)]
        version: Option<String>,
        /// Skip confirmation prompt
        #[arg(long, short)]
        yes: bool,
    },

    // ════════════════════════════════════════════════════════════════════
    // Hidden aliases — old top-level commands still work but don't show in --help
    // ════════════════════════════════════════════════════════════════════
    /// (hidden) Manage inference — use `hex config inference` instead
    #[command(hide = true)]
    Inference {
        #[command(subcommand)]
        action: commands::inference::InferenceAction,
    },
    /// (hidden) Architecture health check — use `hex dev analyze` instead
    #[command(hide = true)]
    Analyze {
        #[arg(default_value = ".")]
        path: String,
        #[arg(long)]
        strict: bool,
        #[arg(long)]
        adr_compliance: bool,
        #[arg(long)]
        json: bool,
        #[arg(long, value_name = "PATH")]
        file: Option<String>,
        #[arg(long)]
        quiet: bool,
        #[arg(long)]
        violations_only: bool,
        #[arg(long)]
        exit_code: bool,
    },
    /// (hidden) Validate — use `hex dev validate` instead
    #[command(hide = true)]
    Validate {
        #[arg(long)]
        skip_test: bool,
        #[arg(long)]
        strict: bool,
        #[arg(long)]
        parallel: bool,
    },
    /// (hidden) Run tests — use `hex dev test` instead
    #[command(hide = true)]
    Test {
        #[command(subcommand)]
        action: commands::test::TestAction,
    },
    /// (hidden) CI gates — use `hex dev ci` instead
    #[command(hide = true)]
    Ci {
        #[arg(long)]
        standalone_gate: bool,
    },
    /// (hidden) Worktree management — use `hex dev worktree` instead
    #[command(hide = true)]
    Worktree {
        #[command(subcommand)]
        action: WorktreeAction,
    },
    /// (hidden) Initialize hex — use `hex dev init` instead
    #[command(hide = true)]
    Init(InitArgs),
    /// (hidden) New project — use `hex dev new` instead
    #[command(hide = true)]
    New {
        path: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        description: Option<String>,
        /// Scaffold language: rust | go | ts
        #[arg(long, default_value = "rust")]
        lang: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let filter = if cli.verbose {
        EnvFilter::new("debug")
    } else {
        EnvFilter::new("info")
    };

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();

    let command = match cli.command {
        Some(cmd) => cmd,
        None => {
            // hex with no args → status + getting-started guide (Layer 1)
            commands::status::run().await?;
            print_getting_started();
            return Ok(());
        }
    };

    match command {
        // ── Grouped parent commands (P2/P3/P4) ──────────────────────
        Commands::Config { action } => match action {
            ConfigAction::Inference { action } => commands::inference::run(action).await,
        },
        Commands::Dev { action } => match action {
            DevGroupAction::Analyze { path, strict, adr_compliance, json, file, quiet, violations_only, exit_code } => {
                analyze::run(&path, strict, adr_compliance, json, file.as_deref(), quiet, violations_only, exit_code).await
            }
            DevGroupAction::Validate { skip_test, strict, parallel } => {
                doctor::run_validate_pipeline(skip_test, strict, parallel).await
            }
            DevGroupAction::Test { action } => commands::test::run(action).await,
            DevGroupAction::Ci { standalone_gate } => {
                if standalone_gate { commands::ci::run_standalone_gate().await }
                else { commands::ci::run().await }
            }
            DevGroupAction::Worktree { action } => commands::worktree::run(action).await,
            DevGroupAction::Init(args) => commands::init::run(args).await,
            DevGroupAction::Refresh(args) => commands::refresh::run(args).await,
            DevGroupAction::New { path, name, description, lang } => {
                commands::new::run(&path, name, description, &lang).await
            }
        },
        // ── Standalone commands ──────────────────────────────────────
        Commands::Bootstrap(args) => commands::bootstrap::run(args).await,
        Commands::Go => commands::go::run().await,
        Commands::Graph { action } => commands::graph::run(action).await,
        Commands::Hey(args) => commands::hey::run(args).await,
        Commands::Verify(args) => commands::verify::run(args).await,
        Commands::Do { action } => commands::direct::run(action).await,
        Commands::Bench { action } => commands::bench::run(action).await,
        Commands::Build(args) => commands::build::run_build(args).await,
        Commands::Harden(args) => commands::build::run_harden(args).await,
        Commands::Insight { action } => commands::insight::run(action).await,
        Commands::Memory { action } => commands::memory::run(action).await,
        Commands::Adr { action } => commands::adr::run(action).await,
        Commands::Spec { action } => commands::spec::run(action).await,
        Commands::Plan { action } => commands::plan::run(action).await,
        Commands::Hook { event } => commands::hook::run(event).await,
        Commands::Skill { action } => commands::skill::run(action).await,
        Commands::Assets { action } => commands::assets_cmd::run(action).await,
        Commands::Status => status::run().await,
        Commands::Docs { action } => commands::docs::run(action).await,
        Commands::Doctor { verbose, fix, check } => {
            match check.as_deref() {
                Some("composition") => {
                    doctor::composition::run_composition_check().await;
                    Ok(())
                }
                _ => doctor::run_doctor(verbose, fix).await,
            }
        }
        Commands::SelfUpdate { check, version, yes } => {
            commands::update::run(check, version, yes).await
        }
        // ── Hidden aliases (old top-level commands) ──────────────────
        Commands::Inference { action } => commands::inference::run(action).await,
        Commands::Analyze { path, strict, adr_compliance, json, file, quiet, violations_only, exit_code } => {
            analyze::run(&path, strict, adr_compliance, json, file.as_deref(), quiet, violations_only, exit_code).await
        }
        Commands::Validate { skip_test, strict, parallel } => {
            doctor::run_validate_pipeline(skip_test, strict, parallel).await
        }
        Commands::Test { action } => commands::test::run(action).await,
        Commands::Ci { standalone_gate } => {
            if standalone_gate { commands::ci::run_standalone_gate().await }
            else { commands::ci::run().await }
        }
        Commands::Worktree { action } => commands::worktree::run(action).await,
        Commands::Init(args) => commands::init::run(args).await,
        Commands::Refresh(args) => commands::refresh::run(args).await,
        Commands::New { path, name, description, lang } => {
            commands::new::run(&path, name, description, &lang).await
        }
    }
}

/// Getting-started guide printed after `hex` with no args.
/// Progressive disclosure: shows the 5 commands a new user needs.
fn print_getting_started() {
    use colored::Colorize;
    println!();
    println!("{}", "  Getting started".bold());
    println!();
    println!("    {}              Do the next right thing (autonomous)", "hex go".cyan());
    println!("    {}           Structured brief of recent activity", "hex brief".cyan());
    println!("    {}      Workplan lifecycle (create, execute, status)", "hex plan list".cyan());
    println!("    {}          Configure trust, taste, inference", "hex config".cyan());
    println!("    {}             Development tools (analyze, test, ci)", "hex dev".cyan());
    println!("    {}          System health check", "hex doctor".cyan());
    println!();
    println!("    {} for full command list", "hex --help".dimmed());
    println!();
}
