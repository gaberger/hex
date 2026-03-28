//! Neural Lab commands.
//!
//! `hex neural-lab config|experiment|frontier|strategies|start-loop|stop-loop`
//! — delegates to hex-nexus neural-lab API endpoints.

use clap::Subcommand;
use colored::Colorize;
use serde_json::json;

use crate::nexus_client::NexusClient;

#[derive(Subcommand)]
pub enum NeuralLabAction {
    /// Manage model configurations
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// Create a new experiment
    Experiment {
        #[command(subcommand)]
        action: ExperimentAction,
    },
    /// List experiments
    Experiments {
        /// Filter by lineage name
        #[arg(long)]
        lineage: Option<String>,
        /// Filter by status (pending, running, completed, failed)
        #[arg(long)]
        status: Option<String>,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Show Pareto frontier for a lineage
    Frontier {
        /// Lineage name (defaults to all)
        lineage: Option<String>,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// List available mutation strategies
    Strategies {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Start the autonomous experiment loop for a lineage
    StartLoop {
        /// Lineage name
        #[arg(long)]
        lineage: Option<String>,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Stop the autonomous experiment loop for a lineage
    StopLoop {
        /// Lineage name
        #[arg(long)]
        lineage: Option<String>,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub enum ConfigAction {
    /// Create a new model configuration
    Create {
        /// Configuration name
        #[arg(long)]
        name: Option<String>,
        /// Parent config ID (for lineage tracking)
        #[arg(long)]
        parent: Option<String>,
        /// Number of transformer layers
        #[arg(long)]
        n_layer: Option<u32>,
        /// Number of attention heads
        #[arg(long)]
        n_head: Option<u32>,
        /// Embedding dimension
        #[arg(long)]
        n_embd: Option<u32>,
        /// Vocabulary size
        #[arg(long)]
        vocab_size: Option<u32>,
        /// Window/attention pattern (e.g. "sliding_window", "global")
        #[arg(long)]
        window_pattern: Option<String>,
        /// Activation function (e.g. "gelu", "swiglu")
        #[arg(long)]
        activation: Option<String>,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// List model configurations
    List {
        /// Filter by status (active, archived)
        #[arg(long)]
        status: Option<String>,
        /// Filter by lineage name
        #[arg(long)]
        lineage: Option<String>,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub enum ExperimentAction {
    /// Create a new experiment
    Create {
        /// Config ID to test
        #[arg(long)]
        config: String,
        /// Hypothesis text
        #[arg(long)]
        hypothesis: String,
        /// Lineage name
        #[arg(long)]
        lineage: Option<String>,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

pub async fn run(action: NeuralLabAction) -> anyhow::Result<()> {
    match action {
        NeuralLabAction::Config { action } => match action {
            ConfigAction::Create {
                name, parent, n_layer, n_head, n_embd, vocab_size,
                window_pattern, activation, json,
            } => config_create(name, parent, n_layer, n_head, n_embd, vocab_size, window_pattern, activation, json).await,
            ConfigAction::List { status, lineage, json } => config_list(status, lineage, json).await,
        },
        NeuralLabAction::Experiment { action } => match action {
            ExperimentAction::Create { config, hypothesis, lineage, json } => {
                experiment_create(&config, &hypothesis, lineage.as_deref(), json).await
            }
        },
        NeuralLabAction::Experiments { lineage, status, json } => {
            experiment_list(lineage.as_deref(), status.as_deref(), json).await
        }
        NeuralLabAction::Frontier { lineage, json } => {
            frontier(lineage.as_deref(), json).await
        }
        NeuralLabAction::Strategies { json } => strategies(json).await,
        NeuralLabAction::StartLoop { lineage, json } => start_loop(lineage.as_deref(), json).await,
        NeuralLabAction::StopLoop { lineage, json } => stop_loop(lineage.as_deref(), json).await,
    }
}

// ─── Config Commands ─────────────────────────────────────

#[allow(clippy::too_many_arguments)]
async fn config_create(
    name: Option<String>,
    parent: Option<String>,
    n_layer: Option<u32>,
    n_head: Option<u32>,
    n_embd: Option<u32>,
    vocab_size: Option<u32>,
    window_pattern: Option<String>,
    activation: Option<String>,
    json_output: bool,
) -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    let mut body = json!({});
    if let Some(n) = name { body["name"] = json!(n); }
    if let Some(p) = parent { body["parent_id"] = json!(p); }
    if let Some(v) = n_layer { body["n_layer"] = json!(v); }
    if let Some(v) = n_head { body["n_head"] = json!(v); }
    if let Some(v) = n_embd { body["n_embd"] = json!(v); }
    if let Some(v) = vocab_size { body["vocab_size"] = json!(v); }
    if let Some(v) = window_pattern { body["window_pattern"] = json!(v); }
    if let Some(v) = activation { body["activation"] = json!(v); }

    let resp = nexus.post("/api/neural-lab/configs", &body).await?;

    if json_output {
        println!("{}", serde_json::to_string_pretty(&resp)?);
    } else {
        let id = resp["id"].as_str().unwrap_or("-");
        let cfg_name = resp["name"].as_str().unwrap_or("-");
        println!("{} Config created", "\u{2b21}".green());
        println!("  ID:   {}", id);
        println!("  Name: {}", cfg_name.bold());
        if let Some(p) = resp["parent_id"].as_str() {
            println!("  Parent: {}", p);
        }
    }

    Ok(())
}

async fn config_list(
    status: Option<String>,
    lineage: Option<String>,
    json_output: bool,
) -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    let mut query_parts = Vec::new();
    if let Some(s) = &status {
        query_parts.push(format!("status={}", s));
    }
    if let Some(l) = &lineage {
        query_parts.push(format!("lineage={}", l));
    }
    let query = if query_parts.is_empty() {
        String::new()
    } else {
        format!("?{}", query_parts.join("&"))
    };

    let resp = nexus.get(&format!("/api/neural-lab/configs{}", query)).await?;

    if json_output {
        println!("{}", serde_json::to_string_pretty(&resp)?);
        return Ok(());
    }

    let configs = resp.as_array().cloned().unwrap_or_default();
    if configs.is_empty() {
        println!("{} No configurations found", "\u{2b21}".dimmed());
        return Ok(());
    }

    println!("{} Configs ({})", "\u{2b21}".cyan(), configs.len());
    println!();
    println!(
        "  {:<36} {:<20} {:<8} {:<8} {:<8} {}",
        "ID".bold(), "NAME".bold(), "LAYERS".bold(), "HEADS".bold(), "EMBD".bold(), "STATUS".bold()
    );
    println!("  {}", "\u{2500}".repeat(95).dimmed());

    for cfg in &configs {
        let id = cfg["id"].as_str().unwrap_or("-");
        let name = cfg["name"].as_str().unwrap_or("-");
        let n_layer = cfg["n_layer"].as_u64().map(|v| v.to_string()).unwrap_or("-".into());
        let n_head = cfg["n_head"].as_u64().map(|v| v.to_string()).unwrap_or("-".into());
        let n_embd = cfg["n_embd"].as_u64().map(|v| v.to_string()).unwrap_or("-".into());
        let status = cfg["status"].as_str().unwrap_or("active");

        let status_colored = match status {
            "active" => status.green().to_string(),
            "archived" => status.dimmed().to_string(),
            _ => status.to_string(),
        };

        let id_short = if id.len() > 34 { &id[..34] } else { id };
        println!(
            "  {:<36} {:<20} {:<8} {:<8} {:<8} {}",
            id_short, name, n_layer, n_head, n_embd, status_colored
        );
    }

    Ok(())
}

// ─── Experiment Commands ─────────────────────────────────

async fn experiment_create(
    config_id: &str,
    hypothesis: &str,
    lineage: Option<&str>,
    json_output: bool,
) -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    let mut body = json!({
        "config_id": config_id,
        "hypothesis": hypothesis,
    });
    if let Some(l) = lineage {
        body["lineage"] = json!(l);
    }

    let resp = nexus.post("/api/neural-lab/experiments", &body).await?;

    if json_output {
        println!("{}", serde_json::to_string_pretty(&resp)?);
    } else {
        let id = resp["id"].as_str().unwrap_or("-");
        println!("{} Experiment created", "\u{2b21}".green());
        println!("  ID:         {}", id);
        println!("  Config:     {}", config_id);
        println!("  Hypothesis: {}", hypothesis.bold());
        if let Some(l) = lineage {
            println!("  Lineage:    {}", l);
        }
    }

    Ok(())
}

async fn experiment_list(
    lineage: Option<&str>,
    status: Option<&str>,
    json_output: bool,
) -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    let mut query_parts = Vec::new();
    if let Some(l) = lineage {
        query_parts.push(format!("lineage={}", l));
    }
    if let Some(s) = status {
        query_parts.push(format!("status={}", s));
    }
    let query = if query_parts.is_empty() {
        String::new()
    } else {
        format!("?{}", query_parts.join("&"))
    };

    let resp = nexus.get(&format!("/api/neural-lab/experiments{}", query)).await?;

    if json_output {
        println!("{}", serde_json::to_string_pretty(&resp)?);
        return Ok(());
    }

    let experiments = resp.as_array().cloned().unwrap_or_default();
    if experiments.is_empty() {
        println!("{} No experiments found", "\u{2b21}".dimmed());
        return Ok(());
    }

    println!("{} Experiments ({})", "\u{2b21}".cyan(), experiments.len());
    println!();
    println!(
        "  {:<36} {:<12} {:<15} {}",
        "ID".bold(), "STATUS".bold(), "LINEAGE".bold(), "HYPOTHESIS".bold()
    );
    println!("  {}", "\u{2500}".repeat(90).dimmed());

    for exp in &experiments {
        let id = exp["id"].as_str().unwrap_or("-");
        let status = exp["status"].as_str().unwrap_or("pending");
        let lineage_name = exp["lineage"].as_str().unwrap_or("-");
        let hypothesis = exp["hypothesis"].as_str().unwrap_or("-");

        let status_colored = match status {
            "completed" => status.green().to_string(),
            "running" => status.yellow().to_string(),
            "pending" => status.dimmed().to_string(),
            "failed" => status.red().to_string(),
            _ => status.to_string(),
        };

        let id_short = if id.len() > 34 { &id[..34] } else { id };
        let hyp_short = if hypothesis.len() > 40 { format!("{}...", &hypothesis[..37]) } else { hypothesis.to_string() };
        println!(
            "  {:<36} {:<21} {:<15} {}",
            id_short, status_colored, lineage_name, hyp_short
        );
    }

    Ok(())
}

// ─── Frontier ────────────────────────────────────────────

async fn frontier(lineage: Option<&str>, json_output: bool) -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    let lineage_param = lineage.unwrap_or("default");
    let resp = nexus.get(&format!("/api/neural-lab/frontier/{}", lineage_param)).await?;

    if json_output {
        println!("{}", serde_json::to_string_pretty(&resp)?);
        return Ok(());
    }

    // The frontier endpoint returns { bestConfig: {...}, frontier: { best_val_bpb, total_kept, ... } }
    let frontier = &resp["frontier"];
    let best_bpb = frontier["best_val_bpb"].as_str().unwrap_or("");
    if best_bpb.is_empty() && resp["bestConfig"].is_null() {
        println!("{} No frontier data for lineage '{}'", "\u{2b21}".dimmed(), lineage_param);
        return Ok(());
    }

    println!("{} Research Frontier — {}", "\u{2b21}".cyan(), lineage_param.bold());
    println!();

    // Best config summary
    if let Some(cfg) = resp["bestConfig"].as_object() {
        let name = cfg.get("name").and_then(|v| v.as_str()).unwrap_or("-");
        let n_layer = cfg.get("n_layer").and_then(|v| v.as_u64()).unwrap_or(0);
        let n_head = cfg.get("n_head").and_then(|v| v.as_u64()).unwrap_or(0);
        let n_embd = cfg.get("n_embd").and_then(|v| v.as_u64()).unwrap_or(0);
        let window = cfg.get("window_pattern").and_then(|v| v.as_str()).unwrap_or("-");
        let activation = cfg.get("activation").and_then(|v| v.as_str()).unwrap_or("-");

        println!("  {} {}", "Best config:".bold(), name.green());
        println!("    val_bpb:    {}", best_bpb.green().bold());
        println!("    layers:     {}  heads: {}  embd: {}", n_layer, n_head, n_embd);
        println!("    window:     {}  activation: {}", window, activation);
    }

    // Experiment stats
    let kept = frontier["total_kept"].as_u64().unwrap_or(0);
    let discarded = frontier["total_discarded"].as_u64().unwrap_or(0);
    let total = frontier["total_experiments"].as_u64().unwrap_or(0);
    if total > 0 {
        println!();
        println!("  {} total: {}  kept: {}  discarded: {}",
            "Experiments:".bold(), total, kept.to_string().green(), discarded.to_string().red());
    }

    Ok(())
}


// ─── Strategies ──────────────────────────────────────────

async fn strategies(json_output: bool) -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    let resp = nexus.get("/api/neural-lab/strategies").await?;

    if json_output {
        println!("{}", serde_json::to_string_pretty(&resp)?);
        return Ok(());
    }

    let strats = resp.as_array().cloned().unwrap_or_default();
    if strats.is_empty() {
        println!("{} No strategies registered", "\u{2b21}".dimmed());
        return Ok(());
    }

    println!("{} Mutation Strategies ({})", "\u{2b21}".cyan(), strats.len());
    println!();
    println!(
        "  {:<16} {:<12} {:<12} {:<10} {}",
        "STRATEGY".bold(), "WEIGHT".bold(), "SUCCESS".bold(), "TRIED".bold(), "KEPT".bold()
    );
    println!("  {}", "\u{2500}".repeat(65).dimmed());

    for s in &strats {
        let name = s["strategy_name"].as_str().unwrap_or("-");
        let weight = s["selection_weight"].as_str().unwrap_or("0");
        let success = s["success_rate"].as_str().unwrap_or("0");
        let tried = s["total_tried"].as_u64().unwrap_or(0);
        let kept = s["total_kept"].as_u64().unwrap_or(0);

        // Render weight as a visual bar
        let w: f64 = weight.parse().unwrap_or(0.0);
        let bar_len = (w * 30.0) as usize;
        let bar = "\u{2588}".repeat(bar_len);

        println!("  {:<16} {:<12} {:<12} {:<10} {}", name, format!("{:.3}", w), format!("{:.3}", success.parse::<f64>().unwrap_or(0.0)), tried, kept);
        println!("  {}", bar.cyan());
    }

    Ok(())
}

// ─── Loop Control ────────────────────────────────────────

async fn start_loop(lineage: Option<&str>, json_output: bool) -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    let lineage_name = lineage.unwrap_or("default");
    let body = json!({
        "lineage": lineage_name,
        "status": "active",
    });
    let resp = nexus.post("/api/neural-lab/loop/start", &body).await?;

    if json_output {
        println!("{}", serde_json::to_string_pretty(&resp)?);
    } else {
        println!("{} Experiment loop started", "\u{2b21}".green());
        println!("  Lineage: {}", lineage_name.bold());
    }

    Ok(())
}

async fn stop_loop(lineage: Option<&str>, json_output: bool) -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    let lineage_name = lineage.unwrap_or("default");
    let body = json!({
        "lineage": lineage_name,
        "status": "stopped",
    });
    let resp = nexus.post("/api/neural-lab/loop/stop", &body).await?;

    if json_output {
        println!("{}", serde_json::to_string_pretty(&resp)?);
    } else {
        println!("{} Experiment loop stopped", "\u{2b21}".yellow());
        println!("  Lineage: {}", lineage_name.bold());
    }

    Ok(())
}
