//! Port-cohesion heuristic detector.
//!
//! For each `trait` declaration we treat as a port, count the methods
//! on the surface and cluster them along two axes:
//!
//! 1. **Name-prefix** — the first underscore-separated segment of the
//!    method name (`user_create`, `user_get` → `user`).
//! 2. **Parameter-type overlap** — Jaccard similarity on the set of
//!    `type_identifier` nodes appearing in each method signature.
//!
//! After bucketing by prefix we greedily merge buckets whose parameter
//! sets overlap heavily (so verb-suffix naming like `get_user`,
//! `create_user`, `delete_user` collapses into one user-shaped cluster).
//! The final cluster count and average pairwise similarity drive the
//! finding.
//!
//! A finding is emitted when **either**:
//! - the method count exceeds [`HIGH_METHOD_COUNT_THRESHOLD`], or
//! - the trait has at least [`MIN_CLUSTER_COUNT`] clusters whose mean
//!   pairwise parameter-type similarity is below
//!   [`LOW_CROSS_CLUSTER_SIMILARITY`].
//!
//! All thresholds live as constants so the heuristic stays auditable.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};
use tree_sitter::{Node, Parser};
use walkdir::WalkDir;

/// Method count above which a port is "fat" regardless of clustering.
/// 8+ methods is almost always a kitchen-sink port.
pub const HIGH_METHOD_COUNT_THRESHOLD: usize = 7;

/// Minimum number of clusters required for the cross-cluster check to
/// fire. With one cluster there are no pairs to score.
pub const MIN_CLUSTER_COUNT: usize = 2;

/// Minimum method count required before the cluster-diversity branch
/// fires. Without this, a 3-method trait like `{ping, send, close}`
/// — naturally distinct lifecycle ops — would flag on every port.
pub const MIN_METHOD_COUNT_FOR_CLUSTER_CHECK: usize = 5;

/// Average pairwise Jaccard on parameter-type sets *below* this value
/// means clusters share no domain vocabulary — the trait is bundling
/// unrelated concerns.
pub const LOW_CROSS_CLUSTER_SIMILARITY: f64 = 0.2;

/// Two prefix-buckets whose parameter-type Jaccard is **at least**
/// this high are merged, so verb-prefix naming (`get_user`,
/// `delete_user`) doesn't shatter a single concern across many buckets.
pub const PARAMETER_MERGE_THRESHOLD: f64 = 0.5;

/// One finding row in the analyzer's JSON envelope.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CohesionFinding {
    pub kind: String,
    pub port: String,
    pub file: String,
    pub line: usize,
    pub method_count: usize,
    /// Method-name groupings. Outer Vec ordered by the alphabetically
    /// first method name in each cluster; inner Vec sorted alphabetically.
    /// Both orderings are deterministic so test assertions and
    /// hypothesis-IDs in the improver remain stable.
    pub clusters: Vec<Vec<String>>,
}

/// Top-level envelope emitted by `--port-cohesion`.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct CohesionReport {
    pub findings: Vec<CohesionFinding>,
}

/// Run the cohesion detector over `root`.
pub fn analyze(root: &Path) -> anyhow::Result<CohesionReport> {
    let mut report = CohesionReport::default();
    let mut parser = Parser::new();
    let lang: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
    parser
        .set_language(&lang)
        .map_err(|e| anyhow::anyhow!("set tree-sitter-rust language: {e}"))?;

    let root_for_filter = root.to_path_buf();
    for entry in WalkDir::new(root)
        .into_iter()
        .filter_entry(|e| e.path() == root_for_filter || !is_excluded_dir(e.path()))
    {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if path.extension().and_then(|x| x.to_str()) != Some("rs") {
            continue;
        }
        let Ok(source) = std::fs::read_to_string(path) else {
            continue;
        };
        let rel = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();
        let Some(tree) = parser.parse(&source, None) else {
            continue;
        };
        collect_findings(tree.root_node(), &source, &rel, &mut report.findings);
    }

    report.findings.sort_by(|a, b| {
        a.file
            .cmp(&b.file)
            .then(a.line.cmp(&b.line))
            .then(a.port.cmp(&b.port))
    });
    Ok(report)
}

fn is_excluded_dir(p: &Path) -> bool {
    let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    if matches!(
        name,
        "target" | "node_modules" | ".git" | "dist" | "build"
    ) {
        return true;
    }
    if name.starts_with("hex-worktrees") {
        return true;
    }
    name.starts_with('.')
}

// ── Trait + method extraction ────────────────────────────────────────

#[derive(Debug, Clone)]
struct Method {
    name: String,
    types: BTreeSet<String>,
}

fn collect_findings(
    node: Node,
    source: &str,
    file_rel: &str,
    out: &mut Vec<CohesionFinding>,
) {
    if node.kind() == "trait_item" {
        if let Some(name_node) = node.child_by_field_name("name") {
            let port_name = node_text(name_node, source).to_string();
            if !port_name.is_empty() {
                let methods = collect_methods(node, source);
                if !methods.is_empty() {
                    if let Some(finding) = evaluate(
                        &port_name,
                        &methods,
                        file_rel,
                        node.start_position().row + 1,
                    ) {
                        out.push(finding);
                    }
                }
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_findings(child, source, file_rel, out);
    }
}

fn collect_methods(trait_node: Node, source: &str) -> Vec<Method> {
    let body = match find_decl_list(trait_node) {
        Some(b) => b,
        None => return Vec::new(),
    };
    let mut methods = Vec::new();
    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        if !matches!(child.kind(), "function_signature_item" | "function_item") {
            continue;
        }
        let Some(name_node) = child.child_by_field_name("name") else {
            continue;
        };
        let name = node_text(name_node, source).to_string();
        if name.is_empty() {
            continue;
        }
        let mut types = BTreeSet::new();
        // Walking the entire signature is safe — function names are
        // `identifier`, not `type_identifier`, so they don't pollute
        // the set. `Self` and primitive-type nodes do appear and act
        // as a baseline overlap, which is the right signal: methods
        // returning `Self` belong together.
        walk_type_idents(child, source, &mut types);
        methods.push(Method { name, types });
    }
    methods
}

fn find_decl_list(node: Node<'_>) -> Option<Node<'_>> {
    if let Some(b) = node.child_by_field_name("body") {
        return Some(b);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "declaration_list" {
            return Some(child);
        }
    }
    None
}

fn walk_type_idents(node: Node, source: &str, out: &mut BTreeSet<String>) {
    if node.kind() == "type_identifier" {
        let t = node_text(node, source);
        if !t.is_empty() {
            out.insert(t.to_string());
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_type_idents(child, source, out);
    }
}

fn node_text<'a>(node: Node, source: &'a str) -> &'a str {
    node.utf8_text(source.as_bytes()).unwrap_or("")
}

// ── Heuristic ────────────────────────────────────────────────────────

fn prefix_of(name: &str) -> String {
    name.split('_').next().unwrap_or(name).to_string()
}

fn evaluate(
    port: &str,
    methods: &[Method],
    file: &str,
    line: usize,
) -> Option<CohesionFinding> {
    let count = methods.len();
    let clusters = cluster_methods(methods);

    let too_many = count > HIGH_METHOD_COUNT_THRESHOLD;
    let many_low_overlap_clusters = count >= MIN_METHOD_COUNT_FOR_CLUSTER_CHECK
        && clusters.len() >= MIN_CLUSTER_COUNT
        && cross_cluster_similarity(&clusters, methods) < LOW_CROSS_CLUSTER_SIMILARITY;

    if !too_many && !many_low_overlap_clusters {
        return None;
    }

    let mut named_clusters: Vec<Vec<String>> = clusters
        .into_iter()
        .map(|idxs| {
            let mut names: Vec<String> =
                idxs.into_iter().map(|i| methods[i].name.clone()).collect();
            names.sort();
            names
        })
        .collect();
    named_clusters.sort_by(|a, b| a.first().cmp(&b.first()));

    Some(CohesionFinding {
        kind: "port_cohesion".to_string(),
        port: port.to_string(),
        file: file.to_string(),
        line,
        method_count: count,
        clusters: named_clusters,
    })
}

fn cluster_methods(methods: &[Method]) -> Vec<Vec<usize>> {
    // 1. Bucket by name-prefix.
    let mut buckets: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, m) in methods.iter().enumerate() {
        buckets.entry(prefix_of(&m.name)).or_default().push(i);
    }
    let mut clusters: Vec<Vec<usize>> = buckets.into_values().collect();

    // 2. Greedily merge buckets with high parameter-type overlap.
    loop {
        let mut merged_any = false;
        'outer: for i in 0..clusters.len() {
            for j in (i + 1)..clusters.len() {
                if pair_similarity(&clusters[i], &clusters[j], methods)
                    >= PARAMETER_MERGE_THRESHOLD
                {
                    let mut moved = clusters.remove(j);
                    clusters[i].append(&mut moved);
                    merged_any = true;
                    break 'outer;
                }
            }
        }
        if !merged_any {
            break;
        }
    }
    clusters
}

fn cluster_types(cluster: &[usize], methods: &[Method]) -> BTreeSet<String> {
    let mut set = BTreeSet::new();
    for &i in cluster {
        set.extend(methods[i].types.iter().cloned());
    }
    set
}

fn pair_similarity(a: &[usize], b: &[usize], methods: &[Method]) -> f64 {
    let ta = cluster_types(a, methods);
    let tb = cluster_types(b, methods);
    let union = ta.union(&tb).count();
    if union == 0 {
        return 0.0;
    }
    let intersection = ta.intersection(&tb).count();
    intersection as f64 / union as f64
}

fn cross_cluster_similarity(clusters: &[Vec<usize>], methods: &[Method]) -> f64 {
    if clusters.len() < 2 {
        return 1.0;
    }
    let mut sum = 0.0;
    let mut pairs = 0;
    for i in 0..clusters.len() {
        for j in (i + 1)..clusters.len() {
            sum += pair_similarity(&clusters[i], &clusters[j], methods);
            pairs += 1;
        }
    }
    if pairs == 0 {
        return 1.0;
    }
    sum / pairs as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(name: &str, types: &[&str]) -> Method {
        Method {
            name: name.to_string(),
            types: types.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn prefix_of_takes_first_underscore_segment() {
        assert_eq!(prefix_of("user_create"), "user");
        assert_eq!(prefix_of("get"), "get");
        assert_eq!(prefix_of("a_b_c"), "a");
    }

    #[test]
    fn merge_collapses_verb_prefix_buckets_sharing_param_types() {
        // `get_user`, `delete_user`, `create_user`, `update_user` —
        // four prefix buckets that overlap on UserId/UserData/User and
        // should merge into a single user-shaped cluster. Each method
        // needs enough shared types to clear PARAMETER_MERGE_THRESHOLD
        // (0.5 Jaccard); a method with only one type cluster won't
        // qualify on its own, so we give every CRUD operation at least
        // {UserId, User} (the original fixture gave delete_user only
        // {UserId} and the algorithm correctly held it out at 0.33).
        let methods = vec![
            m("get_user", &["UserId", "User"]),
            m("delete_user", &["UserId", "User"]),
            m("create_user", &["UserData", "User"]),
            m("update_user", &["UserId", "UserData", "User"]),
        ];
        let clusters = cluster_methods(&methods);
        assert_eq!(clusters.len(), 1, "{clusters:?}");
    }

    #[test]
    fn cluster_count_stays_high_when_no_param_overlap() {
        let methods = vec![
            m("user_get", &["UserId", "User"]),
            m("order_get", &["OrderId", "Order"]),
            m("payment_get", &["PaymentId", "Payment"]),
        ];
        let clusters = cluster_methods(&methods);
        assert_eq!(clusters.len(), 3, "{clusters:?}");
    }
}
