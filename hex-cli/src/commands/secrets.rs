//! Secret management commands (ADR-026).
//!
//! `hex secrets has|status|list|grant|revoke|set|get`
//!
//! `has` and `status` work locally (env vars).
//! `list`, `grant`, `revoke`, `set`, `get` talk to the hex-nexus daemon.
//! `set` stores secret values via HexFlo memory (persisted to SQLite).
//! `grant` + `set` together = agent can claim the key.

use clap::Subcommand;
use colored::Colorize;
use serde_json::json;

use crate::nexus_client::NexusClient;

#[derive(Subcommand)]
pub enum SecretsAction {
    /// Check if a secret exists in the environment
    Has {
        /// Secret key name
        key: String,
    },
    /// Show configured backend and available keys
    Status,
    /// List all active secret grants from hex-nexus
    List,
    /// Create a secret grant for an agent
    Grant {
        /// Agent ID to grant the secret to
        agent_id: String,
        /// Secret key name (e.g. ANTHROPIC_API_KEY)
        secret_key: String,
        /// Time-to-live in seconds
        #[arg(long, default_value_t = 3600)]
        ttl: u64,
        /// Purpose tag (llm, auth, webhook)
        #[arg(long, default_value = "llm")]
        purpose: String,
    },
    /// Revoke secret grants for an agent
    Revoke {
        /// Agent ID whose grants to revoke
        agent_id: String,
        /// Specific key to revoke (omit to revoke all)
        secret_key: Option<String>,
    },
    /// Store a secret value (persisted in nexus)
    Set {
        /// Secret key name (e.g. MINIMAX_API_KEY)
        key: String,
        /// Secret value
        value: String,
    },
    /// Retrieve a stored secret value
    Get {
        /// Secret key name
        key: String,
    },
}

pub async fn run(action: SecretsAction) -> anyhow::Result<()> {
    match action {
        SecretsAction::Has { key } => has(&key).await,
        SecretsAction::Status => status().await,
        SecretsAction::List => list().await,
        SecretsAction::Grant {
            agent_id,
            secret_key,
            ttl,
            purpose,
        } => grant(&agent_id, &secret_key, ttl, &purpose).await,
        SecretsAction::Revoke {
            agent_id,
            secret_key,
        } => revoke(&agent_id, secret_key.as_deref()).await,
        SecretsAction::Set { key, value } => set_secret(&key, &value).await,
        SecretsAction::Get { key } => get_secret(&key).await,
    }
}

async fn has(key: &str) -> anyhow::Result<()> {
    if std::env::var(key).is_ok() {
        println!(
            "{} Secret {} is {}",
            "\u{2b21}".green(),
            key.bold(),
            "available".green()
        );
    } else {
        println!(
            "{} Secret {} is {}",
            "\u{2b21}".yellow(),
            key.bold(),
            "not set".red()
        );
    }
    Ok(())
}

async fn status() -> anyhow::Result<()> {
    println!("{} Secrets status", "\u{2b21}".cyan());

    // Local env var check
    println!("  Backend: {}", "environment variables".dimmed());
    let keys = [
        "ANTHROPIC_API_KEY",
        "OPENAI_API_KEY",
        "HEX_HUB_SECRET",
        "SPACETIMEDB_TOKEN",
    ];

    println!();
    println!("  {}", "Environment keys:".bold());
    for key in &keys {
        let present = std::env::var(key).is_ok();
        let indicator = if present {
            "\u{2713}".green()
        } else {
            "\u{2717}".red()
        };
        println!("    {} {}", indicator, key);
    }

    // Try to get SpacetimeDB grant info from nexus
    let nexus = NexusClient::from_env();
    println!();
    match nexus.get("/secrets/grants").await {
        Ok(resp) => {
            if let Some(grants) = resp.get("grants").and_then(|g| g.as_array()) {
                println!("  {}", "SpacetimeDB grants:".bold());
                if grants.is_empty() {
                    println!("    {} No active grants", "\u{25cb}".dimmed());
                } else {
                    let active = grants.iter().filter(|g| !g["claimed"].as_bool().unwrap_or(true)).count();
                    let claimed = grants.len() - active;
                    println!(
                        "    {} total, {} active, {} claimed",
                        grants.len().to_string().cyan(),
                        active.to_string().green(),
                        claimed.to_string().dimmed()
                    );
                }
            }
        }
        Err(_) => {
            println!(
                "  SpacetimeDB grants: {} (hex-nexus not reachable)",
                "unavailable".dimmed()
            );
        }
    }

    Ok(())
}

async fn list() -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    let resp = nexus.get("/secrets/grants").await?;
    let grants = resp
        .get("grants")
        .and_then(|g| g.as_array())
        .cloned()
        .unwrap_or_default();

    if grants.is_empty() {
        println!("{} No active secret grants", "\u{2b21}".dimmed());
        return Ok(());
    }

    println!("{} Secret grants ({})", "\u{2b21}".cyan(), grants.len());
    println!();
    println!(
        "  {:<20} {:<25} {:<10} {:<8} {}",
        "AGENT".bold(),
        "KEY".bold(),
        "PURPOSE".bold(),
        "CLAIMED".bold(),
        "EXPIRES".bold()
    );
    println!("  {}", "\u{2500}".repeat(80).dimmed());

    for grant in &grants {
        let agent = grant["agentId"].as_str().unwrap_or("-");
        let key = grant["secretKey"].as_str().unwrap_or("-");
        let purpose = grant["purpose"].as_str().unwrap_or("-");
        let claimed = grant["claimed"].as_bool().unwrap_or(false);
        let expires = grant["expiresAt"].as_str().unwrap_or("-");

        let claimed_str = if claimed {
            "yes".dimmed().to_string()
        } else {
            "no".green().to_string()
        };

        // Shorten the ISO timestamp for display
        let expires_short = if expires.len() > 19 {
            &expires[..19]
        } else {
            expires
        };

        println!(
            "  {:<20} {:<25} {:<10} {:<8} {}",
            agent, key, purpose, claimed_str, expires_short
        );
    }

    Ok(())
}

async fn grant(agent_id: &str, secret_key: &str, ttl: u64, purpose: &str) -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    let body = json!({
        "agentId": agent_id,
        "secretKey": secret_key,
        "purpose": purpose,
        "ttlSecs": ttl,
    });

    let resp = nexus.post("/secrets/grant", &body).await?;

    let id = resp["id"].as_str().unwrap_or("-");
    let expires = resp["expiresAt"].as_str().unwrap_or("-");

    println!(
        "{} Grant created: {} → {}",
        "\u{2b21}".green(),
        agent_id.cyan(),
        secret_key.bold()
    );
    println!("  ID: {}", id);
    println!("  Purpose: {}", purpose);
    println!("  TTL: {}s", ttl);
    println!("  Expires: {}", expires);

    Ok(())
}

async fn revoke(agent_id: &str, secret_key: Option<&str>) -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    let body = if let Some(key) = secret_key {
        json!({ "agentId": agent_id, "secretKey": key })
    } else {
        json!({ "agentId": agent_id })
    };

    let resp = nexus.post("/secrets/revoke", &body).await?;
    let revoked = resp["revoked"].as_u64().unwrap_or(0);

    if let Some(key) = secret_key {
        println!(
            "{} Revoked grant: {} → {}",
            "\u{2b21}".green(),
            agent_id.cyan(),
            key.bold()
        );
    } else {
        println!(
            "{} Revoked {} grant(s) for agent {}",
            "\u{2b21}".green(),
            revoked,
            agent_id.cyan()
        );
    }

    Ok(())
}

async fn set_secret(key: &str, value: &str) -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    nexus
        .post(
            "/api/secrets/vault",
            &json!({
                "key": key,
                "value": value,
            }),
        )
        .await?;

    println!(
        "{} Secret stored: {}",
        "\u{2b21}".green(),
        key.bold()
    );
    println!("  {} Value persisted via nexus ({})", "\u{2192}".dimmed(), nexus.url());

    Ok(())
}

async fn get_secret(key: &str) -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    let path = format!("/api/secrets/vault/{}", key);
    match nexus.get(&path).await {
        Ok(resp) => {
            let value = resp["value"].as_str().unwrap_or("");
            // Mask the value for display (show first 4 + last 4 chars)
            let masked = if value.len() > 12 {
                format!("{}...{}", &value[..4], &value[value.len()-4..])
            } else if value.is_empty() {
                "(empty)".to_string()
            } else {
                "****".to_string()
            };
            println!(
                "{} Secret: {} = {}",
                "\u{2b21}".green(),
                key.bold(),
                masked
            );
        }
        Err(e) => {
            let err_str = e.to_string();
            if err_str.contains("404") {
                println!(
                    "{} Secret '{}' not found",
                    "\u{2b21}".yellow(),
                    key
                );
            } else {
                return Err(e);
            }
        }
    }

    Ok(())
}
