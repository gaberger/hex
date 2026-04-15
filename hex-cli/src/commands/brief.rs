//! Developer briefing command.
//!
//! `hex brief` — shows a compact summary of recent events grouped by session.
//! `hex brief --full` — returns all events with full bodies (no truncation).

use clap::{Args, Subcommand};
use colored::Colorize;
use serde_json::Value;

use crate::nexus_client::NexusClient;

#[derive(Debug, Subcommand)]
pub enum BriefAction {
    /// Show the briefing
    Show(BriefArgs),
}

#[derive(Debug, Args)]
pub struct BriefArgs {
    /// Show all events with full bodies (no truncation, no limit)
    #[arg(long)]
    pub full: bool,

    /// Only show events after this ISO-8601 timestamp
    #[arg(long)]
    pub since: Option<String>,

    /// Only show decision-related events
    #[arg(long)]
    pub decisions_only: bool,

    /// Filter by project ID
    #[arg(long)]
    pub project: Option<String>,
}

pub async fn run(args: BriefArgs) -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    // Build query string
    let mut qs = Vec::new();

    if args.full {
        qs.push("limit=0".to_string());
        qs.push("summary=false".to_string());
    }

    if let Some(ref since) = args.since {
        qs.push(format!("since={}", since));
    }

    if args.decisions_only {
        qs.push("decisions=true".to_string());
    }

    if let Some(ref project) = args.project {
        qs.push(format!("project={}", project));
    }

    let path = if qs.is_empty() {
        "/api/briefing".to_string()
    } else {
        format!("/api/briefing?{}", qs.join("&"))
    };

    let response = nexus.get(&path).await?;

    // Render the briefing
    println!("{} hex briefing", "\u{2b21}".cyan());
    println!();

    let sessions = response
        .get("sessions")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    if sessions.is_empty() {
        println!("  {}", "No events found.".dimmed());
        return Ok(());
    }

    let total_sessions = response
        .get("total_sessions")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    println!("  {} session(s)", total_sessions);
    println!();

    for session in &sessions {
        let session_id = session
            .get("session_id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let truncated = session
            .get("truncated")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let total_events = session
            .get("total_events")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let events = session
            .get("events")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        // Truncate session ID for display
        let short_id = if session_id.len() > 12 {
            &session_id[..12]
        } else {
            session_id
        };

        println!(
            "  {} ({} event{})",
            short_id.bold(),
            total_events,
            if total_events == 1 { "" } else { "s" }
        );

        for event in &events {
            print_event(event);
        }

        if truncated {
            println!(
                "    {}",
                "(showing last 5 events per project \u{2014} use --full for complete history)"
                    .dimmed()
            );
        }

        println!();
    }

    Ok(())
}

fn print_event(event: &Value) {
    let event_type = event
        .get("event_type")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let created_at = event
        .get("created_at")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let tool_name = event.get("tool_name").and_then(|v| v.as_str());
    let exit_code = event.get("exit_code").and_then(|v| v.as_i64());

    // Format: "    HH:MM  event_type  [tool_name]  [exit_code]"
    let time_part = if created_at.len() >= 16 {
        // Extract HH:MM from ISO timestamp (e.g. "2026-04-13T14:30:00")
        &created_at[11..16]
    } else {
        created_at
    };

    let mut line = format!("    {}  {}", time_part.dimmed(), event_type);

    if let Some(tool) = tool_name {
        line.push_str(&format!("  {}", tool.cyan()));
    }

    if let Some(code) = exit_code {
        if code == 0 {
            line.push_str(&format!("  {}", "ok".green()));
        } else {
            line.push_str(&format!("  {}", format!("exit {}", code).red()));
        }
    }

    // Show duration if present
    if let Some(dur) = event.get("duration_ms").and_then(|v| v.as_u64()) {
        if dur > 1000 {
            line.push_str(&format!("  {:.1}s", dur as f64 / 1000.0));
        } else {
            line.push_str(&format!("  {}ms", dur));
        }
    }

    println!("{}", line);

    // Show truncated input/result in summary mode
    if let Some(result) = event.get("result_json").and_then(|v| v.as_str()) {
        if !result.is_empty() {
            println!("           {}", result.dimmed());
        }
    }
}
