use hex_nexus::HubConfig;
use tracing_subscriber::EnvFilter;

// jemalloc global allocator (ADR-2026-06-04-1740). glibc malloc under the arena cap
// (MALLOC_ARENA_MAX=2) burned ~6 cores in arena-lock contention from serde_json::Value
// churn across the STDB poll loops. jemalloc's per-thread caches kill the lock storm and
// bound fragmentation, so the arena cap is no longer needed.
#[cfg(unix)]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();

    // Quick introspection flags (no daemon startup needed)
    if args.iter().any(|a| a == "--build-hash") {
        println!("{}", hex_nexus::build_hash());
        return;
    }
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!(
            "hex-nexus {} ({})",
            hex_nexus::version(),
            hex_nexus::build_hash()
        );
        return;
    }

    let _is_daemon = args.iter().any(|a| a == "--daemon");
    let no_agent = args.iter().any(|a| a == "--no-agent");

    let port = args
        .iter()
        .position(|a| a == "--port")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(hex_nexus::DEFAULT_PORT);

    let token = args
        .iter()
        .position(|a| a == "--token")
        .and_then(|i| args.get(i + 1).cloned())
        .or_else(|| std::env::var("HEX_DASHBOARD_TOKEN").ok());

    let bind = args
        .iter()
        .position(|a| a == "--bind")
        .and_then(|i| args.get(i + 1).cloned())
        .unwrap_or_else(|| "0.0.0.0".to_string());

    // Note: --daemon flag is accepted but ignored. The hex CLI handles process daemonization
    // by redirecting stdout/stderr to a log file and managing the process lifecycle.
    // Attempting to fork() here breaks Tokio's event loop (kqueue on macOS, epoll on Linux).

    // Initialize tracing (after daemonization)
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    hex_nexus::start_server(HubConfig {
        port,
        bind,
        token,
        is_daemon: false, // Daemonization handled by CLI, not here
        no_agent,
    })
    .await;
}
