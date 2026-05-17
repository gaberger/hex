//! Multi-project pulse command (ADR-2604131500 P6.1).
//!
//! `hex pulse` — one-glance status across all registered projects.
//! Shows state (blocked/decision/active/complete/idle), agents, and pending decisions.

use colored::Colorize;

use crate::nexus_client::NexusClient;

pub async fn run() -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    let response = nexus.get("/api/pulse").await?;

    let projects = response.as_array().cloned().unwrap_or_default();

    println!("{} hex pulse", "\u{2b21}".cyan());
    println!();

    if projects.is_empty() {
        println!("  {}", "No registered projects.".dimmed());
        return Ok(());
    }

    let mut total_agents: u64 = 0;
    let mut total_decisions: u64 = 0;
    let mut blocked = 0;
    let mut active = 0;

    // Column widths
    let name_w = projects
        .iter()
        .map(|p| p.get("name").and_then(|v| v.as_str()).unwrap_or("").len())
        .max()
        .unwrap_or(10)
        .max(7);

    println!(
        "  {:<w$}  {:<9}  {:>6}  {:>9}",
        "PROJECT".bold(),
        "STATE".bold(),
        "AGENTS".bold(),
        "DECISIONS".bold(),
        w = name_w
    );
    println!(
        "  {}",
        "-".repeat(name_w + 2 + 9 + 2 + 6 + 2 + 9).dimmed()
    );

    for proj in &projects {
        let name = proj.get("name").and_then(|v| v.as_str()).unwrap_or("unknown");
        let state = proj.get("state").and_then(|v| v.as_str()).unwrap_or("idle");
        let agents = proj.get("agent_count").and_then(|v| v.as_u64()).unwrap_or(0);
        let decisions = proj
            .get("decision_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        total_agents += agents;
        total_decisions += decisions;
        match state {
            "blocked" => blocked += 1,
            "active" => active += 1,
            _ => {}
        }

        let state_colored = match state {
            "blocked" => state.red().bold(),
            "decision" => state.yellow().bold(),
            "active" => state.green(),
            "complete" => state.cyan(),
            _ => state.dimmed(),
        };

        println!(
            "  {:<w$}  {:<9}  {:>6}  {:>9}",
            name,
            state_colored,
            agents,
            decisions,
            w = name_w
        );
    }

    println!();
    print_health(projects.len(), blocked, active, total_agents, total_decisions);

    Ok(())
}

fn print_health(project_count: usize, blocked: usize, active: usize, agents: u64, decisions: u64) {
    let health = if blocked > 0 {
        "degraded".red().bold()
    } else if decisions > 0 {
        "attention".yellow().bold()
    } else {
        "healthy".green().bold()
    };

    println!("  {}", "Summary:".bold());
    println!("    Health:    {}", health);
    println!("    Projects:  {}", project_count);
    println!("    Active:    {}", active);
    if blocked > 0 {
        println!("    Blocked:   {}", blocked.to_string().red());
    }
    println!("    Agents:    {}", agents);
    if decisions > 0 {
        println!("    Decisions: {}", decisions.to_string().yellow());
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[test]
    fn parses_pulse_response_shape() {
        let raw = json!([
            { "project_id": "p1", "name": "alpha", "state": "active",
              "agent_count": 2, "decision_count": 0 },
            { "project_id": "p2", "name": "beta", "state": "blocked",
              "agent_count": 0, "decision_count": 3 }
        ]);
        let arr = raw.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["name"], "alpha");
        assert_eq!(arr[1]["state"], "blocked");
        assert_eq!(arr[1]["decision_count"].as_u64(), Some(3));
    }
}
