use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

mod commands;
mod nexus_client;

use commands::{
    adr::AdrAction,
    analyze,
    memory::MemoryAction,
    nexus::NexusAction,
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
    Nexus {
        #[command(subcommand)]
        action: NexusAction,
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
    /// Architecture health check
    Analyze {
        /// Project root path
        #[arg(default_value = ".")]
        path: String,
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
        Commands::Secrets { action } => commands::secrets::run(action).await,
        Commands::Stdb { action } => commands::stdb::run(action).await,
        Commands::Swarm { action } => commands::swarm::run(action).await,
        Commands::Task { action } => commands::task::run(action).await,
        Commands::Memory { action } => commands::memory::run(action).await,
        Commands::Adr { action } => commands::adr::run(action).await,
        Commands::Analyze { path } => analyze::run(&path).await,
        Commands::Mcp => commands::mcp::run_mcp_server().await,
        Commands::Test { action } => commands::test::run(action).await,
        Commands::Status => status::run().await,
    }
}
