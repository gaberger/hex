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
    analyze,
    dev::DevAction,
    git_cmd::GitAction,
    hook::HookEvent,
    inbox::InboxAction,
    init::InitArgs,
    memory::MemoryAction,
    neural_lab::NeuralLabAction,
    nexus::NexusAction,
    plan::PlanAction,
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
    /// Inspect embedded assets baked into the binary (ADR-2603221522)
    Assets,
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
        Commands::Stdb { action } => commands::stdb::run(action).await,
        Commands::Swarm { action } => commands::swarm::run(action).await,
        Commands::Task { action } => commands::task::run(action).await,
        Commands::Inbox { action } => commands::inbox::run(action).await,
        Commands::Memory { action } => commands::memory::run(action).await,
        Commands::NeuralLab { action } => commands::neural_lab::run(action).await,
        Commands::Adr { action } => commands::adr::run(action).await,
        Commands::Project { action } => commands::project::run(action).await,
        Commands::Analyze { path, strict, adr_compliance, json } => {
            analyze::run(&path, strict, adr_compliance, json).await
        }
        Commands::Plan { action } => commands::plan::run(action).await,
        Commands::Inference { action } => commands::inference::run(action).await,
        Commands::Readme { action } => commands::readme::run(action).await,
        Commands::Init(args) => commands::init::run(args).await,
        Commands::Hook { event } => commands::hook::run(event).await,
        Commands::Mcp => commands::mcp::run_mcp_server().await,
        Commands::Test { action } => commands::test::run(action).await,
        Commands::Skill { action } => commands::skill::run(action).await,
        Commands::Enforce { action } => commands::enforce::run(action).await,
        Commands::Git { action } => commands::git_cmd::run(action).await,
        Commands::Assets => commands::assets_cmd::list().await,
        Commands::Status => status::run().await,
        Commands::Opencode { action } => commands::opencode::run(action),
        Commands::Dev { action } => commands::dev::run(action).await,
        Commands::Report { action } => commands::report::run(action).await,
    }
}
