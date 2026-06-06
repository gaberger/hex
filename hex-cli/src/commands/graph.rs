//! `hex graph` — build and query a knowledge graph of the project.
//!
//! Thin client over the nexus `/api/graph/*` endpoints (which run the `hex-graph`
//! engine). Mirrors graphify's core verbs: build / query / path / explain.

use clap::{Args, Subcommand};
use colored::Colorize;
use serde_json::{json, Value};

use crate::nexus_client::NexusClient;

#[derive(Debug, Subcommand)]
pub enum GraphAction {
    /// Build (or rebuild) the knowledge graph for a project directory.
    Build(BuildArgs),
    /// Search the graph with a natural-language question.
    Query(QueryArgs),
    /// Find the shortest path between two nodes (id or exact label).
    Path(PathArgs),
    /// Explain a node — its kind, community, and relationships.
    Explain(ExplainArgs),
}

#[derive(Debug, Args)]
pub struct BuildArgs {
    /// Project directory to analyze (default: detected project root).
    #[arg(default_value = ".")]
    pub path: String,
    /// "ast" (default, no LLM) or "deep" (LLM-inferred edges from docs).
    #[arg(long, default_value = "ast")]
    pub mode: String,
    /// Skip documentation (Markdown) nodes.
    #[arg(long)]
    pub no_docs: bool,
    /// Mirror the graph into the knowledge-graph SpacetimeDB module.
    #[arg(long)]
    pub persist: bool,
    /// Model for deep-mode semantic inference.
    #[arg(long)]
    pub model: Option<String>,
}

#[derive(Debug, Args)]
pub struct QueryArgs {
    /// The question to search the graph with.
    pub question: String,
    #[arg(long, default_value = ".")]
    pub path: String,
    #[arg(long, default_value_t = 15)]
    pub limit: usize,
}

#[derive(Debug, Args)]
pub struct PathArgs {
    /// Source node (id or exact label).
    pub from: String,
    /// Target node (id or exact label).
    pub to: String,
    #[arg(long, default_value = ".")]
    pub path: String,
}

#[derive(Debug, Args)]
pub struct ExplainArgs {
    /// Node to explain (id or exact label).
    pub node: String,
    #[arg(long, default_value = ".")]
    pub path: String,
}

pub async fn run(action: GraphAction) -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;
    match action {
        GraphAction::Build(a) => build(&nexus, a).await,
        GraphAction::Query(a) => query(&nexus, a).await,
        GraphAction::Path(a) => path(&nexus, a).await,
        GraphAction::Explain(a) => explain(&nexus, a).await,
    }
}

async fn build(nexus: &NexusClient, a: BuildArgs) -> anyhow::Result<()> {
    let body = json!({
        "path": a.path,
        "mode": a.mode,
        "include_docs": !a.no_docs,
        "persist": a.persist,
        "model": a.model,
    });
    println!("{} building knowledge graph ({})…", "\u{2b21}".cyan(), a.mode);
    let resp = nexus.post_long("/api/graph/build", &body).await?;

    println!(
        "{} {} nodes, {} edges, {} communities  [{}]",
        "\u{2713}".green(),
        num(&resp, "node_count"),
        num(&resp, "edge_count"),
        num(&resp, "community_count"),
        resp.get("mode").and_then(|v| v.as_str()).unwrap_or("ast"),
    );
    if let Some(out) = resp.get("out_file").and_then(|v| v.as_str()) {
        println!("  {} {}", "graph:".dimmed(), out);
    }
    if let Some(gods) = resp.get("god_nodes").and_then(|v| v.as_array()) {
        let labels: Vec<&str> = gods.iter().filter_map(|v| v.as_str()).take(5).collect();
        if !labels.is_empty() {
            println!("  {} {}", "hubs:".dimmed(), labels.join(", "));
        }
    }
    match (resp.get("persisted").and_then(|v| v.as_bool()), resp.get("persist_error").and_then(|v| v.as_str())) {
        (Some(true), _) => println!("  {} knowledge-graph STDB module", "persisted:".dimmed()),
        (_, Some(e)) => println!("  {} {}", "persist skipped:".yellow(), e),
        _ => {}
    }
    Ok(())
}

async fn query(nexus: &NexusClient, a: QueryArgs) -> anyhow::Result<()> {
    let body = json!({ "path": a.path, "question": a.question, "limit": a.limit });
    let resp = nexus.post("/api/graph/query", &body).await?;
    let results = resp.get("results").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    if results.is_empty() {
        println!("{} no matches for {:?}", "\u{2014}".dimmed(), a.question);
        return Ok(());
    }
    println!("{} results for {:?}:", "\u{2b21}".cyan(), a.question);
    for r in results {
        let label = r.get("label").and_then(|v| v.as_str()).unwrap_or("?");
        let kind = r.get("kind").and_then(|v| v.as_str()).unwrap_or("");
        let file = r.get("file").and_then(|v| v.as_str()).unwrap_or("");
        let score = r.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0);
        println!(
            "  {:<28} {:<10} {}  {}",
            label.bold(),
            kind.dimmed(),
            file.dimmed(),
            format!("{score:.1}").dimmed()
        );
    }
    Ok(())
}

async fn path(nexus: &NexusClient, a: PathArgs) -> anyhow::Result<()> {
    let body = json!({ "path": a.path, "from": a.from, "to": a.to });
    let resp = nexus.post("/api/graph/path", &body).await?;
    if resp.get("found").and_then(|v| v.as_bool()) != Some(true) {
        println!("{} no path between {:?} and {:?}", "\u{2014}".dimmed(), a.from, a.to);
        return Ok(());
    }
    let labels = resp.get("labels").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let chain: Vec<&str> = labels.iter().filter_map(|v| v.as_str()).collect();
    println!("{} {}", "\u{2b21}".cyan(), chain.join(&format!(" {} ", "\u{2192}".dimmed())));
    Ok(())
}

async fn explain(nexus: &NexusClient, a: ExplainArgs) -> anyhow::Result<()> {
    let body = json!({ "path": a.path, "node": a.node });
    let resp = nexus.post("/api/graph/explain", &body).await?;
    let label = resp.get("label").and_then(|v| v.as_str()).unwrap_or(&a.node);
    let kind = resp.get("kind").and_then(|v| v.as_str()).unwrap_or("");
    let file = resp.get("file").and_then(|v| v.as_str()).unwrap_or("");
    println!("{} {} {}", "\u{2b21}".cyan(), label.bold(), format!("({kind})").dimmed());
    if !file.is_empty() {
        let line = resp.get("line").and_then(|v| v.as_u64()).unwrap_or(0);
        println!("  {} {}:{}", "at:".dimmed(), file, line);
    }
    println!(
        "  {} {}  ({})",
        "community:".dimmed(),
        resp.get("community_label").and_then(|v| v.as_str()).unwrap_or(""),
        num(&resp, "degree"),
    );
    if let Some(neighbors) = resp.get("neighbors").and_then(|v| v.as_array()) {
        for n in neighbors.iter().take(20) {
            let nl = n.get("label").and_then(|v| v.as_str()).unwrap_or("?");
            let rel = n.get("relation").and_then(|v| v.as_str()).unwrap_or("");
            let dir = n.get("direction").and_then(|v| v.as_str()).unwrap_or("");
            let conf = n.get("confidence").and_then(|v| v.as_str()).unwrap_or("");
            let arrow = if dir == "out" { "\u{2192}" } else { "\u{2190}" };
            println!("    {} {:<14} {}  {}", arrow.dimmed(), rel.dimmed(), nl, format!("[{conf}]").dimmed());
        }
    }
    Ok(())
}

fn num(v: &Value, key: &str) -> u64 {
    v.get(key).and_then(|x| x.as_u64()).unwrap_or(0)
}
