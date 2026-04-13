// Pre-existing clippy lints — tracked for cleanup in ADR-2603222050
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
pub(crate) mod nexus_client;
pub mod pipeline;
pub mod prompts;
pub mod session;
pub mod tui;

use commands::{
    adr::AdrAction,
    agent::AgentAction,
    brain::BrainAction,
    chat::ChatArgs,
    context::ContextAction,
    spec::SpecAction,
    analyze,
    dev::DevAction,
    doctor,
    git_cmd::GitAction,
    hook::HookEvent,
    inbox::InboxAction,
    init::InitArgs,
    memory::MemoryAction,
    neural_lab::NeuralLabAction,
    nexus::NexusAction,
    sandbox::SandboxAction,
    plan::PlanAction,
    fingerprint::FingerprintAction,
    project::ProjectAction,
    readme::ReadmeAction,
    secrets::SecretsAction,
    skill::SkillAction,
    stdb::StdbAction,
    status,
    swarm::SwarmAction,
    task::TaskAction,
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
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start/stop/manage the hex-nexus daemon
    #[command(alias = "daemon")]
    Nexus {
        #[command(subcommand)]
        action: NexusAction,
    },
    /// Manage remote agents (list, connect, spawn, disconnect)
    Agent {
        #[command(subcommand)]
        action: AgentAction,
    },
    /// Manage secrets and secret grants
    Secrets {
        #[command(subcommand)]
        action: SecretsAction,
    },
    /// Agentic Brain (self-improving model selection)
    Brain {
        #[command(subcommand)]
        action: BrainAction,
    },
    /// Manage local SpacetimeDB instance
    Stdb {
        #[command(subcommand)]
        action: StdbAction,
    },
    /// Swarm coordination
    Swarm {
        #[command(subcommand)]
        action: SwarmAction,
    },
    /// Task management
    Task {
        #[command(subcommand)]
        action: TaskAction,
    },
    /// Agent notification inbox (ADR-060)
    Inbox {
        #[command(subcommand)]
        action: InboxAction,
    },
    /// Persistent memory
    Memory {
        #[command(subcommand)]
        action: MemoryAction,
    },
    /// Neural architecture lab (experiment, mutate, evaluate model configs)
    #[command(name = "neural-lab")]
    NeuralLab {
        #[command(subcommand)]
        action: NeuralLabAction,
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
    /// Project registration and management
    Project {
        #[command(subcommand)]
        action: ProjectAction,
    },
    /// Architecture health check
    Analyze {
        /// Project root path
        #[arg(default_value = ".")]
        path: String,
        /// Promote warnings to errors (exit code 1 on any violation)
        #[arg(long)]
        strict: bool,
        /// Run only ADR compliance checks (skip boundary analysis)
        #[arg(long)]
        adr_compliance: bool,
        /// Output as JSON
        #[arg(long)]
        json: bool,
        /// Analyze a single file instead of the whole project
        #[arg(long, value_name = "PATH")]
        file: Option<String>,
        /// Suppress output when no violations found (for hook use)
        #[arg(long)]
        quiet: bool,
        /// Only print violation lines, skip summary stats
        #[arg(long)]
        violations_only: bool,
        /// Exit with code 1 if any violations found (for Stop hook gate)
        #[arg(long)]
        exit_code: bool,
    },
    /// Workplan management (create, list, status)
    Plan {
        #[command(subcommand)]
        action: PlanAction,
    },
    /// Manage inference providers (Ollama, vLLM, self-hosted)
    Inference {
        #[command(subcommand)]
        action: commands::inference::InferenceAction,
    },
    /// README specification management
    Readme {
        #[command(subcommand)]
        action: ReadmeAction,
    },
    /// Interactive AI chat session (TUI by default, --no-tui for plain stdout)
    Chat(ChatArgs),
    /// Initialize hex in a project directory
    Init(InitArgs),
    /// Claude Code hook handler (called by .claude/settings.json hooks)
    Hook {
        #[command(subcommand)]
        event: HookEvent,
    },
    /// Start the hex MCP server (stdio transport)
    Mcp,
    /// Run integration tests (unit, arch, services, swarm)
    Test {
        #[command(subcommand)]
        action: commands::test::TestAction,
    },
    /// Manage skills (list, sync, show)
    Skill {
        #[command(subcommand)]
        action: SkillAction,
    },
    /// Manage enforcement rules (ADR-2603221959)
    Enforce {
        #[command(subcommand)]
        action: commands::enforce::EnforceAction,
    },
    /// Inspect and sync embedded assets baked into the binary (ADR-2603221522)
    Assets {
        #[command(subcommand)]
        action: commands::assets_cmd::AssetsAction,
    },
    /// Git integration (status, log, diff, branches)
    Git {
        #[command(subcommand)]
        action: GitAction,
    },
    /// Project status
    Status,
    /// Inject hex context into opencode (ADR-2603231800)
    Opencode {
        #[command(subcommand)]
        action: commands::opencode::Commands,
    },
    /// Interactive TUI-driven development pipeline (ADR-2603232005)
    Dev {
        #[command(subcommand)]
        action: DevAction,
    },
    /// Developer audit report for hex dev sessions (ADR-2603232220)
    Report {
        #[command(subcommand)]
        action: commands::report::ReportAction,
    },
    /// Docker AI Sandbox management — build image, check readiness (ADR-2603282000)
    Sandbox {
        #[command(subcommand)]
        action: SandboxAction,
    },
    /// Architecture fingerprint management (ADR-2603301200)
    Fingerprint {
        #[command(subcommand)]
        action: FingerprintAction,
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
    /// Inspect and manage context engineering prompts
    Context {
        #[command(subcommand)]
        action: ContextAction,
    },
    /// Run full build pipeline (build → test → analyze → validate)
    Validate {
        /// Skip test phase
        #[arg(long)]
        skip_test: bool,
        /// Fail on warnings
        #[arg(long)]
        strict: bool,
        /// Run stages in parallel where possible
        #[arg(long)]
        parallel: bool,
    },
    /// Run all hex enforcement gates (ADR-2604061100)
    Ci {
        /// Run the standalone composition gate (ADR-2604112000)
        #[arg(long)]
        standalone_gate: bool,
    },
    /// Update hex to the latest release (ADR-2604080929)
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

    match cli.command {
        Commands::Nexus { action } => commands::nexus::run(action).await,
        Commands::Agent { action } => commands::agent::run(action).await,
        Commands::Secrets { action } => commands::secrets::run(action).await,
        Commands::Brain { action } => commands::brain::run(action).await,
        Commands::Stdb { action } => commands::stdb::run(action).await,
        Commands::Swarm { action } => commands::swarm::run(action).await,
        Commands::Task { action } => commands::task::run(action).await,
        Commands::Inbox { action } => commands::inbox::run(action).await,
        Commands::Memory { action } => commands::memory::run(action).await,
        Commands::NeuralLab { action } => commands::neural_lab::run(action).await,
        Commands::Adr { action } => commands::adr::run(action).await,
        Commands::Spec { action } => commands::spec::run(action).await,
        Commands::Project { action } => commands::project::run(action).await,
        Commands::Analyze { path, strict, adr_compliance, json, file, quiet, violations_only, exit_code } => {
            analyze::run(&path, strict, adr_compliance, json, file.as_deref(), quiet, violations_only, exit_code).await
        }
        Commands::Plan { action } => commands::plan::run(action).await,
        Commands::Inference { action } => commands::inference::run(action).await,
        Commands::Readme { action } => commands::readme::run(action).await,
        Commands::Chat(args) => commands::chat::run(args).await,
        Commands::Init(args) => commands::init::run(args).await,
        Commands::Hook { event } => commands::hook::run(event).await,
        Commands::Mcp => commands::mcp::run_mcp_server().await,
        Commands::Test { action } => commands::test::run(action).await,
        Commands::Skill { action } => commands::skill::run(action).await,
        Commands::Enforce { action } => commands::enforce::run(action).await,
        Commands::Git { action } => commands::git_cmd::run(action).await,
        Commands::Assets { action } => commands::assets_cmd::run(action).await,
        Commands::Status => status::run().await,
        Commands::Opencode { action } => commands::opencode::run(action),
        Commands::Dev { action } => commands::dev::run(action).await,
        Commands::Report { action } => commands::report::run(action).await,
        Commands::Sandbox { action } => commands::sandbox::run(action).await,
        Commands::Fingerprint { action } => commands::fingerprint::run(action).await,
        Commands::Doctor { verbose, fix, check } => {
            if check.as_deref() == Some("composition") {
                doctor::composition::run_composition_check().await;
                Ok(())
            } else {
                doctor::run_doctor(verbose, fix).await
            }
        }
        Commands::Context { action } => commands::context::run(action).await,
        Commands::Validate { skip_test, strict, parallel } => {
            doctor::run_validate_pipeline(skip_test, strict, parallel).await
        }
        Commands::Ci { standalone_gate } => {
            if standalone_gate {
                commands::ci::run_standalone_gate().await
            } else {
                commands::ci::run().await
            }
        }
        Commands::SelfUpdate { check, version, yes } => {
            commands::update::run(check, version, yes).await
        }
    }
}
