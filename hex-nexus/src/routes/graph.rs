//! Knowledge-graph routes — build/query/path/explain over the `hex-graph` engine.
//!
//! `build` runs the engine over a project directory (nexus can touch the FS and call
//! inference; the WASM module can't), writes `graphify-out/graph.json` as the query
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
const OUT_DIR: &str = "graphify-out";
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
