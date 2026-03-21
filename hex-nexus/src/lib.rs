// Re-export hex-core so downstream crates can access shared types and port traits
pub use hex_core;

pub mod adapters;
pub mod analysis;
pub mod cleanup;
pub mod coordination;
pub mod daemon;
pub mod git;
pub mod embed;
pub mod middleware;
pub mod orchestration;
pub mod ports;
pub mod remote;
pub mod routes;
pub mod state;
pub mod usecases;
pub mod state_config;
pub mod spacetime_bindings;
pub mod spacetime_launcher;

use std::sync::Arc;

use state::AppState;
pub use state::SharedState;

/// Re-export axum so embedders (hex-desktop) can call `axum::serve` without
/// adding a separate axum dependency that might version-conflict.
pub use axum;

pub const DEFAULT_PORT: u16 = 5555;

// ── Configuration ──────────────────────────────────────

/// Configuration for the hex-hub server.
pub struct HubConfig {
    pub port: u16,
    pub bind: String,
    pub token: Option<String>,
    pub is_daemon: bool,
}

impl Default for HubConfig {
    fn default() -> Self {
        Self {
            port: DEFAULT_PORT,
            bind: "127.0.0.1".to_string(),
            token: None,
            is_daemon: false,
        }
    }
}

// ── Public API ─────────────────────────────────────────

/// Build the Axum router and shared state without binding to a port.
///
/// This is the primary entry point for embedders (e.g. Tauri).
/// The caller is responsible for:
///   1. Binding a `TcpListener`
///   2. Calling `axum::serve(listener, router)`
///   3. Managing graceful shutdown
///
/// Background cleanup tasks are spawned automatically.
pub async fn build_app(config: &HubConfig) -> (axum::Router, SharedState) {
    // Create shared state
    let mut app_state = AppState::new(config.token.clone());

    // Wire IStatePort → AgentManager + HexFlo (ADR-025 Phase 2/4, ADR-032 Phase 3)
    // Backend: SpacetimeDB (only backend, ADR-032)
    match state_config::create_default_state_backend() {
        Ok(state_port) => {
            let agent_mgr = Arc::new(
                orchestration::agent_manager::AgentManager::new(Arc::clone(&state_port)),
            );
            app_state.agent_manager = Some(agent_mgr);
            app_state.state_port = Some(state_port);
            tracing::info!("IStatePort wired — agent_manager + state_port ready");
        }
        Err(e) => {
            tracing::warn!(
                "Failed to create state backend: {} — orchestration using legacy path",
                e
            );
        }
    }

    // HexFlo coordination via IStatePort (ADR-032)
    // Re-use the same state backend — if SpacetimeDB, the hexflo methods
    // fall through to the SwarmDb fallback internally.
    match state_config::create_default_state_backend() {
        Ok(hexflo_state) => {
            let hexflo = coordination::HexFlo::new(
                hexflo_state,
                app_state.ws_tx.clone(),
                app_state.agent_manager.clone(),
            );
            app_state.hexflo = Some(Arc::new(hexflo));
            tracing::info!("HexFlo coordination ready");
        }
        Err(e) => {
            tracing::warn!("HexFlo coordination unavailable: {}", e);
        }
    }

    // Initialize SpacetimeDB secret client (ADR-026 integration)
    // Connects to the same SpacetimeDB instance used by IStatePort.
    {
        let stdb_host = std::env::var("HEX_SPACETIMEDB_HOST")
            .unwrap_or_else(|_| "http://127.0.0.1:3000".to_string());
        let stdb_database = std::env::var("HEX_SPACETIMEDB_DATABASE")
            .unwrap_or_else(|_| "hex-nexus".to_string());

        let hub_id = std::env::var("HEX_HUB_ID").unwrap_or_else(|_| "hub-local".to_string());
        let client = adapters::spacetime_secrets::SpacetimeSecretClient::new(
            stdb_host,
            stdb_database,
            hub_id,
        );
        if client.connect().await {
            app_state.spacetime_secrets = Some(Arc::new(client));
            tracing::info!("SpacetimeDB secret broker integration active");
        } else {
            tracing::info!(
                "SpacetimeDB secret broker unavailable — using in-memory fallback"
            );
        }
    }

    // Initialize SpacetimeDB inference-gateway + chat-relay clients
    {
        let stdb_host = std::env::var("HEX_SPACETIMEDB_HOST")
            .unwrap_or_else(|_| "http://127.0.0.1:3000".to_string());

        let inference_db = std::env::var("HEX_INFERENCE_STDB_DATABASE")
            .unwrap_or_else(|_| "inference-gateway".to_string());
        let chat_db = std::env::var("HEX_CHAT_STDB_DATABASE")
            .unwrap_or_else(|_| "chat-relay".to_string());

        let inference_client =
            adapters::spacetime_inference::SpacetimeInferenceClient::new(
                stdb_host.clone(),
                inference_db,
            );
        let chat_client =
            adapters::spacetime_chat::SpacetimeChatClient::new(stdb_host, chat_db);

        app_state.inference_stdb = Some(Arc::new(inference_client));
        app_state.chat_stdb = Some(Arc::new(chat_client));
        tracing::info!("SpacetimeDB inference-gateway + chat-relay clients initialized");

    }

    // Initialize session persistence (ADR-036)
    #[cfg(feature = "sqlite-session")]
    {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        let hex_dir = std::path::PathBuf::from(home).join(".hex");
        let _ = std::fs::create_dir_all(&hex_dir);
        let db_path = hex_dir.join("hub.db");
        match adapters::sqlite_session::SqliteSessionAdapter::from_path(
            db_path.to_str().unwrap_or("/tmp/.hex/hub.db"),
        )
        .await
        {
            Ok(adapter) => {
                app_state.session_port = Some(Arc::new(adapter));
                tracing::info!("Session persistence active (SQLite: {:?})", db_path);
            }
            Err(e) => {
                tracing::warn!("Session persistence unavailable: {e}");
            }
        }
    }

    // Wrap in Arc, then create WorkplanExecutor (needs SharedState = Arc<AppState>)
    let state = Arc::new(app_state);

    if state.agent_manager.is_some() {
        if let Ok(state_port) = state_config::create_default_state_backend() {
            let wp = Arc::new(orchestration::workplan_executor::WorkplanExecutor::new(
                state_port,
                state.clone(),
            ));
            state.workplan_executor.set(wp).ok();
        }
    }

    // Background task: prune expired secret grants (every 60s)
    if let Some(ref stdb) = state.spacetime_secrets {
        adapters::spacetime_secrets::spawn_prune_task(
            Arc::clone(stdb),
            std::time::Duration::from_secs(60),
        );
    }

    // ADR-039 Phase 10: Stale session cleanup is now handled by SpacetimeDB
    // scheduled reducer (hexflo-cleanup module, 30s interval). The CleanupService
    // background task is no longer needed when SpacetimeDB is the state backend.
    // Kept as fallback for SQLite-only deployments.
    if state.state_port.is_none() {
        let cleanup_state = state.clone();
        cleanup::CleanupService::spawn(cleanup_state);
    }

    // Background task: evict completed commands older than 1 hour (every 60s)
    let evict_state = state.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        let ttl = chrono::Duration::hours(1);
        loop {
            interval.tick().await;
            let cutoff = chrono::Utc::now() - ttl;
            let cutoff_str = cutoff.to_rfc3339();

            // Evict completed/failed commands
            let mut commands = evict_state.commands.write().await;
            let before = commands.len();
            commands.retain(|_, cmd| {
                if cmd.status == "completed" || cmd.status == "failed" {
                    cmd.issued_at > cutoff_str
                } else {
                    true // keep pending/dispatched/running
                }
            });
            let evicted_cmds = before - commands.len();
            drop(commands);

            // Evict matching results
            let mut results = evict_state.results.write().await;
            let before = results.len();
            results.retain(|_, res| res.completed_at > cutoff_str);
            let evicted_results = before - results.len();
            drop(results);

            if evicted_cmds > 0 || evicted_results > 0 {
                tracing::debug!(
                    "Evicted {} commands, {} results (TTL 1h)",
                    evicted_cmds,
                    evicted_results
                );
            }
        }
    });

    // Background task: git status polling for registered projects (ADR-044 Phase 2)
    git::poller::spawn_git_poller(state.clone(), 10);

    // Build router
    let app = routes::build_router(state.clone());

    (app, state)
}

/// Start the headless Axum server with graceful shutdown.
///
/// This is what the `hex-nexus` binary calls. It handles:
/// - TCP binding
/// - Lock file management
/// - Ctrl+C / SIGTERM graceful shutdown
pub async fn start_server(config: HubConfig) {
    let (app, _state) = build_app(&config).await;

    let lock_token = config
        .token
        .clone()
        .unwrap_or_else(|| daemon::generate_token());

    // Setup graceful shutdown
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    let shutdown = async {
        tokio::select! {
            _ = ctrl_c => {},
            _ = terminate => {},
        }
        tracing::info!("Shutdown signal received");
        daemon::remove_lock();
    };

    // Bind FIRST, then write lock file — prevents clients from connecting before we're ready (H4)
    let addr = format!("{}:{}", config.bind, config.port);
    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            tracing::error!(
                "Port {} is already in use — another hex-nexus may be running.\n  \
                 Stop it with: hex nexus stop\n  \
                 Or use a different port: hex-nexus --port 5556",
                config.port
            );
            std::process::exit(1);
        }
        Err(e) => {
            tracing::error!("Failed to bind to {}: {}", addr, e);
            std::process::exit(1);
        }
    };

    // Write lock file AFTER bind succeeds — clients reading this file can now connect
    if let Err(e) = daemon::write_lock(config.port, &lock_token) {
        tracing::warn!("Failed to write lock file: {}", e);
    }

    if config.is_daemon {
        tracing::info!(
            "hex-hub v{} daemon started on http://{}",
            env!("CARGO_PKG_VERSION"),
            addr
        );
    } else {
        tracing::info!(
            "hex-hub v{} running on http://{}",
            env!("CARGO_PKG_VERSION"),
            addr
        );
    }

    // ADR-037: Spawn default local agent (opt-out with --no-agent)
    let no_agent = std::env::args().any(|a| a == "--no-agent");
    let agent_child = if !no_agent {
        spawn_default_agent(config.port, &lock_token)
    } else {
        None
    };

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await
        .expect("Server error");

    // Cleanup: kill the default agent on shutdown
    if let Some(mut child) = agent_child {
        tracing::info!("Stopping default agent (PID {})", child.id());
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// Spawn a default local hex-agent connected to this nexus (ADR-037).
///
/// Searches for the hex-agent binary in:
/// 1. Same directory as hex-nexus binary
/// 2. PATH
/// 3. cargo target directory (dev mode)
///
/// Returns the child process handle, or None if agent not found.
fn spawn_default_agent(port: u16, token: &str) -> Option<std::process::Child> {
    let agent_bin = find_agent_binary()?;
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));

    tracing::info!(
        agent = %agent_bin.display(),
        project = %cwd.display(),
        "Spawning default local agent"
    );

    match std::process::Command::new(&agent_bin)
        .args([
            "--hub-url", &format!("http://127.0.0.1:{}", port),
            "--hub-token", token,
            "--project-dir", &cwd.to_string_lossy(),
            "--no-preflight",
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(child) => {
            tracing::info!(
                pid = child.id(),
                "Default agent started (PID {})",
                child.id()
            );
            Some(child)
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                "Could not spawn default agent — run with --no-agent to suppress"
            );
            None
        }
    }
}

/// Find the hex-agent binary.
fn find_agent_binary() -> Option<std::path::PathBuf> {
    // 1. Sibling of current executable
    if let Ok(exe) = std::env::current_exe() {
        let sibling = exe.parent()?.join("hex-agent");
        if sibling.exists() {
            return Some(sibling);
        }
    }

    // 2. In PATH
    if let Ok(output) = std::process::Command::new("which")
        .arg("hex-agent")
        .output()
    {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                return Some(std::path::PathBuf::from(path));
            }
        }
    }

    // 3. Cargo target directory (dev mode)
    if let Ok(exe) = std::env::current_exe() {
        // exe is in target/release/hex-nexus or target/debug/hex-nexus
        if let Some(target_dir) = exe.parent() {
            let agent = target_dir.join("hex-agent");
            if agent.exists() {
                return Some(agent);
            }
        }
    }

    tracing::debug!("hex-agent binary not found — no default agent will be spawned");
    None
}

/// Return the compile-time build hash.
pub fn build_hash() -> &'static str {
    env!("HEX_HUB_BUILD_HASH")
}

/// Return the crate version.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
