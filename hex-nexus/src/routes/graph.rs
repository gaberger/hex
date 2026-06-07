//! Knowledge-graph routes — build/query/path/explain over the `hex-graph` engine.
//!
//! `build` runs the engine over a project directory (nexus can touch the FS and call
//! inference; the WASM module can't), writes `graph-out/graph.json` as the query
//! source of truth, and best-effort mirrors the result into the `knowledge-graph`
//! SpacetimeDB module so it survives restarts and feeds dashboard subscriptions.
//! `query`/`path`/`explain` load `graph.json` and run the engine's read APIs.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::{extract::State, Json};
use http::StatusCode;
use serde::Deserialize;
use serde_json::json;

use async_trait::async_trait;
use hex_core::domain::messages::{ContentBlock, Message};
use hex_core::ports::inference::{IInferencePort, InferenceRequest, Priority};
use hex_graph::model::KnowledgeGraph;
use hex_graph::semantic::{SemanticContext, SemanticExtractor, SemanticTriple};
use hex_graph::{query as gquery, BuildOpts, Mode};

use crate::state::SharedState;

const GRAPH_DB: &str = "knowledge-graph";
const OUT_DIR: &str = "graph-out";
const OUT_FILE: &str = "graph.json";

// ── request bodies ───────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct BuildRequest {
    /// Project directory to analyze. Defaults to the detected project root.
    #[serde(default)]
    pub path: Option<String>,
    /// "ast" (default) or "deep" (adds LLM-inferred edges from docs).
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default = "default_true")]
    pub include_docs: bool,
    /// Mirror the graph into the knowledge-graph STDB module.
    #[serde(default)]
    pub persist: bool,
    /// Model for deep-mode semantic inference (optional).
    #[serde(default)]
    pub model: Option<String>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize)]
pub struct QueryRequest {
    #[serde(default)]
    pub path: Option<String>,
    pub question: String,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_limit() -> usize {
    15
}

#[derive(Debug, Deserialize)]
pub struct PathRequest {
    #[serde(default)]
    pub path: Option<String>,
    pub from: String,
    pub to: String,
}

#[derive(Debug, Deserialize)]
pub struct ExplainRequest {
    #[serde(default)]
    pub path: Option<String>,
    pub node: String,
}

#[derive(Debug, Deserialize)]
pub struct ContextRequest {
    #[serde(default)]
    pub path: Option<String>,
    /// File (or any node) to build neighbourhood context for.
    pub target: String,
    /// Max items per list in the bundle.
    #[serde(default = "default_max_each")]
    pub max_each: usize,
}

fn default_max_each() -> usize {
    25
}

// ── handlers ─────────────────────────────────────────────

/// POST /api/graph/build
pub async fn build_graph(
    State(state): State<SharedState>,
    Json(body): Json<BuildRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let root = match resolve_root(&body.path) {
        Some(r) => r,
        None => {
            return err(
                StatusCode::BAD_REQUEST,
                "could not resolve project root (pass `path` or set HEX_PROJECT_ROOT)",
            )
        }
    };
    let project_id = project_id_for(&root);
    let mode = Mode::from_str(body.mode.as_deref().unwrap_or("ast"));

    // Deep mode borrows the inference port; ast mode infers nothing.
    let semantic: Box<dyn SemanticExtractor> = if mode == Mode::Deep {
        match &state.inference_port {
            Some(port) => Box::new(NexusSemanticExtractor {
                inference: port.clone(),
                model: body.model.clone().unwrap_or_default(),
            }),
            None => Box::new(hex_graph::semantic::NoopSemanticExtractor),
        }
    } else {
        Box::new(hex_graph::semantic::NoopSemanticExtractor)
    };

    let opts = BuildOpts {
        project_id: project_id.clone(),
        mode,
        include_docs: body.include_docs,
        ..Default::default()
    };

    let graph = match hex_graph::build(&root, opts, semantic.as_ref()).await {
        Ok(g) => g,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, &format!("build failed: {e}")),
    };

    // Write graph.json (query source of truth).
    let out_path = root.join(OUT_DIR).join(OUT_FILE);
    if let Err(e) = write_graph(&out_path, &graph) {
        return err(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("failed writing {}: {e}", out_path.display()),
        );
    }

    // Best-effort STDB mirror.
    let mut persisted = false;
    let mut persist_error: Option<String> = None;
    if body.persist {
        if let Some(sp) = state.state_port.as_ref() {
            match persist_to_stdb(sp.as_ref(), &project_id, &graph).await {
                Ok(()) => persisted = true,
                Err(e) => persist_error = Some(e),
            }
        } else {
            persist_error = Some("state port not configured".to_string());
        }
    }

    (
        StatusCode::OK,
        Json(json!({
            "project_id": project_id,
            "root": root.to_string_lossy(),
            "out_file": out_path.to_string_lossy(),
            "mode": graph.meta.mode,
            "node_count": graph.meta.node_count,
            "edge_count": graph.meta.edge_count,
            "community_count": graph.meta.community_count,
            "god_nodes": graph.meta.god_nodes,
            "persisted": persisted,
            "persist_error": persist_error,
        })),
    )
}

/// POST /api/graph/query
pub async fn query_graph(
    Json(body): Json<QueryRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let graph = match load_graph(&body.path) {
        Ok(g) => g,
        Err((code, msg)) => return err(code, &msg),
    };
    let hits = gquery::query(&graph, &body.question, body.limit);
    (StatusCode::OK, Json(json!({ "results": hits })))
}

/// POST /api/graph/path
pub async fn path_graph(
    Json(body): Json<PathRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let graph = match load_graph(&body.path) {
        Ok(g) => g,
        Err((code, msg)) => return err(code, &msg),
    };
    match gquery::shortest_path(&graph, &body.from, &body.to) {
        Some(ids) => {
            let labels: Vec<String> = ids
                .iter()
                .map(|id| graph.node(id).map(|n| n.label.clone()).unwrap_or_else(|| id.clone()))
                .collect();
            (
                StatusCode::OK,
                Json(json!({ "found": true, "path": ids, "labels": labels })),
            )
        }
        None => (StatusCode::OK, Json(json!({ "found": false }))),
    }
}

/// POST /api/graph/explain
pub async fn explain_graph(
    Json(body): Json<ExplainRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let graph = match load_graph(&body.path) {
        Ok(g) => g,
        Err((code, msg)) => return err(code, &msg),
    };
    match gquery::explain(&graph, &body.node) {
        Some(ex) => (StatusCode::OK, Json(serde_json::to_value(ex).unwrap_or(json!({})))),
        None => err(StatusCode::NOT_FOUND, &format!("node not found: {}", body.node)),
    }
}

/// POST /api/graph/context
pub async fn context_graph(
    Json(body): Json<ContextRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let graph = match load_graph(&body.path) {
        Ok(g) => g,
        Err((code, msg)) => return err(code, &msg),
    };
    let opts = hex_graph::context::ContextOpts {
        max_each: body.max_each.clamp(1, 200),
    };
    match hex_graph::context::context_for(&graph, &body.target, opts) {
        Some(bundle) => {
            let mut markdown = hex_graph::context::render_markdown(&bundle);
            // Graph-relevant memory: lessons whose text mentions this file's
            // neighbourhood (path/symbols), ranked — not arbitrary recency.
            let lessons = crate::direct_exec::fetch_lessons().await;
            let ranked = hex_graph::context::rank_lessons(&bundle, &lessons, 6);
            if !ranked.is_empty() {
                markdown.push_str("\n## Lessons (most relevant to this file)\n");
                for l in &ranked {
                    markdown.push_str(&format!("- [{}] {}\n", l.key, l.value));
                }
            }
            let mut value = serde_json::to_value(&bundle).unwrap_or(json!({}));
            if let Some(obj) = value.as_object_mut() {
                obj.insert("markdown".to_string(), json!(markdown));
                obj.insert("lessons".to_string(), serde_json::to_value(&ranked).unwrap_or(json!([])));
            }
            (StatusCode::OK, Json(value))
        }
        None => err(
            StatusCode::NOT_FOUND,
            &format!("no file node for target: {}", body.target),
        ),
    }
}

#[derive(Debug, Deserialize)]
pub struct SummaryRequest {
    #[serde(default)]
    pub path: Option<String>,
}

/// POST /api/graph/summary — lightweight stats for the dashboard: counts, god
/// nodes, and the largest communities. 404 when no graph has been built.
pub async fn summary_graph(
    Json(body): Json<SummaryRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let graph = match load_graph(&body.path) {
        Ok(g) => g,
        Err((code, msg)) => return err(code, &msg),
    };
    // Largest communities (top 12 by member count).
    let mut comms: Vec<&hex_graph::model::Community> = graph.communities.iter().collect();
    comms.sort_by(|a, b| b.members.len().cmp(&a.members.len()));
    let top: Vec<serde_json::Value> = comms
        .iter()
        .take(12)
        .map(|c| json!({ "id": c.id, "label": c.label, "size": c.members.len() }))
        .collect();
    // Node-kind breakdown.
    let mut kinds: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for n in &graph.nodes {
        *kinds.entry(n.kind.as_str().to_string()).or_insert(0) += 1;
    }
    (
        StatusCode::OK,
        Json(json!({
            "mode": graph.meta.mode,
            "node_count": graph.meta.node_count,
            "edge_count": graph.meta.edge_count,
            "community_count": graph.meta.community_count,
            "god_nodes": graph.meta.god_nodes,
            "communities": top,
            "kinds": kinds,
        })),
    )
}

// ── semantic extractor (deep mode) ───────────────────────

struct NexusSemanticExtractor {
    inference: Arc<dyn IInferencePort>,
    model: String,
}

#[async_trait]
impl SemanticExtractor for NexusSemanticExtractor {
    async fn infer_edges(&self, ctx: &SemanticContext) -> Vec<SemanticTriple> {
        // Trim very large prose to keep the prompt bounded.
        let prose: String = ctx.text.chars().take(6000).collect();
        let known = ctx.known_labels.join(", ");
        let prompt = format!(
            "Extract concept relationships from the documentation below. \
Return ONLY a JSON array of objects with keys: source, target, relation, confident (boolean). \
Prefer linking to these known entities when relevant: {known}.\n\nDOC ({}):\n{prose}",
            ctx.file
        );
        let req = InferenceRequest {
            model: self.model.clone(),
            system_prompt: "You are a precise knowledge-graph relationship extractor. Output JSON only.".to_string(),
            messages: vec![Message::user(&prompt)],
            tools: vec![],
            max_tokens: 1024,
            temperature: 0.0,
            thinking_budget: None,
            cache_control: false,
            priority: Priority::Low,
            grammar: None,
        };
        let resp = match self.inference.complete(req).await {
            Ok(r) => r,
            Err(_) => return Vec::new(), // degrade gracefully — AST graph still stands
        };
        let text: String = resp
            .content
            .iter()
            .filter_map(|cb| match cb {
                ContentBlock::Text { text } => Some(text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");
        parse_triples(&text)
    }
}

/// Leniently parse a JSON array of triples from an LLM response.
fn parse_triples(text: &str) -> Vec<SemanticTriple> {
    let (start, end) = match (text.find('['), text.rfind(']')) {
        (Some(s), Some(e)) if e > s => (s, e),
        _ => return Vec::new(),
    };
    let slice = &text[start..=end];
    let parsed: serde_json::Value = match serde_json::from_str(slice) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
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
            let relation = o
                .get("relation")
                .and_then(|v| v.as_str())
                .unwrap_or("related")
                .to_string();
            let confident = o.get("confident").and_then(|v| v.as_bool()).unwrap_or(false);
            Some(SemanticTriple {
                source,
                target,
                relation,
                confident,
            })
        })
        .collect()
}

// ── persistence + io helpers ─────────────────────────────

async fn persist_to_stdb(
    sp: &dyn crate::ports::state::IStatePort,
    project_id: &str,
    graph: &KnowledgeGraph,
) -> Result<(), String> {
    sp.graph_reducer(GRAPH_DB, "clear_graph", json!({ "project_id": project_id }))
        .await
        .map_err(|e| format!("clear_graph: {e}"))?;

    for n in &graph.nodes {
        sp.graph_reducer(
            GRAPH_DB,
            "upsert_node",
            json!({
                "project_id": project_id,
                "node_id": n.id,
                "kind": n.kind.as_str(),
                "label": n.label,
                "file": n.file,
                "line": n.line as u32,
                "degree": n.degree as u32,
                "community": n.community as u32,
            }),
        )
        .await
        .map_err(|e| format!("upsert_node: {e}"))?;
    }
    for e in &graph.edges {
        sp.graph_reducer(
            GRAPH_DB,
            "upsert_edge",
            json!({
                "project_id": project_id,
                "edge_id": e.id,
                "src": e.src,
                "dst": e.dst,
                "kind": e.kind.as_str(),
                "confidence": e.confidence.as_str(),
                "weight": e.weight,
            }),
        )
        .await
        .map_err(|err| format!("upsert_edge: {err}"))?;
    }
    for c in &graph.communities {
        sp.graph_reducer(
            GRAPH_DB,
            "set_community",
            json!({
                "project_id": project_id,
                "community_id": c.id as u32,
                "label": c.label,
                "members_json": serde_json::to_string(&c.members).unwrap_or_else(|_| "[]".into()),
            }),
        )
        .await
        .map_err(|e| format!("set_community: {e}"))?;
    }
    sp.graph_reducer(
        GRAPH_DB,
        "set_meta",
        json!({
            "project_id": project_id,
            "mode": graph.meta.mode,
            "node_count": graph.meta.node_count as u32,
            "edge_count": graph.meta.edge_count as u32,
            "community_count": graph.meta.community_count as u32,
            "god_nodes_json": serde_json::to_string(&graph.meta.god_nodes).unwrap_or_else(|_| "[]".into()),
            "built_at": "",
        }),
    )
    .await
    .map_err(|e| format!("set_meta: {e}"))?;
    Ok(())
}

fn write_graph(out_path: &Path, graph: &KnowledgeGraph) -> std::io::Result<()> {
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = graph
        .to_json()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    std::fs::write(out_path, json)
}

fn load_graph(path: &Option<String>) -> Result<KnowledgeGraph, (StatusCode, String)> {
    let root = resolve_root(path)
        .ok_or((StatusCode::BAD_REQUEST, "could not resolve project root".to_string()))?;
    let out_path = root.join(OUT_DIR).join(OUT_FILE);
    let raw = std::fs::read_to_string(&out_path).map_err(|_| {
        (
            StatusCode::NOT_FOUND,
            format!(
                "no graph at {} — run `hex graph build` first",
                out_path.display()
            ),
        )
    })?;
    KnowledgeGraph::from_json(&raw)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("corrupt graph.json: {e}")))
}

/// Resolve a request `path` (absolute or cwd-relative) or fall back to the
/// detected project root.
fn resolve_root(path: &Option<String>) -> Option<PathBuf> {
    if let Some(p) = path {
        if !p.is_empty() {
            let pb = PathBuf::from(p);
            return if pb.is_dir() {
                Some(pb)
            } else {
                std::env::current_dir().ok().map(|c| c.join(p)).filter(|c| c.is_dir())
            };
        }
    }
    find_project_root()
}

fn find_project_root() -> Option<PathBuf> {
    if let Ok(root) = std::env::var("HEX_PROJECT_ROOT") {
        let p = PathBuf::from(&root);
        if p.is_dir() {
            return Some(p);
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        let mut dir = cwd.as_path();
        loop {
            if dir.join("CLAUDE.md").exists() || dir.join(".git").exists() {
                return Some(dir.to_path_buf());
            }
            match dir.parent() {
                Some(p) => dir = p,
                None => break,
            }
        }
        return Some(cwd);
    }
    None
}

fn project_id_for(root: &Path) -> String {
    root.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "project".to_string())
}

fn err(code: StatusCode, msg: &str) -> (StatusCode, Json<serde_json::Value>) {
    (code, Json(json!({ "error": msg })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn write(dir: &Path, rel: &str, contents: &str) {
        let p = dir.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(p, contents).unwrap();
    }

    /// Build a small graph and persist it to `<dir>/graph-out/graph.json`,
    /// exactly as the build handler would, so the read handlers have a source.
    async fn fixture() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        write(d, "src/a.ts", "export function alpha() { return 1; }\n");
        write(
            d,
            "src/b.ts",
            "import { alpha } from './a.js';\nexport class Beta { run() { return alpha(); } }\n",
        );
        let opts = hex_graph::BuildOpts {
            project_id: "t".into(),
            mode: hex_graph::Mode::Ast,
            include_docs: true,
            ..Default::default()
        };
        let graph = hex_graph::build(d, opts, &hex_graph::semantic::NoopSemanticExtractor)
            .await
            .unwrap();
        let out = d.join(OUT_DIR).join(OUT_FILE);
        std::fs::create_dir_all(out.parent().unwrap()).unwrap();
        std::fs::write(&out, graph.to_json().unwrap()).unwrap();
        tmp
    }

    fn root(tmp: &tempfile::TempDir) -> Option<String> {
        Some(tmp.path().to_string_lossy().to_string())
    }

    #[tokio::test]
    async fn query_returns_ranked_results() {
        let tmp = fixture().await;
        let (code, Json(body)) = query_graph(Json(QueryRequest {
            path: root(&tmp),
            question: "alpha".into(),
            limit: 10,
        }))
        .await;
        assert_eq!(code, StatusCode::OK);
        let results = body.get("results").and_then(|v| v.as_array()).unwrap();
        assert!(results.iter().any(|r| r.get("label").and_then(|v| v.as_str()) == Some("alpha")));
    }

    #[tokio::test]
    async fn path_finds_route_between_nodes() {
        let tmp = fixture().await;
        let (code, Json(body)) = path_graph(Json(PathRequest {
            path: root(&tmp),
            from: "file:src/b.ts".into(),
            to: "function:src/a.ts:alpha".into(),
        }))
        .await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(body.get("found").and_then(|v| v.as_bool()), Some(true));
    }

    #[tokio::test]
    async fn explain_returns_node_with_neighbours() {
        let tmp = fixture().await;
        let (code, Json(body)) = explain_graph(Json(ExplainRequest {
            path: root(&tmp),
            node: "Beta".into(),
        }))
        .await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(body.get("label").and_then(|v| v.as_str()), Some("Beta"));
    }

    #[tokio::test]
    async fn context_returns_bundle_and_markdown() {
        let tmp = fixture().await;
        let (code, Json(body)) = context_graph(Json(ContextRequest {
            path: root(&tmp),
            target: "src/a.ts".into(),
            max_each: 25,
        }))
        .await;
        assert_eq!(code, StatusCode::OK);
        assert!(body.get("markdown").and_then(|v| v.as_str()).unwrap().contains("src/a.ts"));
        // a.ts is used by b.ts (the consumer signal agents need).
        let used_by = body.get("used_by").and_then(|v| v.as_array()).unwrap();
        assert!(used_by.iter().any(|u| u.get("file").and_then(|v| v.as_str()) == Some("src/b.ts")));
    }

    #[tokio::test]
    async fn query_without_built_graph_is_404() {
        let tmp = tempfile::tempdir().unwrap(); // no graph-out/graph.json
        let (code, _body) = query_graph(Json(QueryRequest {
            path: root(&tmp),
            question: "anything".into(),
            limit: 5,
        }))
        .await;
        assert_eq!(code, StatusCode::NOT_FOUND);
    }
}
