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
    brief::BriefArgs,
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
    secrets::SecretsAction,
    skill::SkillAction,
    stdb::StdbAction,
    status,
    swarm::SwarmAction,
    task::TaskAction,
    worktree::WorktreeAction,
    decide::DecideAction,
    pause::PauseAction,
    taste::TasteAction,
    trust::TrustAction,
    steer::SteerAction,
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
enum ConfigAction {
    /// Manage delegation trust levels per scope
    Trust {
        #[command(subcommand)]
        action: TrustAction,
    },
    /// Manage developer taste preferences
    Taste {
        #[command(subcommand)]
        action: TasteAction,
    },
    /// Manage inference providers (Ollama, vLLM, self-hosted)
    Inference {
        #[command(subcommand)]
        action: commands::inference::InferenceAction,
    },
    /// Manage enforcement rules
    Enforce {
        #[command(subcommand)]
        action: commands::enforce::EnforceAction,
    },
    /// Manage secrets and secret grants
    Secrets {
        #[command(subcommand)]
        action: SecretsAction,
    },
}

// ── P3: hex dev — groups analyze, validate, test, ci, worktree, init, new, report + session ──
#[derive(Subcommand)]
enum DevGroupAction {
    /// Start/resume/list dev sessions (TUI pipeline)
    Session {
        #[command(subcommand)]
        action: DevAction,
    },
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
    /// Run integration tests (unit, arch, services, swarm)
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
    /// Structured project intake — create, init, register, seed trust
    New {
        /// Target directory path
        path: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        description: Option<String>,
        #[arg(long)]
        taste_from: Option<String>,
    },
    /// Developer audit report for hex dev sessions
    Report {
        #[command(subcommand)]
        action: commands::report::ReportAction,
    },
}

// ── P4: hex override — groups steer, pause, decide ──
#[derive(Subcommand)]
enum OverrideAction {
    /// Send natural-language directives to a project
    Steer {
        #[command(subcommand)]
        action: SteerAction,
    },
    /// Emergency pause/resume the active workplan
    Pause {
        #[command(subcommand)]
        action: PauseAction,
    },
    /// Resolve, approve, or explain pending project decisions
    Decide {
        #[command(subcommand)]
        action: DecideAction,
    },
}

#[derive(Subcommand)]
enum Commands {
    // ════════════════════════════════════════════════════════════════════
    // Grouped parent commands (P2/P3/P4)
    // ════════════════════════════════════════════════════════════════════

    /// Project configuration (trust, taste, inference, enforce, secrets)
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// Development tools (analyze, validate, test, ci, worktree, init, new, report, session)
    Dev {
        #[command(subcommand)]
        action: DevGroupAction,
    },
    /// Emergency overrides (steer, pause, decide)
    #[command(name = "override")]
    OverrideCmd {
        #[command(subcommand)]
        action: OverrideAction,
    },

    // ════════════════════════════════════════════════════════════════════
    // Standalone commands (not grouped)
    // ════════════════════════════════════════════════════════════════════

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
    /// Developer briefing — recent events, decisions, health
    Brief(BriefArgs),
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
    /// Workplan management (create, list, status)
    Plan {
        #[command(subcommand)]
        action: PlanAction,
    },
    /// Interactive AI chat session (TUI by default, --no-tui for plain stdout)
    Chat(ChatArgs),
    /// Claude Code hook handler (called by .claude/settings.json hooks)
    Hook {
        #[command(subcommand)]
        event: HookEvent,
    },
    /// Start the hex MCP server (stdio transport)
    Mcp,
    /// Manage skills (list, sync, show)
    Skill {
        #[command(subcommand)]
        action: SkillAction,
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
    /// Emergency override — send priority-2 directive to all agents (ADR-2604131500)
    #[command(name = "send-override")]
    OverrideDirect {
        /// Project name
        project: String,
        /// Override instruction (natural language)
        instruction: String,
    },

    // ════════════════════════════════════════════════════════════════════
    // Hidden aliases — old top-level commands still work but don't show in --help
    // ════════════════════════════════════════════════════════════════════

    /// (hidden) Manage secrets — use `hex config secrets` instead
    #[command(hide = true)]
    Secrets {
        #[command(subcommand)]
        action: SecretsAction,
    },
    /// (hidden) Manage inference — use `hex config inference` instead
    #[command(hide = true)]
    Inference {
        #[command(subcommand)]
        action: commands::inference::InferenceAction,
    },
    /// (hidden) Manage enforcement — use `hex config enforce` instead
    #[command(hide = true)]
    Enforce {
        #[command(subcommand)]
        action: commands::enforce::EnforceAction,
    },
    /// (hidden) Manage trust — use `hex config trust` instead
    #[command(hide = true)]
    Trust {
        #[command(subcommand)]
        action: TrustAction,
    },
    /// (hidden) Manage taste — use `hex config taste` instead
    #[command(hide = true)]
    Taste {
        #[command(subcommand)]
        action: TasteAction,
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
        #[arg(long)]
        taste_from: Option<String>,
    },
    /// (hidden) Report — use `hex dev report` instead
    #[command(hide = true)]
    Report {
        #[command(subcommand)]
        action: commands::report::ReportAction,
    },
    /// (hidden) Steer — use `hex override steer` instead
    #[command(hide = true)]
    Steer {
        #[command(subcommand)]
        action: SteerAction,
    },
    /// (hidden) Pause — use `hex override pause` instead
    #[command(hide = true)]
    Pause {
        #[command(subcommand)]
        action: PauseAction,
    },
    /// (hidden) Decide — use `hex override decide` instead
    #[command(hide = true)]
    Decide {
        #[command(subcommand)]
        action: DecideAction,
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
            // hex with no args → pulse (Layer 1: one-glance status)
            return commands::status::run().await;
        }
    };

    match command {
        // ── Grouped parent commands (P2/P3/P4) ──────────────────────
        Commands::Config { action } => match action {
            ConfigAction::Trust { action } => commands::trust::run(action).await,
            ConfigAction::Taste { action } => commands::taste::run(action).await,
            ConfigAction::Inference { action } => commands::inference::run(action).await,
            ConfigAction::Enforce { action } => commands::enforce::run(action).await,
            ConfigAction::Secrets { action } => commands::secrets::run(action).await,
        },
        Commands::Dev { action } => match action {
            DevGroupAction::Session { action } => commands::dev::run(action).await,
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
            DevGroupAction::New { path, name, description, taste_from } => {
                commands::new::run(&path, name, description, taste_from).await
            }
            DevGroupAction::Report { action } => commands::report::run(action).await,
        },
        Commands::OverrideCmd { action } => match action {
            OverrideAction::Steer { action } => commands::steer::run(action).await,
            OverrideAction::Pause { action } => {
                match action {
                    PauseAction::Pause => commands::pause::run_pause().await,
                    PauseAction::Resume => commands::pause::run_resume().await,
                }
            }
            OverrideAction::Decide { action } => commands::decide::run(action).await,
        },

        // ── Standalone commands ──────────────────────────────────────
        Commands::Nexus { action } => commands::nexus::run(action).await,
        Commands::Agent { action } => commands::agent::run(action).await,
        Commands::Brief(args) => commands::brief::run(args).await,
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
        Commands::Plan { action } => commands::plan::run(action).await,
        Commands::Chat(args) => commands::chat::run(args).await,
        Commands::Hook { event } => commands::hook::run(event).await,
        Commands::Mcp => commands::mcp::run_mcp_server().await,
        Commands::Skill { action } => commands::skill::run(action).await,
        Commands::Assets { action } => commands::assets_cmd::run(action).await,
        Commands::Git { action } => commands::git_cmd::run(action).await,
        Commands::Status => status::run().await,
        Commands::Opencode { action } => commands::opencode::run(action),
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
        Commands::SelfUpdate { check, version, yes } => {
            commands::update::run(check, version, yes).await
        }
        Commands::OverrideDirect { project, instruction } => {
            commands::override_cmd::run(&project, &instruction).await
        }

        // ── Hidden aliases (old top-level commands) ──────────────────
        Commands::Secrets { action } => commands::secrets::run(action).await,
        Commands::Inference { action } => commands::inference::run(action).await,
        Commands::Enforce { action } => commands::enforce::run(action).await,
        Commands::Trust { action } => commands::trust::run(action).await,
        Commands::Taste { action } => commands::taste::run(action).await,
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
        Commands::New { path, name, description, taste_from } => {
            commands::new::run(&path, name, description, taste_from).await
        }
        Commands::Report { action } => commands::report::run(action).await,
        Commands::Steer { action } => commands::steer::run(action).await,
        Commands::Pause { action } => {
            match action {
                PauseAction::Pause => commands::pause::run_pause().await,
                PauseAction::Resume => commands::pause::run_resume().await,
            }
        }
        Commands::Decide { action } => commands::decide::run(action).await,
    }
}
