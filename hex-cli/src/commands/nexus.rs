use std::path::PathBuf;

use clap::Subcommand;
use colored::Colorize;

/// Default port for the hex-nexus daemon.
const DEFAULT_PORT: u16 = 5555;

/// Find the hex-nexus binary.
///
/// Search order:
/// 1. `HEX_NEXUS_BIN` env var
/// 2. `hex-nexus` on `$PATH`
/// 3. `./target/release/hex-nexus`
/// 4. `./target/debug/hex-nexus`
fn find_nexus_binary() -> Option<PathBuf> {
    // 1. Explicit env var
    if let Ok(p) = std::env::var("HEX_NEXUS_BIN") {
        let path = PathBuf::from(p);
        if path.is_file() {
            return Some(path);
        }
    }

    // 2. PATH lookup
    let path_var = std::env::var("PATH").unwrap_or_default();
    for dir in path_var.split(':') {
        let candidate = PathBuf::from(dir).join("hex-nexus");
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    // 3-4. Local build artifacts
    for profile in &["release", "debug"] {
        let candidate = PathBuf::from(format!("target/{}/hex-nexus", profile));
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    None
}

#[derive(Subcommand)]
pub enum NexusAction {
    /// Start the hex-nexus daemon
    Start {
        /// Port to listen on
        #[arg(short, long, default_value_t = DEFAULT_PORT)]
        port: u16,
        /// Auth token for dashboard/chat access
        #[arg(short, long)]
        token: Option<String>,
    },
    /// Stop the hex-nexus daemon
    Stop,
    /// Show daemon status
    Status,
    /// Tail daemon logs
    Logs {
        /// Number of lines to show
        #[arg(short = 'n', long, default_value_t = 50)]
        lines: usize,
        /// Follow log output
        #[arg(short, long)]
        follow: bool,
    },
}

fn hex_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".hex")
}

fn pid_file() -> PathBuf {
    hex_dir().join("nexus.pid")
}

fn port_file() -> PathBuf {
    hex_dir().join("nexus.port")
}

fn log_file() -> PathBuf {
    hex_dir().join("nexus.log")
}

/// Read the persisted port, falling back to DEFAULT_PORT.
fn read_port() -> u16 {
    std::fs::read_to_string(port_file())
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(DEFAULT_PORT)
}

/// Ensure SpacetimeDB is running. Starts it as a subprocess if not.
async fn ensure_spacetimedb() {
    let stdb_host = std::env::var("HEX_SPACETIMEDB_HOST")
        .unwrap_or_else(|_| "http://127.0.0.1:3000".to_string());

    // Check if already running
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .unwrap();

    if http.get(format!("{}/v1/ping", stdb_host))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
    {
        println!("{} SpacetimeDB already running at {}", "\u{2b21}".green(), stdb_host);
        return;
    }

    // Try to start SpacetimeDB
    println!("{} Starting SpacetimeDB...", "\u{2b21}".cyan());

    let stdb_log = hex_dir().join("spacetimedb.log");
    let log = match std::fs::OpenOptions::new().create(true).append(true).open(&stdb_log) {
        Ok(f) => f,
        Err(e) => {
            println!("  {} Could not open log file: {}", "!".yellow(), e);
            println!("  {} SpacetimeDB may need to be started manually: spacetime start", "\u{2192}".dimmed());
            return;
        }
    };
    let log_err = log.try_clone().unwrap();

    // Try `spacetime start` or `spacetimedb start`
    let started = try_start_spacetimedb("spacetime", &["start"], &log, &log_err)
        || try_start_spacetimedb("spacetimedb", &["start"], &log, &log_err);

    if !started {
        println!("  {} SpacetimeDB binary not found", "!".yellow());
        println!("  {} Install: https://spacetimedb.com/docs/getting-started", "\u{2192}".dimmed());
        println!("  {} Or start manually: spacetime start", "\u{2192}".dimmed());
        return;
    }

    // Wait for SpacetimeDB to become responsive
    for i in 0..15 {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        if http.get(format!("{}/v1/ping", stdb_host))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
        {
            println!("{} SpacetimeDB ready at {} ({}ms)", "\u{2b21}".green(), stdb_host, (i + 1) * 500);
            // Save PID for cleanup
            return;
        }
    }

    println!("  {} SpacetimeDB started but not responsive after 7.5s", "!".yellow());
    println!("  {} Check logs: {}", "\u{2192}".dimmed(), stdb_log.display());
}

fn try_start_spacetimedb(bin: &str, args: &[&str], stdout: &std::fs::File, stderr: &std::fs::File) -> bool {
    match std::process::Command::new(bin)
        .args(args)
        .stdout(stdout.try_clone().unwrap())
        .stderr(stderr.try_clone().unwrap())
        .spawn()
    {
        Ok(_child) => {
            println!("  {} Spawned: {} {}", "\u{2192}".dimmed(), bin, args.join(" "));
            true
        }
        Err(_) => false,
    }
}

pub async fn run(action: NexusAction) -> anyhow::Result<()> {
    match action {
        NexusAction::Start { port, token } => start(port, token.as_deref()).await,
        NexusAction::Stop => stop().await,
        NexusAction::Status => status().await,
        NexusAction::Logs { lines, follow } => logs(lines, follow).await,
    }
}

async fn start(port: u16, token: Option<&str>) -> anyhow::Result<()> {
    let pid_path = pid_file();

    // Check if already running
    if pid_path.exists() {
        if let Ok(pid_str) = tokio::fs::read_to_string(&pid_path).await {
            if let Ok(pid) = pid_str.trim().parse::<u32>() {
                if is_process_alive(pid) {
                    println!(
                        "{} hex-nexus is already running (PID {})",
                        "\u{2b21}".cyan(),
                        pid
                    );
                    return Ok(());
                }
            }
        }
    }

    // Ensure ~/.hex directory exists
    if let Some(parent) = pid_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    // ── Check if port is already in use (catches orphan processes) ──
    if is_port_in_use(port) {
        println!(
            "{} Port {} is already in use by another process",
            "\u{2b21}".red(),
            port
        );
        println!(
            "  {} Find it: lsof -i :{}", "\u{2192}".dimmed(), port
        );
        println!(
            "  {} Kill it: kill $(lsof -t -i :{})", "\u{2192}".dimmed(), port
        );
        println!(
            "  {} Or use a different port: hex nexus start --port {}",
            "\u{2192}".dimmed(),
            port + 1
        );
        return Ok(());
    }

    // ── Start SpacetimeDB if not running ────────────────
    ensure_spacetimedb().await;

    // Find the hex-nexus binary
    let nexus_bin = find_nexus_binary();
    let Some(nexus_bin) = nexus_bin else {
        println!(
            "{} hex-nexus binary not found",
            "\u{2b21}".red()
        );
        println!(
            "  {} Build it with: cargo build -p hex-nexus",
            "\u{2192}".dimmed()
        );
        return Ok(());
    };

    let log_path = log_file();
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;
    let log_err = log.try_clone()?;

    println!(
        "{} Starting hex-nexus on port {}...",
        "\u{2b21}".cyan(),
        port
    );

    let mut cmd = std::process::Command::new(&nexus_bin);
    cmd.args(["--port", &port.to_string(), "--daemon"]);
    if let Some(t) = token {
        cmd.args(["--token", t]);
    }
    let child = cmd.stdout(log).stderr(log_err).spawn()?;

    let pid = child.id();
    tokio::fs::write(&pid_path, pid.to_string()).await?;
    tokio::fs::write(port_file(), port.to_string()).await?;

    // Wait for the daemon to become responsive
    let url = format!("http://127.0.0.1:{}/api/version", port);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .ok();

    let mut ready = false;
    if let Some(ref client) = client {
        for _ in 0..10 {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;

            // Check if the process is still alive — if not, it crashed (e.g. port conflict)
            if !is_process_alive(pid) {
                // Read last few lines of log for the error
                let log_content = tokio::fs::read_to_string(&log_path).await.unwrap_or_default();
                let last_lines: Vec<&str> = log_content.lines().rev().take(5).collect();

                let is_port_conflict = last_lines.iter().any(|l| l.contains("AddrInUse") || l.contains("Address already in use"));

                if is_port_conflict {
                    println!(
                        "{} Port {} is already in use",
                        "\u{2b21}".red(),
                        port
                    );
                    println!(
                        "  {} Try a different port: hex nexus start --port {}",
                        "\u{2192}".dimmed(),
                        port + 1
                    );
                    println!(
                        "  {} Or check what's using it: lsof -i :{}",
                        "\u{2192}".dimmed(),
                        port
                    );
                } else {
                    println!(
                        "{} hex-nexus exited unexpectedly",
                        "\u{2b21}".red()
                    );
                    for line in last_lines.iter().rev() {
                        if !line.is_empty() {
                            println!("  {}", line.dimmed());
                        }
                    }
                }
                println!(
                    "  {} Logs: {}",
                    "\u{2192}".dimmed(),
                    log_path.display()
                );
                // Clean up stale PID + port files
                tokio::fs::remove_file(&pid_path).await.ok();
                tokio::fs::remove_file(port_file()).await.ok();
                return Ok(());
            }

            if client.get(&url).send().await.map(|r| r.status().is_success()).unwrap_or(false) {
                ready = true;
                break;
            }
        }
    }

    if ready {
        println!(
            "{} hex-nexus started (PID {}, port {})",
            "\u{2b21}".green(),
            pid,
            port
        );

        // Auto-start hex-chat web dashboard
        if let Some(chat_bin) = find_chat_binary() {
            let nexus_url = format!("http://127.0.0.1:{}", port);
            match std::process::Command::new(&chat_bin)
                .args(["web", "--nexus", &nexus_url])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
            {
                Ok(child) => {
                    println!(
                        "{} hex-chat web started (PID {}) at {}",
                        "\u{2b21}".green(),
                        child.id(),
                        "http://127.0.0.1:5556".blue().underline()
                    );
                }
                Err(e) => {
                    tracing::debug!("hex-chat web failed to start: {e}");
                }
            }
        }
    } else {
        println!(
            "{} hex-nexus spawned (PID {}) — not yet responsive",
            "\u{2b21}".yellow(),
            pid
        );
    }
    println!(
        "  {} Logs: {}",
        "\u{2192}".dimmed(),
        log_path.display()
    );

    Ok(())
}

/// Find the hex-chat binary (same search strategy as nexus).
fn find_chat_binary() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("HEX_CHAT_BIN") {
        let path = PathBuf::from(p);
        if path.is_file() { return Some(path); }
    }
    let path_var = std::env::var("PATH").unwrap_or_default();
    for dir in path_var.split(':') {
        let candidate = PathBuf::from(dir).join("hex-chat");
        if candidate.is_file() { return Some(candidate); }
    }
    for profile in &["release", "debug"] {
        let candidate = PathBuf::from(format!("target/{}/hex-chat", profile));
        if candidate.is_file() { return Some(candidate); }
    }
    let bin = PathBuf::from("./bin/hex-chat");
    if bin.is_file() { return Some(bin); }
    None
}

async fn stop() -> anyhow::Result<()> {
    let pid_path = pid_file();

    if !pid_path.exists() {
        println!("{} hex-nexus is not running (no PID file)", "\u{2b21}".dimmed());
        return Ok(());
    }

    let pid_str = tokio::fs::read_to_string(&pid_path).await?;
    let pid: u32 = pid_str
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid PID in {}", pid_path.display()))?;

    if !is_process_alive(pid) {
        println!(
            "{} hex-nexus process {} is not running (stale PID file)",
            "\u{2b21}".yellow(),
            pid
        );
        tokio::fs::remove_file(&pid_path).await?;
        tokio::fs::remove_file(port_file()).await.ok();
        return Ok(());
    }

    // Send SIGTERM
    #[cfg(unix)]
    {
        use std::process::Command;
        Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status()?;
    }

    // Wait briefly then clean up PID + port files
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    if pid_path.exists() {
        tokio::fs::remove_file(&pid_path).await?;
    }
    tokio::fs::remove_file(port_file()).await.ok();

    println!("{} hex-nexus stopped (PID {})", "\u{2b21}".green(), pid);

    // Optionally stop SpacetimeDB
    let _ = std::process::Command::new("spacetime")
        .args(["stop"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    println!("{} SpacetimeDB stop signal sent", "\u{2b21}".green());

    Ok(())
}

async fn status() -> anyhow::Result<()> {
    let pid_path = pid_file();

    if !pid_path.exists() {
        println!("{} hex-nexus is {}", "\u{2b21}".dimmed(), "not running".red());
        return Ok(());
    }

    let pid_str = tokio::fs::read_to_string(&pid_path).await?;
    let pid: u32 = pid_str
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid PID in {}", pid_path.display()))?;

    if is_process_alive(pid) {
        let port = read_port();
        println!("{} hex-nexus is {}", "\u{2b21}".cyan(), "running".green());
        println!("  PID:  {}", pid);
        println!("  Port: {}", port);

        // HTTP health check for SpacetimeDB and grant status
        let nexus_url = std::env::var("HEX_NEXUS_URL")
            .unwrap_or_else(|_| format!("http://127.0.0.1:{}", port));
        let nexus = crate::nexus_client::NexusClient::new(nexus_url);
        if nexus.ensure_running().await.is_ok() {
            println!("  API:  {}", nexus.url().green());

            // SpacetimeDB status
            match nexus.get("/secrets/grants").await {
                Ok(resp) => {
                    if let Some(grants) = resp.get("grants").and_then(|g| g.as_array()) {
                        let active = grants.iter().filter(|g| !g["claimed"].as_bool().unwrap_or(true)).count();
                        println!(
                            "  Grants: {} total ({} active)",
                            grants.len(),
                            active.to_string().green()
                        );
                    }
                }
                Err(_) => {}
            }

            // Check SpacetimeDB connectivity
            let stdb_host = std::env::var("HEX_SPACETIMEDB_HOST")
                .unwrap_or_else(|_| "http://127.0.0.1:3000".to_string());
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(2))
                .build()
                .ok();
            if let Some(ref client) = client {
                let stdb_ok = client
                    .get(format!("{}/v1/ping", stdb_host))
                    .send()
                    .await
                    .map(|r| r.status().is_success())
                    .unwrap_or(false);
                if stdb_ok {
                    println!("  SpacetimeDB: {} ({})", "connected".green(), stdb_host);
                } else {
                    println!("  SpacetimeDB: {} ({})", "unavailable".yellow(), stdb_host);
                }
            }

            // Inference providers
            match nexus.get("/api/inference/endpoints").await {
                Ok(data) => {
                    let endpoints = data.get("endpoints").and_then(|v| v.as_array());
                    if let Some(eps) = endpoints {
                        if eps.is_empty() {
                            println!("  Inference: {} (hex inference add to register)", "none".dimmed());
                        } else {
                            println!("  Inference: {} provider(s)", eps.len().to_string().green());
                            for ep in eps {
                                let provider = ep.get("provider").and_then(|v| v.as_str()).unwrap_or("?");
                                let model = ep.get("model").and_then(|v| v.as_str()).unwrap_or("default");
                                let url = ep.get("url").and_then(|v| v.as_str()).unwrap_or("?");
                                let status = ep.get("status").and_then(|v| v.as_str()).unwrap_or("unknown");
                                let icon = if status == "healthy" || status == "ok" { "\u{25cf}".green() } else { "\u{25cb}".yellow() };
                                println!("    {} {} {} ({})", icon, provider, model, url);
                            }
                        }
                    }
                }
                Err(_) => {}
            }

            // Agents
            match nexus.get("/api/agents").await {
                Ok(agents) => {
                    if let Some(arr) = agents.as_array() {
                        if arr.is_empty() {
                            println!("  Agents: {}", "none".dimmed());
                        } else {
                            println!("  Agents: {}", arr.len().to_string().green());
                            for a in arr {
                                let name = a.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                                let status = a.get("status").and_then(|v| v.as_str()).unwrap_or("?");
                                let icon = if status == "running" { "\u{25cf}".green() } else { "\u{25cb}".dimmed() };
                                println!("    {} {} ({})", icon, name, status);
                            }
                        }
                    }
                }
                Err(_) => {}
            }

            // Swarms
            match nexus.get("/api/swarms/active").await {
                Ok(swarms) => {
                    if let Some(arr) = swarms.as_array() {
                        if arr.is_empty() {
                            println!("  Swarms: {}", "none".dimmed());
                        } else {
                            println!("  Swarms: {} active", arr.len().to_string().green());
                            for s in arr {
                                let name = s.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                                println!("    \u{2b21} {}", name);
                            }
                        }
                    }
                }
                Err(_) => {}
            }

            // Sessions
            match nexus.get("/api/sessions?project_id=&limit=5").await {
                Ok(sessions) => {
                    if let Some(arr) = sessions.as_array() {
                        if arr.is_empty() {
                            println!("  Sessions: {}", "none".dimmed());
                        } else {
                            println!("  Sessions: {}", arr.len().to_string().green());
                        }
                    }
                }
                Err(_) => println!("  Sessions: {} (SQLite not enabled)", "unavailable".dimmed()),
            }

            // hex-chat web dashboard
            if let Some(ref client) = client {
                let chat_ok = client
                    .get("http://127.0.0.1:5556")
                    .send()
                    .await
                    .map(|r| r.status().is_success())
                    .unwrap_or(false);
                if chat_ok {
                    println!("  Dashboard: {} ({})", "running".green(), "http://127.0.0.1:5556".blue());
                } else {
                    println!("  Dashboard: {}", "not running".dimmed());
                }
            }
        } else {
            println!("  API:  {} (not responding)", nexus.url().yellow());
        }
    } else {
        println!(
            "{} hex-nexus is {} (stale PID file — cleaning up)",
            "\u{2b21}".yellow(),
            "not running".red()
        );
        tokio::fs::remove_file(&pid_path).await.ok();
        tokio::fs::remove_file(port_file()).await.ok();
    }

    Ok(())
}

async fn logs(lines: usize, follow: bool) -> anyhow::Result<()> {
    let log_path = log_file();

    if !log_path.exists() {
        println!("{} No log file found at {}", "\u{2b21}".dimmed(), log_path.display());
        return Ok(());
    }

    if follow {
        // Use tail -f for follow mode
        let mut child = tokio::process::Command::new("tail")
            .args(["-n", &lines.to_string(), "-f", &log_path.to_string_lossy()])
            .spawn()?;
        child.wait().await?;
    } else {
        let content = tokio::fs::read_to_string(&log_path).await?;
        let all_lines: Vec<&str> = content.lines().collect();
        let start = all_lines.len().saturating_sub(lines);
        for line in &all_lines[start..] {
            println!("{}", line);
        }
    }

    Ok(())
}

fn is_port_in_use(port: u16) -> bool {
    std::net::TcpListener::bind(format!("127.0.0.1:{}", port)).is_err()
}

fn is_process_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // kill -0 checks if process exists without sending a signal
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}
