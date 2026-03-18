use hex_agent::{domain, ports, adapters, usecases};

use clap::Parser;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use adapters::primary::cli::CliAdapter;
use adapters::primary::migrate::ConfigMigrator;
use adapters::secondary::anthropic::AnthropicAdapter;
use adapters::secondary::context_manager::ContextManagerAdapter;
use adapters::secondary::tools::ToolExecutorAdapter;
use adapters::secondary::skill_loader::SkillLoaderAdapter;
use adapters::secondary::agent_loader::AgentLoaderAdapter;
use adapters::secondary::spacetime_skill::SpacetimeSkillLoader;
use adapters::secondary::spacetime_agent::SpacetimeAgentLoader;
use adapters::secondary::hub_client::HubClientAdapter;
use adapters::secondary::rl_client::{RlClientAdapter, NoopRlAdapter};
use domain::{TokenBudget, tools::builtin_tools};
use ports::skills::SkillLoaderPort;
use ports::agents::AgentLoaderPort;
use usecases::context_packer::ContextPacker;
use usecases::conversation::ConversationLoop;

#[derive(Parser, Debug)]
#[command(name = "hex-agent", version, about = "Autonomous AI agent for hex architecture")]
struct Args {
    #[command(subcommand)]
    command: Option<Command>,

    /// Anthropic API model to use
    #[arg(long, default_value = "claude-sonnet-4-20250514")]
    model: String,

    /// Project directory to operate in
    #[arg(long, default_value = ".")]
    project_dir: String,

    /// Agent definition to load (by name)
    #[arg(long)]
    agent: Option<String>,

    /// Hex-hub URL to connect to (enables SpacetimeDB-backed adapters)
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

#[derive(clap::Subcommand, Debug)]
enum Command {
    /// Migrate .claude/ skills, agents, and hooks into hex-hub registry
    MigrateConfig {
        /// hex-hub URL (required)
        #[arg(long)]
        hub_url: String,
    },
    /// Print the build hash and exit
    BuildHash,
}

/// Generate a unique, human-readable agent name from the agent UUID.
/// Format: "hex-<adjective>-<noun>" — deterministic from the ID so it's
/// stable across reconnects but unique per agent instance.
fn generate_agent_name(agent_id: &str) -> String {
    const ADJECTIVES: &[&str] = &[
        "swift", "bright", "keen", "bold", "calm", "sharp", "warm", "pure",
        "clear", "deep", "fair", "firm", "glad", "kind", "neat", "prime",
        "quick", "sage", "true", "wise", "agile", "brave", "crisp", "deft",
        "eager", "fleet", "grand", "hardy", "lucid", "noble", "rapid", "vivid",
    ];
    const NOUNS: &[&str] = &[
        "arc", "bolt", "core", "dart", "edge", "flux", "glyph", "hive",
        "iris", "jade", "knot", "link", "mesh", "node", "opus", "prism",
        "quill", "relay", "shard", "trace", "unit", "vault", "wave", "apex",
        "beam", "cipher", "delta", "ember", "forge", "grain", "helix", "orbit",
    ];

    // Use first 8 bytes of the UUID to pick adjective and noun
    let hash: u64 = agent_id.bytes().fold(0u64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64));
    let adj = ADJECTIVES[(hash as usize) % ADJECTIVES.len()];
    let noun = NOUNS[((hash >> 16) as usize) % NOUNS.len()];
    // Append 4 hex chars from the ID for extra uniqueness
    let suffix = &agent_id[..4.min(agent_id.len())];
    format!("hex-{}-{}-{}", adj, noun, suffix)
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

    let project_dir = PathBuf::from(&args.project_dir)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(&args.project_dir));

    // --- Handle subcommands ---

    if let Some(Command::BuildHash) = &args.command {
        println!("{}", env!("HEX_AGENT_BUILD_HASH"));
        return Ok(());
    }

    if let Some(Command::MigrateConfig { hub_url }) = &args.command {
        eprintln!("Migrating .claude/ config → hex-hub at {}", hub_url);
        let migrator = ConfigMigrator::new(hub_url);
        match migrator.migrate(&project_dir.to_string_lossy()).await {
            Ok(report) => {
                eprintln!("{}", report);
                return Ok(());
            }
            Err(e) => {
                eprintln!("\x1b[31mMigration failed: {}\x1b[0m", e);
                std::process::exit(1);
            }
        }
    }

    // --- Composition Root: wire adapters to ports ---

    // Resolve API key
    let api_key = std::env::var("ANTHROPIC_API_KEY").unwrap_or_else(|_| {
        eprintln!("\x1b[31mError: ANTHROPIC_API_KEY environment variable not set\x1b[0m");
        std::process::exit(1);
    });

    // Secondary adapters
    let anthropic: Arc<dyn ports::AnthropicPort> =
        Arc::new(AnthropicAdapter::new(api_key, args.model.clone()));
    let context_mgr: Arc<dyn ports::ContextManagerPort> =
        Arc::new(ContextManagerAdapter::new());
    let tool_executor: Arc<dyn ports::ToolExecutorPort> =
        Arc::new(ToolExecutorAdapter::new(project_dir.clone()));

    // --- Adapter Selection: SpacetimeDB (via hub) or filesystem fallback ---
    //
    // If --hub-url is provided and reachable, use SpacetimeDB-backed adapters
    // that subscribe to hub's state tables. Otherwise, fall back to filesystem
    // loaders (the original behavior). This is the hexagonal architecture
    // composition root pattern — same ports, different adapters.

    // SpacetimeDB config is injected by hex-hub at spawn time via env vars.
    // Agents don't resolve config themselves — that's the hub's responsibility.
    // Per-module database names allow each loader to connect to its own DB.
    let stdb_host = std::env::var("HEX_STDB_HOST").unwrap_or_default();
    let stdb_skill_db = std::env::var("HEX_STDB_SKILL_DB")
        .or_else(|_| std::env::var("HEX_STDB_DATABASE"))
        .unwrap_or_default();
    let stdb_agent_def_db = std::env::var("HEX_STDB_AGENT_DEF_DB")
        .or_else(|_| std::env::var("HEX_STDB_DATABASE"))
        .unwrap_or_default();

    let hub_connected = if let Some(ref hub_url) = args.hub_url {
        // Probe hub health endpoint
        reqwest::get(format!("{}/health", hub_url)).await.is_ok()
    } else {
        false
    };

    let (skills, agent_def) = if hub_connected {
        let hub_url = args.hub_url.as_deref().unwrap();

        // Try SpacetimeDB-backed loaders first, fall back to filesystem.
        // Each loader connects to its own per-module database.
        let skill_loader_st = SpacetimeSkillLoader::new(hub_url);
        let st_skills_ok = skill_loader_st.connect(&stdb_host, &stdb_skill_db).await.is_ok();
        let st_skills = if st_skills_ok {
            skill_loader_st.load(&[]).await.unwrap_or_default()
        } else {
            Default::default()
        };

        let agent_loader_st = SpacetimeAgentLoader::new(hub_url);
        let st_agents_ok = agent_loader_st.connect(&stdb_host, &stdb_agent_def_db).await.is_ok();
        let st_agent_def = if st_agents_ok {
            if let Some(agent_name) = &args.agent {
                agent_loader_st.load_by_name(&[], agent_name).await.ok()
            } else {
                None
            }
        } else {
            None
        };

        // If SpacetimeDB didn't have what we need, fall back to filesystem
        let use_fs = st_skills.skills.is_empty() || (args.agent.is_some() && st_agent_def.is_none());

        if use_fs {
            tracing::info!("Hub state APIs not available — loading skills/agents from filesystem");
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
                        tracing::info!(agent = %def.name, "Loaded agent from filesystem");
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

            (skills, agent_def)
        } else {
            tracing::info!("Using SpacetimeDB-backed adapters via hub");
            (st_skills, st_agent_def)
        }
    } else {
        // Filesystem fallback — original behavior
        if args.hub_url.is_some() {
            tracing::warn!("Hub not reachable, falling back to filesystem loaders");
        }

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
                    tracing::info!(agent = %def.name, "Loaded agent from filesystem");
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

        (skills, agent_def)
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

    // RL adapter: use hub-backed RL when hub is reachable, otherwise noop
    let rl: Arc<dyn ports::rl::RlPort> = if hub_connected {
        let hub_url = args.hub_url.as_deref().unwrap();
        tracing::info!(hub_url = %hub_url, "RL engine connected via hub");
        Arc::new(RlClientAdapter::new(hub_url))
    } else {
        Arc::new(NoopRlAdapter)
    };

    // Build conversation loop (use case)
    let tools = builtin_tools();
    let conversation = ConversationLoop::new(
        anthropic,
        context_mgr,
        tool_executor,
        rl,
        tools,
        budget,
        args.max_response,
    );

    // Decide mode: hub-managed or interactive CLI
    if let (Some(hub_url), Some(hub_token)) = (&args.hub_url, &args.hub_token) {
        use ports::hub::{HubClientPort, HubMessage};
        use ports::conversation::{ConversationEvent, ConversationPort};

        let hub_client = Arc::new(HubClientAdapter::new());

        hub_client.connect(hub_url, hub_token).await.map_err(|e| {
            anyhow::anyhow!("Failed to connect to hub at {}: {}", hub_url, e)
        })?;

        let agent_id = uuid::Uuid::new_v4().to_string();
        let agent_display_name = args.agent.clone().unwrap_or_else(|| generate_agent_name(&agent_id));
        hub_client
            .send(HubMessage::Register {
                agent_id: agent_id.clone(),
                agent_name: agent_display_name.clone(),
                project_dir: project_dir.to_string_lossy().into(),
            })
            .await
            .map_err(|e| anyhow::anyhow!("Hub registration failed: {}", e))?;

        tracing::info!(agent_id = %agent_id, name = %agent_display_name, hub = %hub_url, "Running in hub-managed mode");

        // Shared agent status: 0=idle, 1=thinking, 2=executing
        let agent_status_flag = Arc::new(AtomicU8::new(0));

        // Spawn heartbeat background task
        let hb_client = hub_client.clone();
        let hb_agent_id = agent_id.clone();
        let hb_agent_name = agent_display_name.clone();
        let hb_status = agent_status_flag.clone();
        let hb_start = std::time::Instant::now();
        let heartbeat_handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(15));
            loop {
                interval.tick().await;
                if !hb_client.is_connected() {
                    break;
                }
                let status_str = match hb_status.load(Ordering::Relaxed) {
                    1 => "thinking",
                    2 => "executing",
                    _ => "idle",
                };
                let uptime = hb_start.elapsed().as_secs();
                let _ = hb_client.send(HubMessage::Heartbeat {
                    agent_id: hb_agent_id.clone(),
                    agent_name: hb_agent_name.clone(),
                    status: status_str.to_string(),
                    uptime_secs: uptime,
                }).await;
            }
        });

        // Conversation state persists across turns
        let mut conv_state = domain::ConversationState::new(uuid::Uuid::new_v4().to_string());
        conv_state.system_prompt = system_prompt.clone();

        loop {
            let msg = match hub_client.recv().await {
                Ok(msg) => msg,
                Err(e) => {
                    tracing::warn!("Hub connection lost: {}", e);
                    break;
                }
            };

            match msg {
                HubMessage::ChatMessage { content } => {
                    agent_status_flag.store(1, Ordering::Relaxed); // thinking
                    hub_client
                        .send(HubMessage::AgentStatus {
                            status: "thinking".into(),
                            detail: String::new(),
                            agent_name: Some(agent_display_name.clone()),
                        })
                        .await
                        .ok();

                    // Create event channel and forward events to hub
                    let (event_tx, mut event_rx) =
                        tokio::sync::mpsc::unbounded_channel::<ConversationEvent>();

                    let hub_fwd = hub_client.clone();
                    let fwd_name = agent_display_name.clone();
                    let fwd_status = agent_status_flag.clone();
                    let forwarder = tokio::spawn(async move {
                        while let Some(event) = event_rx.recv().await {
                            let hub_msg = match &event {
                                ConversationEvent::TextChunk(text) => {
                                    Some(HubMessage::StreamChunk {
                                        text: text.clone(),
                                        agent_name: Some(fwd_name.clone()),
                                    })
                                }
                                ConversationEvent::ToolCallStart { name, input } => {
                                    fwd_status.store(2, Ordering::Relaxed); // executing
                                    Some(HubMessage::ToolCall {
                                        tool_name: name.clone(),
                                        tool_input: serde_json::Value::String(input.clone()),
                                        agent_name: Some(fwd_name.clone()),
                                    })
                                }
                                ConversationEvent::ToolCallResult {
                                    name,
                                    content,
                                    is_error,
                                } => {
                                    fwd_status.store(1, Ordering::Relaxed); // back to thinking
                                    Some(HubMessage::ToolResultMsg {
                                        tool_name: name.clone(),
                                        content: content.clone(),
                                        is_error: *is_error,
                                        agent_name: Some(fwd_name.clone()),
                                    })
                                }
                                ConversationEvent::TokenUpdate(usage) => {
                                    Some(HubMessage::TokenUpdate {
                                        input_tokens: usage.input_tokens,
                                        output_tokens: usage.output_tokens,
                                        total_input: usage.input_tokens as u64,
                                        total_output: usage.output_tokens as u64,
                                        agent_name: Some(fwd_name.clone()),
                                    })
                                }
                                ConversationEvent::TurnComplete { .. } => {
                                    fwd_status.store(0, Ordering::Relaxed); // idle
                                    Some(HubMessage::AgentStatus {
                                        status: "idle".into(),
                                        detail: String::new(),
                                        agent_name: Some(fwd_name.clone()),
                                    })
                                }
                                ConversationEvent::Error(msg) => {
                                    fwd_status.store(0, Ordering::Relaxed);
                                    Some(HubMessage::AgentStatus {
                                        status: "error".into(),
                                        detail: msg.clone(),
                                        agent_name: Some(fwd_name.clone()),
                                    })
                                }
                                _ => None,
                            };
                            if let Some(m) = hub_msg {
                                hub_fwd.send(m).await.ok();
                            }
                        }
                    });

                    let result = conversation
                        .process_message(&mut conv_state, &content, &event_tx)
                        .await;
                    drop(event_tx); // Signal forwarder to stop
                    forwarder.await.ok();

                    agent_status_flag.store(0, Ordering::Relaxed); // idle
                    if let Err(e) = result {
                        hub_client
                            .send(HubMessage::AgentStatus {
                                status: "error".into(),
                                detail: e.to_string(),
                                agent_name: Some(agent_display_name.clone()),
                            })
                            .await
                            .ok();
                    }

                    hub_client
                        .send(HubMessage::AgentStatus {
                            status: "idle".into(),
                            detail: String::new(),
                            agent_name: Some(agent_display_name.clone()),
                        })
                        .await
                        .ok();
                }
                HubMessage::Connected { session_id, .. } => {
                    tracing::info!(session_id = %session_id, "Hub confirmed connection");
                }
                HubMessage::Done { .. } => {
                    tracing::info!("Hub requested shutdown");
                    break;
                }
                _ => {}
            }
        }

        heartbeat_handle.abort();
        hub_client.disconnect().await.ok();
        return Ok(());
    }

    // Interactive CLI mode
    eprintln!(
        "\x1b[36mhex-agent\x1b[0m v{} ({}) | model: {} | project: {}",
        env!("CARGO_PKG_VERSION"),
        env!("HEX_AGENT_BUILD_HASH"),
        args.model,
        project_dir.display()
    );

    let cli = CliAdapter::new(Box::new(conversation));
    cli.run().await?;

    Ok(())
}
