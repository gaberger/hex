use hex_nexus::HubConfig;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

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

    let is_daemon = args.iter().any(|a| a == "--daemon");

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
        .unwrap_or_else(|| "127.0.0.1".to_string());

    hex_nexus::start_server(HubConfig {
        port,
        bind,
        token,
        is_daemon,
    })
    .await;
}
