//! Neighbourhood context for a file — the graph slice an agent needs before
//! touching it: what it declares, what it imports, what references it, and the
//! module cluster it belongs to.
//!
//! This is the "L3 context" the agent factory wants (ADR-2026-03-24-0130): instead
//! of dumping whole files, hand the agent the file's graph neighbourhood so it
//! traces consumers *before* editing — directly countering the "trace ALL consumers
//! before deleting" failure mode in CLAUDE.md. Output is capped and token-efficient.

use std::collections::HashMap;

use serde::Serialize;

use crate::model::{EdgeKind, KnowledgeGraph, NodeKind};

#[derive(Debug, Clone, Serialize)]
pub struct EntityRef {
    pub label: String,
    pub kind: String,
    #[serde(default)]
    pub file: String,
    #[serde(default)]
    pub line: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct UsedByRef {
    /// The file that references an entity defined in the target.
    pub file: String,
    /// The entity (in the target) being referenced.
    pub entity: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContextBundle {
    pub target: String,
    pub label: String,
    pub kind: String,
    pub community: usize,
    pub community_label: String,
    pub degree: usize,
    /// Entities the target file declares.
    pub defines: Vec<EntityRef>,
    /// Files the target imports (outbound).
    pub imports: Vec<String>,
    /// Files that import the target (inbound).
    pub imported_by: Vec<String>,
    /// Entities (elsewhere) the target references.
    pub uses: Vec<EntityRef>,
    /// Who references the target's own entities (inbound — the consumers).
    pub used_by: Vec<UsedByRef>,
    /// Other files in the same community.
    pub community_siblings: Vec<String>,
    /// `(actual_count - shown)` per list, so truncation is never silent.
    pub truncated: HashMap<String, usize>,
}

#[derive(Debug, Clone, Copy)]
pub struct ContextOpts {
    /// Max items shown per list.
    pub max_each: usize,
}

impl Default for ContextOpts {
    fn default() -> Self {
        Self { max_each: 25 }
    }
}

/// Build a context bundle for `target` (a file id/label, or any node — an entity
/// resolves to its file). Returns `None` if the target can't be found.
pub fn context_for(
    graph: &KnowledgeGraph,
    target: &str,
    opts: ContextOpts,
) -> Option<ContextBundle> {
    let by_id: HashMap<&str, &crate::model::Node> =
        graph.nodes.iter().map(|n| (n.id.as_str(), n)).collect();

    // Resolve the target to its File node.
    let start = graph
        .node(target)
        .or_else(|| graph.node_by_label(target))?;
    // A File node maps to itself; any entity maps to its declaring file.
    let file_rel = start.file.clone();
    let file_id = crate::model::id_for(NodeKind::File, &file_rel, &file_rel);
    let file_node = by_id.get(file_id.as_str()).copied()?;

    let label_of = |id: &str| by_id.get(id).map(|n| n.label.clone()).unwrap_or_else(|| id.to_string());

    let mut truncated: HashMap<String, usize> = HashMap::new();

    // Entities this file defines.
    let mut defines = Vec::new();
    let mut own_entity_ids = Vec::new();
    for e in &graph.edges {
        if e.kind == EdgeKind::Defines && e.src == file_id {
            if let Some(n) = by_id.get(e.dst.as_str()) {
                own_entity_ids.push(n.id.clone());
                defines.push(EntityRef {
                    label: n.label.clone(),
                    kind: n.kind.as_str().to_string(),
                    file: String::new(),
                    line: n.line,
                });
            }
        }
    }
    defines.sort_by(|a, b| a.label.cmp(&b.label));

    // Imports (out) and imported-by (in).
    let mut imports = Vec::new();
    let mut imported_by = Vec::new();
    // Uses (out references to other files' entities).
    let mut uses = Vec::new();
    // Used-by (in references to our own entities).
    let mut used_by = Vec::new();
    let own_set: std::collections::HashSet<&str> =
        own_entity_ids.iter().map(|s| s.as_str()).collect();

    for e in &graph.edges {
        match e.kind {
            EdgeKind::Imports if e.src == file_id => imports.push(label_of(&e.dst)),
            EdgeKind::Imports if e.dst == file_id => imported_by.push(label_of(&e.src)),
            EdgeKind::References if e.src == file_id => {
                if let Some(n) = by_id.get(e.dst.as_str()) {
                    uses.push(EntityRef {
                        label: n.label.clone(),
                        kind: n.kind.as_str().to_string(),
                        file: n.file.clone(),
                        line: n.line,
                    });
                }
            }
            EdgeKind::References if own_set.contains(e.dst.as_str()) => {
                used_by.push(UsedByRef {
                    file: label_of(&e.src),
                    entity: label_of(&e.dst),
                });
            }
            _ => {}
        }
    }
    imports.sort();
    imports.dedup();
    imported_by.sort();
    imported_by.dedup();
    uses.sort_by(|a, b| a.label.cmp(&b.label));
    used_by.sort_by(|a, b| (a.file.clone(), a.entity.clone()).cmp(&(b.file.clone(), b.entity.clone())));

    // Community siblings (other files in the same cluster).
    let community = file_node.community;
    let community_label = graph
        .communities
        .iter()
        .find(|c| c.id == community)
        .map(|c| c.label.clone())
        .unwrap_or_default();
    let mut community_siblings: Vec<String> = graph
        .nodes
        .iter()
        .filter(|n| {
            n.community == community && n.kind == NodeKind::File && n.id != file_id
        })
        .map(|n| n.label.clone())
        .collect();
    community_siblings.sort();

    cap(&mut defines, "defines", opts.max_each, &mut truncated);
    cap(&mut imports, "imports", opts.max_each, &mut truncated);
    cap(&mut imported_by, "imported_by", opts.max_each, &mut truncated);
    cap(&mut uses, "uses", opts.max_each, &mut truncated);
    cap(&mut used_by, "used_by", opts.max_each, &mut truncated);
    cap(&mut community_siblings, "community_siblings", opts.max_each, &mut truncated);

    Some(ContextBundle {
        target: file_id,
        label: file_node.label.clone(),
        kind: file_node.kind.as_str().to_string(),
        community,
        community_label,
        degree: file_node.degree,
        defines,
        imports,
        imported_by,
        uses,
        used_by,
        community_siblings,
        truncated,
    })
}

/// Truncate a list to `max`, recording how many were dropped so nothing is hidden.
fn cap<T>(v: &mut Vec<T>, key: &str, max: usize, trunc: &mut HashMap<String, usize>) {
    if v.len() > max {
        trunc.insert(key.to_string(), v.len() - max);
        v.truncate(max);
    }
}

/// Render a bundle as compact Markdown for injection into an agent prompt.
pub fn render_markdown(b: &ContextBundle) -> String {
    let mut out = String::new();
    let more = |key: &str| {
        b.truncated
            .get(key)
            .map(|n| format!(" (+{n} more)"))
            .unwrap_or_default()
    };

    out.push_str(&format!("## Graph context: {}\n", b.label));
    out.push_str(&format!(
        "kind={}  community=\"{}\" (#{})  degree={}\n\n",
        b.kind, b.community_label, b.community, b.degree
    ));

    if !b.defines.is_empty() {
        let items: Vec<String> = b
            .defines
            .iter()
            .map(|d| format!("{}({})", d.label, d.kind))
            .collect();
        out.push_str(&format!("Defines{}: {}\n", more("defines"), items.join(", ")));
    }
    if !b.imports.is_empty() {
        out.push_str(&format!("Imports{}: {}\n", more("imports"), b.imports.join(", ")));
    }
    if !b.imported_by.is_empty() {
        out.push_str(&format!(
            "Imported by{}: {}\n",
            more("imported_by"),
            b.imported_by.join(", ")
        ));
    }
    if !b.uses.is_empty() {
        let items: Vec<String> = b
            .uses
            .iter()
            .map(|u| format!("{} ({})", u.label, u.file))
            .collect();
        out.push_str(&format!("Uses{}: {}\n", more("uses"), items.join(", ")));
    }
    if !b.used_by.is_empty() {
        let items: Vec<String> = b
            .used_by
            .iter()
            .map(|u| format!("{} \u{2192} {}", u.file, u.entity))
            .collect();
        out.push_str(&format!(
            "Used by (consumers){}: {}\n",
            more("used_by"),
            items.join(", ")
        ));
    }
    if !b.community_siblings.is_empty() {
        out.push_str(&format!(
            "Community siblings{}: {}\n",
            more("community_siblings"),
            b.community_siblings.join(", ")
        ));
    }
    out
}
