//! hex-graph — Rust-native knowledge-graph engine (graphify-style core).
//!
//! Builds a typed knowledge graph from a project's source + docs: AST entities and
//! imports become `Extracted` nodes/edges; an injected [`SemanticExtractor`] can add
//! `Inferred`/`Ambiguous` relationships from prose. Communities are detected via
//! label propagation. Querying (`query`/`shortest_path`/`explain`) lives in [`query`].
//!
//! The engine is network-free and deterministic — the LLM, the daemon, and the
//! filesystem walk policy are the only impure inputs, all explicit.

pub mod community;
pub mod extract;
pub mod model;
pub mod query;
pub mod semantic;

use std::collections::{HashMap, HashSet};
use std::path::Path;

use walkdir::WalkDir;

use extract::code::{self, Language};
use extract::markdown;
use model::{edge_id, id_for, Confidence, Edge, EdgeKind, KnowledgeGraph, Node, NodeKind};
use semantic::{SemanticContext, SemanticExtractor};

/// Build mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// AST + docs only — no LLM calls (deterministic, free).
    Ast,
    /// AST + docs + LLM-inferred semantic edges from prose.
    Deep,
}

impl Mode {
    pub fn from_str(s: &str) -> Self {
        match s {
            "deep" => Mode::Deep,
            _ => Mode::Ast,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Ast => "ast",
            Mode::Deep => "deep",
        }
    }
}

/// Build options.
#[derive(Debug, Clone)]
pub struct BuildOpts {
    pub project_id: String,
    pub mode: Mode,
    /// Include Markdown/text docs as nodes.
    pub include_docs: bool,
    /// Max file size to read, in bytes (skip larger).
    pub max_file_bytes: u64,
}

impl Default for BuildOpts {
    fn default() -> Self {
        Self {
            project_id: String::new(),
            mode: Mode::Ast,
            include_docs: true,
            max_file_bytes: 2 * 1024 * 1024,
        }
    }
}

/// Max distinct entities an ambiguous imported name may link to before we drop it
/// as too noisy (prevents common names from forming false hubs).
const MAX_AMBIGUOUS_REFS: usize = 3;

const EXCLUDE_DIRS: &[&str] = &[
    "node_modules",
    "target",
    "dist",
    "build",
    "vendor",
    ".git",
    ".hex",
    "graphify-out",
    ".next",
    "__pycache__",
];

/// Build a knowledge graph rooted at `root`.
pub async fn build(
    root: &Path,
    opts: BuildOpts,
    semantic: &dyn SemanticExtractor,
) -> anyhow::Result<KnowledgeGraph> {
    let mut graph = KnowledgeGraph {
        project_id: opts.project_id.clone(),
        ..Default::default()
    };

    // Dedup sets so repeated ids never produce duplicate nodes/edges.
    let mut node_ids: HashSet<String> = HashSet::new();
    let mut edge_ids: HashSet<String> = HashSet::new();
    // label (lowercased) → entity node ids, for cross-file reference edges.
    let mut label_index: HashMap<String, Vec<String>> = HashMap::new();
    // entity node id → its file, to resolve a reference to the right definition.
    let mut id_file: HashMap<String, String> = HashMap::new();
    // set of known relative file paths, for import resolution.
    let mut file_set: HashSet<String> = HashSet::new();
    // collected (file_rel, raw_path, names) imports to resolve after all files seen.
    let mut pending_imports: Vec<(String, String, Vec<String>)> = Vec::new();
    // prose chunks for the deep semantic pass: (file_rel, prose).
    let mut prose_chunks: Vec<(String, String)> = Vec::new();

    let push_node = |graph: &mut KnowledgeGraph,
                     seen: &mut HashSet<String>,
                     node: Node| {
        if seen.insert(node.id.clone()) {
            graph.nodes.push(node);
        }
    };

    for entry in WalkDir::new(root)
        .sort_by_file_name() // deterministic traversal, independent of filesystem order
        .into_iter()
        .filter_entry(|e| !is_excluded(e))
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let rel = rel_path(root, path);
        let rel_lossy = rel.clone();

        // Size guard.
        if let Ok(meta) = entry.metadata() {
            if meta.len() > opts.max_file_bytes {
                continue;
            }
        }

        let is_code = Language::from_path(&rel_lossy).is_some();
        let is_doc = opts.include_docs && markdown::is_doc_file(&rel_lossy);
        if !is_code && !is_doc {
            continue;
        }

        let source = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(_) => continue, // binary / unreadable — skip
        };

        // File node.
        let file_id = id_for(NodeKind::File, &rel_lossy, &rel_lossy);
        file_set.insert(rel_lossy.clone());
        push_node(
            &mut graph,
            &mut node_ids,
            Node {
                id: file_id.clone(),
                kind: NodeKind::File,
                label: rel_lossy.clone(),
                file: rel_lossy.clone(),
                line: 0,
                degree: 0,
                community: 0,
            },
        );

        if let Some(lang) = Language::from_path(&rel_lossy) {
            let fx = code::extract_file(&source, lang);
            for ent in fx.entities {
                let eid = id_for(ent.kind, &rel_lossy, &ent.name);
                push_node(
                    &mut graph,
                    &mut node_ids,
                    Node {
                        id: eid.clone(),
                        kind: ent.kind,
                        label: ent.name.clone(),
                        file: rel_lossy.clone(),
                        line: ent.line,
                        degree: 0,
                        community: 0,
                    },
                );
                add_edge(
                    &mut graph,
                    &mut edge_ids,
                    EdgeKind::Defines,
                    &file_id,
                    &eid,
                    Confidence::Extracted,
                );
                id_file.insert(eid.clone(), rel_lossy.clone());
                label_index
                    .entry(ent.name.to_ascii_lowercase())
                    .or_default()
                    .push(eid);
            }
            for imp in fx.imports {
                pending_imports.push((rel_lossy.clone(), imp.raw_path, imp.names));
            }
        }

        if is_doc {
            let dx = markdown::extract_doc(&source);
            for h in dx.headings {
                let cid = id_for(NodeKind::DocConcept, &rel_lossy, &h.title);
                push_node(
                    &mut graph,
                    &mut node_ids,
                    Node {
                        id: cid.clone(),
                        kind: NodeKind::DocConcept,
                        label: h.title.clone(),
                        file: rel_lossy.clone(),
                        line: h.line,
                        degree: 0,
                        community: 0,
                    },
                );
                add_edge(
                    &mut graph,
                    &mut edge_ids,
                    EdgeKind::Defines,
                    &file_id,
                    &cid,
                    Confidence::Extracted,
                );
            }
            if opts.mode == Mode::Deep && !dx.prose.trim().is_empty() {
                prose_chunks.push((rel_lossy.clone(), dx.prose));
            }
        }
    }

    // ── Resolve import + reference edges (now that all files/entities are known).
    for (from_rel, raw_path, names) in &pending_imports {
        let from_file_id = id_for(NodeKind::File, from_rel, from_rel);
        let resolved = resolve_import(from_rel, raw_path, &file_set);

        // File→file import edge when the path resolves inside the project.
        if let Some(target_rel) = &resolved {
            let to_file_id = id_for(NodeKind::File, target_rel, target_rel);
            add_edge(
                &mut graph,
                &mut edge_ids,
                EdgeKind::Imports,
                &from_file_id,
                &to_file_id,
                Confidence::Extracted,
            );
        }

        // File→entity reference edges. Precision matters: linking an imported name
        // to *every* entity that shares that label repo-wide turns common type names
        // ("State", "Config") into false hubs that collapse community detection.
        // So we prefer the definition in the resolved import file, then a unique
        // repo-wide match, and only tag a small ambiguous fan-out — never a large one.
        for name in names {
            if name == "*" || name == "default" {
                continue;
            }
            let Some(targets) = label_index.get(&name.to_ascii_lowercase()) else {
                continue;
            };
            // Entities not defined in the importing file itself.
            let candidates: Vec<&String> = targets
                .iter()
                .filter(|tid| {
                    id_file.get(tid.as_str()).map(|f| f.as_str()) != Some(from_rel.as_str())
                })
                .collect();
            if candidates.is_empty() {
                continue;
            }

            // 1. Precise — the resolved import file defines this symbol.
            if let Some(target_rel) = &resolved {
                if let Some(tid) = candidates.iter().find(|tid| {
                    id_file.get(tid.as_str()).map(|f| f.as_str()) == Some(target_rel.as_str())
                }) {
                    add_edge(
                        &mut graph,
                        &mut edge_ids,
                        EdgeKind::References,
                        &from_file_id,
                        tid,
                        Confidence::Extracted,
                    );
                    continue;
                }
            }

            // 2. Unambiguous — exactly one entity repo-wide carries this label.
            if candidates.len() == 1 {
                add_edge(
                    &mut graph,
                    &mut edge_ids,
                    EdgeKind::References,
                    &from_file_id,
                    candidates[0],
                    Confidence::Extracted,
                );
                continue;
            }

            // 3. Mildly ambiguous — link a few, tagged Ambiguous. Names matching
            //    many entities are dropped as too noisy to be meaningful.
            if candidates.len() <= MAX_AMBIGUOUS_REFS {
                for tid in candidates {
                    add_edge(
                        &mut graph,
                        &mut edge_ids,
                        EdgeKind::References,
                        &from_file_id,
                        tid,
                        Confidence::Ambiguous,
                    );
                }
            }
        }
    }

    // ── Deep mode: LLM-inferred semantic edges from doc prose.
    if opts.mode == Mode::Deep {
        let known_labels: Vec<String> =
            graph.nodes.iter().map(|n| n.label.clone()).take(400).collect();
        for (file_rel, prose) in &prose_chunks {
            let ctx = SemanticContext {
                text: prose.clone(),
                known_labels: known_labels.clone(),
                file: file_rel.clone(),
            };
            for tri in semantic.infer_edges(&ctx).await {
                let src_id = ensure_concept(&mut graph, &mut node_ids, &label_index, &tri.source);
                let dst_id = ensure_concept(&mut graph, &mut node_ids, &label_index, &tri.target);
                if src_id == dst_id {
                    continue;
                }
                let conf = if tri.confident {
                    Confidence::Inferred
                } else {
                    Confidence::Ambiguous
                };
                add_edge(
                    &mut graph,
                    &mut edge_ids,
                    EdgeKind::Related,
                    &src_id,
                    &dst_id,
                    conf,
                );
            }
        }
    }

    // ── Degrees, communities, metadata.
    community::compute_degrees(&mut graph);
    community::detect_communities(&mut graph, 20);
    let gods = community::god_nodes(&graph, 10);
    graph.meta = model::GraphMeta {
        mode: opts.mode.as_str().to_string(),
        node_count: graph.nodes.len(),
        edge_count: graph.edges.len(),
        community_count: graph.communities.len(),
        god_nodes: gods,
    };

    Ok(graph)
}

// ── helpers ──────────────────────────────────────────────

fn add_edge(
    graph: &mut KnowledgeGraph,
    seen: &mut HashSet<String>,
    kind: EdgeKind,
    src: &str,
    dst: &str,
    conf: Confidence,
) {
    let id = edge_id(kind, src, dst);
    if seen.insert(id.clone()) {
        graph.edges.push(Edge {
            id,
            src: src.to_string(),
            dst: dst.to_string(),
            kind,
            confidence: conf,
            weight: 1.0,
        });
    }
}

/// Resolve a concept label to an existing node id, or create a `DocConcept` node.
fn ensure_concept(
    graph: &mut KnowledgeGraph,
    seen: &mut HashSet<String>,
    label_index: &HashMap<String, Vec<String>>,
    label: &str,
) -> String {
    let key = label.to_ascii_lowercase();
    if let Some(ids) = label_index.get(&key) {
        if let Some(first) = ids.first() {
            return first.clone();
        }
    }
    if let Some(existing) = graph.node_by_label(label) {
        return existing.id.clone();
    }
    let id = id_for(NodeKind::DocConcept, "", label);
    if seen.insert(id.clone()) {
        graph.nodes.push(Node {
            id: id.clone(),
            kind: NodeKind::DocConcept,
            label: label.to_string(),
            file: String::new(),
            line: 0,
            degree: 0,
            community: 0,
        });
    }
    id
}

fn is_excluded(entry: &walkdir::DirEntry) -> bool {
    if entry.file_type().is_dir() {
        if let Some(name) = entry.file_name().to_str() {
            return EXCLUDE_DIRS.contains(&name);
        }
    }
    false
}

/// Project-relative, forward-slashed path.
fn rel_path(root: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    rel.to_string_lossy().replace('\\', "/")
}

/// Resolve an import path to a known project-relative file, if possible.
/// Handles TypeScript relative imports and Rust `self::mod` declarations.
fn resolve_import(from_rel: &str, raw: &str, files: &HashSet<String>) -> Option<String> {
    if raw.starts_with("./") || raw.starts_with("../") {
        return resolve_relative_ts(from_rel, raw, files);
    }
    if let Some(modname) = raw.strip_prefix("self::") {
        return resolve_rust_mod(from_rel, modname, files);
    }
    None
}

fn resolve_relative_ts(from_rel: &str, raw: &str, files: &HashSet<String>) -> Option<String> {
    let base_dir = parent_dir(from_rel);
    // Strip a NodeNext `.js`/`.jsx` suffix; the on-disk file is `.ts`/`.tsx`.
    let raw_noext = raw
        .strip_suffix(".js")
        .or_else(|| raw.strip_suffix(".jsx"))
        .unwrap_or(raw);
    let joined = normalize_join(&base_dir, raw_noext);

    let candidates = [
        joined.clone(),
        format!("{joined}.ts"),
        format!("{joined}.tsx"),
        format!("{joined}.js"),
        format!("{joined}.jsx"),
        format!("{joined}/index.ts"),
        format!("{joined}/index.tsx"),
        format!("{joined}/index.js"),
    ];
    candidates.into_iter().find(|c| files.contains(c))
}

fn resolve_rust_mod(from_rel: &str, modname: &str, files: &HashSet<String>) -> Option<String> {
    let parent = parent_dir(from_rel);
    let stem = file_stem(from_rel);
    // `mod.rs`/`lib.rs`/`main.rs` host modules as siblings; otherwise modules live
    // in a subdirectory named after the file stem.
    let base = if stem == "mod" || stem == "lib" || stem == "main" {
        parent
    } else if parent.is_empty() {
        stem.to_string()
    } else {
        format!("{parent}/{stem}")
    };
    let candidates = [
        format!("{base}/{modname}.rs"),
        format!("{base}/{modname}/mod.rs"),
    ];
    candidates.into_iter().find(|c| files.contains(c))
}

fn parent_dir(rel: &str) -> String {
    match rel.rfind('/') {
        Some(i) => rel[..i].to_string(),
        None => String::new(),
    }
}

fn file_stem(rel: &str) -> &str {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    name.rsplit_once('.').map(|(s, _)| s).unwrap_or(name)
}

/// Lexically join `base` with a relative `rel` (handling `.` and `..`).
fn normalize_join(base: &str, rel: &str) -> String {
    let mut parts: Vec<&str> = if base.is_empty() {
        Vec::new()
    } else {
        base.split('/').collect()
    };
    for seg in rel.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    parts.join("/")
}
