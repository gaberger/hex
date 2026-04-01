//! `hex chat` — Interactive AI chat session
//!
//! With TUI (default): launches a full-screen ratatui streaming chat.
//! With --no-tui: plain stdout, pipe-friendly.

use clap::Args;
use anyhow::Result;

use crate::nexus_client::NexusClient;

#[derive(Args, Debug)]
pub struct ChatArgs {
    /// Message to send (skips interactive input in --no-tui mode)
    #[arg(short = 'm', long)]
    pub message: Option<String>,

    /// Model override (e.g. "llama3", "claude-sonnet-4-20250514")
    #[arg(short = 'M', long)]
    pub model: Option<String>,

    /// Plain stdout mode — no TUI, suitable for pipes and scripts
    #[arg(long)]
    pub no_tui: bool,

    /// System prompt to prepend to the conversation
    #[arg(short = 's', long)]
    pub system: Option<String>,
}

pub async fn run(args: ChatArgs) -> Result<()> {
    if args.no_tui {
        run_plain(args).await
    } else {
        crate::tui::chat::run(args).await
    }
}

async fn run_plain(args: ChatArgs) -> Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    let message = args.message.unwrap_or_else(|| {
        eprintln!("Error: --message is required in --no-tui mode");
        std::process::exit(1);
    });

    let messages = vec![serde_json::json!({"role": "user", "content": message})];

    let mut req_body = serde_json::json!({
        "messages": messages,
    });
    if let Some(model) = &args.model {
        req_body["model"] = serde_json::Value::String(model.clone());
    }
    if let Some(system) = &args.system {
        req_body["system"] = serde_json::Value::String(system.clone());
    }

    let json = nexus.post_long("/api/inference/complete", &req_body).await?;
    let content = json["content"].as_str().unwrap_or_default();
    println!("{}", content);
    Ok(())
}
