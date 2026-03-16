mod daemon;
mod embed;
mod middleware;
mod routes;
mod state;

use std::sync::Arc;
use tracing_subscriber::EnvFilter;

const DEFAULT_PORT: u16 = 5555;

#[tokio::main]
async fn main() {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let args: Vec<String> = std::env::args().collect();
    let is_daemon = args.iter().any(|a| a == "--daemon");

    let port = args
        .iter()
        .position(|a| a == "--port")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(DEFAULT_PORT);

    let token = args
        .iter()
        .position(|a| a == "--token")
        .and_then(|i| args.get(i + 1).cloned())
        .or_else(|| std::env::var("HEX_DASHBOARD_TOKEN").ok());

    // Create shared state
    let state = Arc::new(state::AppState::new(token.clone()));

    // Build router
    let app = routes::build_router(state);

    // Write lock file
    let lock_token = token.unwrap_or_else(|| daemon::generate_token());
    if let Err(e) = daemon::write_lock(port, &lock_token) {
        tracing::warn!("Failed to write lock file: {}", e);
    }

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

    // Bind and serve
    let addr = format!("127.0.0.1:{}", port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("Failed to bind");

    if is_daemon {
        tracing::info!("hex-hub daemon started on http://{}", addr);
    } else {
        tracing::info!("hex-hub running on http://{}", addr);
    }

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await
        .expect("Server error");
}
