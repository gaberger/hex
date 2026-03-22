use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

mod commands;
mod nexus_client;

use commands::{
    adr::AdrAction,
    agent::AgentAction,
    analyze,
    hook::HookEvent,
    init::InitArgs,
    memory::MemoryAction,
    nexus::NexusAction,
    plan::PlanAction,
    project::ProjectAction,
    secrets::SecretsAction,
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
    /// Project status
    Status,
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
        Commands::Memory { action } => commands::memory::run(action).await,
        Commands::Adr { action } => commands::adr::run(action).await,
        Commands::Project { action } => commands::project::run(action).await,
        Commands::Analyze { path, strict, adr_compliance } => {
            analyze::run(&path, strict, adr_compliance).await
        }
        Commands::Plan { action } => commands::plan::run(action).await,
        Commands::Inference { action } => commands::inference::run(action).await,
        Commands::Init(args) => commands::init::run(args).await,
        Commands::Hook { event } => commands::hook::run(event).await,
        Commands::Mcp => commands::mcp::run_mcp_server().await,
        Commands::Test { action } => commands::test::run(action).await,
        Commands::Status => status::run().await,
    }
}
