//! Querying a built graph: lexical search, shortest path, and explain.
//!
//! Embedding-free by design (the engine has no model): `query` ranks nodes by
//! token overlap between the question and each node's label plus its immediate
//! neighbourhood, lightly weighted by degree. `shortest_path` is BFS over the
//! undirected projection. `explain` returns a node with its neighbours/community.

use std::collections::{HashMap, HashSet, VecDeque};

use serde::Serialize;

use crate::model::KnowledgeGraph;

/// A node returned from a lexical query.
#[derive(Debug, Clone, Serialize)]
pub struct RankedNode {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub file: String,
    pub score: f32,
}

/// Rank nodes by lexical relevance to `question`. Returns up to `limit` results.
pub fn query(graph: &KnowledgeGraph, question: &str, limit: usize) -> Vec<RankedNode> {
    let terms = tokenize(question);
    if terms.is_empty() {
        return Vec::new();
    }
    let term_set: HashSet<&str> = terms.iter().map(|s| s.as_str()).collect();

    // Precompute neighbour labels per node for context matching.
    let neighbours = neighbour_labels(graph);

    let mut scored: Vec<RankedNode> = graph
        .nodes
        .iter()
        .filter_map(|n| {
            let label_tokens = tokenize(&n.label);
            let mut score = 0.0f32;
            for t in &label_tokens {
                if term_set.contains(t.as_str()) {
                    score += 3.0; // direct label hit weighted highest
                }
            }
            // Context: neighbour-label hits contribute less.
            if let Some(ctx) = neighbours.get(n.id.as_str()) {
                for t in ctx {
                    if term_set.contains(t.as_str()) {
                        score += 0.5;
                    }
                }
            }
            if score <= 0.0 {
                return None;
            }
            // Mild degree boost so hub nodes float up among equal matches.
            score += (n.degree as f32).ln_1p() * 0.25;
            Some(RankedNode {
                id: n.id.clone(),
                label: n.label.clone(),
                kind: n.kind.as_str().to_string(),
                file: n.file.clone(),
                score,
            })
        })
        .collect();

    scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.label.cmp(&b.label))
    });
    scored.truncate(limit);
    scored
}

/// Shortest path (BFS, undirected) between two nodes, returned as node ids.
/// `from`/`to` may be node ids or exact labels.
pub fn shortest_path(graph: &KnowledgeGraph, from: &str, to: &str) -> Option<Vec<String>> {
    let start = resolve(graph, from)?;
    let goal = resolve(graph, to)?;
    if start == goal {
        return Some(vec![start]);
    }

    let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();
    for e in &graph.edges {
        adj.entry(e.src.as_str()).or_default().push(e.dst.as_str());
        adj.entry(e.dst.as_str()).or_default().push(e.src.as_str());
    }

    let mut prev: HashMap<&str, &str> = HashMap::new();
    let mut seen: HashSet<&str> = HashSet::new();
    let mut q: VecDeque<&str> = VecDeque::new();
    seen.insert(start.as_str());
    q.push_back(start.as_str());

    while let Some(cur) = q.pop_front() {
        if cur == goal {
            // Reconstruct.
            let mut path = vec![cur.to_string()];
            let mut node = cur;
            while let Some(&p) = prev.get(node) {
                path.push(p.to_string());
                node = p;
            }
            path.reverse();
            return Some(path);
        }
        if let Some(neighbors) = adj.get(cur) {
            for &nb in neighbors {
                if seen.insert(nb) {
                    prev.insert(nb, cur);
                    q.push_back(nb);
                }
            }
        }
    }
    None
}

/// Explanation of a single node.
#[derive(Debug, Clone, Serialize)]
pub struct Explanation {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub file: String,
    pub line: usize,
    pub degree: usize,
    pub community: usize,
    pub community_label: String,
    pub neighbors: Vec<NeighborRef>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NeighborRef {
    pub label: String,
    pub relation: String,
    pub confidence: String,
    pub direction: &'static str,
}

/// Build an explanation for `target` (node id or exact label).
pub fn explain(graph: &KnowledgeGraph, target: &str) -> Option<Explanation> {
    let id = resolve(graph, target)?;
    let node = graph.node(&id)?;
    let label_of = |nid: &str| {
        graph
            .node(nid)
            .map(|n| n.label.clone())
            .unwrap_or_else(|| nid.to_string())
    };

    let mut neighbors = Vec::new();
    for e in &graph.edges {
        if e.src == id {
            neighbors.push(NeighborRef {
                label: label_of(&e.dst),
                relation: e.kind.as_str().to_string(),
                confidence: e.confidence.as_str().to_string(),
                direction: "out",
            });
        } else if e.dst == id {
            neighbors.push(NeighborRef {
                label: label_of(&e.src),
                relation: e.kind.as_str().to_string(),
                confidence: e.confidence.as_str().to_string(),
                direction: "in",
            });
        }
    }

    let community_label = graph
        .communities
        .iter()
        .find(|c| c.id == node.community)
        .map(|c| c.label.clone())
        .unwrap_or_default();

    Some(Explanation {
        id: node.id.clone(),
        label: node.label.clone(),
        kind: node.kind.as_str().to_string(),
        file: node.file.clone(),
        line: node.line,
        degree: node.degree,
        community: node.community,
        community_label,
        neighbors,
    })
}

// ── helpers ──────────────────────────────────────────────

/// Resolve a query string to a node id: exact id, else exact (case-insensitive) label.
fn resolve(graph: &KnowledgeGraph, s: &str) -> Option<String> {
    if graph.node(s).is_some() {
        return Some(s.to_string());
    }
    graph.node_by_label(s).map(|n| n.id.clone())
}

fn neighbour_labels(graph: &KnowledgeGraph) -> HashMap<&str, Vec<String>> {
    let mut map: HashMap<&str, Vec<String>> = HashMap::new();
    let label_of = |id: &str| graph.node(id).map(|n| n.label.clone());
    for e in &graph.edges {
        if let Some(l) = label_of(&e.dst) {
            map.entry(e.src.as_str())
                .or_default()
                .extend(tokenize(&l));
        }
        if let Some(l) = label_of(&e.src) {
            map.entry(e.dst.as_str())
                .or_default()
                .extend(tokenize(&l));
        }
    }
    map
}

/// Split into lowercase alphanumeric tokens, also splitting camelCase / snake_case.
fn tokenize(s: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut cur = String::new();
    let mut prev_lower = false;
    for ch in s.chars() {
        if ch.is_alphanumeric() {
            // Split camelCase boundaries: lower→Upper.
            if ch.is_uppercase() && prev_lower && !cur.is_empty() {
                tokens.push(std::mem::take(&mut cur));
            }
            cur.push(ch.to_ascii_lowercase());
            prev_lower = ch.is_lowercase();
        } else {
            if !cur.is_empty() {
                tokens.push(std::mem::take(&mut cur));
            }
            prev_lower = false;
        }
    }
    if !cur.is_empty() {
        tokens.push(cur);
    }
    tokens.into_iter().filter(|t| t.len() >= 2).collect()
}
