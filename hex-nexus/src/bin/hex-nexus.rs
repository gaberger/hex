use hex_nexus::HubConfig;
use tracing_subscriber::EnvFilter;

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

    let is_daemon = args.iter().any(|a| a == "--daemon");
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
        .unwrap_or_else(|| "127.0.0.1".to_string());

    // Daemonize if requested (fork so parent can exit, child continues)
    if is_daemon {
        let hex_dir = dirs::home_dir()
            .map(|d| d.join(".hex"))
            .unwrap_or_else(|| std::path::PathBuf::from("/tmp"));
        let _ = std::fs::create_dir_all(&hex_dir);

        let pid_path = hex_dir.join("nexus.pid");

        // Fork: parent exits immediately, child continues in background
        // The child continues with all file descriptors intact (including redirected stdout/stderr).
        unsafe {
            match libc::fork() {
                -1 => {
                    eprintln!("Failed to fork");
                    std::process::exit(1);
                }
                0 => {
                    // Child process continues here
                    // Just write the PID file and continue to server initialization
                    let pid = libc::getpid();
                    let _ = std::fs::write(&pid_path, pid.to_string());
                    // Note: DO NOT call setsid() or redirect FDs — Tokio doesn't like it
                    // The CLI parent already has stdout/stderr redirected to the log file.
                }
                _ => {
                    // Parent process exits immediately
                    std::process::exit(0);
                }
            }
        }
    }

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
        is_daemon,
        no_agent,
    })
    .await;
}
