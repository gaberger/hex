pub mod adapters;
pub mod cleanup;
pub mod daemon;
pub mod embed;
pub mod middleware;
pub mod orchestration;
pub mod persistence;
pub mod ports;
pub mod remote;
pub mod rl;
pub mod routes;
pub mod state;
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
    // Initialize persistent swarm database
    let swarm_db = match persistence::SwarmDb::open() {
        Ok(db) => {
            tracing::info!("Swarm persistence initialized at ~/.hex/hub.db");
            Some(db)
        }
        Err(e) => {
            tracing::warn!(
                "Failed to open swarm database: {} — running without persistence",
                e
            );
            None
        }
    };

    // Create shared state
    let mut app_state = AppState::new(config.token.clone(), swarm_db);

    // Wire IStatePort → AgentManager + WorkplanExecutor (ADR-025 Phase 2)
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    let db_path = format!("{}/.hex/hub.db", home);

    if app_state.swarm_db.is_some() {
        match adapters::sqlite_state::SqliteStateAdapter::new(&db_path) {
            Ok(state_port) => {
                let state_port: Arc<dyn ports::state::IStatePort> = Arc::new(state_port);
                let agent_mgr = Arc::new(
                    orchestration::agent_manager::AgentManager::new(Arc::clone(&state_port)),
                );
                app_state.agent_manager = Some(agent_mgr);
                tracing::info!("IStatePort wired — agent_manager using SqliteStateAdapter");
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to create SqliteStateAdapter: {} — orchestration using legacy path",
                    e
                );
            }
        }
    }

    // Wrap in Arc, then create WorkplanExecutor (needs SharedState = Arc<AppState>)
    let state = Arc::new(app_state);

    if state.agent_manager.is_some() {
        if let Ok(state_port) = adapters::sqlite_state::SqliteStateAdapter::new(&db_path) {
            let sp: Arc<dyn ports::state::IStatePort> = Arc::new(state_port);
            let wp = Arc::new(orchestration::workplan_executor::WorkplanExecutor::new(
                sp,
                state.clone(),
            ));
            state.workplan_executor.set(wp).ok();
        }
    }

    // Background task: cleanup stale coordination sessions
    let cleanup_state = state.clone();
    cleanup::CleanupService::spawn(cleanup_state);

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

    // Background task: prune expired secret grants (every 60s) — ADR-026
    let prune_state = state.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            interval.tick().await;
            let now = chrono::Utc::now().to_rfc3339();
            let mut grants = prune_state.secret_grants.write().await;
            let before = grants.len();
            grants.retain(|_, g| g.expires_at > now);
            let pruned = before - grants.len();
            if pruned > 0 {
                tracing::debug!("Pruned {} expired secret grants", pruned);
            }
        }
    });

    // Build router
    let app = routes::build_router(state.clone());

    (app, state)
}

/// Start the headless Axum server with graceful shutdown.
///
/// This is what the `hex-hub` binary calls. It handles:
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
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("Failed to bind");

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

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await
        .expect("Server error");
}

/// Return the compile-time build hash.
pub fn build_hash() -> &'static str {
    env!("HEX_HUB_BUILD_HASH")
}

/// Return the crate version.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
