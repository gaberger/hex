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
        #[arg(long, default_value = "http://127.0.0.1:3033")]
        host: String,
        /// Database name
        #[arg(long, default_value = "hex")]
        database: String,
    },
    /// Hydrate SpacetimeDB with all WASM module schemas (no application data)
    Hydrate {
        /// SpacetimeDB host
        #[arg(long, default_value = "http://127.0.0.1:3033")]
        host: String,
        /// Database name
        #[arg(long, default_value = "hex")]
        database: String,
        /// Force re-publish even if modules exist
        #[arg(short, long)]
        force: bool,
        /// Show what would be done without doing it
        #[arg(long)]
        dry_run: bool,
    },
    /// Regenerate Rust SDK bindings from published modules
    Generate {
        /// Output directory for bindings
        #[arg(short, long, default_value = "hex-nexus/src/spacetime_bindings")]
        out: String,
        /// SpacetimeDB host
        #[arg(long, default_value = "http://127.0.0.1:3033")]
        host: String,
        /// Database name
        #[arg(long, default_value = "hex")]
        database: String,
    },
    /// Run a SQL SELECT query against SpacetimeDB (no nexus required)
    Query {
        /// SQL query to execute
        sql: String,
        /// Database to query
        #[arg(long, default_value = "hex")]
        db: String,
        /// Output raw JSON instead of a formatted table
        #[arg(long)]
        json: bool,
        /// Print only the row count
        #[arg(long)]
        count: bool,
    },
    /// List tables in a SpacetimeDB database (no nexus required)
    Tables {
        /// Database to inspect
        #[arg(long, default_value = "hex")]
        db: String,
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
        StdbAction::Hydrate {
            host,
            database,
            force,
            dry_run,
        } => hydrate(&host, &database, force, dry_run).await,
        StdbAction::Generate {
            out,
            host,
            database,
        } => generate(&out, &host, &database).await,
        StdbAction::Query { sql, db, json, count } => query_stdb(&sql, &db, json, count).await,
        StdbAction::Tables { db } => tables_stdb(&db).await,
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
        .unwrap_or_else(|_| "http://127.0.0.1:3033".to_string());

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()?;

    let ping_url = format!("{}{}", host, hex_core::SPACETIMEDB_PING_PATH);
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
        .get(format!("{}{}", host, hex_core::SPACETIMEDB_PING_PATH))
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
                    .get(format!("{}{}", host, hex_core::SPACETIMEDB_PING_PATH))
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
        .unwrap_or_else(|_| "http://127.0.0.1:3033".to_string());

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
                .get(format!("{}{}", host, hex_core::SPACETIMEDB_PING_PATH))
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
        .get(format!("{}{}", host, hex_core::SPACETIMEDB_PING_PATH))
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

        // Each module is published to its own database (module name = database name).
        // The `database` arg here is just the fallback/legacy alias; per-module names
        // are canonical so `hex stdb generate` and SDK bindings resolve correctly.
        let db_name = name.clone();
        let output = tokio::process::Command::new(&binary)
            .arg("publish")
            .arg("--server")
            .arg(host)
            .arg(&db_name)
            .arg("--module-path")
            .arg(entry.path())
            .arg("--delete-data=on-conflict")
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

/// Module publish order — tiered by dependency (ADR-2604050900: right-sized to 7).
/// Tier 0 has no cross-module deps; Tier 1 depends on Tier 0.
const MODULE_TIERS: &[&[&str]] = &[
    // Tier 0: Foundation — no cross-module dependencies
    &[
        "hexflo-coordination",
        "agent-registry",
        "secret-grant",
    ],
    // Tier 1: Services — reference agent/project IDs from tier 0
    &[
        "inference-gateway",
        "rl-engine",
        "chat-relay",
        "neural-lab",
    ],
];

async fn hydrate(host: &str, database: &str, force: bool, dry_run: bool) -> anyhow::Result<()> {
    println!(
        "{} Hydrating SpacetimeDB schemas ({})",
        "\u{2b21}".cyan(),
        database
    );

    // 1. Try delegating to hex-nexus (it has embedded WASM modules)
    let nexus = crate::nexus_client::NexusClient::from_env();
    if nexus.ensure_running().await.is_ok() {
        println!("  {} Delegating to hex-nexus...", "\u{2192}".dimmed());
        let body = serde_json::json!({
            "host": host,
            "database": database,
            "force": force,
            "dry_run": dry_run,
        });

        match nexus.post("/api/stdb/hydrate", &body).await {
            Ok(resp) => {
                // Report results from nexus
                let modules = resp.get("modules_published")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let status = resp.get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");

                if dry_run {
                    println!();
                    println!(
                        "{} Dry run: would publish {} modules in {} tiers",
                        "\u{2b21}".cyan(),
                        MODULE_TIERS.iter().map(|t| t.len()).sum::<usize>(),
                        MODULE_TIERS.len()
                    );
                    for (i, tier) in MODULE_TIERS.iter().enumerate() {
                        println!("  Tier {}: {}", i, tier.join(", "));
                    }
                    return Ok(());
                }

                println!();
                let status_colored = match status {
                    "hydrated" => status.green().to_string(),
                    "partial" => status.yellow().to_string(),
                    _ => status.red().to_string(),
                };
                println!(
                    "{} Hydration complete: {} modules published, status: {}",
                    "\u{2b21}".green(),
                    modules,
                    status_colored
                );

                // Show per-tier breakdown if available
                if let Some(tiers) = resp.get("tiers").and_then(|v| v.as_array()) {
                    for tier_info in tiers {
                        let tier_num = tier_info.get("tier").and_then(|v| v.as_u64()).unwrap_or(0);
                        let tier_ok = tier_info.get("ok").and_then(|v| v.as_u64()).unwrap_or(0);
                        let tier_total = tier_info.get("total").and_then(|v| v.as_u64()).unwrap_or(0);
                        let mark = if tier_ok == tier_total {
                            "\u{2713}".green().to_string()
                        } else {
                            "\u{2717}".red().to_string()
                        };
                        println!("  {} Tier {}: {}/{}", mark, tier_num, tier_ok, tier_total);
                    }
                }

                return Ok(());
            }
            Err(e) => {
                println!(
                    "  {} Nexus hydrate endpoint unavailable ({}), falling back to local publish",
                    "\u{2717}".yellow(),
                    e
                );
            }
        }
    } else {
        println!(
            "  {} hex-nexus not running, using local publish",
            "\u{2192}".dimmed()
        );
    }

    // 2. Fallback: local ordered publish using spacetime CLI
    let binary = find_binary()?;
    let modules_path = PathBuf::from("spacetime-modules");
    if !modules_path.is_dir() {
        anyhow::bail!(
            "spacetime-modules/ not found.\n  Run from the project root or use hex-nexus for embedded modules"
        );
    }

    // Verify SpacetimeDB is running
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()?;
    if !client
        .get(format!("{}{}", host, hex_core::SPACETIMEDB_PING_PATH))
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

    if dry_run {
        println!();
        println!(
            "{} Dry run: would publish {} modules in {} tiers",
            "\u{2b21}".cyan(),
            MODULE_TIERS.iter().map(|t| t.len()).sum::<usize>(),
            MODULE_TIERS.len()
        );
        for (i, tier) in MODULE_TIERS.iter().enumerate() {
            println!("  Tier {}: {}", i, tier.join(", "));
        }
        return Ok(());
    }

    let mut total_ok = 0u32;
    let mut total_fail = 0u32;

    for (tier_idx, tier_modules) in MODULE_TIERS.iter().enumerate() {
        println!();
        println!(
            "  {} Tier {} ({} modules)",
            "\u{25b6}".cyan(),
            tier_idx,
            tier_modules.len()
        );

        let mut tier_ok = 0u32;

        for module_name in *tier_modules {
            let module_path = modules_path.join(module_name);
            if !module_path.is_dir() {
                println!("    {} {} {}", "\u{25cb}".dimmed(), module_name, "SKIP (not found)".dimmed());
                continue;
            }

            print!("    {} {} ... ", "\u{25cb}".dimmed(), module_name);

            let output = tokio::process::Command::new(&binary)
                .arg("publish")
                .arg("--server")
                .arg(host)
                .arg(*module_name)
                .arg("--module-path")
                .arg(&module_path)
                .arg("--delete-data=on-conflict")
                .arg("--yes")
                .output()
                .await;

            match output {
                Ok(o) if o.status.success() => {
                    println!("{}", "OK".green());
                    tier_ok += 1;
                    total_ok += 1;
                }
                Ok(o) => {
                    let stderr = String::from_utf8_lossy(&o.stderr);
                    println!("{}", "FAILED".red());
                    eprintln!("      {}", stderr.trim().dimmed());
                    total_fail += 1;
                }
                Err(e) => {
                    println!("{}", "ERROR".red());
                    eprintln!("      {}", e.to_string().dimmed());
                    total_fail += 1;
                }
            }
        }

        if tier_ok < tier_modules.len() as u32 {
            println!(
                "    {} Tier {} incomplete ({}/{}), subsequent tiers may fail",
                "\u{26a0}".yellow(),
                tier_idx,
                tier_ok,
                tier_modules.len()
            );
        }
    }

    println!();
    let status = if total_fail == 0 { "hydrated" } else { "partial" };
    let status_colored = if total_fail == 0 {
        status.green().to_string()
    } else {
        status.yellow().to_string()
    };
    println!(
        "{} Hydration complete: {} OK, {} failed — status: {}",
        "\u{2b21}".green(),
        total_ok.to_string().green(),
        if total_fail > 0 {
            total_fail.to_string().red()
        } else {
            total_fail.to_string().green()
        },
        status_colored
    );

    Ok(())
}

async fn query_stdb(sql: &str, db: &str, json_mode: bool, count_only: bool) -> anyhow::Result<()> {
    let host = std::env::var("HEX_SPACETIMEDB_HOST")
        .unwrap_or_else(|_| "http://127.0.0.1:3033".to_string());
    let url = format!("{}/v1/database/{}/sql", host, db);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;

    let resp = client
        .post(&url)
        .header("Content-Type", "text/plain")
        .body(sql.to_string())
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Cannot reach SpacetimeDB at {} — is it running?\n  {}", host, e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("SpacetimeDB returned {}: {}", status, body.trim());
    }

    let results: serde_json::Value = resp.json().await
        .map_err(|e| anyhow::anyhow!("Failed to parse SpacetimeDB response: {}", e))?;

    if json_mode {
        println!("{}", serde_json::to_string_pretty(&results)?);
        return Ok(());
    }

    let result = results.as_array()
        .and_then(|a| a.first())
        .ok_or_else(|| anyhow::anyhow!("Unexpected response format"))?;

    // Extract column names from schema
    let cols: Vec<String> = result["schema"]["elements"]
        .as_array()
        .map(|els| {
            els.iter()
                .map(|el| {
                    el["name"]["some"].as_str().unwrap_or("?").to_string()
                })
                .collect()
        })
        .unwrap_or_default();

    let rows = result["rows"].as_array().map(|r| r.as_slice()).unwrap_or_default();

    if count_only {
        println!("{}", rows.len());
        return Ok(());
    }

    if cols.is_empty() {
        println!("(no columns)");
        return Ok(());
    }

    // Calculate column widths
    let mut widths: Vec<usize> = cols.iter().map(|c| c.len()).collect();
    for row in rows {
        if let Some(vals) = row.as_array() {
            for (i, val) in vals.iter().enumerate() {
                if i < widths.len() {
                    let s = val_to_str(val);
                    widths[i] = widths[i].max(s.len());
                }
            }
        }
    }

    // Print header
    let header: Vec<String> = cols.iter().enumerate()
        .map(|(i, c)| format!("{:<width$}", c, width = widths[i]))
        .collect();
    println!("{}", header.join("  "));
    println!("{}", widths.iter().map(|w| "-".repeat(*w)).collect::<Vec<_>>().join("  "));

    // Print rows
    for row in rows {
        if let Some(vals) = row.as_array() {
            let cells: Vec<String> = vals.iter().enumerate()
                .map(|(i, v)| {
                    let w = widths.get(i).copied().unwrap_or(0);
                    format!("{:<width$}", val_to_str(v), width = w)
                })
                .collect();
            println!("{}", cells.join("  "));
        }
    }

    println!();
    println!("({} row{})", rows.len(), if rows.len() == 1 { "" } else { "s" });
    Ok(())
}

fn val_to_str(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => "NULL".to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        other => other.to_string(),
    }
}

async fn tables_stdb(db: &str) -> anyhow::Result<()> {
    let host = std::env::var("HEX_SPACETIMEDB_HOST")
        .unwrap_or_else(|_| "http://127.0.0.1:3033".to_string());
    let url = format!("{}/v1/database/{}/schema", host, db);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()?;

    // Try schema endpoint first (lists tables + their types)
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Cannot reach SpacetimeDB at {} — is it running?\n  {}", host, e))?;

    if resp.status().is_success() {
        let schema: serde_json::Value = resp.json().await
            .map_err(|e| anyhow::anyhow!("Failed to parse schema: {}", e))?;

        // SpacetimeDB schema response has "tables" array
        if let Some(tables) = schema["tables"].as_array() {
            println!("{} Tables in '{}' ({} total):", "\u{2b21}".cyan(), db, tables.len());
            for t in tables {
                let name = t["name"].as_str().unwrap_or("?");
                println!("  {}", name);
            }
            return Ok(());
        }
    }

    // Fallback: SQL query
    println!("{} Querying tables via SQL...", "\u{2192}".dimmed());
    query_stdb("SELECT * FROM st_table", db, false, false).await
}

async fn generate(out_dir: &str, host: &str, database: &str) -> anyhow::Result<()> {
    let binary = find_binary()?;

    // Check SpacetimeDB is running
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()?;

    if !client
        .get(format!("{}{}", host, hex_core::SPACETIMEDB_PING_PATH))
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
        .arg("--server")
        .arg(host)
        .arg(database)
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
