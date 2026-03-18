mod domain;
mod ports;
mod adapters;
mod usecases;

use clap::Parser;
use std::path::PathBuf;
use std::sync::Arc;

use adapters::primary::cli::CliAdapter;
use adapters::secondary::anthropic::AnthropicAdapter;
use adapters::secondary::context_manager::ContextManagerAdapter;
use adapters::secondary::tools::ToolExecutorAdapter;
use adapters::secondary::skill_loader::SkillLoaderAdapter;
use adapters::secondary::agent_loader::AgentLoaderAdapter;
use domain::{TokenBudget, tools::builtin_tools};
use ports::skills::SkillLoaderPort;
use ports::agents::AgentLoaderPort;
use usecases::context_packer::ContextPacker;
use usecases::conversation::ConversationLoop;

#[derive(Parser, Debug)]
#[command(name = "hex-agent", version, about = "Autonomous AI agent for hex architecture")]
struct Args {
    /// Anthropic API model to use
    #[arg(long, default_value = "claude-sonnet-4-20250514")]
    model: String,

    /// Project directory to operate in
    #[arg(long, default_value = ".")]
    project_dir: String,

    /// Agent definition to load (by name)
    #[arg(long)]
    agent: Option<String>,

    /// Hex-hub WebSocket URL to connect back to (when spawned by hub)
    #[arg(long)]
    hub_url: Option<String>,

    /// Auth token for hub connection
    #[arg(long)]
    hub_token: Option<String>,

    /// Max context window tokens
    #[arg(long, default_value = "200000")]
    max_context: u32,

    /// Max tokens for model response
    #[arg(long, default_value = "8192")]
    max_response: u32,

    /// Enable verbose logging
    #[arg(long, short)]
    verbose: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    // Initialize tracing
    let filter = if args.verbose { "debug" } else { "info" };
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .init();

    tracing::info!(
        model = %args.model,
        project_dir = %args.project_dir,
        "hex-agent starting"
    );

    // --- Composition Root: wire adapters to ports ---

    // Resolve API key
    let api_key = std::env::var("ANTHROPIC_API_KEY").unwrap_or_else(|_| {
        eprintln!("\x1b[31mError: ANTHROPIC_API_KEY environment variable not set\x1b[0m");
        std::process::exit(1);
    });

    let project_dir = PathBuf::from(&args.project_dir)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(&args.project_dir));

    // Secondary adapters
    let anthropic: Arc<dyn ports::AnthropicPort> =
        Arc::new(AnthropicAdapter::new(api_key, args.model.clone()));
    let context_mgr: Arc<dyn ports::ContextManagerPort> =
        Arc::new(ContextManagerAdapter::new());
    let tool_executor: Arc<dyn ports::ToolExecutorPort> =
        Arc::new(ToolExecutorAdapter::new(project_dir.clone()));

    // Load skills and agent definitions
    let skill_loader = SkillLoaderAdapter::new();
    let agent_loader = AgentLoaderAdapter::new();

    let skill_dirs = vec![
        format!("{}/.claude/skills", project_dir.display()),
        format!("{}/skills", project_dir.display()),
    ];
    let skill_dir_refs: Vec<&str> = skill_dirs.iter().map(|s| s.as_str()).collect();
    let skills = skill_loader.load(&skill_dir_refs).await.unwrap_or_default();

    let agent_def = if let Some(agent_name) = &args.agent {
        let agent_dirs = vec![
            format!("{}/.claude/agents", project_dir.display()),
            format!("{}/agents", project_dir.display()),
        ];
        let agent_dir_refs: Vec<&str> = agent_dirs.iter().map(|s| s.as_str()).collect();
        match agent_loader.load_by_name(&agent_dir_refs, agent_name).await {
            Ok(def) => {
                tracing::info!(agent = %def.name, "Loaded agent definition");
                Some(def)
            }
            Err(e) => {
                tracing::warn!("Agent '{}' not found: {}", agent_name, e);
                None
            }
        }
    } else {
        None
    };

    // Build token budget
    let budget = TokenBudget::for_model(args.max_context);

    // Build system prompt
    let system_prompt = ContextPacker::build_system_prompt(
        &project_dir.to_string_lossy(),
        agent_def.as_ref(),
        &skills,
        None, // No active workplan in interactive mode
    )
    .await;

    tracing::info!(
        system_tokens = context_mgr.count_tokens(&system_prompt),
        skills = skills.skills.len(),
        "Context assembled"
    );

    // Build conversation loop (use case)
    let tools = builtin_tools();
    let conversation = ConversationLoop::new(
        anthropic,
        context_mgr,
        tool_executor,
        tools,
        budget,
        args.max_response,
    );

    // Decide mode: hub-managed or interactive CLI
    if let Some(_hub_url) = &args.hub_url {
        // TODO: Hub-managed mode (Phase P5)
        // Connect via WebSocket, receive commands, stream output back
        eprintln!("Hub-managed mode not yet implemented. Use interactive mode.");
        std::process::exit(1);
    }

    // Interactive CLI mode
    eprintln!(
        "\x1b[36mhex-agent\x1b[0m v{} | model: {} | project: {}",
        env!("CARGO_PKG_VERSION"),
        args.model,
        project_dir.display()
    );

    let cli = CliAdapter::new(conversation);
    cli.run().await?;

    Ok(())
}
