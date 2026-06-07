//! Core knowledge-graph value types.
//!
//! A `KnowledgeGraph` is a set of typed nodes (files, code entities, doc concepts)
//! and typed edges between them, plus detected communities and build metadata.
//! Everything here is pure data — serde-serializable, network-free, deterministic —
//! so the engine can be unit-tested without a daemon or an LLM.

use serde::{Deserialize, Serialize};

/// What a node represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    /// A source or documentation file.
    File,
    Function,
    Struct,
    Class,
    Interface,
    Type,
    Enum,
    Const,
    Trait,
    /// A concept lifted from documentation (a Markdown heading, etc.).
    DocConcept,
}

impl NodeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            NodeKind::File => "file",
            NodeKind::Function => "function",
            NodeKind::Struct => "struct",
            NodeKind::Class => "class",
            NodeKind::Interface => "interface",
            NodeKind::Type => "type",
            NodeKind::Enum => "enum",
            NodeKind::Const => "const",
            NodeKind::Trait => "trait",
            NodeKind::DocConcept => "doc_concept",
        }
    }
}

/// What a relationship represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    /// File → entity it declares.
    Defines,
    /// File → file it imports.
    Imports,
    /// File/entity → entity it references by name.
    References,
    /// Caller → callee (reserved for future call-graph work).
    Calls,
    /// Doc concept → entity it mentions.
    Mentions,
    /// LLM-inferred association between two concepts.
    Related,
}

impl EdgeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            EdgeKind::Defines => "defines",
            EdgeKind::Imports => "imports",
            EdgeKind::References => "references",
            EdgeKind::Calls => "calls",
            EdgeKind::Mentions => "mentions",
            EdgeKind::Related => "related",
        }
    }
}

/// Confidence tag: `EXTRACTED` / `INFERRED` / `AMBIGUOUS`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    /// Found directly in the source (AST edges).
    Extracted,
    /// Derived by an LLM or heuristic with reasonable certainty.
    Inferred,
    /// Derived but uncertain.
    Ambiguous,
}

impl Confidence {
    pub fn as_str(self) -> &'static str {
        match self {
            Confidence::Extracted => "extracted",
            Confidence::Inferred => "inferred",
            Confidence::Ambiguous => "ambiguous",
        }
    }
}

/// A graph node. `id` is stable and deterministic (see `id_for`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    pub kind: NodeKind,
    /// Human-readable label (entity name or relative file path).
    pub label: String,
    /// Project-relative file the node lives in (empty for none).
    #[serde(default)]
    pub file: String,
    /// 1-based source line (0 when not applicable).
    #[serde(default)]
    pub line: usize,
    /// Number of incident edges (filled in by `community::compute_degrees`).
    #[serde(default)]
    pub degree: usize,
    /// Community id this node was assigned (filled in by community detection).
    #[serde(default)]
    pub community: usize,
}

/// A typed, weighted, directed edge.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    pub id: String,
    pub src: String,
    pub dst: String,
    pub kind: EdgeKind,
    pub confidence: Confidence,
    #[serde(default = "default_weight")]
    pub weight: f32,
}

fn default_weight() -> f32 {
    1.0
}

/// A detected cluster of related nodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Community {
    pub id: usize,
    /// Best-effort label (the highest-degree member's label).
    pub label: String,
    pub members: Vec<String>,
}

/// Build metadata.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GraphMeta {
    pub mode: String,
    pub node_count: usize,
    pub edge_count: usize,
    pub community_count: usize,
    /// Highest-degree node labels.
    pub god_nodes: Vec<String>,
}

/// The full knowledge graph.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KnowledgeGraph {
    pub project_id: String,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub communities: Vec<Community>,
    pub meta: GraphMeta,
}

impl KnowledgeGraph {
    pub fn to_json(&self) -> serde_json::Result<String> {
        serde_json::to_string_pretty(self)
    }

    pub fn from_json(s: &str) -> serde_json::Result<Self> {
        serde_json::from_str(s)
    }

    /// Find a node by exact id.
    pub fn node(&self, id: &str) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id)
    }

    /// Find a node by exact (case-insensitive) label. On ties, prefer a concrete
    /// code/File node over a `DocConcept` (doc headings often quote file paths or
    /// symbol names, which would otherwise shadow the real node).
    pub fn node_by_label(&self, label: &str) -> Option<&Node> {
        let mut doc_fallback: Option<&Node> = None;
        for n in &self.nodes {
            if n.label.eq_ignore_ascii_case(label) {
                if n.kind == NodeKind::DocConcept {
                    doc_fallback.get_or_insert(n);
                } else {
                    return Some(n);
                }
            }
        }
        doc_fallback
    }
}

/// Deterministic, collision-resistant id for a node.
pub fn id_for(kind: NodeKind, file: &str, label: &str) -> String {
    match kind {
        NodeKind::File => format!("file:{file}"),
        _ => format!("{}:{}:{}", kind.as_str(), file, label),
    }
}

/// Deterministic id for an edge.
pub fn edge_id(kind: EdgeKind, src: &str, dst: &str) -> String {
    format!("{}:{}->{}", kind.as_str(), src, dst)
}
// End of file marker
