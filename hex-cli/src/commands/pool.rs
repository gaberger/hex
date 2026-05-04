//! `hex pool` — operator interface for the STDB-backed worker supervisor
//! (wp-stdb-supervisor P4).
//!
//! Each subcommand calls a REST endpoint on hex-nexus, which calls the
//! corresponding STDB reducer. Falls back to a clear error if nexus is
//! down (no direct STDB call from the CLI).

use clap::Subcommand;
use colored::Colorize;
use serde_json::json;

use crate::nexus_client::NexusClient;

#[derive(Debug, Subcommand)]
pub enum PoolAction {
    /// Create or update a worker pool (declarative "I want N workers of role X")
    Create {
        /// Pool ID (free-form, e.g. "hex-coder-default")
        id: String,
        /// Persona role (must match a YAML in agents/hex/hex/<role>.yml)
        role: String,
        /// Number of workers to keep alive
        #[arg(long, short = 'n', default_value_t = 1)]
        count: u32,
        /// Restart strategy: permanent | transient | temporary
        #[arg(long, default_value = "permanent")]
        restart: String,
        /// Max restarts inside `--window` before crash-loop trips
        #[arg(long, default_value_t = 5)]
        max_restarts: u32,
        /// Window for crash-loop accounting (seconds)
        #[arg(long, default_value_t = 60)]
        window: u32,
        /// Create paused (operator must `resume` to start spawning)
        #[arg(long, default_value_t = false)]
        paused: bool,
    },
    /// List all pools with desired/alive/exited counts and crash-loop status
    List,
    /// Adjust desired_count without recreating. Useful for the auto-seeded
    /// placeholders: `hex pool scale hex-coder-default 3` opens the pool to 3
    /// workers AND clears the seed-default `paused` flag so the supervisor
    /// starts spawning.
    Scale {
        id: String,
        count: u32,
    },
    /// Pause a pool (supervisor stops emitting spawn_request)
    Pause {
        id: String,
    },
    /// Resume a paused pool (also clears any sticky in_crash_loop flag)
    Resume {
        id: String,
    },
    /// Delete a pool. Does NOT terminate running workers — they exit naturally.
    Delete {
        id: String,
    },
}

pub async fn run(action: PoolAction) -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    match action {
        PoolAction::Create { id, role, count, restart, max_restarts, window, paused } => {
            let body = json!({
                "id": id,
                "role": role,
                "desired_count": count,
                "restart_strategy": restart,
                "max_restarts": max_restarts,
                "max_restart_window_secs": window,
                "paused": paused,
                "owner_agent_id": "operator",
            });
            nexus.post("/api/pools", &body).await?;
            println!("{} pool {} created (role={}, desired={}, restart={})",
                "✓".green(), id.bold(), role, count, restart);
        }
        PoolAction::List => {
            let resp = nexus.get("/api/pools").await?;
            let pools = resp["pools"].as_array().cloned().unwrap_or_default();
            if pools.is_empty() {
                println!("{}", "no pools defined — `hex pool create <id> <role>` to start".dimmed());
                return Ok(());
            }
            println!("{:<28} {:<20} {:>9}  {:<11}  {}",
                "ID".bold(), "ROLE".bold(), "ALIVE".bold(), "STRATEGY".bold(), "STATUS".bold());
            for p in &pools {
                let id = p["id"].as_str().unwrap_or("?");
                let role = p["role"].as_str().unwrap_or("?");
                let desired = p["desiredCount"].as_u64().unwrap_or(0);
                let alive = p["aliveCount"].as_u64().unwrap_or(0);
                let exited = p["exitedCount"].as_u64().unwrap_or(0);
                let strat = p["restartStrategy"].as_str().unwrap_or("?");
                let paused = p["paused"].as_bool().unwrap_or(false);
                let crash = p["inCrashLoop"].as_bool().unwrap_or(false);
                let alive_text = format!("{}/{}", alive, desired);
                let alive_colored = if crash { alive_text.red().to_string() }
                    else if alive == desired { alive_text.green().to_string() }
                    else { alive_text.yellow().to_string() };
                let status = if crash { "CRASH-LOOP".red().to_string() }
                    else if paused { "paused".yellow().to_string() }
                    else { "active".green().to_string() };
                let exited_str = if exited > 0 { format!(" ({} exited)", exited).dimmed().to_string() } else { String::new() };
                println!("{:<28} {:<20} {:>9}  {:<11}  {}{}",
                    id, role, alive_colored, strat, status, exited_str);
            }
        }
        PoolAction::Scale { id, count } => {
            // Read the existing pool to preserve role / strategy / window etc.
            let resp = nexus.get("/api/pools").await?;
            let pools = resp["pools"].as_array().cloned().unwrap_or_default();
            let existing = pools.iter()
                .find(|p| p["id"].as_str() == Some(&id))
                .ok_or_else(|| anyhow::anyhow!("pool '{}' not found — use `hex pool create` first", id))?;
            let body = json!({
                "id": id,
                "role": existing["role"].as_str().unwrap_or(""),
                "desired_count": count,
                "restart_strategy": existing["restartStrategy"].as_str().unwrap_or("permanent"),
                "max_restarts": existing["maxRestarts"].as_u64().unwrap_or(5),
                "max_restart_window_secs": existing["maxRestartWindowSecs"].as_u64().unwrap_or(60),
                "paused": false,  // scaling implies enabling
                "owner_agent_id": "operator",
            });
            nexus.post("/api/pools", &body).await?;
            println!("{} pool {} scaled to {} (paused/crash-loop flags cleared)", "↗".cyan(), id.bold(), count);
        }
        PoolAction::Pause { id } => {
            nexus.patch(&format!("/api/pools/{}/paused", id), &json!({"paused": true})).await?;
            println!("{} pool {} paused", "⏸".yellow(), id.bold());
        }
        PoolAction::Resume { id } => {
            nexus.patch(&format!("/api/pools/{}/paused", id), &json!({"paused": false})).await?;
            println!("{} pool {} resumed (crash-loop flag cleared if set)", "▶".green(), id.bold());
        }
        PoolAction::Delete { id } => {
            nexus.delete(&format!("/api/pools/{}", id)).await?;
            println!("{} pool {} deleted", "✗".red(), id.bold());
        }
    }
    Ok(())
}
