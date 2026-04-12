// Pre-existing clippy lints — tracked for cleanup in ADR-2603222050
#![allow(
    clippy::literal_string_with_formatting_args,
    clippy::too_many_arguments,
    clippy::doc_overindented_list_items,
    clippy::type_complexity,
    clippy::manual_strip,
    clippy::vec_init_then_push,
    clippy::collapsible_match,
    clippy::single_match,
    clippy::explicit_counter_loop,
    clippy::should_implement_trait
)]
// Re-export hex-core so downstream crates can access shared types and port traits
pub use hex_core;

pub mod adapters;
pub mod analysis;
pub mod composition;
pub mod composition_root;
pub mod complexity;
pub mod neural_lab_quant;
pub mod quant_router;
pub mod rate_limiter;
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
pub mod config_sync;
pub mod spacetime_launcher;
pub mod templates;
pub mod brain_service;

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
    /// Skip auto-spawning the default local hex-agent (ADR-037).
    /// Set via `--no-agent` flag or `HEX_NO_AGENT=1` env var.
    pub no_agent: bool,
}

impl Default for HubConfig {
    fn default() -> Self {
        Self {
            port: DEFAULT_PORT,
            bind: "127.0.0.1".to_string(),
            token: None,
            is_daemon: false,
            no_agent: false,
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
    let anthropic_api_key = std::env::var("ANTHROPIC_API_KEY").ok();
    let mut app_state = AppState::new(config.token.clone(), anthropic_api_key);

    // Wire IStatePort → AgentManager + HexFlo (ADR-025 Phase 2/4, ADR-032 Phase 3)
    // Backend: SpacetimeDB (only backend, ADR-032)
    match state_config::create_default_state_backend_with_inference(app_state.inference_tx.clone()) {
        Ok(state_port) => {
            let secret_resolver: orchestration::agent_manager::SecretResolver =
                Arc::new(|key: &str| std::env::var(key).ok());
            let agent_mgr = Arc::new(
                orchestration::agent_manager::AgentManager::new(
                    Arc::clone(&state_port),
                    secret_resolver,
                    Arc::clone(&app_state.capability_token_service),
                ),
            );
            app_state.agent_manager = Some(agent_mgr);
            app_state.state_port = Some(state_port);

            // Wire inference port for Path C headless dispatch (ADR-2604120202 P5.1).
            // In standalone mode, this is OllamaInferenceAdapter pointed at OLLAMA_HOST.
            if !orchestration::is_claude_code_session() {
                app_state.inference_port = Some(composition::standalone::default_inference_adapter());
                tracing::info!("Path C inference port wired (standalone Ollama)");
            }

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
            .unwrap_or_else(|_| "http://127.0.0.1:3033".to_string());
        let stdb_database = std::env::var("HEX_SPACETIMEDB_DATABASE")
            .unwrap_or_else(|_| hex_core::stdb_database_for_module("secret-grant").to_string());

        let hub_id = std::env::var("HEX_HUB_ID").unwrap_or_else(|_| "hub-local".to_string());
        let client = adapters::spacetime_secrets::SpacetimeSecretClient::new(
            stdb_host,
            stdb_database,
            hub_id,
        );
        if client.connect().await {
            // Resolve API keys: vault first, then env fallback.
            // This allows keys stored via `hex secrets vault set` to work without
            // requiring environment variables to be set on the host.
            use crate::ports::secret_grant::ISecretGrantPort;
            let anthropic_key = client.vault_get("ANTHROPIC_API_KEY").await
                .ok().flatten()
                .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok());
            let openrouter_key = client.vault_get("OPENROUTER_API_KEY").await
                .ok().flatten()
                .or_else(|| std::env::var("OPENROUTER_API_KEY").ok());

            if anthropic_key.is_some() {
                tracing::info!("ANTHROPIC_API_KEY resolved from vault");
                app_state.anthropic_api_key = anthropic_key;
            }
            if openrouter_key.is_some() {
                tracing::info!("OPENROUTER_API_KEY resolved from vault");
                app_state.openrouter_api_key = openrouter_key;
            }

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
            .unwrap_or_else(|_| "http://127.0.0.1:3033".to_string());

        let inference_db = std::env::var("HEX_INFERENCE_STDB_DATABASE")
            .unwrap_or_else(|_| hex_core::stdb_database_for_module("inference-gateway").to_string());
        let chat_db = std::env::var("HEX_CHAT_STDB_DATABASE")
            .unwrap_or_else(|_| hex_core::stdb_database_for_module("chat-relay").to_string());

        let inference_client =
            adapters::spacetime_inference::SpacetimeInferenceClient::new(
                stdb_host.clone(),
                inference_db,
            );
        let chat_client =
            adapters::spacetime_chat::SpacetimeChatClient::new(stdb_host, chat_db);

        // Push resolved API keys into inference-gateway's private `inference_api_key` table.
        // This is required for `execute_inference` to include auth headers — the table is
        // only writable via `set_api_key` reducer, not populated automatically at startup.
        {
            let or_key = app_state.openrouter_api_key.clone();
            let an_key = app_state.anthropic_api_key.clone();
            if or_key.is_some() || an_key.is_some() {
                match inference_client.list_providers().await {
                    Ok(providers) => {
                        for p in &providers {
                            let key = match p.api_key_ref.as_str() {
                                "OPENROUTER_API_KEY" => or_key.as_deref(),
                                "ANTHROPIC_API_KEY" => an_key.as_deref(),
                                _ => None,
                            };
                            if let Some(k) = key {
                                if let Err(e) = inference_client.set_api_key(&p.provider_id, k).await {
                                    tracing::warn!(provider = %p.provider_id, error = %e, "set_api_key at startup failed (non-fatal)");
                                } else {
                                    tracing::info!(provider = %p.provider_id, "inference API key pushed to WASM module");
                                }
                            }
                        }
                    }
                    Err(e) => tracing::warn!(error = %e, "Failed to list inference providers for key push (non-fatal)"),
                }
            }
        }

        app_state.inference_stdb = Some(Arc::new(inference_client));
        app_state.chat_stdb = Some(Arc::new(chat_client));
        tracing::info!("SpacetimeDB inference-gateway + chat-relay clients initialized");

        // P4: Background stale-model prune pass (ADR-2603311000).
        // Test each registered OpenRouter provider with a minimal prompt.
        // Providers returning empty content are removed — they are placeholder
        // model IDs that don't exist yet (e.g. gpt-5.x, grok-4.x) and would
        // waste the free-provider fallback budget on every request.
        {
            let prune_stdb = app_state.inference_stdb.clone();
            let prune_or_key = app_state.openrouter_api_key.clone();
            tokio::spawn(async move {
                if let Some(stdb) = prune_stdb {
                    let providers = match stdb.list_providers().await {
                        Ok(p) => p,
                        Err(e) => {
                            tracing::warn!(error = %e, "P4 prune: failed to list providers");
                            return;
                        }
                    };
                    let openrouter_candidates: Vec<_> = providers
                        .into_iter()
                        .filter(|p| p.provider_type == "openrouter")
                        .collect();
                    if openrouter_candidates.is_empty() {
                        return;
                    }
                    tracing::info!(count = openrouter_candidates.len(), "P4 prune: testing registered OpenRouter providers");
                    for p in &openrouter_candidates {
                        // Resolve the API key for this provider
                        let api_key = if p.api_key_ref.starts_with("sk-") {
                            p.api_key_ref.clone()
                        } else {
                            prune_or_key.clone().unwrap_or_default()
                        };
                        if api_key.is_empty() {
                            tracing::debug!(provider_id = %p.provider_id, "P4 prune: no key available — skipping");
                            continue;
                        }
                        let model = p.models_json
                            .trim_start_matches('[')
                            .trim_end_matches(']')
                            .split(',')
                            .next()
                            .unwrap_or(&p.models_json)
                            .trim()
                            .trim_matches('"')
                            .to_string();
                        let test_ep = crate::routes::secrets::InferenceEndpointEntry {
                            id: p.provider_id.clone(),
                            url: p.base_url.clone(),
                            provider: "openrouter".into(),
                            model: model.clone(),
                            status: "unknown".into(),
                            requires_auth: true,
                            secret_key: api_key,
                            health_checked_at: String::new(),
                        };
                        let test_messages = vec![
                            serde_json::json!({"role": "user", "content": "Reply with the single word: ok"})
                        ];
                        match crate::routes::chat::call_inference_endpoint(&test_ep, &test_messages).await {
                            Ok((content, _, _, _, _)) if content.trim().is_empty() => {
                                tracing::warn!(
                                    provider_id = %p.provider_id,
                                    model = %model,
                                    "P4 prune: provider returned empty content — removing stale model ID"
                                );
                                if let Err(e) = stdb.remove_provider(&p.provider_id).await {
                                    tracing::warn!(provider_id = %p.provider_id, error = %e, "P4 prune: remove_provider failed");
                                }
                            }
                            Ok((content, _, _, _, _)) => {
                                tracing::info!(
                                    provider_id = %p.provider_id,
                                    model = %model,
                                    content_len = content.len(),
                                    "P4 prune: provider OK"
                                );
                            }
                            Err(e) => {
                                tracing::warn!(
                                    provider_id = %p.provider_id,
                                    model = %model,
                                    error = %e,
                                    "P4 prune: provider test error (keeping — may be transient)"
                                );
                            }
                        }
                        // Brief pause between tests to avoid rate-limit burst
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    }
                    tracing::info!("P4 prune: stale model prune pass complete");
                }
            });
        }
    }

    // Auto-hydrate SpacetimeDB schemas on startup (T9: zero-setup first boot).
    // Runs in background — publishes WASM modules if SpacetimeDB is empty.
    {
        let stdb_host = std::env::var("HEX_SPACETIMEDB_HOST")
            .unwrap_or_else(|_| "http://127.0.0.1:3033".to_string());
        let stdb_database = std::env::var("HEX_SPACETIMEDB_DATABASE")
            .unwrap_or_else(|_| hex_core::STDB_DATABASE_CORE.to_string());
        let stdb_host_clone = stdb_host.clone();
        let stdb_db_clone = stdb_database.clone();

        tokio::spawn(async move {
            // Check if SpacetimeDB is reachable first
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(3))
                .build()
                .unwrap_or_default();

            let ping_ok = client
                .get(format!("{}{}", stdb_host_clone, hex_core::SPACETIMEDB_PING_PATH))
                .send()
                .await
                .map(|r| r.status().is_success())
                .unwrap_or(false);

            if !ping_ok {
                tracing::info!(
                    "SpacetimeDB not reachable at {} — skipping auto-hydration. \
                     Run `hex stdb hydrate` manually after starting SpacetimeDB.",
                    stdb_host_clone
                );
                return;
            }

            // Look for spacetime-modules directory
            let modules_dir = if let Ok(cwd) = std::env::current_dir() {
                let candidate = cwd.join("spacetime-modules");
                if candidate.is_dir() {
                    Some(candidate)
                } else {
                    None
                }
            } else {
                None
            };

            if let Some(modules_dir) = modules_dir {
                tracing::info!(
                    "Auto-hydrating SpacetimeDB schemas ({} → {})",
                    stdb_host_clone,
                    stdb_db_clone
                );

                match spacetime_launcher::publish_modules_ordered(
                    &stdb_host_clone,
                    &stdb_db_clone,
                    &modules_dir,
                    false,
                )
                .await
                {
                    Ok(result) => {
                        tracing::info!(
                            status = result.status(),
                            published = result.total_ok,
                            failed = result.total_failed,
                            skipped = result.total_skipped,
                            "SpacetimeDB hydration complete: {} — {}/{} modules published",
                            result.status(),
                            result.total_ok,
                            result.total_ok + result.total_failed + result.total_skipped
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            "SpacetimeDB auto-hydration failed: {} — run `hex stdb hydrate` manually",
                            e
                        );
                    }
                }
            } else {
                tracing::debug!(
                    "spacetime-modules/ not found — skipping auto-hydration (normal for installed hex)"
                );
            }
        });
    }

    // Auto-register project + sync config files to SpacetimeDB (fire-and-forget)
    if let Ok(cwd) = std::env::current_dir() {
        let stdb_host = std::env::var("HEX_SPACETIMEDB_HOST")
            .unwrap_or_else(|_| "http://127.0.0.1:3033".to_string());
        let stdb_db = std::env::var("HEX_SPACETIMEDB_DATABASE")
            .unwrap_or_else(|_| hex_core::STDB_DATABASE_CORE.to_string());
        let sp_for_sync = app_state.state_port.clone();
        tokio::spawn(async move {
            // ADR-043: Auto-register project from .hex/project.yaml
            config_sync::auto_register_project(&cwd, &stdb_host, &stdb_db).await;
            // ADR-053: Sync config files with reporting (blueprint, skills, agents, hooks, MCP tools)
            let report = config_sync::sync_project_config_with_report(&cwd, &stdb_host, &stdb_db).await;
            tracing::info!("Config sync: {} synced, {} failed", report.synced, report.failed);
            // ADR-060: Notify agents of config changes
            if let Some(ref sp) = sp_for_sync {
                let project_id = std::env::var("HEX_PROJECT_ID").unwrap_or_default();
                config_sync::notify_config_change(sp.as_ref(), &project_id, &report).await;
            }
        });
    }

    // Initialize session persistence (ADR-036 / ADR-042 P2.5)
    // Try SpacetimeDB first (chat-relay module), fall back to SQLite
    {
        let stdb_host = std::env::var("HEX_SPACETIMEDB_HOST")
            .unwrap_or_else(|_| "http://127.0.0.1:3033".to_string());
        let chat_db = std::env::var("HEX_CHAT_STDB_DATABASE")
            .unwrap_or_else(|_| hex_core::stdb_database_for_module("chat-relay").to_string());

        let stdb_adapter = adapters::spacetime_session::SpacetimeSessionAdapter::new(
            stdb_host.clone(),
            chat_db.clone(),
        );

        if stdb_adapter.probe().await {
            app_state.session_port = Some(Arc::new(stdb_adapter));
            tracing::info!(
                "Session persistence active (SpacetimeDB: {}/{})",
                stdb_host,
                chat_db
            );
        } else {
            tracing::warn!(
                "SpacetimeDB chat-relay not reachable at {}/{} — session persistence unavailable",
                stdb_host,
                chat_db
            );
        }
    }
    // Tool-call event log uses in-memory ring buffer (ADR-2604020900) — initialized in AppState::new().

    // P9.5: Wire live context adapter (composition root) — must be set before
    // WorkplanExecutor is created so enrich_prompt can call it.
    app_state.live_context = Some(
        composition_root::build_live_context_adapter(config.port),
    );

    // Wrap in Arc, then create WorkplanExecutor (needs SharedState = Arc<AppState>)
    let state = Arc::new(app_state);

    if state.agent_manager.is_some() {
        // Pass inference_tx so workplan executor's inference_task_create broadcasts
        // to /ws/inference subscribers (ADR-2604011200 P2.T3 + P3.T1).
        if let Ok(state_port) = state_config::create_default_state_backend_with_inference(
            state.inference_tx.clone(),
        ) {
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

    // Background cleanup: hex_agent mark_inactive + evict_dead (ADR-058),
    // swarm_agent stale/dead cleanup, session cleanup, inbox expiry (ADR-060).
    // Always runs — SpacetimeDB scheduled reducers don't cover hex_agent lifecycle.
    {
        let cleanup_state = state.clone();
        cleanup::CleanupService::spawn(cleanup_state);
    }

    // Background brain self-improvement service (ADR-2604102200):
    // Tests local models periodically, records outcomes to RL engine.
    {
        let brain_state = state.clone();
        brain_service::spawn(brain_state);
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
        .unwrap_or_else(daemon::generate_token);

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

    // ADR-037: Spawn default local agent (opt-out with --no-agent or HEX_NO_AGENT=1)
    let no_agent = config.no_agent
        || std::env::var("HEX_NO_AGENT").map(|v| v == "1" || v.eq_ignore_ascii_case("true")).unwrap_or(false);

    if !no_agent {
        if let Some(ref agent_mgr) = _state.agent_manager {
            let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            let hub_url = format!("http://127.0.0.1:{}", config.port);
            match agent_mgr.spawn_local_agent(&hub_url, &cwd).await {
                Ok(pid) => {
                    tracing::info!(
                        pid = pid,
                        project = %cwd.display(),
                        "hex-agent started (PID {}) — project: {}",
                        pid,
                        cwd.display()
                    );
                }
                Err(e) => {
                    tracing::warn!("Could not auto-spawn local agent: {} — run with --no-agent to suppress", e);
                }
            }
        } else {
            // Fallback: spawn without AgentManager tracking
            let _agent_child = spawn_default_agent(config.port, &lock_token);
        }
    } else {
        tracing::info!("Agent auto-spawn disabled (--no-agent or HEX_NO_AGENT=1)");
    }

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await
        .expect("Server error");

    // ADR-037: Stop all locally-spawned agents on shutdown
    if let Some(ref agent_mgr) = _state.agent_manager {
        agent_mgr.stop_local_agents().await;
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

    // 2. ~/.hex/bin/hex-agent
    if let Ok(home) = std::env::var("HOME") {
        let hex_bin = std::path::PathBuf::from(home).join(".hex").join("bin").join("hex-agent");
        if hex_bin.exists() {
            return Some(hex_bin);
        }
    }

    // 3. In PATH
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

    // 4. Cargo target directory (dev mode)
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
