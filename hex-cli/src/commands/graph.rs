//! `hex graph` — build and query a knowledge graph of the project.
//!
//! Thin client over the nexus `/api/graph/*` endpoints (which run the `hex-graph`
//! engine). Core verbs: build / query / path / explain.

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
    /// Emit a file's graph neighbourhood as agent-ready context (defines, uses,
    /// consumers, community) — trace consumers before you edit.
    Context(ContextArgs),
    /// Who depends on a module/file — the excision-safety oracle (ADR-2606071340).
    ///
    /// Loads the graph FILE directly (no nexus needed) and reports inbound
    /// importers + entity consumers, with a SAFE-TO-REMOVE / BLOCKED verdict.
    /// This is the graph-driven dead-code check that drives safe excision —
    /// `hex` doing "trace ALL consumers before deleting" itself, deterministically.
    Consumers(ConsumersArgs),
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

#[derive(Debug, Args)]
pub struct ContextArgs {
    /// File (or any node) to build neighbourhood context for.
    pub target: String,
    #[arg(long, default_value = ".")]
    pub path: String,
    /// Max items per list.
    #[arg(long, default_value_t = 25)]
    pub max_each: usize,
    /// Emit the raw JSON bundle instead of rendered Markdown.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct ConsumersArgs {
    /// Module or file to check (a repo-relative path like
    /// `hex-nexus/src/orchestration/foo.rs`, or a node id/label).
    pub target: String,
    /// Project directory holding `graph-out/graph.json` (default: detected root).
    #[arg(long, default_value = ".")]
    pub path: String,
    /// Max consumers listed per category.
    #[arg(long, default_value_t = 50)]
    pub max_each: usize,
    /// Emit JSON `{ target, safe_to_remove, imported_by, used_by }`.
    #[arg(long)]
    pub json: bool,
}

pub async fn run(action: GraphAction) -> anyhow::Result<()> {
    // `consumers` is STANDALONE — it reads the graph file directly so the
    // dead-code/excision check works even when nexus is down (ADR-2606071340).
    if let GraphAction::Consumers(a) = action {
        return consumers(a);
    }
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;
    match action {
        GraphAction::Build(a) => build(&nexus, a).await,
        GraphAction::Query(a) => query(&nexus, a).await,
        GraphAction::Path(a) => path(&nexus, a).await,
        GraphAction::Explain(a) => explain(&nexus, a).await,
        GraphAction::Context(a) => context(&nexus, a).await,
        GraphAction::Consumers(_) => unreachable!("handled above"),
    }
}

/// Standalone graph-driven consumer trace + delete-safety verdict. Reuses the
/// same `hex_graph::context` engine the executor uses, so "trace ALL consumers
/// before deleting" (ADR-2026-04-05-0900) becomes a deterministic hex verb
/// instead of a manual grep — runnable with no daemon.
fn consumers(a: ConsumersArgs) -> anyhow::Result<()> {
    let graph_path = std::path::Path::new(&a.path)
        .join("graph-out")
        .join("graph.json");
    let raw = std::fs::read_to_string(&graph_path).map_err(|e| {
        anyhow::anyhow!(
            "no graph at {} ({e}). Build it first: `hex graph build`",
            graph_path.display()
        )
    })?;
    let graph = hex_graph::model::KnowledgeGraph::from_json(&raw)
        .map_err(|e| anyhow::anyhow!("parse {}: {e}", graph_path.display()))?;

    let bundle = hex_graph::context::context_for(
        &graph,
        &a.target,
        hex_graph::context::ContextOpts { max_each: a.max_each },
    );
    let Some(b) = bundle else {
        anyhow::bail!(
            "'{}' not found in the graph — check the path, or rebuild with `hex graph build`",
            a.target
        );
    };

    let importers: Vec<String> = b.imported_by.clone();
    let users: Vec<String> = b
        .used_by
        .iter()
        .map(|u| format!("{} (uses {})", u.file, u.entity))
        .collect();
    let safe = importers.is_empty() && users.is_empty();

    if a.json {
        println!(
            "{}",
            json!({
                "target": b.label,
                "safe_to_remove": safe,
                "imported_by": importers,
                "used_by": users,
            })
        );
        return Ok(());
    }

    println!("{} {}", "⬡ consumers of".cyan().bold(), b.label.bold());
    println!("  {} {}  ·  degree {}", "kind".dimmed(), b.kind, b.degree);
    if safe {
        println!(
            "\n  {} no inbound importers or entity consumers in the graph.",
            "SAFE TO REMOVE —".green().bold()
        );
        println!(
            "  {}",
            "Confirm with `cargo check --workspace` after cutting any wiring.".dimmed()
        );
    } else {
        println!("\n  {} {} consumer(s):", "BLOCKED —".red().bold(), importers.len() + users.len());
        if !importers.is_empty() {
            println!("  {} ({})", "imported by".yellow(), importers.len());
            for f in &importers {
                println!("    • {}", f);
            }
        }
        if !users.is_empty() {
            println!("  {} ({})", "entities used by".yellow(), users.len());
            for u in &users {
                println!("    • {}", u);
            }
        }
        println!(
            "\n  {}",
            "Sever these references (or the spawn/route wiring) before deleting.".dimmed()
        );
    }
    Ok(())
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

async fn context(nexus: &NexusClient, a: ContextArgs) -> anyhow::Result<()> {
    let body = json!({ "path": a.path, "target": a.target, "max_each": a.max_each });
    let resp = nexus.post("/api/graph/context", &body).await?;
    if a.json {
        println!("{}", serde_json::to_string_pretty(&resp)?);
        return Ok(());
    }
    // Print the engine-rendered Markdown block (agent-ready).
    match resp.get("markdown").and_then(|v| v.as_str()) {
        Some(md) => print!("{md}"),
        None => println!("{}", serde_json::to_string_pretty(&resp)?),
    }
    Ok(())
}

fn num(v: &Value, key: &str) -> u64 {
    v.get(key).and_then(|x| x.as_u64()).unwrap_or(0)
}
