//! Degree computation and community detection.
//!
//! We use synchronous **label propagation** over the undirected projection of the
//! graph: dependency-free, near-linear, and deterministic when node visit order
//! and tie-breaks are fixed. This stands in for Leiden clustering —
//! good enough to surface module-like clusters; swap for a Leiden crate later if
//! cluster quality demands it.

use std::collections::HashMap;

use crate::model::{Community, KnowledgeGraph};

/// Fill each node's `degree` from incident edges (undirected count).
pub fn compute_degrees(graph: &mut KnowledgeGraph) {
    let mut deg: HashMap<&str, usize> = HashMap::new();
    for e in &graph.edges {
        *deg.entry(e.src.as_str()).or_insert(0) += 1;
        *deg.entry(e.dst.as_str()).or_insert(0) += 1;
    }
    let snapshot: HashMap<String, usize> =
        deg.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
    for n in &mut graph.nodes {
        n.degree = snapshot.get(&n.id).copied().unwrap_or(0);
    }
}

/// Detect communities via label propagation, assign `node.community`, and populate
/// `graph.communities`. Deterministic: nodes are processed in index order and ties
/// break toward the lowest current label.
pub fn detect_communities(graph: &mut KnowledgeGraph, max_iters: usize) {
    let n = graph.nodes.len();
    if n == 0 {
        return;
    }

    // Index nodes and build an undirected adjacency list by node index.
    let index: HashMap<&str, usize> = graph
        .nodes
        .iter()
        .enumerate()
        .map(|(i, node)| (node.id.as_str(), i))
        .collect();

    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for e in &graph.edges {
        if let (Some(&a), Some(&b)) = (index.get(e.src.as_str()), index.get(e.dst.as_str())) {
            if a != b {
                adj[a].push(b);
                adj[b].push(a);
            }
        }
    }

    // Hub suppression: weight each node's vote by 1/√degree so a few very
    // high-degree nodes (big modules, barrel files) can't drag whole subgraphs
    // into a single label — the failure mode that collapses plain label
    // propagation into one giant community. Deterministic (degree is fixed).
    let weight: Vec<f32> = graph
        .nodes
        .iter()
        .map(|node| 1.0 / ((node.degree.max(1)) as f32).sqrt())
        .collect();

    // Each node starts in its own community.
    let mut labels: Vec<usize> = (0..n).collect();
    const EPS: f32 = 1e-6;

    for _ in 0..max_iters {
        let mut changed = false;
        for v in 0..n {
            if adj[v].is_empty() {
                continue;
            }
            // Tally neighbour labels by summed (hub-suppressed) vote weight.
            let mut counts: HashMap<usize, f32> = HashMap::new();
            for &u in &adj[v] {
                *counts.entry(labels[u]).or_insert(0.0) += weight[u];
            }
            // Pick the highest-weighted label; break ties toward the smallest
            // label so the result is independent of HashMap iteration order.
            let mut best_label = labels[v];
            let mut best_count = 0.0f32;
            for (&lab, &cnt) in &counts {
                if cnt > best_count + EPS || ((cnt - best_count).abs() <= EPS && lab < best_label) {
                    best_label = lab;
                    best_count = cnt;
                }
            }
            if labels[v] != best_label {
                labels[v] = best_label;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    // Compact labels into 0..k and group members.
    let mut canonical: HashMap<usize, usize> = HashMap::new();
    let mut members: Vec<Vec<usize>> = Vec::new();
    for (i, &lab) in labels.iter().enumerate() {
        let cid = *canonical.entry(lab).or_insert_with(|| {
            members.push(Vec::new());
            members.len() - 1
        });
        members[cid].push(i);
        graph.nodes[i].community = cid;
    }

    // Build community records, labelled by their highest-degree member.
    graph.communities = members
        .into_iter()
        .enumerate()
        .map(|(cid, idxs)| {
            let label = idxs
                .iter()
                .max_by_key(|&&i| graph.nodes[i].degree)
                .map(|&i| graph.nodes[i].label.clone())
                .unwrap_or_default();
            Community {
                id: cid,
                label,
                members: idxs.into_iter().map(|i| graph.nodes[i].id.clone()).collect(),
            }
        })
        .collect();
}

/// Labels of the top-`k` highest-degree nodes ("god nodes").
pub fn god_nodes(graph: &KnowledgeGraph, k: usize) -> Vec<String> {
    let mut idx: Vec<usize> = (0..graph.nodes.len()).collect();
    idx.sort_by(|&a, &b| {
        graph.nodes[b]
            .degree
            .cmp(&graph.nodes[a].degree)
            .then_with(|| graph.nodes[a].label.cmp(&graph.nodes[b].label))
    });
    idx.into_iter()
        .take(k)
        .map(|i| graph.nodes[i].label.clone())
        .collect()
}
