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
        /// Bind address (use 0.0.0.0 for remote agent access)
        #[arg(short, long, default_value = "127.0.0.1")]
        bind: String,
        /// Auth token for dashboard/chat access
        #[arg(short, long)]
        token: Option<String>,
        /// Don't auto-spawn default agent
        #[arg(long)]
        no_agent: bool,
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

fn token_file() -> PathBuf {
    hex_dir().join("nexus.token")
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

    if is_spacetimedb_reachable(&http, &stdb_host).await {
        println!("{} SpacetimeDB already running at {}", "\u{2b21}".green(), stdb_host);
        return;
    }

    // Something might be on port 3000 that isn't SpacetimeDB
    if http.get(format!("{}{}", stdb_host, hex_core::SPACETIMEDB_PING_PATH))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
    {
        println!(
            "  {} Port 3000 is in use by another service (not SpacetimeDB)",
            "!".yellow()
        );
        println!(
            "  {} Find it: lsof -i :3000",
            "\u{2192}".dimmed()
        );
        println!(
            "  {} Set HEX_SPACETIMEDB_HOST to use a different port",
            "\u{2192}".dimmed()
        );
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
        if is_spacetimedb_reachable(&http, &stdb_host).await {
            println!("{} SpacetimeDB ready at {} ({}ms)", "\u{2b21}".green(), stdb_host, (i + 1) * 500);
            return;
        }
    }

    println!("  {} SpacetimeDB started but not responsive after 7.5s", "!".yellow());
    println!("  {} Check logs: {}", "\u{2192}".dimmed(), stdb_log.display());
}

/// Verify that SpacetimeDB is actually running on the given host.
///
/// A simple 200 check is insufficient — any web server (e.g. Next.js on :3000)
/// will return 200 for unknown paths. We verify by checking that the response
/// Content-Type is NOT text/html (SpacetimeDB returns text/plain or JSON).
async fn is_spacetimedb_reachable(http: &reqwest::Client, host: &str) -> bool {
    match http.get(format!("{}{}", host, hex_core::SPACETIMEDB_PING_PATH)).send().await {
        Ok(r) if r.status().is_success() => {
            let ct = r.headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            // SpacetimeDB ping returns text/plain or no content-type, never text/html
            !ct.contains("text/html")
        }
        _ => false,
    }
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
        NexusAction::Start { port, bind, token, no_agent } => start(port, &bind, token.as_deref(), no_agent).await,
        NexusAction::Stop => stop().await,
        NexusAction::Status => status().await,
        NexusAction::Logs { lines, follow } => logs(lines, follow).await,
    }
}

async fn start(port: u16, bind: &str, token: Option<&str>, no_agent: bool) -> anyhow::Result<()> {
    let pid_path = pid_file();

    // Check if already running
    if pid_path.exists() {
        if let Ok(pid_str) = tokio::fs::read_to_string(&pid_path).await {
            if let Ok(pid) = pid_str.trim().parse::<u32>() {
                if is_process_alive(pid) {
                    // Check if the running daemon is stale (binary rebuilt since last start)
                    let port = read_port();
                    if is_daemon_stale(port).await {
                        println!(
                            "{} hex-nexus binary was rebuilt — restarting daemon...",
                            "\u{2b21}".yellow()
                        );
                        stop().await.ok();
                        // Fall through to normal start path below
                    } else {
                        // Nexus is running and up-to-date — ensure SpacetimeDB is up
                        ensure_spacetimedb().await;
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
    }

    // Ensure ~/.hex directory exists
    if let Some(parent) = pid_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    // ── Check if port is already in use (catches orphan processes) ──
    if is_port_in_use(port) {
        // Try to detect an orphaned hex-nexus on this port
        if let Some(orphan_pid) = find_orphan_nexus_pid(port) {
            println!(
                "{} Found orphaned hex-nexus (PID {}) on port {} — adopting",
                "\u{2b21}".cyan(),
                orphan_pid,
                port
            );
            adopt_orphan(orphan_pid, port).await?;
            println!(
                "{} hex-nexus is already running (PID {})",
                "\u{2b21}".green(),
                orphan_pid
            );
            return Ok(());
        }

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
    cmd.args(["--port", &port.to_string(), "--bind", bind, "--daemon"]);
    if let Some(t) = token {
        cmd.args(["--token", t]);
    }
    let child = cmd.stdout(log).stderr(log_err).spawn()?;

    let pid = child.id();
    tokio::fs::write(&pid_path, pid.to_string()).await?;
    tokio::fs::write(port_file(), port.to_string()).await?;
    if let Some(t) = token {
        tokio::fs::write(token_file(), t).await?;
    } else {
        // Remove stale token file if no token is set
        let _ = tokio::fs::remove_file(token_file()).await;
    }

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

        // Auto-register current project if not already registered
        let project_dir = std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| ".".to_string());
        let nexus_url = format!("http://127.0.0.1:{}", port);
        let nexus = crate::nexus_client::NexusClient::new(nexus_url.clone());
        auto_register_project(&nexus, &project_dir).await;

        // Auto-start default hex-agent (ADR-037)
        if !no_agent {
            if let Some(agent_bin) = find_agent_binary() {

                // Query nexus for registered inference endpoints to pass to agent
                let mut cmd = std::process::Command::new(&agent_bin);
                cmd.args(["--hub-url", &nexus_url, "--project-dir", &project_dir]);

                // Forward inference provider config as env vars
                if let Ok(resp) = nexus.get("/api/inference/endpoints").await {
                    let resp: serde_json::Value = resp;
                    if let Some(eps) = resp.get("endpoints").and_then(|v| v.as_array()) {
                        for ep in eps {
                            let provider = ep["provider"].as_str().unwrap_or("");
                            let url = ep["url"].as_str().unwrap_or("");
                            let model = ep["model"].as_str().unwrap_or("");
                            match provider {
                                "ollama" => {
                                    cmd.env("HEX_OLLAMA_HOST", url);
                                    cmd.env("HEX_OLLAMA_MODEL", model);
                                    cmd.args(["--model", model]);
                                }
                                "vllm" | "openai_compat" | "openai-compatible" => {
                                    cmd.env("HEX_INFERENCE_URL", format!("{}/v1", url));
                                    cmd.env("HEX_INFERENCE_MODEL", model);
                                    cmd.args(["--model", model]);
                                }
                                _ => {}
                            }
                        }
                    }
                }

                // Also forward ANTHROPIC_API_KEY if available
                if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
                    cmd.env("ANTHROPIC_API_KEY", key);
                }

                match cmd
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                {
                    Ok(child) => {
                        let agent_pid = child.id();
                        println!(
                            "{} hex-agent started (PID {}) \u{2014} project: {}",
                            "\u{2b21}".green(),
                            agent_pid,
                            project_dir
                        );

                        // Register this agent in the unified hex_agent table (ADR-058)
                        let hostname = gethostname::gethostname().to_string_lossy().to_string();
                        let agent_name = format!("nexus-agent-{}", &hostname);
                        let reg_body = serde_json::json!({
                            "name": agent_name,
                            "host": hostname,
                            "project_dir": project_dir,
                            "session_id": format!("nexus-{}", agent_pid),
                        });
                        let _ = nexus.post("/api/hex-agents/connect", &reg_body).await
                            .map(|_| {
                                tracing::debug!("Nexus agent registered as {}", agent_name);
                            })
                            .map_err(|e| {
                                tracing::debug!("Nexus agent registration failed (non-fatal): {e}");
                            });
                    }
                    Err(e) => {
                        tracing::debug!("hex-agent failed to start: {e}");
                    }
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

/// Find the hex-agent binary (same search strategy as nexus).
fn find_agent_binary() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("HEX_AGENT_BIN") {
        let path = PathBuf::from(p);
        if path.is_file() { return Some(path); }
    }
    let path_var = std::env::var("PATH").unwrap_or_default();
    for dir in path_var.split(':') {
        let candidate = PathBuf::from(dir).join("hex-agent");
        if candidate.is_file() { return Some(candidate); }
    }
    for profile in &["release", "debug"] {
        let candidate = PathBuf::from(format!("target/{}/hex-agent", profile));
        if candidate.is_file() { return Some(candidate); }
    }
    let bin = PathBuf::from("./bin/hex-agent");
    if bin.is_file() { return Some(bin); }
    None
}

async fn stop() -> anyhow::Result<()> {
    let pid_path = pid_file();

    if !pid_path.exists() {
        // Fallback: check for orphaned hex-nexus before giving up
        let port = read_port();
        if let Some(orphan_pid) = find_orphan_nexus_pid(port) {
            println!(
                "{} Found orphaned hex-nexus (PID {}) on port {} — adopting before stop",
                "\u{2b21}".cyan(),
                orphan_pid,
                port
            );
            adopt_orphan(orphan_pid, port).await?;
            // Fall through to the normal stop logic
        } else {
            println!("{} hex-nexus is not running (no PID file)", "\u{2b21}".dimmed());
            return Ok(());
        }
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
        // Fallback: probe the default port for an orphaned hex-nexus
        let port = read_port();
        if let Some(orphan_pid) = find_orphan_nexus_pid(port) {
            println!(
                "{} Found orphaned hex-nexus (PID {}) — adopting",
                "\u{2b21}".cyan(),
                orphan_pid
            );
            adopt_orphan(orphan_pid, port).await?;
            // Fall through to the normal status display with the adopted PID
        } else {
            println!("{} hex-nexus is {}", "\u{2b21}".dimmed(), "not running".red());
            return Ok(());
        }
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

        // Stale binary warning
        if is_daemon_stale(port).await {
            println!(
                "  Binary: {} (run `hex nexus start` to auto-restart)",
                "STALE — rebuilt since last start".yellow()
            );
        }

        // HTTP health check for SpacetimeDB and grant status
        let nexus_url = std::env::var("HEX_NEXUS_URL")
            .unwrap_or_else(|_| format!("http://127.0.0.1:{}", port));
        let nexus = crate::nexus_client::NexusClient::new(nexus_url);
        if nexus.ensure_running().await.is_ok() {
            println!("  API:  {}", nexus.url().green());

            // SpacetimeDB status
            if let Ok(resp) = nexus.get("/secrets/grants").await {
                if let Some(grants) = resp.get("grants").and_then(|g| g.as_array()) {
                    let active = grants.iter().filter(|g| !g["claimed"].as_bool().unwrap_or(true)).count();
                    println!(
                        "  Grants: {} total ({} active)",
                        grants.len(),
                        active.to_string().green()
                    );
                }
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
                    .get(format!("{}{}", stdb_host, hex_core::SPACETIMEDB_PING_PATH))
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
            if let Ok(data) = nexus.get("/api/inference/endpoints").await {
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

            // Agents
            if let Ok(agents) = nexus.get("/api/agents").await {
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

            // Swarms
            if let Ok(swarms) = nexus.get("/api/swarms/active").await {
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

            // Dashboard is served by hex-nexus at the same port
            println!("  Dashboard: {}", format!("http://127.0.0.1:{}", 5555).blue());
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

/// Auto-register the current working directory as a project if not already registered.
///
/// Checks `GET /api/projects` for an existing entry matching `project_dir`.
/// If none exists, registers via `POST /api/projects/register`.
/// Failures are non-fatal — logged and swallowed so nexus startup isn't blocked.
async fn auto_register_project(nexus: &crate::nexus_client::NexusClient, project_dir: &str) {
    // Give SpacetimeDB state port time to initialize after daemon starts.
    // The version endpoint responds before the state port is fully connected.
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // Derive project name from directory basename
    let name = project_dir
        .rsplit('/')
        .next()
        .unwrap_or("unknown")
        .to_string();

    let body = serde_json::json!({
        "rootPath": project_dir,
        "name": name,
    });

    // Try registration up to 2 times — first attempt may hit state port not ready
    for attempt in 0..2 {
        // Check if already registered
        if let Ok(resp) = nexus.get("/api/projects").await {
            if let Some(projects) = resp.get("projects").and_then(|v| v.as_array()) {
                if !projects.is_empty() {
                    let already = projects.iter().any(|p| {
                        p.get("rootPath").and_then(|v| v.as_str()) == Some(project_dir)
                    });
                    if already {
                        return;
                    }
                }
            }
        }

        match nexus.post("/api/projects/register", &body).await {
            Ok(resp) => {
                let id = resp.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                println!(
                    "{} Project registered: {} ({})",
                    "\u{2b21}".green(),
                    name,
                    &id[..8.min(id.len())]
                );
                return;
            }
            Err(e) => {
                if attempt == 0 {
                    // Retry after a short delay — state port may still be connecting
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                } else {
                    println!(
                        "  {} Auto-register project failed: {}",
                        "!".yellow(),
                        e
                    );
                }
            }
        }
    }
}

fn is_port_in_use(port: u16) -> bool {
    std::net::TcpListener::bind(format!("127.0.0.1:{}", port)).is_err()
}

/// Find an orphaned hex-nexus process listening on `port` when the PID file is missing.
///
/// Uses `lsof` to ask the kernel which process owns the LISTEN socket, then verifies
/// the process name contains "hex-nexus". Returns `None` if the port is free or owned
/// by a non-nexus process.
fn find_orphan_nexus_pid(port: u16) -> Option<u32> {
    #[cfg(unix)]
    {
        // lsof -t -i :<port> -sTCP:LISTEN returns just the PID(s)
        let output = std::process::Command::new("lsof")
            .args(["-t", &format!("-i:{}", port), "-sTCP:LISTEN"])
            .output()
            .ok()?;
        let pids_str = String::from_utf8_lossy(&output.stdout);
        for line in pids_str.lines() {
            if let Ok(pid) = line.trim().parse::<u32>() {
                // Verify this is actually hex-nexus (not some random process on the port)
                if let Ok(ps_out) = std::process::Command::new("ps")
                    .args(["-p", &pid.to_string(), "-o", "comm="])
                    .output()
                {
                    let comm = String::from_utf8_lossy(&ps_out.stdout);
                    if comm.trim().contains("hex-nexus") {
                        return Some(pid);
                    }
                }
            }
        }
        None
    }
    #[cfg(not(unix))]
    {
        let _ = port;
        None
    }
}

/// Adopt an orphaned hex-nexus by writing its PID and port to the tracking files.
async fn adopt_orphan(pid: u32, port: u16) -> anyhow::Result<()> {
    let dir = hex_dir();
    tokio::fs::create_dir_all(&dir).await?;
    tokio::fs::write(pid_file(), pid.to_string()).await?;
    tokio::fs::write(port_file(), port.to_string()).await?;
    Ok(())
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

/// Check if the running daemon's build hash differs from the on-disk binary.
/// Returns true if the daemon should be restarted (stale binary).
async fn is_daemon_stale(port: u16) -> bool {
    // 1. Get the running daemon's build hash via /api/version
    let running_hash = match get_running_build_hash(port).await {
        Some(h) => h,
        None => return false, // Can't reach daemon — don't interfere
    };

    // 2. Get the on-disk binary's build hash
    let disk_hash = match get_disk_build_hash() {
        Some(h) => h,
        None => return false, // Can't find binary — don't interfere
    };

    if running_hash != disk_hash {
        tracing::debug!(
            running = %running_hash,
            disk = %disk_hash,
            "Daemon build hash mismatch — binary was rebuilt"
        );
        true
    } else {
        false
    }
}

/// Query the running daemon's /api/version endpoint for its buildHash.
async fn get_running_build_hash(port: u16) -> Option<String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .ok()?;

    let resp = client
        .get(format!("http://127.0.0.1:{}/api/version", port))
        .send()
        .await
        .ok()?;

    let json: serde_json::Value = resp.json().await.ok()?;
    json.get("buildHash")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Run `hex-nexus --build-hash` on the on-disk binary to get its compiled hash.
fn get_disk_build_hash() -> Option<String> {
    let nexus_bin = find_nexus_binary()?;
    let output = std::process::Command::new(&nexus_bin)
        .arg("--build-hash")
        .output()
        .ok()?;

    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}
