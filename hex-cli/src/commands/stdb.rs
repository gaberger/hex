//! SpacetimeDB management commands.
//!
//! Provides `hex stdb status|start|stop|publish|generate` for managing
//! the local SpacetimeDB instance used by hex-nexus.

use std::path::PathBuf;
use std::process::Stdio;

use clap::Subcommand;
use colored::Colorize;

#[derive(Subcommand)]
pub enum StdbAction {
    /// Check SpacetimeDB status and health
    Status,
    /// Start a local SpacetimeDB instance
    Start {
        /// Listen port
        #[arg(short, long, default_value_t = 3000)]
        port: u16,
    },
    /// Stop the local SpacetimeDB instance
    Stop,
    /// Publish WASM modules from spacetime-modules/
    Publish {
        /// Path to spacetime-modules workspace
        #[arg(short, long, default_value = "spacetime-modules")]
        modules: String,
        /// SpacetimeDB host
        #[arg(long, default_value = "http://127.0.0.1:3000")]
        host: String,
        /// Database name
        #[arg(long, default_value = "hex")]
        database: String,
    },
    /// Regenerate Rust SDK bindings from published modules
    Generate {
        /// Output directory for bindings
        #[arg(short, long, default_value = "hex-nexus/src/spacetime_bindings")]
        out: String,
        /// SpacetimeDB host
        #[arg(long, default_value = "http://127.0.0.1:3000")]
        host: String,
        /// Database name
        #[arg(long, default_value = "hex")]
        database: String,
    },
}

pub async fn run(action: StdbAction) -> anyhow::Result<()> {
    match action {
        StdbAction::Status => status().await,
        StdbAction::Start { port } => start(port).await,
        StdbAction::Stop => stop().await,
        StdbAction::Publish {
            modules,
            host,
            database,
        } => publish(&modules, &host, &database).await,
        StdbAction::Generate {
            out,
            host,
            database,
        } => generate(&out, &host, &database).await,
    }
}

// ── Binary discovery ─────────────────────────────────────

/// Find the spacetime CLI binary on PATH.
fn find_binary() -> anyhow::Result<PathBuf> {
    for name in &["spacetime", "spacetimedb"] {
        if let Ok(path) = which(name) {
            return Ok(path);
        }
    }
    anyhow::bail!(
        "SpacetimeDB CLI not found on PATH.\n  \
         Install: https://spacetimedb.com/install\n  \
         Or set PATH to include the spacetime binary"
    )
}

fn which(name: &str) -> Result<PathBuf, ()> {
    let path_var = std::env::var("PATH").unwrap_or_default();
    for dir in path_var.split(':') {
        let candidate = PathBuf::from(dir).join(name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(())
}

// ── Subcommands ──────────────────────────────────────────

async fn status() -> anyhow::Result<()> {
    println!("{} SpacetimeDB status", "\u{2b21}".cyan());

    // Check binary
    match find_binary() {
        Ok(path) => {
            println!("  Binary: {}", path.display().to_string().green());

            // Get version (with timeout — some CLI versions phone home)
            let version_fut = tokio::process::Command::new(&path)
                .arg("version")
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .output();

            match tokio::time::timeout(std::time::Duration::from_secs(3), version_fut).await {
                Ok(Ok(out)) if out.status.success() => {
                    let ver = String::from_utf8_lossy(&out.stdout);
                    println!("  Version: {}", ver.trim());
                }
                _ => {
                    println!("  Version: {}", "(timeout)".dimmed());
                }
            }
        }
        Err(_) => {
            println!("  Binary: {}", "not found".red());
            println!(
                "  {} Install from https://spacetimedb.com/install",
                "\u{2192}".dimmed()
            );
            return Ok(());
        }
    }

    // Check if running (try ping)
    let host = std::env::var("HEX_SPACETIMEDB_HOST")
        .unwrap_or_else(|_| "http://127.0.0.1:3000".to_string());

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()?;

    let ping_url = format!("{}/v1/ping", host);
    match client.get(&ping_url).send().await {
        Ok(r) if r.status().is_success() => {
            println!("  Status: {}", "running".green());
            println!("  Host: {}", host);
        }
        _ => {
            println!("  Status: {}", "not running".red());
            println!("  Host: {} (expected)", host.dimmed());
            println!(
                "  {} Start with: hex stdb start",
                "\u{2192}".dimmed()
            );
        }
    }

    // Check database
    let database = std::env::var("HEX_SPACETIMEDB_DATABASE")
        .unwrap_or_else(|_| "hex".to_string());
    println!("  Database: {}", database);

    // List modules
    let modules_dir = PathBuf::from("spacetime-modules");
    if modules_dir.is_dir() {
        println!();
        println!("  {}", "Modules:".bold());
        if let Ok(entries) = std::fs::read_dir(&modules_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() && path.join("Cargo.toml").exists() {
                    let name = path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    println!("    {} {}", "\u{25cb}".dimmed(), name);
                }
            }
        }
    }

    Ok(())
}

async fn start(port: u16) -> anyhow::Result<()> {
    let binary = find_binary()?;

    // Check if already running
    let host = format!("http://127.0.0.1:{}", port);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()?;

    if client
        .get(format!("{}/v1/ping", host))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
    {
        println!(
            "{} SpacetimeDB already running on port {}",
            "\u{2b21}".cyan(),
            port
        );
        return Ok(());
    }

    println!(
        "{} Starting SpacetimeDB on port {}...",
        "\u{2b21}".cyan(),
        port
    );

    // Spawn as background process
    let child = tokio::process::Command::new(&binary)
        .arg("start")
        .arg("--listen-addr")
        .arg(format!("127.0.0.1:{}", port))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();

    match child {
        Ok(_) => {
            // Wait for it to become ready
            let mut ready = false;
            for _ in 0..20 {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                if client
                    .get(format!("{}/v1/ping", host))
                    .send()
                    .await
                    .map(|r| r.status().is_success())
                    .unwrap_or(false)
                {
                    ready = true;
                    break;
                }
            }

            if ready {
                println!(
                    "{} SpacetimeDB started on {}",
                    "\u{2b21}".green(),
                    host
                );
            } else {
                println!(
                    "{} SpacetimeDB process spawned but not yet responsive on {}",
                    "\u{2b21}".yellow(),
                    host
                );
            }
        }
        Err(e) => {
            anyhow::bail!("Failed to start SpacetimeDB: {}", e);
        }
    }

    Ok(())
}

async fn stop() -> anyhow::Result<()> {
    let host = std::env::var("HEX_SPACETIMEDB_HOST")
        .unwrap_or_else(|_| "http://127.0.0.1:3000".to_string());

    // SpacetimeDB doesn't have a clean stop endpoint in all versions,
    // so we find and kill the process.
    let output = tokio::process::Command::new("pkill")
        .args(["-f", "spacetime.*start"])
        .output()
        .await;

    match output {
        Ok(o) if o.status.success() => {
            println!("{} SpacetimeDB stopped", "\u{2b21}".green());
        }
        _ => {
            // Check if it was even running
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(2))
                .build()?;

            if client
                .get(format!("{}/v1/ping", host))
                .send()
                .await
                .map(|r| r.status().is_success())
                .unwrap_or(false)
            {
                println!(
                    "{} SpacetimeDB is running but could not be stopped",
                    "\u{2b21}".yellow()
                );
            } else {
                println!(
                    "{} SpacetimeDB is not running",
                    "\u{2b21}".dimmed()
                );
            }
        }
    }

    Ok(())
}

async fn publish(modules_dir: &str, host: &str, database: &str) -> anyhow::Result<()> {
    let binary = find_binary()?;
    let modules_path = PathBuf::from(modules_dir);

    if !modules_path.is_dir() {
        anyhow::bail!(
            "Modules directory not found: {}\n  Run from the project root",
            modules_dir
        );
    }

    // Check SpacetimeDB is running
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()?;

    if !client
        .get(format!("{}/v1/ping", host))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
    {
        anyhow::bail!(
            "SpacetimeDB is not running at {}\n  Start with: hex stdb start",
            host
        );
    }

    println!(
        "{} Publishing modules to {} (database: {})",
        "\u{2b21}".cyan(),
        host,
        database
    );

    let mut entries: Vec<_> = std::fs::read_dir(&modules_path)?
        .filter_map(|e| e.ok())
        .filter(|e| {
            let p = e.path();
            p.is_dir() && p.join("Cargo.toml").exists()
        })
        .collect();
    entries.sort_by_key(|e| e.file_name());

    let mut success = 0;
    let mut failed = 0;

    for entry in &entries {
        let name = entry.file_name().to_string_lossy().to_string();
        print!("  {} {} ... ", "\u{25cb}".dimmed(), name);

        let output = tokio::process::Command::new(&binary)
            .arg("publish")
            .arg("--server")
            .arg(host)
            .arg(database)
            .arg("--project-path")
            .arg(entry.path())
            .arg("--yes")
            .output()
            .await;

        match output {
            Ok(o) if o.status.success() => {
                println!("{}", "OK".green());
                success += 1;
            }
            Ok(o) => {
                let stderr = String::from_utf8_lossy(&o.stderr);
                println!("{}", "FAILED".red());
                eprintln!("    {}", stderr.trim().dimmed());
                failed += 1;
            }
            Err(e) => {
                println!("{}", "ERROR".red());
                eprintln!("    {}", e.to_string().dimmed());
                failed += 1;
            }
        }
    }

    println!();
    println!(
        "{} Published: {} OK, {} failed",
        "\u{2b21}".cyan(),
        success.to_string().green(),
        if failed > 0 {
            failed.to_string().red()
        } else {
            failed.to_string().green()
        }
    );

    Ok(())
}

async fn generate(out_dir: &str, host: &str, database: &str) -> anyhow::Result<()> {
    let binary = find_binary()?;

    // Check SpacetimeDB is running
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()?;

    if !client
        .get(format!("{}/v1/ping", host))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
    {
        anyhow::bail!(
            "SpacetimeDB is not running at {}\n  Start with: hex stdb start",
            host
        );
    }

    println!(
        "{} Generating Rust bindings from {} → {}",
        "\u{2b21}".cyan(),
        database,
        out_dir
    );

    let output = tokio::process::Command::new(&binary)
        .arg("generate")
        .arg("--lang")
        .arg("rust")
        .arg("--out-dir")
        .arg(out_dir)
        .arg("--project-path")
        .arg(database)
        .arg("--server")
        .arg(host)
        .output()
        .await?;

    if output.status.success() {
        println!("{} Bindings generated at {}", "\u{2b21}".green(), out_dir);
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("spacetime generate failed: {}", stderr.trim());
    }

    Ok(())
}
