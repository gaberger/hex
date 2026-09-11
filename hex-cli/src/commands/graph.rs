//! `hex graph` — build and query a knowledge graph of the project.
//!
//! In-process over the `hex-graph` engine (ADR-2608241500 P6.2). Every verb
//! here used to POST to the daemon's `/api/graph/*` routes, which then called
//! the same library functions this file now calls directly. `consumers` was
//! already standalone — and that asymmetry was the tell: the excision oracle
//! read `graph-out/graph.json` off disk, while `build` could not run at all
//! without a daemon, so the file the oracle depends on could go stale with no
//! way to refresh it.
//!
//! Core verbs: build / query / path / explain / context / consumers.
//!
//! # What was dropped
//!
//! `--persist`, which mirrored the graph into the `knowledge-graph`
//! SpacetimeDB module. `graph-out/graph.json` was always the query source of
//! truth; the mirror served the dashboard, which is going with the daemon.

use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};
use colored::Colorize;
use serde_json::json;

use hex_graph::model::KnowledgeGraph;
use hex_graph::query as gquery;
use hex_graph::semantic::{
    NoopSemanticExtractor, SemanticContext, SemanticExtractor, SemanticTriple,
};
use hex_graph::{BuildOpts, Mode};

const OUT_DIR: &str = "graph-out";
const OUT_FILE: &str = "graph.json";

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
    /// Reports inbound importers + entity consumers, with a SAFE-TO-REMOVE /
    /// BLOCKED verdict. This is the graph-driven dead-code check that drives
    /// safe excision — `hex` doing "trace ALL consumers before deleting"
    /// itself, deterministically.
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
    /// `hex-exec/src/tools/delegate.rs`, or a node id/label).
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
    match action {
        GraphAction::Build(a) => build(a).await,
        GraphAction::Query(a) => query(a),
        GraphAction::Path(a) => path(a),
        GraphAction::Explain(a) => explain(a),
        GraphAction::Context(a) => context(a).await,
        GraphAction::Consumers(a) => consumers(a),
    }
}

// ── root and graph io ────────────────────────────────────────────────────────

/// Resolve an argument `path` (absolute or cwd-relative), or detect the root.
fn resolve_root(path: &str) -> anyhow::Result<PathBuf> {
    if !path.is_empty() && path != "." {
        let pb = PathBuf::from(path);
        if pb.is_dir() {
            return Ok(pb);
        }
        let joined = std::env::current_dir()?.join(path);
        if joined.is_dir() {
            return Ok(joined);
        }
        anyhow::bail!("not a directory: {path}");
    }
    if let Ok(root) = std::env::var("HEX_PROJECT_ROOT") {
        let p = PathBuf::from(&root);
        if p.is_dir() {
            return Ok(p);
        }
    }
    let cwd = std::env::current_dir()?;
    let mut dir = cwd.as_path();
    loop {
        if dir.join("CLAUDE.md").exists() || dir.join(".git").exists() {
            return Ok(dir.to_path_buf());
        }
        match dir.parent() {
            Some(p) => dir = p,
            None => return Ok(cwd),
        }
    }
}

fn graph_path(root: &Path) -> PathBuf {
    root.join(OUT_DIR).join(OUT_FILE)
}

fn load_graph(path: &str) -> anyhow::Result<KnowledgeGraph> {
    let root = resolve_root(path)?;
    let out_path = graph_path(&root);
    let raw = std::fs::read_to_string(&out_path).map_err(|e| {
        anyhow::anyhow!(
            "no graph at {} ({e}). Build it first: `hex graph build`",
            out_path.display()
        )
    })?;
    KnowledgeGraph::from_json(&raw)
        .map_err(|e| anyhow::anyhow!("corrupt {}: {e}", out_path.display()))
}

fn write_graph(out_path: &Path, graph: &KnowledgeGraph) -> anyhow::Result<()> {
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = graph.to_json().map_err(|e| anyhow::anyhow!("serialize graph: {e}"))?;
    std::fs::write(out_path, json)?;
    Ok(())
}

// ── deep-mode semantic inference ─────────────────────────────────────────────

/// Mines relationship triples out of prose with one inference call per chunk.
///
/// Lifted from the daemon's `NexusSemanticExtractor`, which held an
/// `Arc<dyn IInferencePort>` off `AppState`. It now goes through
/// `hex_exec::infer`, which resolves the provider from configuration — so no
/// provider name appears here (founding goal G1).
struct LocalSemanticExtractor {
    model: Option<String>,
}

#[async_trait::async_trait]
impl SemanticExtractor for LocalSemanticExtractor {
    async fn infer_edges(&self, ctx: &SemanticContext) -> Vec<SemanticTriple> {
        // Trim very large prose to keep the prompt bounded.
        let prose: String = ctx.text.chars().take(6000).collect();
        let known = ctx.known_labels.join(", ");
        let prompt = format!(
            "Extract concept relationships from the documentation below. \
             Return ONLY a JSON array of objects with keys: source, target, relation, \
             confident (boolean). Prefer linking to these known entities when \
             relevant: {known}.\n\nDOC ({}):\n{prose}",
            ctx.file
        );
        let body = json!({
            "model": self.model,
            "max_tokens": 1024,
            "system": "You are a precise knowledge-graph relationship extractor. Output JSON only.",
            "messages": [{ "role": "user", "content": prompt }],
        });
        // Degrade gracefully — the AST graph still stands without these edges.
        let Ok(resp) = hex_exec::infer::complete_json("graph-build", "infer doc edges", body).await
        else {
            return Vec::new();
        };
        parse_triples(resp.get("content").and_then(|v| v.as_str()).unwrap_or(""))
    }
}

/// Leniently parse a JSON array of triples from a model response.
fn parse_triples(text: &str) -> Vec<SemanticTriple> {
    let (start, end) = match (text.find('['), text.rfind(']')) {
        (Some(s), Some(e)) if e > s => (s, e),
        _ => return Vec::new(),
    };
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&text[start..=end]) else {
        return Vec::new();
    };
    let Some(arr) = parsed.as_array() else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|o| {
            let source = o.get("source")?.as_str()?.trim().to_string();
            let target = o.get("target")?.as_str()?.trim().to_string();
            if source.is_empty() || target.is_empty() {
                return None;
            }
            Some(SemanticTriple {
                source,
                target,
                relation: o
                    .get("relation")
                    .and_then(|v| v.as_str())
                    .unwrap_or("related")
                    .to_string(),
                confident: o.get("confident").and_then(|v| v.as_bool()).unwrap_or(false),
            })
        })
        .collect()
}

// ── verbs ────────────────────────────────────────────────────────────────────

async fn build(a: BuildArgs) -> anyhow::Result<()> {
    let root = resolve_root(&a.path)?;
    let project_id = root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "project".to_string());
    let mode = Mode::from_str(&a.mode);

    let semantic: Box<dyn SemanticExtractor> = if mode == Mode::Deep {
        Box::new(LocalSemanticExtractor { model: a.model.clone() })
    } else {
        Box::new(NoopSemanticExtractor)
    };

    let opts = BuildOpts {
        project_id,
        mode,
        include_docs: !a.no_docs,
        ..Default::default()
    };

    println!("{} building knowledge graph ({})…", "\u{2b21}".cyan(), a.mode);
    let graph = hex_graph::build(&root, opts, semantic.as_ref()).await?;

    let out_path = graph_path(&root);
    write_graph(&out_path, &graph)?;

    println!(
        "{} {} nodes, {} edges, {} communities  [{}]",
        "\u{2713}".green(),
        graph.meta.node_count,
        graph.meta.edge_count,
        graph.meta.community_count,
        graph.meta.mode,
    );
    println!("  {} {}", "graph:".dimmed(), out_path.display());
    let hubs: Vec<&str> = graph.meta.god_nodes.iter().map(String::as_str).take(5).collect();
    if !hubs.is_empty() {
        println!("  {} {}", "hubs:".dimmed(), hubs.join(", "));
    }
    Ok(())
}

fn query(a: QueryArgs) -> anyhow::Result<()> {
    let graph = load_graph(&a.path)?;
    let results = gquery::query(&graph, &a.question, a.limit);
    if results.is_empty() {
        println!("{} no matches for {:?}", "\u{2014}".dimmed(), a.question);
        return Ok(());
    }
    println!("{} results for {:?}:", "\u{2b21}".cyan(), a.question);
    for r in &results {
        println!(
            "  {:<28} {:<10} {}  {}",
            r.label.bold(),
            r.kind.dimmed(),
            r.file.dimmed(),
            format!("{:.1}", r.score).dimmed()
        );
    }
    Ok(())
}

fn path(a: PathArgs) -> anyhow::Result<()> {
    let graph = load_graph(&a.path)?;
    match gquery::shortest_path(&graph, &a.from, &a.to) {
        Some(ids) => {
            let labels: Vec<String> = ids
                .iter()
                .map(|id| graph.node(id).map(|n| n.label.clone()).unwrap_or_else(|| id.clone()))
                .collect();
            println!(
                "{} {}",
                "\u{2b21}".cyan(),
                labels.join(&format!(" {} ", "\u{2192}".dimmed()))
            );
        }
        None => println!("{} no path between {:?} and {:?}", "\u{2014}".dimmed(), a.from, a.to),
    }
    Ok(())
}

fn explain(a: ExplainArgs) -> anyhow::Result<()> {
    let graph = load_graph(&a.path)?;
    let Some(ex) = gquery::explain(&graph, &a.node) else {
        anyhow::bail!("node not found: {}", a.node);
    };
    println!("{} {} {}", "\u{2b21}".cyan(), ex.label.bold(), format!("({})", ex.kind).dimmed());
    if !ex.file.is_empty() {
        println!("  {} {}:{}", "at:".dimmed(), ex.file, ex.line);
    }
    println!("  {} {}  ({})", "community:".dimmed(), ex.community_label, ex.degree);
    for n in ex.neighbors.iter().take(20) {
        let arrow = if n.direction == "out" { "\u{2192}" } else { "\u{2190}" };
        println!(
            "    {} {:<14} {}  {}",
            arrow.dimmed(),
            n.relation.dimmed(),
            n.label,
            format!("[{}]", n.confidence).dimmed()
        );
    }
    Ok(())
}

async fn context(a: ContextArgs) -> anyhow::Result<()> {
    let graph = load_graph(&a.path)?;
    let opts = hex_graph::context::ContextOpts { max_each: a.max_each.clamp(1, 200) };
    let Some(bundle) = hex_graph::context::context_for(&graph, &a.target, opts) else {
        anyhow::bail!("no file node for target: {}", a.target);
    };

    let mut markdown = hex_graph::context::render_markdown(&bundle);
    // Graph-relevant memory: lessons whose text mentions this file's
    // neighbourhood (path/symbols), ranked — not arbitrary recency.
    let lessons = hex_exec::direct_exec::fetch_lessons().await;
    let ranked = hex_graph::context::rank_lessons(&bundle, &lessons, 6);
    if !ranked.is_empty() {
        markdown.push_str("\n## Lessons (most relevant to this file)\n");
        for l in &ranked {
            markdown.push_str(&format!("- [{}] {}\n", l.key, l.value));
        }
    }

    if a.json {
        let mut value = serde_json::to_value(&bundle)?;
        if let Some(obj) = value.as_object_mut() {
            obj.insert("markdown".to_string(), json!(markdown));
            obj.insert("lessons".to_string(), serde_json::to_value(&ranked)?);
        }
        println!("{}", serde_json::to_string_pretty(&value)?);
        return Ok(());
    }
    print!("{markdown}");
    Ok(())
}

/// Standalone graph-driven consumer trace + delete-safety verdict. Reuses the
/// same `hex_graph::context` engine the executor uses, so "trace ALL consumers
/// before deleting" (ADR-2026-04-05-0900) is a deterministic hex verb instead
/// of a manual grep.
fn consumers(a: ConsumersArgs) -> anyhow::Result<()> {
    let graph = load_graph(&a.path)?;
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
    let users: Vec<String> =
        b.used_by.iter().map(|u| format!("{} (uses {})", u.file, u.entity)).collect();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triples_parse_out_of_a_fenced_reply() {
        let reply = "Sure!\n```json\n[{\"source\":\"A\",\"target\":\"B\",\
                     \"relation\":\"depends on\",\"confident\":true}]\n```";
        let got = parse_triples(reply);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].source, "A");
        assert_eq!(got[0].relation, "depends on");
        assert!(got[0].confident);
    }

    #[test]
    fn a_reply_with_no_array_yields_nothing() {
        assert!(parse_triples("I could not find any relationships.").is_empty());
        assert!(parse_triples("").is_empty());
        assert!(parse_triples("[not json]").is_empty());
    }

    #[test]
    fn triples_missing_a_side_are_dropped() {
        let got = parse_triples(r#"[{"source":"A"},{"source":"A","target":"  "},
                                    {"source":"A","target":"B"}]"#);
        assert_eq!(got.len(), 1, "only the complete triple survives");
        assert_eq!(got[0].relation, "related", "relation defaults");
        assert!(!got[0].confident, "confidence defaults to uncertain");
    }

    #[test]
    fn an_explicit_directory_argument_wins_over_detection() {
        let dir = tempfile::tempdir().unwrap();
        let got = resolve_root(&dir.path().to_string_lossy()).unwrap();
        assert_eq!(got, dir.path());
    }

    #[test]
    fn a_missing_graph_names_the_file_and_the_fix() {
        let dir = tempfile::tempdir().unwrap();
        let err = load_graph(&dir.path().to_string_lossy()).unwrap_err().to_string();
        assert!(err.contains("graph.json"), "{err}");
        assert!(err.contains("hex graph build"), "{err}");
    }
}
